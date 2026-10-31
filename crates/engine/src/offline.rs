//! The offline render of a project to a WAV file (export, issue #297): the same engine as
//! the live one, fed with the same `ControlMsg` stream, driven block by block without an
//! audio device and faster than real time.
//!
//! [`Offline`] owns both sides of the engine channel ([`crate::host::channel`]) and a
//! small state machine that the host steps from its own loop (a Worker in the page, a
//! thread in the helper, a test):
//!
//! 1. **Body**: [`Plan::body_frames`] frames, the content of the song (the host computes
//!    the length from the project). Every frame is written. The last block is cut short so
//!    that the content ends on its last frame, and the transport stops there.
//! 2. **Tail**: once the body is rendered the transport is stopped ([`Offline::set_stop`],
//!    which cuts a loop region or what plays on past the end) and the render goes on while
//!    the effects ring out. A frame past the body is written only up to the last one
//!    louder than [`SILENCE_DB`]: the output ends where the sound ends. The render stops
//!    after [`SILENCE_SECS`] of quiet, or at [`MAX_TAIL_SECS`] whatever sounds
//!    ([`Offline::tail_cut`]).
//!
//! Samples are encoded here, in the engine, so that every host writes the same bytes: the
//! page, the helper and the tests read PCM from [`Offline::pcm`] and the header from
//! [`Offline::header`]. Nothing here runs on an audio thread; the engine's `process` path
//! it drives is the one the allocation detector watches.

use crate::host::{Host, Prepare, channel};
use crate::msg::{MsgError, limits};

/// Frames per block: the AudioWorklet's render quantum, so that an export renders in the
/// blocks the live engine does.
pub const QUANTUM: usize = 128;
/// The level under which a frame counts as silence in the tail, in dBFS.
pub const SILENCE_DB: f32 = -90.0;
/// Seconds of silence after the body that end the render.
pub const SILENCE_SECS: f32 = 1.0;
/// The longest tail rendered after the body, in seconds (a self-oscillating effect never
/// falls silent).
pub const MAX_TAIL_SECS: f32 = 30.0;
/// The longest body accepted, in seconds: thirty minutes, which is 518 MB of 24-bit
/// stereo at 48 kHz, well under the 4 GiB a WAV file can hold.
pub const MAX_BODY_SECS: f64 = 30.0 * 60.0;
/// The most frames one [`Offline::step`] renders.
pub const MAX_STEP_FRAMES: usize = 16_384;
/// Rounds of "apply what arrived, push what waited" a stream may take before the engine is
/// declared stuck.
const MAX_DRAIN_ROUNDS: usize = 1_000_000;

/// How the samples are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// 16-bit integers.
    Pcm16,
    /// 24-bit integers.
    Pcm24,
    /// 32-bit floats. No format clips: the output protection (#402) holds every sample under
    /// [`crate::limiter::CEILING`] (−0.3 dBFS) before this encoding, floats included.
    Float32,
}

impl SampleFormat {
    /// The format of a code (`16`, `24` or `32`), `None` for another.
    pub fn from_bits(bits: u32) -> Option<SampleFormat> {
        match bits {
            16 => Some(SampleFormat::Pcm16),
            24 => Some(SampleFormat::Pcm24),
            32 => Some(SampleFormat::Float32),
            _ => None,
        }
    }

    /// Bits per sample.
    pub fn bits(self) -> u32 {
        match self {
            SampleFormat::Pcm16 => 16,
            SampleFormat::Pcm24 => 24,
            SampleFormat::Float32 => 32,
        }
    }

    /// Bytes of one stereo frame.
    pub fn frame_bytes(self) -> usize {
        2 * (self.bits() / 8) as usize
    }

    /// Bytes of the file's header: 44 for integers, 58 for floats (which carry a `fact`
    /// chunk).
    pub fn header_bytes(self) -> usize {
        match self {
            SampleFormat::Float32 => 58,
            _ => 44,
        }
    }
}

/// The header of a stereo WAV file of `frames` frames at `sample_rate`. The data of a file
/// longer than 4 GiB does not fit a WAV header: its size saturates.
pub fn wav_header(sample_rate: u32, format: SampleFormat, frames: u64) -> Vec<u8> {
    let frame_bytes = format.frame_bytes() as u64;
    let data = frames
        .saturating_mul(frame_bytes)
        .min(u64::from(u32::MAX - 64)) as u32;
    let float = format == SampleFormat::Float32;
    let mut out = Vec::with_capacity(format.header_bytes());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(format.header_bytes() as u32 - 8 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&(if float { 18u32 } else { 16 }).to_le_bytes());
    out.extend_from_slice(&(if float { 3u16 } else { 1 }).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate.saturating_mul(frame_bytes as u32)).to_le_bytes());
    out.extend_from_slice(&(frame_bytes as u16).to_le_bytes());
    out.extend_from_slice(&(format.bits() as u16).to_le_bytes());
    if float {
        out.extend_from_slice(&0u16.to_le_bytes()); // cbSize
        out.extend_from_slice(b"fact");
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&(frames.min(u64::from(u32::MAX)) as u32).to_le_bytes());
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    debug_assert_eq!(out.len(), format.header_bytes());
    out
}

/// What to render.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// Frames of the content: the song from its start to the end of its last placement
    /// (or the loop region), at the sample rate. Written whatever the signal.
    pub body_frames: u64,
    pub format: SampleFormat,
}

/// Where the render is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Rendering the content.
    Body,
    /// Rendering the tail until it falls silent.
    Tail,
    /// Finished: nothing more to write.
    Done,
}

/// Why a render cannot go on.
#[derive(Clone, Debug, PartialEq)]
pub enum OfflineError {
    /// The engine refused the stream.
    Stream(MsgError),
    /// The engine rejected or skipped a message: the render would not be the project.
    Messages { rejected: u32, skipped: u32 },
    /// The engine did not take the stream (its ring never drained).
    Stuck,
    /// The body is empty or longer than [`MAX_BODY_SECS`].
    Length,
}

/// The offline render (module doc).
pub struct Offline {
    prepare: Prepare,
    host: Host,
    sample_rate: u32,
    plan: Plan,
    stop: Vec<u8>,
    stopped: bool,
    phase: Phase,
    /// Frames rendered, written or not.
    rendered: u64,
    /// Frames written to the output.
    written: u64,
    /// Quiet frames since the last loud one, past the body; they wait in `held`.
    quiet: u32,
    held_left: Vec<f32>,
    held_right: Vec<f32>,
    silence_frames: u32,
    tail_frames: u64,
    tail_cut: bool,
    threshold: f32,
    peak: f32,
    clipped: u64,
    pcm: Vec<u8>,
    inbox: Vec<u8>,
    /// Times a stream stopped on a full engine and went on after the engine applied some.
    busy: u32,
}

impl core::fmt::Debug for Offline {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Offline")
            .field("phase", &self.phase)
            .field("rendered", &self.rendered)
            .field("written", &self.written)
            .finish_non_exhaustive()
    }
}

impl Offline {
    /// An offline render at `sample_rate`. Refused when the plan's body is empty or longer
    /// than [`MAX_BODY_SECS`], or when the memory is.
    pub fn new(sample_rate: u32, plan: Plan) -> Result<Offline, OfflineError> {
        let rate = f64::from(sample_rate);
        if plan.body_frames == 0 || plan.body_frames as f64 > MAX_BODY_SECS * rate {
            return Err(OfflineError::Length);
        }
        let (prepare, mut host) = channel(sample_rate as f32).map_err(OfflineError::Stream)?;
        // The reduction and silenced counters cover this render only (the channel is fresh, so
        // this is belt-and-braces): clear them before the first frame.
        host.engine_mut().reset_limiter_reduction();
        host.engine_mut().reset_limiter_silenced();
        let silence_frames = (SILENCE_SECS * sample_rate as f32) as u32;
        let reserve = |n: usize| -> Result<Vec<f32>, OfflineError> {
            let mut v = Vec::new();
            v.try_reserve_exact(n)
                .map_err(|_| OfflineError::Stream(MsgError::OutOfMemory { bytes: n * 4 }))?;
            Ok(v)
        };
        let pcm_cap =
            (MAX_STEP_FRAMES + silence_frames as usize + QUANTUM) * plan.format.frame_bytes();
        let mut pcm = Vec::new();
        pcm.try_reserve_exact(pcm_cap)
            .map_err(|_| OfflineError::Stream(MsgError::OutOfMemory { bytes: pcm_cap }))?;
        Ok(Offline {
            prepare,
            host,
            sample_rate,
            plan,
            stop: Vec::new(),
            stopped: false,
            phase: Phase::Body,
            rendered: 0,
            written: 0,
            quiet: 0,
            held_left: reserve(silence_frames as usize + QUANTUM)?,
            held_right: reserve(silence_frames as usize + QUANTUM)?,
            silence_frames,
            tail_frames: (MAX_TAIL_SECS * sample_rate as f32) as u64,
            tail_cut: false,
            threshold: 10f32.powf(SILENCE_DB / 20.0),
            peak: 0.0,
            clipped: 0,
            pcm,
            inbox: Vec::new(),
            busy: 0,
        })
    }

    /// The stream (already encoded in the render's epoch) that stops the transport once the
    /// body is rendered.
    pub fn set_stop(&mut self, stream: &[u8]) {
        self.stop.clear();
        self.stop.extend_from_slice(stream);
    }

    /// [`Offline::set_stop`] with the first `len` bytes of the inbox.
    pub fn set_stop_from_inbox(&mut self, len: usize) {
        let len = len.min(self.inbox.len());
        self.stop.clear();
        self.stop.extend_from_slice(&self.inbox[..len]);
    }

    /// Bytes of the file's header.
    pub fn header_len(&self) -> usize {
        self.plan.format.header_bytes()
    }

    /// Submits a `ControlMsg` stream and applies it before the next render, however long:
    /// a stream the engine's ring cannot hold at once is sent in pieces, each applied
    /// before the next goes in. The last message of the setup is the one that starts the
    /// transport.
    pub fn submit(&mut self, stream: &[u8]) -> Result<(), OfflineError> {
        let mut rest = stream;
        for _ in 0..MAX_DRAIN_ROUNDS {
            match self.prepare.submit(rest) {
                Ok(()) => return self.drain(),
                Err(MsgError::EngineBusy) => {
                    self.busy += 1;
                    let at = self.prepare.preparer().stopped_at().unwrap_or(0);
                    rest = &rest[at.min(rest.len())..];
                    self.host.pump();
                    self.prepare.flush();
                }
                Err(error) => return Err(OfflineError::Stream(error)),
            }
        }
        Err(OfflineError::Stuck)
    }

    /// Applies everything the engine holds: what waited on the queue and what sits in the ring
    /// (the engine applies at most `MAX_APPLY_PER_BLOCK` messages per block), so that the
    /// transport starts, and the first frame is rendered, from the whole setup, however long.
    /// The engine takes its live notes at each of these blocks and keeps only the last ones:
    /// a stream for an export carries none (the project plays itself).
    fn drain(&mut self) -> Result<(), OfflineError> {
        for _ in 0..MAX_DRAIN_ROUNDS {
            let applied = self.host.pump();
            self.prepare.flush();
            if applied == 0 && self.prepare.preparer().pending() == 0 {
                return Ok(());
            }
        }
        Err(OfflineError::Stuck)
    }

    /// The address-stable inbox of at least `len` bytes, for a host that writes a stream
    /// into memory it shares with the engine (the WASM build); `None` when the memory is
    /// refused.
    pub fn inbox(&mut self, len: usize) -> Option<&mut [u8]> {
        if len > self.inbox.len() {
            self.inbox.try_reserve_exact(len - self.inbox.len()).ok()?;
            self.inbox.resize(len, 0);
        }
        Some(&mut self.inbox)
    }

    /// Submits the first `len` bytes of the inbox ([`Offline::submit`]).
    pub fn submit_inbox(&mut self, len: usize) -> Result<(), OfflineError> {
        let stream = std::mem::take(&mut self.inbox);
        let result = self.submit(&stream[..len.min(stream.len())]);
        self.inbox = stream;
        result
    }

    /// How many times the engine was full while a stream went in (the stream then waited for
    /// the engine to apply what it held).
    pub fn busy(&self) -> u32 {
        self.busy
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Frames rendered so far, whether written or not.
    pub fn rendered(&self) -> u64 {
        self.rendered
    }

    /// Frames written to the output so far: the length of the file once the render is done.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// The body's length in frames.
    pub fn body_frames(&self) -> u64 {
        self.plan.body_frames
    }

    /// Whether the tail was cut at [`MAX_TAIL_SECS`] while still sounding.
    pub fn tail_cut(&self) -> bool {
        self.tail_cut
    }

    /// The largest sample written, after the output protection (#402) and before the integer
    /// rounding of the format (linear). Since the output protection holds every sample to
    /// [`crate::limiter::CEILING`] (−0.3 dBFS), `peak` is always ≤ that value and `clipped`
    /// is always 0. They remain because the WAV export result shows a "highest level" figure
    /// to the user, and future formats may have a different ceiling. The overload the user
    /// cares about (a mix pushed into the limiter) is [`Offline::reduction`], not `peak`.
    pub fn peak(&self) -> f32 {
        self.peak
    }

    /// The peak gain reduction the output protection applied over this render, in `[0, 1]`
    /// (`0.0` = never engaged). This is a gain-reduction ratio, not a level: `-20·log10(1 -
    /// reduction)` is the deepest attenuation in dB. The export shows a warning when it is above
    /// a small threshold: the file does not clip (the limiter holds it under the ceiling), but
    /// the mix was pushed into the limiter, so it was audibly attenuated. It measures the same
    /// over-the-ceiling condition the master meter shows live before the limiter, but as the
    /// reduction the limiter applied, not as the pre-limiter level (#402, 2026-10-31).
    pub fn reduction(&self) -> f32 {
        self.host.engine().limiter_reduction()
    }

    /// How many stereo frames the protection turned to silence because a sample was non-finite
    /// (NaN or infinity) over this render (frames, not samples). `0` on a clean render. The
    /// limiter is the last stage, so a device or a non-device path (master strip, automation,
    /// summing) that blew up leaves the engine as silence that the output `assert_finite` can no
    /// longer catch; this counter is how an offline render still catches it (#402).
    pub fn silenced(&self) -> u64 {
        self.host.engine().limiter_silenced()
    }

    /// Samples an integer format clipped to full scale. Always 0 since the output protection
    /// (#402) holds every sample below the integer range of any standard format. Use
    /// [`Offline::reduction`] to warn the user about an overloaded mix.
    pub fn clipped(&self) -> u64 {
        self.clipped
    }

    /// The bytes written by the last [`Offline::step`].
    pub fn pcm(&self) -> &[u8] {
        &self.pcm
    }

    /// The file's header for what was written. Call it once the render is done.
    pub fn header(&self) -> Vec<u8> {
        wav_header(self.sample_rate, self.plan.format, self.written)
    }

    /// Renders up to `max_frames` (clamped to [`MAX_STEP_FRAMES`], at least one block)
    /// and encodes what is to be written into [`Offline::pcm`], replacing the last step's.
    /// Returns the phase after the step. Cheap to call again once [`Phase::Done`].
    pub fn step(&mut self, max_frames: usize) -> Result<Phase, OfflineError> {
        self.pcm.clear();
        if self.phase == Phase::Done {
            return Ok(Phase::Done);
        }
        let budget = max_frames.clamp(QUANTUM, MAX_STEP_FRAMES);
        let mut used = 0;
        while used < budget && self.phase != Phase::Done {
            if self.phase == Phase::Body && self.rendered >= self.plan.body_frames {
                self.phase = Phase::Tail;
            }
            if self.phase == Phase::Tail && !self.stopped {
                self.stopped = true;
                if !self.stop.is_empty() {
                    let stop = std::mem::take(&mut self.stop);
                    self.submit(&stop)?;
                    self.stop = stop;
                }
            }
            self.prepare.flush();
            // The last block of the content ends on its last frame, so that the stop lands there
            // and not up to a block later (what follows the range must not be heard).
            let block = match self.phase {
                Phase::Body => (self.plan.body_frames - self.rendered).min(QUANTUM as u64) as usize,
                _ => QUANTUM,
            };
            let n = self.host.render(block);
            used += n;
            self.take_block(n);
        }
        let (rejected, skipped) = (
            self.prepare.preparer().rejected(),
            self.host.engine().skipped(),
        );
        if rejected > 0 || skipped > 0 {
            return Err(OfflineError::Messages { rejected, skipped });
        }
        Ok(self.phase)
    }

    /// Writes or holds the `n` frames just rendered.
    fn take_block(&mut self, n: usize) {
        for i in 0..n {
            let (l, r) = (self.host.left()[i], self.host.right()[i]);
            let (l, r) = (
                if l.is_finite() { l } else { 0.0 },
                if r.is_finite() { r } else { 0.0 },
            );
            let index = self.rendered;
            self.rendered += 1;
            if index < self.plan.body_frames {
                self.write(l, r);
                continue;
            }
            if self.phase == Phase::Done {
                continue;
            }
            if l.abs() > self.threshold || r.abs() > self.threshold {
                for k in 0..self.held_left.len() {
                    let (hl, hr) = (self.held_left[k], self.held_right[k]);
                    self.write(hl, hr);
                }
                self.held_left.clear();
                self.held_right.clear();
                self.quiet = 0;
                self.write(l, r);
            } else {
                self.held_left.push(l);
                self.held_right.push(r);
                self.quiet += 1;
                if self.quiet >= self.silence_frames {
                    self.phase = Phase::Done;
                    continue;
                }
            }
            if self.rendered - self.plan.body_frames >= self.tail_frames {
                self.tail_cut = true;
                self.phase = Phase::Done;
            }
        }
    }

    /// Encodes one frame into the output.
    fn write(&mut self, l: f32, r: f32) {
        self.written += 1;
        self.peak = self.peak.max(l.abs()).max(r.abs());
        for s in [l, r] {
            match self.plan.format {
                SampleFormat::Float32 => self.pcm.extend_from_slice(&s.to_le_bytes()),
                SampleFormat::Pcm16 => {
                    let v = self.quantize(s, 32_767.0);
                    self.pcm.extend_from_slice(&(v as i16).to_le_bytes());
                }
                SampleFormat::Pcm24 => {
                    let v = self.quantize(s, 8_388_607.0);
                    self.pcm.extend_from_slice(&v.to_le_bytes()[..3]);
                }
            }
        }
    }

    /// `s` as an integer of full scale `scale`, rounded and clipped (and counted).
    fn quantize(&mut self, s: f32, scale: f32) -> i32 {
        if s.abs() > 1.0 {
            self.clipped += 1;
        }
        (s.clamp(-1.0, 1.0) * scale).round() as i32
    }
}

/// A render block is never larger than the engine's block limit.
const _: () = assert!(QUANTUM <= limits::MAX_BLOCK_FRAMES);
