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
//! caller times its renders ([`Host::add_render_time`]), the CPU load. Not produced yet,
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

use crate::device::{DeviceSetup, Factory};
use crate::engine::{Engine, Preparer, channel as engine_channel};
use crate::msg::{
    ACTIVE_CLIP_BYTES, ActiveClip, ClipState, DeviceKind, FEEDBACK_HEADER_BYTES, FEEDBACK_VERSION,
    Feedback, HEADER_BYTES, MAX_RECORDED_PER_REPORT, MSG_VERSION, MsgError, Note,
    RECORDED_NOTE_WIRE_BYTES, RecordedNote, TRACK_METER_BYTES, Tag, TrackMeter, TrackSlot,
    TransportState, limits,
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

/// The CPU load of a report whose period was not timed (no clock on the host).
pub const NOT_MEASURED: f32 = -1.0;

/// Reports that still carry a take's notes after its capture ended: about 3 s, longer than the
/// page waits for them, so that a finished take does not ride in every report until the next.
pub const TAKE_REPORTS: u32 = 90;

/// Reports per second (the page reads the region at about this rate).
pub const REPORTS_PER_SECOND: f32 = 30.0;

/// The audio side (module doc).
pub struct Host {
    engine: Engine,
    status: Arc<Status>,
    sample_rate: f32,
    left: Box<[f32]>,
    right: Box<[f32]>,
    /// Frames rendered since the last report: the meters' period.
    meter_frames: u32,
    /// Seconds the caller spent rendering since the last report ([`Host::add_render_time`]).
    busy_secs: f32,
    /// Whether the caller timed a render since the last report: else the load is not measured.
    timed: bool,
    /// Where the engine puts the track meters of a report.
    meters: Box<[TrackMeter]>,
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
    /// The clock's rate the host last saw (#236): a change makes the report period and the
    /// load follow it.
    clock_rate: u32,
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
    let host = Host {
        engine,
        status: Arc::clone(&status),
        sample_rate,
        left: try_slice(frames, 0.0)?,
        right: try_slice(frames, 0.0)?,
        meter_frames: 0,
        busy_secs: 0.0,
        timed: false,
        meters: try_slice(limits::MAX_TRACKS as usize, TrackMeter::default())?,
        scratch: Feedback::default(),
        voices: 0,
        period_voices: 0,
        idle_reports: 0,
        report_frames: report_frames(sample_rate),
        feedback: Arc::from(feedback.into_boxed_slice()),
        feedback_seq: 0,
        clock_rate: 0,
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

    /// Adds `seconds` of render time to the reporting period: the caller (the helper's audio
    /// callback) times its renders with its own clock, the engine has none. The report's CPU
    /// load is this time over the period's duration. A host without a clock (the web
    /// worklet) never calls it and the load reads [`NOT_MEASURED`].
    pub fn add_render_time(&mut self, seconds: f32) {
        self.timed = true;
        if seconds.is_finite() && seconds > 0.0 {
            self.busy_secs += seconds;
        }
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

    /// Writes the report now and starts a new metering period (the render calls it about
    /// [`REPORTS_PER_SECOND`] times per second). The `seq` word is odd while the body is
    /// written: a reader takes an even `seq`, the body, then the same `seq` again.
    ///
    /// Written: version, transport state and playhead, CPU load, epoch and the counters the
    /// prepare side published, the most voices sounding during the period, the meters of
    /// every track slot up to the last track and the output's, then the active clips (GR-1),
    /// then the take's number, count and last notes (RE-2). Not produced yet, written as
    /// zero: the disk underruns (AU-3).
    /// No allocation.
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
        self.engine.report_transport(&mut self.scratch);
        let load = if !self.timed {
            NOT_MEASURED
        } else if self.sample_rate > 0.0 && frames > 0 {
            let period = frames as f32 / self.sample_rate;
            (self.busy_secs / period).clamp(0.0, 1.0)
        } else {
            0.0
        };

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
        put(24, load.to_bits());
        put(28, self.status.live_overflows.load(Ordering::Relaxed));
        put(32, self.status.rejected.load(Ordering::Relaxed));
        put(36, 0);
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
        put(56, recorder.take());
        put(60, recorder.total());
        put(64, recorder.dropped());
        put(68, u32::from(recorder.capturing()));
        self.meter_frames = 0;
        self.busy_secs = 0.0;
        self.timed = false;
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

/// Bytes of a report of `tracks` tracks, `clips` active clips (at most two per track) and
/// `recorded` notes: the fixed part, the track meters and the output's, the clips, then the
/// notes.
fn report_len(tracks: u32, clips: u32, recorded: u32) -> usize {
    let tracks = (tracks as usize).min(limits::MAX_TRACKS as usize);
    let clips = (clips as usize).min(2 * tracks);
    let recorded = (recorded as usize).min(MAX_RECORDED_PER_REPORT);
    Feedback::size_for(tracks, clips, recorded)
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
    let len = report_len(load(10), load(11), load(12)).min(region.len() * 4);
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
    if tracks > limits::MAX_TRACKS
        || clips > 2 * tracks
        || recorded as usize > MAX_RECORDED_PER_REPORT
        || bytes.len() < report_len(tracks, clips, recorded)
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
        cpu_load: f32::from_bits(u32_at(24)?),
        live_overflows: u32_at(28)?,
        rejected: u32_at(32)?,
        disk_underruns: u32_at(36)?,
        meters,
        master: meter(tracks as usize)?,
        voices: u32_at(52)?,
        active_clips,
        recorded,
        record_take: u32_at(56)?,
        record_total: u32_at(60)?,
        record_dropped: u32_at(64)?,
        record_capturing: u32_at(68)? & 1 == 1,
    })
}
