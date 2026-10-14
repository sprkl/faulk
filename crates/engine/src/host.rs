//! The engine's host in the shipping targets (MO-2, `docs/specs/02-decoupage-mvp.md` § 2.3):
//! what the helper's audio callback and the web worklet run on top of the prepare/apply
//! cycle of [`crate::engine`].
//!
//! [`channel`] builds the two sides for a sample rate:
//!
//! - [`Prepare`], off the audio path (the helper's control thread, the web prepare Worker):
//!   [`Prepare::submit`] takes a `ControlMsg` byte stream, [`Prepare::flush`] pushes what
//!   waited and drops the garbage; both publish the counters the audio side reports.
//! - [`Host`], on the audio path: [`Host::pump`] applies what arrived
//!   (`Engine::begin_block`), [`Host::render_block`] renders the track graph
//!   (`Engine::render`, MO-3), and [`Host::render`] does both; about 30 times per second it
//!   writes the [`Feedback`] report (layout of `crate::msg`) into a region the page reads
//!   without stopping it. Nothing here allocates, frees, locks or panics.
//!
//! Interim, documented as such:
//!
//! - **The report's meters**: the graph has no per-track meters yet (MO-5), so the report
//!   carries one meter, the output's (after the master), as its only "track" meter, and
//!   the synth's voice count in the word at offset 52, which the synth adds to after each
//!   block (`add_block_voices`).
//!
//! [`factory`] resolves a device kind through the registry of the built-in devices
//! ([`crate::devices`], DV-1).
//!
//! [`Feedback`]: crate::msg::Feedback

use core::cell::Cell;
use core::sync::atomic::{AtomicU32, Ordering, fence};
use std::sync::Arc;

use crate::device::{DeviceSetup, Factory};
use crate::engine::{Engine, Preparer, channel as engine_channel};
use crate::msg::{
    DeviceKind, FEEDBACK_HEADER_BYTES, FEEDBACK_VERSION, Feedback, MsgError, TRACK_METER_BYTES,
    TransportState, limits,
};

// ---------------------------------------------------------------- the voice count

thread_local! {
    /// Voices sounding in the synths rendered by this thread's host during the current
    /// block: the synth (`crate::devices::osc_synth`) adds its own after each `process`
    /// (interim, module doc). A thread-local, so that two hosts on two threads (tests, two
    /// instances) never mix.
    static BLOCK_VOICES: Cell<u32> = const { Cell::new(0) };
}

/// Adds `voices` to the voice count of the block being rendered on this thread (interim,
/// module doc). No allocation, no lock: a thread-local cell.
pub(crate) fn add_block_voices(voices: u32) {
    let _ = BLOCK_VOICES.try_with(|v| v.set(v.get().saturating_add(voices)));
}

/// The device kinds the shipping hosts build: the registry of the built-in devices
/// ([`crate::devices`], DV-1).
pub fn factory(kind: DeviceKind) -> Option<Factory> {
    crate::devices::factory(kind)
}

// ---------------------------------------------------------------- the two sides

/// Counters the prepare side publishes for the audio side's report.
#[derive(Debug, Default)]
struct Status {
    epoch: AtomicU32,
    rejected: AtomicU32,
    live_overflows: AtomicU32,
}

/// The prepare side (module doc). Off the audio path.
#[derive(Debug)]
pub struct Prepare {
    preparer: Preparer,
    status: Arc<Status>,
}

impl Prepare {
    /// Submits a `ControlMsg` byte stream ([`Preparer::submit`]), then pushes what waited.
    /// The error is the one that stopped the stream; [`Preparer::stopped_at`] says where.
    pub fn submit(&mut self, stream: &[u8]) -> Result<(), MsgError> {
        let result = self.preparer.submit(stream);
        self.preparer.flush();
        self.publish();
        result
    }

    /// Pushes what waited on the rt ring and drops the garbage; the host calls it
    /// regularly, so that the queue drains even when nothing is sent.
    pub fn flush(&mut self) {
        self.preparer.flush();
        self.publish();
    }

    /// The prepare/apply cycle's prepare side.
    pub fn preparer(&self) -> &Preparer {
        &self.preparer
    }

    fn publish(&self) {
        let s = &self.status;
        s.epoch
            .store(u32::from(self.preparer.epoch()), Ordering::Relaxed);
        s.rejected
            .store(self.preparer.rejected(), Ordering::Relaxed);
        s.live_overflows
            .store(self.preparer.overflow().total(), Ordering::Relaxed);
    }
}

/// A track's meter, accumulated between two reports.
#[derive(Clone, Copy, Debug, Default)]
struct MeterAcc {
    peak: [f32; 2],
    squares: [f32; 2],
}

/// Words of the feedback region: the largest report (every track, two grid slots each,
/// the most recorded notes), so that it is never resized.
fn feedback_words() -> usize {
    Feedback::max_size_for(limits::MAX_TRACKS as usize).div_ceil(4)
}

/// Reports per second (the page reads the region at about this rate).
pub const REPORTS_PER_SECOND: f32 = 30.0;

/// The audio side (module doc).
pub struct Host {
    engine: Engine,
    status: Arc<Status>,
    sample_rate: f32,
    left: Box<[f32]>,
    right: Box<[f32]>,
    /// The output's meter over the reporting period.
    meter: MeterAcc,
    meter_frames: u32,
    /// Voices of the last block, and the most of the reporting period.
    voices: u32,
    period_voices: u32,
    report_frames: u32,
    feedback: Box<[AtomicU32]>,
    feedback_seq: u32,
}

impl core::fmt::Debug for Host {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Host")
            .field("engine", &self.engine)
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}

fn try_slice<T: Clone>(n: usize, value: T) -> Result<Box<[T]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| MsgError::OutOfMemory {
        bytes: n.saturating_mul(size_of::<T>()),
    })?;
    v.resize(n, value);
    Ok(v.into_boxed_slice())
}

/// Builds the two sides for `sample_rate`, blocks up to [`limits::MAX_BLOCK_FRAMES`], the
/// devices of [`factory`]. A new sample rate is a new channel (MO-1).
pub fn channel(sample_rate: f32) -> Result<(Prepare, Host), MsgError> {
    let setup = DeviceSetup {
        sample_rate,
        max_block_frames: limits::MAX_BLOCK_FRAMES,
    };
    let (preparer, engine) = engine_channel(setup, factory)?;
    let status = Arc::new(Status::default());
    let words = feedback_words();
    let mut feedback = Vec::new();
    feedback
        .try_reserve_exact(words)
        .map_err(|_| MsgError::OutOfMemory { bytes: words * 4 })?;
    feedback.resize_with(words, || AtomicU32::new(0));
    let frames = limits::MAX_BLOCK_FRAMES;
    let host = Host {
        engine,
        status: Arc::clone(&status),
        sample_rate,
        left: try_slice(frames, 0.0)?,
        right: try_slice(frames, 0.0)?,
        meter: MeterAcc::default(),
        meter_frames: 0,
        voices: 0,
        period_voices: 0,
        report_frames: report_frames(sample_rate),
        feedback: feedback.into_boxed_slice(),
        feedback_seq: 0,
    };
    let prepare = Prepare { preparer, status };
    prepare.publish();
    Ok((prepare, host))
}

/// Frames between two reports at `sample_rate` (at least one block's worth of 1 frame).
fn report_frames(sample_rate: f32) -> u32 {
    let frames = sample_rate / REPORTS_PER_SECOND;
    if frames.is_finite() && frames >= 1.0 {
        frames as u32
    } else {
        1600
    }
}

/// A word of the region holding four little-endian bytes, whatever the host's endianness.
fn word(bytes: [u8; 4]) -> u32 {
    u32::from_ne_bytes(bytes)
}

impl Host {
    /// The audio side of the cycle.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The start of a block: applies what arrived and takes the live events
    /// (`Engine::begin_block`). Returns how many rt messages were applied.
    pub fn pump(&mut self) -> u32 {
        let before = self.engine.applied();
        self.engine.begin_block();
        (self.engine.applied() - before) as u32
    }

    /// One block: [`Host::pump`] then [`Host::render_block`]. Returns the frames rendered.
    pub fn render(&mut self, frames: usize) -> usize {
        self.pump();
        self.render_block(frames)
    }

    /// Renders one block of `frames` (clamped to [`limits::MAX_BLOCK_FRAMES`]) from what
    /// the last [`Host::pump`] took, without applying anything; writes the report when it
    /// is due. Returns the frames rendered.
    pub fn render_block(&mut self, frames: usize) -> usize {
        let frames = frames.min(self.left.len());
        let (left, right) = (&mut self.left[..frames], &mut self.right[..frames]);
        let _ = BLOCK_VOICES.try_with(|v| v.set(0));
        self.engine.render(left, right);
        let acc = &mut self.meter;
        for (i, s) in [left.iter(), right.iter()].into_iter().enumerate() {
            for s in s {
                acc.peak[i] = acc.peak[i].max(s.abs());
                acc.squares[i] += s * s;
            }
        }
        self.voices = BLOCK_VOICES.try_with(Cell::get).unwrap_or(0);
        self.period_voices = self.period_voices.max(self.voices);
        self.meter_frames = self.meter_frames.saturating_add(frames as u32);
        if self.meter_frames >= self.report_frames {
            self.write_feedback();
        }
        frames
    }

    /// Voices sounding in the synths after the last block (the synth's own count:
    /// releasing voices included).
    pub fn voices(&self) -> u32 {
        self.voices
    }

    /// The left channel of the last block (its first `frames` samples are valid).
    pub fn left(&self) -> &[f32] {
        &self.left
    }

    /// The right channel of the last block.
    pub fn right(&self) -> &[f32] {
        &self.right
    }

    /// The report region: [`Feedback::max_size_for`] the track limit, in 32-bit words that
    /// hold the little-endian bytes of the `Feedback` layout. Its address and size never
    /// change.
    pub fn feedback(&self) -> &[AtomicU32] {
        &self.feedback
    }

    /// Writes the report now and starts a new metering period (the render calls it about
    /// [`REPORTS_PER_SECOND`] times per second). The `seq` word is odd while the body is
    /// written: a reader takes an even `seq`, the body, then the same `seq` again.
    ///
    /// Written: version, epoch and the counters the prepare side published, the most voices
    /// sounding during the period (offset 52, interim until MO-5's meters: the synth adds its voices, `crate::msg`), and one
    /// meter: the output's, after the master (interim until MO-5's track meters).
    /// Not produced yet, written as zero: the transport state and playhead (TR-1), the CPU
    /// load, the disk underruns (AU-3), the active clips (LA), the recorded notes (RE-2).
    pub fn write_feedback(&mut self) {
        let seq = self.feedback_seq.wrapping_add(1) | 1;
        let fb = &self.feedback;
        let put = |at: usize, value: u32| {
            if let Some(w) = fb.get(at / 4) {
                w.store(word(value.to_le_bytes()), Ordering::Relaxed);
            }
        };
        put(4, seq);
        fence(Ordering::Release);

        let epoch = self.status.epoch.load(Ordering::Relaxed) as u16;
        let [e0, e1] = epoch.to_le_bytes();
        put(
            0,
            u32::from_le_bytes([FEEDBACK_VERSION, TransportState::Stopped as u8, e0, e1]),
        );
        for at in (8..FEEDBACK_HEADER_BYTES).step_by(4) {
            put(at, 0);
        }
        put(28, self.status.live_overflows.load(Ordering::Relaxed));
        put(32, self.status.rejected.load(Ordering::Relaxed));
        // One meter, the output's (module doc), until MO-5 meters the tracks.
        put(40, 1);
        put(52, self.period_voices);
        let period = self.meter_frames.max(1) as f32;
        let acc = self.meter;
        let at = FEEDBACK_HEADER_BYTES;
        put(at, acc.peak[0].to_bits());
        put(at + 4, acc.peak[1].to_bits());
        put(at + 8, (acc.squares[0] / period).sqrt().to_bits());
        put(at + 12, (acc.squares[1] / period).sqrt().to_bits());
        self.meter = MeterAcc::default();
        self.meter_frames = 0;
        self.period_voices = self.voices;

        let done = seq.wrapping_add(1);
        if let Some(w) = fb.get(1) {
            w.store(word(done.to_le_bytes()), Ordering::Release);
        }
        self.feedback_seq = done;
    }
}

/// The voices word of a report region (offset 52), as [`read_feedback`] reads it.
pub fn feedback_voices(region: &[AtomicU32]) -> u32 {
    region.get(13).map_or(0, |w| {
        u32::from_le_bytes(w.load(Ordering::Relaxed).to_ne_bytes())
    })
}

/// Reads a report region as the page does (tests and the helper): `None` while a write is
/// in progress or was torn by one, else the decoded report.
pub fn read_feedback(region: &[AtomicU32]) -> Option<Feedback> {
    let load = |at: usize| -> u32 {
        region.get(at / 4).map_or(0, |w| {
            u32::from_le_bytes(w.load(Ordering::Relaxed).to_ne_bytes())
        })
    };
    let seq = region
        .get(1)
        .map(|w| u32::from_le_bytes(w.load(Ordering::Acquire).to_ne_bytes()))?;
    if seq & 1 == 1 {
        return None;
    }
    let head = load(0).to_le_bytes();
    let tracks = (load(40) as usize).min(limits::MAX_TRACKS as usize);
    let mut meters = Vec::with_capacity(tracks);
    for i in 0..tracks {
        let at = FEEDBACK_HEADER_BYTES + i * TRACK_METER_BYTES;
        meters.push(crate::msg::TrackMeter {
            peak_l: f32::from_bits(load(at)),
            peak_r: f32::from_bits(load(at + 4)),
            rms_l: f32::from_bits(load(at + 8)),
            rms_r: f32::from_bits(load(at + 12)),
        });
    }
    let lo = |at: usize| u64::from(load(at)) | (u64::from(load(at + 4)) << 32);
    let feedback = Feedback {
        transport: match head[1] {
            1 => TransportState::Playing,
            2 => TransportState::Recording,
            _ => TransportState::Stopped,
        },
        epoch: u16::from_le_bytes([head[2], head[3]]),
        seq,
        playhead_tick: crate::time::Tick(lo(8) as i64),
        playhead_frame: crate::time::Frame(lo(16)),
        cpu_load: f32::from_bits(load(24)),
        live_overflows: load(28),
        rejected: load(32),
        disk_underruns: load(36),
        meters,
        active_clips: Vec::new(),
        recorded: Vec::new(),
    };
    fence(Ordering::Acquire);
    let again = region
        .get(1)
        .map(|w| u32::from_le_bytes(w.load(Ordering::Relaxed).to_ne_bytes()))?;
    (again == seq && head[0] == FEEDBACK_VERSION).then_some(feedback)
}
