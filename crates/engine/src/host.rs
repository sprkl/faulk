//! The engine's host in the shipping targets (MO-2, `docs/specs/02-decoupage-mvp.md` § 2.3):
//! what the helper's audio callback and the web worklet run on top of the prepare/apply
//! cycle of [`crate::engine`].
//!
//! [`channel`] builds the two sides for a sample rate:
//!
//! - [`Prepare`], off the audio path (the helper's control thread, the web prepare Worker):
//!   [`Prepare::submit`] takes a `ControlMsg` byte stream, [`Prepare::flush`] pushes what
//!   waited and drops the garbage; both publish the counters the audio side reports.
//!   [`StreamQueue`] puts it behind a queue of streams, so that a stream longer than the
//!   engine takes at once (a big project's snapshot) is sent in pieces as the engine
//!   consumes it (#245); the web prepare Worker does the same in `push-queue.ts`.
//! - [`Host`], on the audio path: [`Host::pump`] applies what arrived
//!   (`Engine::begin_block`), [`Host::render_block`] renders the track graph
//!   (`Engine::render`, MO-3), and [`Host::render`] does both; about 30 times per second it
//!   writes the [`Feedback`] report (layout of `crate::msg`) into a region the page reads
//!   without stopping it. Nothing here allocates, frees, locks or panics.
//!
//! The report (MO-5): the track meters (peak and RMS after each fader), the output meter
//! after the master, the voices of the tracks' devices, the transport state and playhead
//! (`Engine::report_transport`), the counters the prepare side published and, when the
//! caller times its renders ([`Host::add_render_time`]), the CPU load and its peak. Not produced yet,
//! written as zero: the disk underruns (AU-3). The active clips of the launch grid (GR-1)
//! follow the meters, then the last notes of the take ([`crate::recorder`], RE-2).
//!
//! [`factory`] resolves a device kind through the registry of the built-in devices
//! ([`crate::devices`], DV-1).
//!
//! [`Feedback`]: crate::msg::Feedback

use core::sync::atomic::{AtomicU32, Ordering, fence};
use std::collections::VecDeque;
use std::sync::Arc;

use crate::analysis::{
    Analysis, MAX_SPECTRA, MAX_WINDOW, SPECTRUM_REGION_WORDS, SPECTRUM_WIRE_BYTES, Source,
    Spectrum, WINDOW_REGION_WORDS, WindowWriter, read_spectra, try_region,
};
use crate::device::{DeviceSetup, Factory};
use crate::engine::{Engine, Preparer, channel as engine_channel};
use crate::msg::try_box;
use crate::msg::{
    ACTIVE_CLIP_BYTES, ActiveClip, ClipState, DEVICE_READOUT_BYTES, DeviceKind, DeviceReadout,
    DeviceSlot, FEEDBACK_HEADER_BYTES, FEEDBACK_LATENCY_OFFSET, FEEDBACK_READOUTS_OFFSET,
    FEEDBACK_SPECTRA_OFFSET, FEEDBACK_VERSION, Feedback, HEADER_BYTES, MAX_READOUTS_PER_REPORT,
    MAX_RECORDED_PER_REPORT, MSG_VERSION, MsgError, Note, ParamId, RECORDED_NOTE_WIRE_BYTES,
    RecordedNote, TRACK_METER_BYTES, Tag, TrackMeter, TrackSlot, TransportState, limits,
};
use crate::time::SampleRate;

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
    /// The spectrum analysis (#156, [`crate::analysis`]): the windows the audio side copies,
    /// the spectra this side computes from them, and the analyser.
    windows: Arc<[AtomicU32]>,
    spectra: Arc<[AtomicU32]>,
    analysis: Box<Analysis>,
}

impl Prepare {
    /// Submits a `ControlMsg` byte stream ([`Preparer::submit`]), then pushes what waited.
    /// The error is the one that stopped the stream; [`Preparer::stopped_at`] says where.
    pub fn submit(&mut self, stream: &[u8]) -> Result<(), MsgError> {
        let result = self.preparer.submit(stream);
        self.preparer.flush();
        self.publish();
        self.analyse();
        result
    }

    /// Pushes what waited on the rt ring and drops the garbage; the host calls it
    /// regularly, so that the queue drains even when nothing is sent.
    pub fn flush(&mut self) {
        self.preparer.flush();
        self.publish();
        self.analyse();
    }

    /// Computes the spectra of the windows the audio side copied since the last call, if any,
    /// and publishes them for its next report ([`crate::analysis`]); `true` when it did. The
    /// host calls it through [`Prepare::flush`], every few milliseconds.
    pub fn analyse(&mut self) -> bool {
        self.analysis.run(&self.windows, &self.spectra)
    }

    /// Changes the sample rate, keeping the playhead: every device is built again for
    /// `rate` here, off the audio thread ([`Preparer::set_sample_rate`], #236), then what
    /// waited is pushed. Nothing changes on an error.
    pub fn set_sample_rate(&mut self, rate: SampleRate) -> Result<(), MsgError> {
        let result = self.preparer.set_sample_rate(rate);
        self.preparer.flush();
        self.publish();
        result
    }

    /// The prepare/apply cycle's prepare side.
    pub fn preparer(&self) -> &Preparer {
        &self.preparer
    }

    /// The disk planner's poll at `now_ms` (AU-3, [`crate::audio_source::Disk::poll`]): its
    /// messages are sent, its new read jobs returned.
    pub fn disk_poll(&mut self, now_ms: u64) -> &[crate::audio_source::Job] {
        self.preparer.disk_poll(now_ms)
    }

    /// A read job's bytes ([`crate::audio_source::Disk::done`]).
    pub fn disk_done(&mut self, id: u32, bytes: &[u8], file_len: u64) {
        self.preparer.disk_done(id, bytes, file_len);
    }

    /// A read job failed ([`crate::audio_source::Disk::failed`]).
    pub fn disk_failed(&mut self, id: u32, why: crate::audio_source::Failure) {
        self.preparer.disk_failed(id, why);
    }

    /// The disk planner.
    pub fn disk(&self) -> &crate::audio_source::Disk {
        self.preparer.disk()
    }

    /// The disk planner, to set its budget.
    pub fn disk_mut(&mut self) -> &mut crate::audio_source::Disk {
        self.preparer.disk_mut()
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

// ---------------------------------------------------------------- paced streams

/// The most bytes of streams a [`StreamQueue`] keeps waiting behind the one the engine is
/// taking (#245): a stream that would go past it is refused, so that an engine that stopped
/// consuming (a lost device) cannot make the queue grow without limit. The stream the engine
/// is taking is never refused, whatever its size: a valid project's snapshot may be larger
/// than this (21 bytes per note, up to `limits::MAX_NOTES` notes), and it is what a host
/// sends to recover.
pub const MAX_WAITING_BYTES: usize = 64 * 1024 * 1024;

/// Whether a stream's first message is a `Reset` (the header's tag, `crate::msg`).
fn starts_with_reset(stream: &[u8]) -> bool {
    stream.len() >= HEADER_BYTES
        && stream[0] == MSG_VERSION
        && u16::from_le_bytes([stream[2], stream[3]]) == Tag::Reset as u16
}

/// The rest of a stream a busy engine stopped: the stream and the offset of the message to
/// send next.
#[derive(Debug)]
struct Waiting {
    bytes: Vec<u8>,
    at: usize,
}

/// The prepare side behind a queue of streams (#245), off the audio path: the native twin of
/// the web prepare Worker's `PushQueue` (`web/src/lib/engine/push-queue.ts`), with the same
/// contract. A stream the engine stops because it is busy (`EngineBusy`: its ring and its own
/// queue are full, as a big project's snapshot makes them) keeps its rest, from the message
/// that stopped it ([`Preparer::stopped_at`]; the refused message left nothing behind), and
/// is sent again at the next [`StreamQueue::flush`], before anything sent after it: nothing
/// is lost nor reordered among the streams. (Once nothing waits here, the prepare side's own
/// queue may still hold the tail of a stream, and a live note sent then goes straight to the
/// live queue, ahead of that tail: `Preparer::handle`.) Any other stop drops that stream (the engine is then behind the
/// project until a resync) and is returned.
///
/// [`Preparer::stopped_at`]: crate::engine::Preparer::stopped_at
#[derive(Debug)]
pub struct StreamQueue {
    prepare: Prepare,
    waiting: VecDeque<Waiting>,
    /// Bytes of the waiting streams not sent yet.
    waiting_bytes: usize,
    /// The bound of the bytes waiting behind the front stream: [`MAX_WAITING_BYTES`] but
    /// in tests.
    max_waiting: usize,
}

impl StreamQueue {
    pub fn new(prepare: Prepare) -> Self {
        Self::with_max_waiting(prepare, MAX_WAITING_BYTES)
    }

    /// A queue that keeps at most `max_waiting` bytes waiting behind the front stream, for
    /// tests of the bound.
    pub fn with_max_waiting(prepare: Prepare, max_waiting: usize) -> Self {
        StreamQueue {
            prepare,
            waiting: VecDeque::new(),
            waiting_bytes: 0,
            max_waiting,
        }
    }

    /// The prepare side (its counters).
    pub fn prepare(&self) -> &Prepare {
        &self.prepare
    }

    /// The prepare side, to drive its disk planner (AU-3: [`Prepare::disk_poll`]).
    pub fn prepare_mut(&mut self) -> &mut Prepare {
        &mut self.prepare
    }

    /// Changes the sample rate ([`Prepare::set_sample_rate`], #236): the devices are built
    /// again for `rate` and go in before the streams that wait here, which the prepare side
    /// then builds at `rate` too. Nothing changes on an error.
    pub fn set_sample_rate(&mut self, rate: SampleRate) -> Result<(), MsgError> {
        self.prepare.set_sample_rate(rate)
    }

    /// Streams waiting for the engine.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Bytes of the waiting streams not sent yet.
    pub fn waiting_bytes(&self) -> usize {
        self.waiting_bytes
    }

    /// Sends a `ControlMsg` byte stream after the ones that wait: what a busy engine does
    /// not take yet waits for [`StreamQueue::flush`]. The error is the first that stopped a
    /// stream other than a busy engine (that stream is dropped, the others go on), or
    /// `EngineBusy` for a stream refused because the streams waiting behind the front one
    /// would go past the bound, [`MAX_WAITING_BYTES`] (nothing of it is handled nor
    /// queued), or `OutOfMemory` when its rest could not be kept (its messages before the
    /// one that stopped it stay handled). After any error the engine is behind the project:
    /// the host resyncs it ([`StreamQueue::clear`], then a `Reset` and a snapshot).
    pub fn send(&mut self, stream: &[u8]) -> Result<(), MsgError> {
        if stream.is_empty() {
            return Ok(());
        }
        // A stream that starts the engine over (a `Reset` and a snapshot: `project.open`, a
        // resync) makes what waits pointless: the `Reset` would wipe it once applied.
        if starts_with_reset(stream) {
            self.clear();
        }
        if !self.waiting.is_empty() {
            let front = self.waiting.front().map_or(0, |w| w.bytes.len() - w.at);
            let behind = self.waiting_bytes - front;
            if behind.saturating_add(stream.len()) > self.max_waiting {
                return Err(MsgError::EngineBusy);
            }
            self.keep(stream)?;
            return self.drain();
        }
        // Nothing waits: the stream goes straight in, and only its rest is copied.
        match self.prepare.submit(stream) {
            Err(MsgError::EngineBusy) => {
                let at = self.prepare.preparer().stopped_at().unwrap_or(0);
                self.keep(&stream[at.min(stream.len())..])
            }
            other => other,
        }
    }

    /// The control thread's regular tick: the prepare side's flush ([`Prepare::flush`]),
    /// then what waited here, in order. The error is as for [`StreamQueue::send`].
    pub fn flush(&mut self) -> Result<(), MsgError> {
        self.prepare.flush();
        self.drain()
    }

    /// Drops the waiting streams: what a host does before it resyncs the engine (the
    /// `Reset` of the snapshot would wipe what they bring), or when it quits.
    pub fn clear(&mut self) {
        self.waiting.clear();
        self.waiting_bytes = 0;
    }

    /// Queues a copy of `bytes` behind the waiting streams.
    fn keep(&mut self, bytes: &[u8]) -> Result<(), MsgError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut copy = Vec::new();
        copy.try_reserve_exact(bytes.len())
            .map_err(|_| MsgError::OutOfMemory { bytes: bytes.len() })?;
        copy.extend_from_slice(bytes);
        self.waiting
            .try_reserve(1)
            .map_err(|_| MsgError::OutOfMemory {
                bytes: size_of::<Waiting>(),
            })?;
        self.waiting.push_back(Waiting { bytes: copy, at: 0 });
        self.waiting_bytes += bytes.len();
        Ok(())
    }

    /// Sends the waiting streams in order until the engine is busy again.
    fn drain(&mut self) -> Result<(), MsgError> {
        let mut first = Ok(());
        while let Some(front) = self.waiting.front_mut() {
            let rest = front.bytes.len() - front.at;
            match self.prepare.submit(&front.bytes[front.at..]) {
                Err(MsgError::EngineBusy) => {
                    let at = self.prepare.preparer().stopped_at().unwrap_or(0).min(rest);
                    front.at += at;
                    self.waiting_bytes -= at;
                    break;
                }
                outcome => {
                    self.waiting.pop_front();
                    self.waiting_bytes -= rest;
                    if let (Ok(()), Err(error)) = (&first, outcome) {
                        first = Err(error);
                    }
                }
            }
        }
        first
    }
}

/// Words of the feedback region: the largest report (every track and the output, two grid
/// slots each, the most recorded notes), so that it is never resized.
fn feedback_words() -> usize {
    Feedback::max_size_for(limits::MAX_TRACKS as usize).div_ceil(4)
}

/// The CPU load of a report whose host never timed a block (no clock on the host).
pub const NOT_MEASURED: f32 = -1.0;

/// A peak window lasts at least this many steps of the caller's clock
/// ([`Host::set_clock_resolution`]): a clock of 1 ms (the web worklet's `Date.now()`) reads
/// a 2.7 ms quantum as 0, 1 or 2 ms, so its peak is taken over about 100 ms, where that error
/// is a few percent of the budget; with a fine clock (the helper's) a window is one block.
pub const PEAK_WINDOW_STEPS: f32 = 100.0;

/// The CPU load accounting of [`Host`] (#486): the caller times each block it renders with its
/// own clock and hands the time and the block's frames to [`Host::add_render_time`]. Plain
/// numbers: no allocation, no lock.
#[derive(Debug, Clone, Copy)]
struct Load {
    /// The caller timed a block once: from then on a report carries a load (0 for a period
    /// with no timed block), before it [`NOT_MEASURED`].
    clocked: bool,
    /// Seconds of render and frames timed since the last report.
    busy: f32,
    frames: u32,
    /// The peak window being filled: seconds and frames.
    window_busy: f32,
    window_frames: u32,
    /// The largest load of a window closed since the last report, [`NOT_MEASURED`] while none
    /// closed.
    peak: f32,
    /// The load of the last window closed, [`NOT_MEASURED`] before the first: the peak of a
    /// period shorter than a window (a coarse clock), so that it is never one noisy period.
    last_window: f32,
    /// Seconds of one step of the caller's clock (0: fine).
    resolution: f32,
}

impl Load {
    fn new() -> Self {
        Load {
            clocked: false,
            busy: 0.0,
            frames: 0,
            window_busy: 0.0,
            window_frames: 0,
            peak: NOT_MEASURED,
            last_window: NOT_MEASURED,
            resolution: 0.0,
        }
    }

    fn add(&mut self, seconds: f32, frames: u32, sample_rate: f32) {
        self.clocked = true;
        let seconds = if seconds.is_finite() && seconds > 0.0 {
            seconds
        } else {
            0.0
        };
        self.busy += seconds;
        self.frames = self.frames.saturating_add(frames);
        self.window_busy += seconds;
        self.window_frames = self.window_frames.saturating_add(frames);
        if sample_rate.is_nan() || sample_rate <= 0.0 || self.window_frames == 0 {
            return;
        }
        let window = self.window_frames as f32 / sample_rate;
        if window >= PEAK_WINDOW_STEPS * self.resolution {
            let load = (self.window_busy / window).clamp(0.0, 1.0);
            self.peak = self.peak.max(load);
            self.last_window = load;
            self.window_busy = 0.0;
            self.window_frames = 0;
        }
    }

    /// The load and peak of the period, then a new period (the open window carries over).
    /// The peak is the largest window closed in the period; with none, the last window
    /// closed; before any, the period's load.
    fn take(&mut self, sample_rate: f32) -> (f32, f32) {
        if !self.clocked {
            return (NOT_MEASURED, NOT_MEASURED);
        }
        let load = if sample_rate > 0.0 && self.frames > 0 {
            (self.busy / (self.frames as f32 / sample_rate)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let peak = if self.peak >= 0.0 {
            self.peak
        } else if self.last_window >= 0.0 {
            self.last_window
        } else {
            load
        };
        self.busy = 0.0;
        self.frames = 0;
        self.peak = NOT_MEASURED;
        (load, peak)
    }
}

/// Reports that still carry a take's notes after its capture ended: about 3 s, longer than the
/// page waits for them, so that a finished take does not ride in every report until the next.
pub const TAKE_REPORTS: u32 = 90;

/// Reports per second (the page reads the region at about this rate).
pub const REPORTS_PER_SECOND: f32 = 30.0;

/// The level of a strip over a stretch of render, linear amplitude: the largest sample of
/// each channel and the root mean square of each channel over all the frames. Non-finite
/// samples are left out of both, as in the live meters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Level {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
}

/// What [`Host::levels`] measured: the frames it covers, the level of every track slot up
/// to the last one that was occupied at a report, and the output's.
#[derive(Clone, Debug, PartialEq)]
pub struct Levels {
    /// Frames the output's level covers: rendered since the tally started up to the last
    /// report, or the frames of its window ([`Host::tally_levels_over`]).
    pub frames: u64,
    /// By track slot.
    pub tracks: Vec<Level>,
    /// The output, after the master and before the output protection.
    pub master: Level,
}

/// The sums behind [`Levels`]: one `(peak, sum of squares)` pair per channel, per slot and
/// for the output (last), each over its own window of the tally's frames (#755).
#[derive(Debug)]
struct Tally {
    /// Frames since the tally started, up to the last report.
    frames: u64,
    /// Slots up to the last one a report carried.
    used: usize,
    peak: Box<[[f32; 2]]>,
    squares: Box<[[f64; 2]]>,
    /// Where each strip's window starts, in the tally's frames: a strip folds the reports whose
    /// period starts in `[from, from + window)`.
    from: Box<[u64]>,
    /// The length of every strip's window (`u64::MAX`: every report from `from` on).
    window: u64,
    /// The frames each strip folded.
    folded: Box<[u64]>,
}

impl Tally {
    fn new() -> Result<Box<Tally>, MsgError> {
        let n = limits::MAX_TRACKS as usize + 1;
        Ok(Box::new(Tally {
            frames: 0,
            used: 0,
            peak: try_slice(n, [0.0; 2])?,
            squares: try_slice(n, [0.0; 2])?,
            from: try_slice(n, 0)?,
            window: u64::MAX,
            folded: try_slice(n, 0)?,
        }))
    }

    /// Folds one report's meters, taken over `frames` frames, into every strip whose window
    /// holds the period's first frame (a caller with windows cuts the periods at their edges).
    /// A meter's RMS is the root of its mean square over its period, so the squares are put
    /// back with the period's length. No allocation.
    fn add(&mut self, frames: u32, tracks: &[TrackMeter], master: TrackMeter) {
        let master_at = self.peak.len() - 1;
        let n = f64::from(frames.max(1));
        let (now, window) = (self.frames, self.window);
        let mut fold = |at: usize, m: &TrackMeter| {
            let from = self.from[at];
            if now < from || now - from >= window {
                return;
            }
            self.peak[at][0] = self.peak[at][0].max(m.peak_l);
            self.peak[at][1] = self.peak[at][1].max(m.peak_r);
            self.squares[at][0] += f64::from(m.rms_l) * f64::from(m.rms_l) * n;
            self.squares[at][1] += f64::from(m.rms_r) * f64::from(m.rms_r) * n;
            self.folded[at] = self.folded[at].saturating_add(u64::from(frames));
        };
        for (at, m) in tracks.iter().enumerate().take(master_at) {
            fold(at, m);
        }
        fold(master_at, &master);
        self.used = self.used.max(tracks.len().min(master_at));
        self.frames = self.frames.saturating_add(u64::from(frames));
    }

    fn levels(&self) -> Levels {
        let level = |at: usize| {
            let n = self.folded[at].max(1) as f64;
            Level {
                peak_l: self.peak[at][0],
                peak_r: self.peak[at][1],
                rms_l: (self.squares[at][0] / n).sqrt() as f32,
                rms_r: (self.squares[at][1] / n).sqrt() as f32,
            }
        };
        Levels {
            frames: self.folded[self.peak.len() - 1],
            tracks: (0..self.used).map(level).collect(),
            master: level(self.peak.len() - 1),
        }
    }
}

/// The audio side (module doc).
pub struct Host {
    engine: Engine,
    status: Arc<Status>,
    sample_rate: f32,
    left: Box<[f32]>,
    right: Box<[f32]>,
    /// Frames rendered since the last report: the meters' period.
    meter_frames: u32,
    /// The CPU load accounting ([`Host::add_render_time`]).
    load: Load,
    /// Where the engine puts the track meters of a report.
    meters: Box<[TrackMeter]>,
    /// The meters of every report folded together, when a caller asked for them
    /// ([`Host::tally_levels`]); `None` on the real-time hosts.
    tally: Option<Box<Tally>>,
    /// Scalar fields of the report the engine fills (`Engine::report_transport`); its
    /// vectors stay empty.
    scratch: Feedback,
    /// Voices of the last block, and the most of the reporting period.
    voices: u32,
    period_voices: u32,
    /// Reports written since the recorder stopped capturing: the take's notes ride along for
    /// [`TAKE_REPORTS`] more reports, then the reports stop carrying them (RE-2).
    idle_reports: u32,
    report_frames: u32,
    feedback: Arc<[AtomicU32]>,
    feedback_seq: u32,
    /// The spectrum analysis (#156, [`crate::analysis`]): the window region this side writes,
    /// the spectrum region it reads, the spectra read from it and the seq they were published
    /// under, the next device of the round robin, and a window's scratch.
    windows: Arc<[AtomicU32]>,
    spectrum_region: Arc<[AtomicU32]>,
    window_writer: WindowWriter,
    spectra: Box<[Spectrum; MAX_SPECTRA]>,
    spectra_seq: u32,
    robin: usize,
    window: Box<[f32]>,
    /// The clock's rate the host last saw (#236): a change makes the report period and the
    /// load follow it.
    clock_rate: u32,
    /// A test voice ([`Host::probe_start`]): reads one port's ring and segments outside any
    /// clip, so the stream layer (AU-3) is tested on its own. Test builds only: the audio
    /// clip player (AU-4) is the consumer of the rings.
    #[cfg(feature = "testing")]
    probe: crate::audio_source::Voice,
    #[cfg(feature = "testing")]
    probe_port: (crate::msg::TrackSlot, u8),
    #[cfg(feature = "testing")]
    probe_out: Box<[f32]>,
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
/// devices of [`factory`]. A later rate change keeps the channel: [`Prepare::set_sample_rate`].
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
    let windows = try_region(WINDOW_REGION_WORDS)?;
    let spectrum_region = try_region(SPECTRUM_REGION_WORDS)?;
    let analysis = try_box(Analysis::try_new()?)?;
    let host = Host {
        engine,
        status: Arc::clone(&status),
        sample_rate,
        left: try_slice(frames, 0.0)?,
        right: try_slice(frames, 0.0)?,
        meter_frames: 0,
        load: Load::new(),
        meters: try_slice(limits::MAX_TRACKS as usize, TrackMeter::default())?,
        tally: None,
        scratch: Feedback::default(),
        voices: 0,
        period_voices: 0,
        idle_reports: 0,
        report_frames: report_frames(sample_rate),
        feedback: Arc::from(feedback.into_boxed_slice()),
        feedback_seq: 0,
        windows: Arc::clone(&windows),
        spectrum_region: Arc::clone(&spectrum_region),
        window_writer: WindowWriter::default(),
        spectra: try_box([Spectrum::default(); MAX_SPECTRA])?,
        spectra_seq: 0,
        robin: 0,
        window: try_slice(MAX_WINDOW, 0.0)?,
        clock_rate: 0,
        #[cfg(feature = "testing")]
        probe: crate::audio_source::Voice::default(),
        #[cfg(feature = "testing")]
        probe_port: (TrackSlot(0), 0),
        #[cfg(feature = "testing")]
        probe_out: try_slice(frames * crate::audio_source::MAX_CHANNELS, 0.0)?,
    };
    let prepare = Prepare {
        preparer,
        status,
        windows,
        spectra: spectrum_region,
        analysis,
    };
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
    /// Starts the test voice on `port` of `track`, playing `path` of `source` (AU-3's stream
    /// layer on its own; test builds only). `false` when the track or the source does not
    /// exist. No allocation.
    #[cfg(feature = "testing")]
    pub fn probe_start(
        &mut self,
        track: TrackSlot,
        port: u8,
        source: crate::msg::SourceSlot,
        path: crate::audio_source::Path,
    ) -> bool {
        let (state, tx, _) = self.engine.streams();
        let Some(src) = state
            .sources
            .as_slice()
            .get(source.0 as usize)
            .and_then(|s| s.as_deref())
        else {
            return false;
        };
        let Some(t) = state
            .tracks
            .as_mut_slice()
            .get_mut(track.0 as usize)
            .and_then(|t| t.as_deref_mut())
        else {
            return false;
        };
        let instance = t.instance;
        let Some(p) = t.ports.get_mut(port as usize) else {
            return false;
        };
        let path = crate::audio_source::Path::new(path.start, path.end, path.looping, src.frames.0);
        self.probe.start(
            crate::audio_source::PortId {
                track,
                instance,
                port,
            },
            p,
            source,
            src.sample_rate,
            u16::from(src.channels),
            &src.segments,
            path,
            tx,
        );
        self.probe_port = (track, port);
        true
    }

    /// Reads `frames` frames of the test voice into [`Host::probe_out`]; returns how many
    /// came from the source. No allocation.
    #[cfg(feature = "testing")]
    pub fn probe_read(&mut self, frames: usize) -> usize {
        let (track, port) = self.probe_port;
        let source = self.probe.source();
        let (state, tx, stats) = self.engine.streams();
        let Some(t) = state
            .tracks
            .as_mut_slice()
            .get_mut(track.0 as usize)
            .and_then(|t| t.as_deref_mut())
        else {
            self.probe.stop();
            return 0;
        };
        let Some(p) = t.ports.get_mut(port as usize) else {
            return 0;
        };
        let segments = state
            .sources
            .as_slice()
            .get(source.0 as usize)
            .and_then(|s| s.as_deref())
            .map_or(&[][..], |s| &s.segments[..]);
        self.probe
            .read(p, segments, tx, stats, &mut self.probe_out, frames)
    }

    /// The test voice's last frames, interleaved ([`crate::audio_source::Voice::channels`]).
    #[cfg(feature = "testing")]
    pub fn probe_out(&self) -> &[f32] {
        &self.probe_out
    }

    /// The test voice.
    #[cfg(feature = "testing")]
    pub fn probe(&self) -> &crate::audio_source::Voice {
        &self.probe
    }

    /// The audio side of the cycle.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The audio side of the cycle, to mutate it: for tests that need to drive
    /// `Engine::set_sample_rate` or inspect the limiter state.
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// The start of a block: applies what arrived and takes the live events
    /// (`Engine::begin_block`). Returns how many rt messages were applied.
    pub fn pump(&mut self) -> u32 {
        let before = self.engine.applied();
        self.engine.begin_block();
        (self.engine.applied() - before) as u32
    }

    /// The host's frame of the first sample of the next block (#499,
    /// `Engine::set_host_frame`): live notes stamped in that clock are recorded where they
    /// were played. Call it before [`Host::pump`]; a host that never does has the engine's
    /// sample counter as its clock.
    pub fn set_host_frame(&mut self, frame: u64) {
        self.engine.set_host_frame(frame);
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
        // A new sample rate (#236, applied by the last pump): the report period and the
        // load follow it. Only a change: the rate the host was built for stays as given.
        let hz = self.engine.clock().sample_rate().hz();
        if hz != self.clock_rate {
            if self.clock_rate != 0 {
                self.sample_rate = hz as f32;
                self.report_frames = report_frames(self.sample_rate);
            }
            self.clock_rate = hz;
        }
        let (left, right) = (&mut self.left[..frames], &mut self.right[..frames]);
        self.engine.render(left, right);
        self.voices = self.engine.voices();
        self.period_voices = self.period_voices.max(self.voices);
        self.meter_frames = self.meter_frames.saturating_add(frames as u32);
        if self.meter_frames >= self.report_frames {
            self.write_feedback();
        }
        frames
    }

    /// One timed block (#486): the caller (the helper's audio callback, the web worklet)
    /// timed the render of `frames` with its own clock, the engine has none, and it took
    /// `seconds` (a time that is not finite or not positive counts as 0 with its frames).
    /// The report's CPU load is the time over the duration of the frames timed in its period,
    /// its peak the largest load of a peak window: consecutive timed blocks lasting at least
    /// [`PEAK_WINDOW_STEPS`] steps of the clock ([`Host::set_clock_resolution`]), one block
    /// with a fine clock. A block timed after the render that wrote a report counts in the
    /// next one, with its frames. A host that never calls it reports [`NOT_MEASURED`].
    /// No allocation.
    pub fn add_render_time(&mut self, seconds: f32, frames: u32) {
        self.load.add(seconds, frames, self.sample_rate);
    }

    /// The step of the caller's clock, in seconds (0, the default, for a fine clock; the web
    /// worklet's `Date.now()` steps by 1 ms): it sets the length of the peak windows
    /// ([`Host::add_render_time`]). Not finite or negative: 0.
    pub fn set_clock_resolution(&mut self, seconds: f32) {
        self.load.resolution = if seconds.is_finite() && seconds > 0.0 {
            seconds
        } else {
            0.0
        };
    }

    /// Voices sounding in the tracks' devices after the last block (releasing ones
    /// included).
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

    /// Frames between two reports, at the current rate ([`REPORTS_PER_SECOND`]).
    pub fn report_period(&self) -> u32 {
        self.report_frames
    }

    /// The report region: [`Feedback::max_size_for`] the track limit, in 32-bit words that
    /// hold the little-endian bytes of the `Feedback` layout. Its address and size never
    /// change.
    pub fn feedback(&self) -> &[AtomicU32] {
        &self.feedback
    }

    /// The report region, shared: a non-real-time thread (the helper's) reads it with
    /// [`copy_feedback`] while the audio thread writes it.
    pub fn feedback_shared(&self) -> Arc<[AtomicU32]> {
        Arc::clone(&self.feedback)
    }

    /// Starts folding the meters of every report into one set of levels over all the frames
    /// rendered from now on ([`Host::levels`]), for a caller that measures a whole render (the
    /// offline render). Allocates, so it is called before rendering, never on the audio
    /// thread. A second call starts again from zero. `Err` when the memory is refused.
    pub fn tally_levels(&mut self) -> Result<(), MsgError> {
        self.tally = Some(Tally::new()?);
        Ok(())
    }

    /// [`Host::tally_levels`] over one window of `frames` frames per strip (#755): track slot
    /// `s` folds the reports whose period starts in `[track_lead(s), track_lead(s) + frames)`
    /// frames from now, the output those that start in `[master_lead, master_lead + frames)`.
    /// The offline render gives each strip the latency of its signal, so that every strip
    /// measures the same stretch of the song. The caller cuts its blocks and writes a report
    /// ([`Host::write_feedback`]) at every edge, so that no period crosses one. Allocates:
    /// never on the audio thread. `Err` when the memory is refused.
    pub fn tally_levels_over(
        &mut self,
        frames: u64,
        track_lead: impl Fn(usize) -> u64,
        master_lead: u64,
    ) -> Result<(), MsgError> {
        let mut tally = Tally::new()?;
        let master_at = tally.from.len() - 1;
        for (at, from) in tally.from.iter_mut().enumerate().take(master_at) {
            *from = track_lead(at);
        }
        tally.from[master_at] = master_lead;
        tally.window = frames;
        self.tally = Some(tally);
        Ok(())
    }

    /// The levels folded since [`Host::tally_levels`] up to the last report written
    /// ([`Host::write_feedback`] writes the one that closes the period now); `None` when
    /// the tally was not started.
    pub fn levels(&self) -> Option<Levels> {
        self.tally.as_deref().map(Tally::levels)
    }

    /// Writes the report now and starts a new metering period (the render calls it about
    /// [`REPORTS_PER_SECOND`] times per second). The `seq` word is odd while the body is
    /// written: a reader takes an even `seq`, the body, then the same `seq` again.
    ///
    /// Written: version, transport state and playhead, CPU load and peak, epoch and the counters the
    /// prepare side published, the most voices sounding during the period, the meters of
    /// every track slot up to the last track and the output's, then the active clips (GR-1),
    /// then the take's number, count and last notes (RE-2), the disk's underruns, offline
    /// sources, budget flag and PCM in memory (AU-3), and the frames the output protection
    /// silenced because a sample was non-finite (#667), the host's frame at the playhead (#733),
    /// the device latency the graph compensates (#749), and after the notes the devices'
    /// readouts (#686). No allocation.
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

        let frames = self.meter_frames;
        let (tracks, master) = self.engine.take_meters(frames, &mut self.meters);
        if let Some(tally) = self.tally.as_deref_mut() {
            tally.add(frames, &self.meters[..tracks], master);
        }
        self.engine.report_transport(&mut self.scratch);
        let (load, peak) = self.load.take(self.sample_rate);

        let epoch = self.status.epoch.load(Ordering::Relaxed) as u16;
        let [e0, e1] = epoch.to_le_bytes();
        put(
            0,
            u32::from_le_bytes([FEEDBACK_VERSION, self.scratch.transport as u8, e0, e1]),
        );
        let tick = self.scratch.playhead_tick.0 as u64;
        put(8, tick as u32);
        put(12, (tick >> 32) as u32);
        let frame = self.scratch.playhead_frame.0;
        put(16, frame as u32);
        put(20, (frame >> 32) as u32);
        let host = self.scratch.host_frame;
        put(92, host as u32);
        put(96, (host >> 32) as u32);
        put(24, load.to_bits());
        put(28, self.status.live_overflows.load(Ordering::Relaxed));
        put(32, self.status.rejected.load(Ordering::Relaxed));
        let disk = self.engine.disk_stats();
        put(36, disk.underruns.load(Ordering::Relaxed));
        put(76, disk.offline.load(Ordering::Relaxed));
        put(80, disk.flags.load(Ordering::Relaxed));
        put(
            84,
            (disk.pcm_bytes.load(Ordering::Relaxed) / 1024).min(u64::from(u32::MAX)) as u32,
        );
        put(88, silenced_word(self.engine.limiter_silenced()));
        let graph = self.engine.graph();
        put(
            FEEDBACK_LATENCY_OFFSET,
            crate::msg::latency_word(graph.latency(), graph.latency_capped()),
        );
        put(40, tracks as u32);
        put(52, self.period_voices);
        let mut at = FEEDBACK_HEADER_BYTES;
        for m in self.meters[..tracks].iter().chain([&master]) {
            put(at, m.peak_l.to_bits());
            put(at + 4, m.peak_r.to_bits());
            put(at + 8, m.rms_l.to_bits());
            put(at + 12, m.rms_r.to_bits());
            put(at + 16, m.gain_reduction_db.to_bits());
            at += TRACK_METER_BYTES;
        }
        // The grid cells that are not idle, after the meters: at most two per track, which is
        // the room the region has for them.
        let mut clips = 0usize;
        self.engine.active_clips(|c| {
            if clips < 2 * tracks && (c.track.0 as usize) < tracks {
                put(at, c.track.0);
                put(at + 4, u32::from(c.scene.0) | (c.state as u32) << 16);
                put(at + 8, c.anchor.0 as u64 as u32);
                put(at + 12, ((c.anchor.0 as u64) >> 32) as u32);
                at += ACTIVE_CLIP_BYTES;
                clips += 1;
            }
        });
        put(44, clips as u32);
        // The take's last notes, after the clips (RE-2): the region has room for
        // `MAX_RECORDED_PER_REPORT`, the recorder keeps no more.
        let recorder = self.engine.recorder();
        self.idle_reports = if recorder.capturing() {
            0
        } else {
            self.idle_reports.saturating_add(1)
        };
        let carry = self.idle_reports <= TAKE_REPORTS;
        let mut recorded = 0u32;
        recorder.recent(|_, r| {
            if carry && (recorded as usize) < MAX_RECORDED_PER_REPORT {
                let start = r.note.start.0 as u64;
                let length = r.note.length.0 as u64;
                put(at, r.track.0);
                put(at + 4, 0);
                put(at + 8, start as u32);
                put(at + 12, (start >> 32) as u32);
                put(at + 16, length as u32);
                put(at + 20, (length >> 32) as u32);
                put(
                    at + 24,
                    u32::from_le_bytes([r.note.pitch, r.note.velocity, 0, 0]),
                );
                put(at + 28, 0);
                at += RECORDED_NOTE_WIRE_BYTES;
                recorded += 1;
            }
        });
        put(48, recorded);
        // The devices' readouts, after the notes (#686): the region has room for
        // `MAX_READOUTS_PER_REPORT`; past it the rest is left out.
        let mut readouts = 0u32;
        self.engine.device_readouts(|r| {
            if (readouts as usize) < MAX_READOUTS_PER_REPORT {
                put(at, r.track.0);
                put(at + 4, u32::from(r.device.0) | (r.param.0 << 16));
                put(at + 8, r.value.to_bits());
                at += DEVICE_READOUT_BYTES;
                readouts += 1;
            }
        });
        put(FEEDBACK_READOUTS_OFFSET, readouts);
        // The spectra the prepare side computed from the last windows (#156), after the readouts,
        // once each: a report carries only spectra published since the last one, so that a
        // stalled prepare side lets the page's spectra go stale rather than repeat. A write of
        // theirs under way leaves `self.spectra` partly written and this report without spectra.
        let fresh = match read_spectra(&self.spectrum_region, &mut self.spectra) {
            Some((seq, count)) if seq != self.spectra_seq => {
                self.spectra_seq = seq;
                count
            }
            _ => 0,
        };
        for s in &self.spectra[..fresh] {
            put(at, s.track);
            put(at + 4, s.device);
            put(at + 8, s.sample_rate.to_bits());
            for (i, b) in s.bands.iter().enumerate() {
                put(at + 12 + 4 * i, b.to_bits());
            }
            at += SPECTRUM_WIRE_BYTES;
        }
        put(FEEDBACK_SPECTRA_OFFSET, fresh as u32);
        put(56, recorder.take());
        put(60, recorder.total());
        put(64, recorder.dropped());
        put(68, u32::from(recorder.capturing()));
        put(72, peak.to_bits());
        self.meter_frames = 0;
        self.period_voices = self.voices;

        let done = seq.wrapping_add(1);
        if let Some(w) = fb.get(1) {
            w.store(word(done.to_le_bytes()), Ordering::Release);
        }
        self.feedback_seq = done;
        self.copy_windows();
    }

    /// The spectrum region this side reads (test builds only: a test makes a publication look
    /// torn).
    #[cfg(feature = "testing")]
    pub fn spectrum_region(&self) -> Arc<[AtomicU32]> {
        Arc::clone(&self.spectrum_region)
    }

    /// Copies the windows of at most [`MAX_SPECTRA`] analysing devices into the window region
    /// for the prepare side (#156, [`crate::analysis`]): the enabled devices of every chain in
    /// slot order, from the next one of the round robin, so that with more devices each one
    /// gets its turn. No allocation.
    fn copy_windows(&mut self) {
        let mut devices = 0usize;
        self.engine.enabled_devices(|_, _, _| devices += 1);
        let start = if devices == 0 {
            0
        } else {
            self.robin % devices
        };
        let region = &*self.windows;
        let writer = &mut self.window_writer;
        let window = &mut *self.window;
        let rate = self.sample_rate;
        writer.begin(region);
        // Two turns: from `start` to the end, then from the start to `start`.
        let mut taken = 0usize;
        let mut last = start;
        for pass in 0..2 {
            let mut i = 0usize;
            self.engine.enabled_devices(|track, device, entry| {
                let mine = if pass == 0 { i >= start } else { i < start };
                if mine && taken < MAX_SPECTRA {
                    // A device that answers more than it was given is cut to the scratch.
                    let n = entry.device.analysis_window(window).min(window.len());
                    if n > 0
                        && writer.push(
                            region,
                            Source {
                                track: track.0,
                                device: u32::from(device.0),
                            },
                            rate,
                            &window[..n],
                        )
                    {
                        taken += 1;
                        last = i;
                    }
                }
                i += 1;
            });
        }
        writer.end(region);
        self.robin = if taken == MAX_SPECTRA { last + 1 } else { 0 };
    }
}

/// The report's word for the protection's silenced-frame count (offset 88): the count,
/// saturated at `u32::MAX` (at 48 kHz, about a day of frames silenced).
pub fn silenced_word(frames: u64) -> u32 {
    frames.min(u64::from(u32::MAX)) as u32
}

/// The voices word of a report region (offset 52), as [`read_feedback`] reads it.
pub fn feedback_voices(region: &[AtomicU32]) -> u32 {
    region.get(13).map_or(0, |w| {
        u32::from_le_bytes(w.load(Ordering::Relaxed).to_ne_bytes())
    })
}

/// Bytes of a report of `tracks` tracks, `clips` active clips (at most two per track),
/// `recorded` notes, `readouts` device readouts and `spectra` spectra: the fixed part, the
/// track meters and the output's, the clips, the notes, the readouts, then the spectra.
fn report_len(tracks: u32, clips: u32, recorded: u32, readouts: u32, spectra: u32) -> usize {
    let tracks = (tracks as usize).min(limits::MAX_TRACKS as usize);
    let clips = (clips as usize).min(2 * tracks);
    let recorded = (recorded as usize).min(MAX_RECORDED_PER_REPORT);
    let readouts = (readouts as usize).min(MAX_READOUTS_PER_REPORT);
    let spectra = (spectra as usize).min(MAX_SPECTRA);
    Feedback::size_for(tracks, clips, recorded, readouts, spectra)
}

/// Copies the report of a region into `out` as the little-endian bytes of the `Feedback`
/// layout, for a thread that sends it elsewhere (the helper's link to the page). `false`,
/// with `out` untouched, while a write is in progress or was torn by one, or before the
/// first report. `out` is cleared and reuses its capacity.
pub fn copy_feedback(region: &[AtomicU32], out: &mut Vec<u8>) -> bool {
    let load = |i: usize| -> u32 {
        region.get(i).map_or(0, |w| {
            u32::from_le_bytes(w.load(Ordering::Relaxed).to_ne_bytes())
        })
    };
    let Some(seq) = region
        .get(1)
        .map(|w| u32::from_le_bytes(w.load(Ordering::Acquire).to_ne_bytes()))
    else {
        return false;
    };
    if seq == 0 || seq & 1 == 1 {
        return false;
    }
    let len = report_len(
        load(10),
        load(11),
        load(12),
        load(FEEDBACK_READOUTS_OFFSET / 4),
        load(FEEDBACK_SPECTRA_OFFSET / 4),
    )
    .min(region.len() * 4);
    out.clear();
    out.extend((0..len.div_ceil(4)).flat_map(|i| load(i).to_le_bytes()));
    out.truncate(len);
    fence(Ordering::Acquire);
    let again = region
        .get(1)
        .map(|w| u32::from_le_bytes(w.load(Ordering::Relaxed).to_ne_bytes()));
    if again == Some(seq) {
        true
    } else {
        out.clear();
        false
    }
}

/// Reads a report region as the page does (tests and the helper): `None` while a write is
/// in progress or was torn by one, else the decoded report.
pub fn read_feedback(region: &[AtomicU32]) -> Option<Feedback> {
    let mut bytes = Vec::new();
    if !copy_feedback(region, &mut bytes) {
        return None;
    }
    decode_feedback(&bytes)
}

/// Decodes the bytes of a report ([`copy_feedback`]): `None` for a version this code does
/// not read, a buffer shorter than its meters and active clips, or a clip state it does not
/// know.
pub fn decode_feedback(bytes: &[u8]) -> Option<Feedback> {
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let u64_at = |at: usize| -> Option<u64> {
        Some(u64::from(u32_at(at)?) | (u64::from(u32_at(at + 4)?) << 32))
    };
    if *bytes.first()? != FEEDBACK_VERSION {
        return None;
    }
    let tracks = u32_at(40)?;
    let clips = u32_at(44)?;
    let recorded = u32_at(48)?;
    let readouts = u32_at(FEEDBACK_READOUTS_OFFSET)?;
    let spectra = u32_at(FEEDBACK_SPECTRA_OFFSET)?;
    if tracks > limits::MAX_TRACKS
        || clips > 2 * tracks
        || recorded as usize > MAX_RECORDED_PER_REPORT
        || readouts as usize > MAX_READOUTS_PER_REPORT
        || spectra as usize > MAX_SPECTRA
        || bytes.len() < report_len(tracks, clips, recorded, readouts, spectra)
    {
        return None;
    }
    let meter = |i: usize| -> Option<TrackMeter> {
        let at = FEEDBACK_HEADER_BYTES + i * TRACK_METER_BYTES;
        Some(TrackMeter {
            peak_l: f32::from_bits(u32_at(at)?),
            peak_r: f32::from_bits(u32_at(at + 4)?),
            rms_l: f32::from_bits(u32_at(at + 8)?),
            rms_r: f32::from_bits(u32_at(at + 12)?),
            gain_reduction_db: f32::from_bits(u32_at(at + 16)?),
        })
    };
    let meters = (0..tracks as usize)
        .map(meter)
        .collect::<Option<Vec<_>>>()?;
    let clips_at = FEEDBACK_HEADER_BYTES + (tracks as usize + 1) * TRACK_METER_BYTES;
    let active_clips = (0..clips as usize)
        .map(|i| {
            let at = clips_at + i * ACTIVE_CLIP_BYTES;
            let scene = u32_at(at + 4)?;
            Some(ActiveClip {
                track: crate::msg::TrackSlot(u32_at(at)?),
                scene: crate::msg::SceneSlot(scene as u16),
                state: ClipState::from_byte((scene >> 16) as u8)?,
                anchor: crate::time::Tick(u64_at(at + 8)? as i64),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let notes_at = clips_at + clips as usize * ACTIVE_CLIP_BYTES;
    let recorded = (0..recorded as usize)
        .map(|i| {
            let at = notes_at + i * RECORDED_NOTE_WIRE_BYTES;
            let keys = u32_at(at + 24)?.to_le_bytes();
            Some(RecordedNote {
                track: TrackSlot(u32_at(at)?),
                note: Note {
                    start: crate::time::Tick(u64_at(at + 8)? as i64),
                    length: crate::time::Tick(u64_at(at + 16)? as i64),
                    pitch: keys[0],
                    velocity: keys[1],
                    ..Note::default()
                },
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let readouts_at = notes_at + recorded.len() * RECORDED_NOTE_WIRE_BYTES;
    let readouts = (0..readouts as usize)
        .map(|i| {
            let at = readouts_at + i * DEVICE_READOUT_BYTES;
            let ids = u32_at(at + 4)?;
            Some(DeviceReadout {
                track: TrackSlot(u32_at(at)?),
                device: DeviceSlot(ids as u16),
                param: ParamId(ids >> 16),
                value: f32::from_bits(u32_at(at + 8)?),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let spectra_at = readouts_at + readouts.len() * DEVICE_READOUT_BYTES;
    let spectra = (0..spectra as usize)
        .map(|i| {
            let at = spectra_at + i * SPECTRUM_WIRE_BYTES;
            let mut s = Spectrum {
                track: u32_at(at)?,
                device: u32_at(at + 4)?,
                sample_rate: f32::from_bits(u32_at(at + 8)?),
                ..Spectrum::default()
            };
            for (k, b) in s.bands.iter_mut().enumerate() {
                *b = f32::from_bits(u32_at(at + 12 + 4 * k)?);
            }
            Some(s)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Feedback {
        transport: match bytes[1] {
            1 => TransportState::Playing,
            2 => TransportState::Recording,
            _ => TransportState::Stopped,
        },
        epoch: u16::from_le_bytes([bytes[2], bytes[3]]),
        seq: u32_at(4)?,
        playhead_tick: crate::time::Tick(u64_at(8)? as i64),
        playhead_frame: crate::time::Frame(u64_at(16)?),
        host_frame: u64_at(92)?,
        cpu_load: f32::from_bits(u32_at(24)?),
        cpu_peak: f32::from_bits(u32_at(72)?),
        live_overflows: u32_at(28)?,
        rejected: u32_at(32)?,
        disk_underruns: u32_at(36)?,
        offline_sources: u32_at(76)?,
        pcm_flags: u32_at(80)?,
        pcm_kib: u32_at(84)?,
        silenced_frames: u32_at(88)?,
        latency_frames: u32_at(FEEDBACK_LATENCY_OFFSET)? & !crate::msg::FEEDBACK_LATENCY_CAPPED,
        latency_capped: u32_at(FEEDBACK_LATENCY_OFFSET)? & crate::msg::FEEDBACK_LATENCY_CAPPED != 0,
        meters,
        master: meter(tracks as usize)?,
        voices: u32_at(52)?,
        active_clips,
        recorded,
        record_take: u32_at(56)?,
        record_total: u32_at(60)?,
        record_dropped: u32_at(64)?,
        record_capturing: u32_at(68)? & 1 == 1,
        readouts,
        spectra,
    })
}

/// Test scenarios of the host (test builds only).
#[cfg(feature = "testing")]
pub mod testing {
    use super::{channel, copy_feedback};

    /// The non-finite scenario (#667): a host at 48 kHz renders clean blocks and writes a
    /// report, then 3 frames turned NaN before the output protection and a report, then a whole
    /// block of 128 and a report. Returns the three reports' bytes, end to end, then the left
    /// and right output of the last block as little-endian `f32` bytes: the reports carry 0, 3
    /// and 131 silenced frames, the output is finite. Empty if the channel could not be built.
    pub fn nonfinite_scenario() -> Vec<u8> {
        let Ok((_prepare, mut host)) = channel(48_000.0) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut report = Vec::new();
        let mut take = |host: &mut super::Host, out: &mut Vec<u8>| {
            host.write_feedback();
            if copy_feedback(host.feedback(), &mut report) {
                out.extend_from_slice(&report);
            }
        };
        host.render(128);
        host.render(128);
        take(&mut host, &mut out);
        host.engine_mut().poison_next_frames(3);
        host.render(128);
        take(&mut host, &mut out);
        host.engine_mut().poison_next_frames(128);
        host.render(128);
        take(&mut host, &mut out);
        for s in host.left()[..128].iter().chain(&host.right()[..128]) {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    /// The spectrum scenario (#156): a host at 48 kHz with a track playing the synth through
    /// two Contours (the first lifting 2 kHz by 12 dB), rendered for half a second in blocks of
    /// 128 with the prepare side flushed between blocks, as the hosts do. Returns the bytes of
    /// the last report, its spectra included. Empty if the channel could not be built.
    pub fn spectrum_scenario() -> Vec<u8> {
        use crate::devices::contour::{self, BandParam};
        use crate::msg::{ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot};
        let Ok((mut prepare, mut host)) = channel(48_000.0) else {
            return Vec::new();
        };
        let (Some(gain), Some(freq)) = (
            contour::band_param(contour::BELL_2, BandParam::Gain),
            contour::band_param(contour::BELL_2, BandParam::Freq),
        ) else {
            return Vec::new();
        };
        let t = TrackSlot(0);
        let stream = crate::msg::encode_stream(
            1,
            0,
            &[
                M::Reset { epoch: 1 },
                M::TrackCreate {
                    track: t,
                    kind: TrackKind::Midi,
                },
                M::DeviceInsert {
                    track: t,
                    device: DeviceSlot(0),
                    kind: crate::OSC_SYNTH.kind_id,
                    state: Vec::new(),
                },
                M::DeviceInsert {
                    track: t,
                    device: DeviceSlot(1),
                    kind: contour::DESCRIPTOR.kind_id,
                    state: crate::msg::encode_state(&[
                        (ParamId(gain), 12.0),
                        (ParamId(freq), 2000.0),
                    ]),
                },
                M::DeviceInsert {
                    track: t,
                    device: DeviceSlot(2),
                    kind: contour::DESCRIPTOR.kind_id,
                    state: Vec::new(),
                },
                M::NoteOn {
                    track: t,
                    pitch: 45,
                    velocity: 110,
                    at: None,
                },
            ],
        );
        let Ok(stream) = stream else {
            return Vec::new();
        };
        if prepare.submit(&stream).is_err() {
            return Vec::new();
        }
        for _ in 0..190 {
            host.render(128);
            prepare.flush();
        }
        let mut report = Vec::new();
        if copy_feedback(host.feedback(), &mut report) {
            report
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tally_tests {
    use super::*;

    fn meter(peak: f32, rms: f32) -> TrackMeter {
        TrackMeter {
            peak_l: peak,
            peak_r: peak / 2.0,
            rms_l: rms,
            rms_r: rms,
            gain_reduction_db: 0.0,
        }
    }

    #[test]
    fn periods_fold_into_the_largest_peak_and_a_root_mean_square_weighted_by_frames() {
        let mut tally = Tally::new().unwrap();
        // 100 frames at RMS 1, then 300 frames of silence: the mean square is 100/400.
        tally.add(100, &[meter(0.5, 1.0)], meter(0.9, 1.0));
        tally.add(300, &[meter(0.25, 0.0)], meter(0.1, 0.0));
        let levels = tally.levels();
        assert_eq!(levels.frames, 400);
        assert_eq!(levels.tracks.len(), 1);
        let t = levels.tracks[0];
        assert_eq!((t.peak_l, t.peak_r), (0.5, 0.25));
        assert!((t.rms_l - 0.5).abs() < 1e-6, "{}", t.rms_l);
        assert!((levels.master.peak_l - 0.9).abs() < 1e-6);
        assert!((levels.master.rms_r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_period_without_frames_weighs_nothing() {
        let mut tally = Tally::new().unwrap();
        tally.add(0, &[meter(0.0, 0.0)], meter(0.0, 0.0));
        let levels = tally.levels();
        assert_eq!(levels.frames, 0);
        assert!(levels.master.rms_l == 0.0 && levels.master.peak_l == 0.0);
    }
}
