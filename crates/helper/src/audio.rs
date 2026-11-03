//! Real-time side of the helper: the audio callback and what feeds it (MO-2).
//!
//! The helper runs the same engine channel as the web build (`engine::host`) and holds no
//! project (BO-1): the page owns it in both modes, runs every command in its project core and
//! sends the helper the `ControlMsg` bytes it encoded, the same bytes its own engine takes.
//! The helper submits each stream unchanged to the prepare side ([`EventSender`], behind a
//! mutex, non-real-time threads only); a control thread flushes it every few milliseconds
//! ([`spawn_flusher`]). A stream longer than the engine takes at once (a big project's
//! snapshot) waits in the sender's queue and is sent in pieces as the engine consumes it
//! (`engine::host::StreamQueue`, #245). The rt messages and the live notes reach the audio
//! callback through the engine's lock-free rings.
//!
//! A stream the engine stops (corrupt, out of memory, past the waiting bound) is recorded as a
//! [`Refusal`] and told to the page, which starts the engine over from a snapshot: the helper
//! cannot, it has no project. The helper writes two streams of its own: when a page leaves or
//! the helper quits, the transport stops and every track the engine holds releases its notes
//! ([`EventSender::release_all`]); and the notes of its MIDI input, on the tracks the page's
//! table names ([`EventSender::submit_live`], `crate::live`, #598).
//!
//! Everything in [`AudioCore::process`] runs on the audio thread: it never allocates,
//! locks or blocks. A full live queue releases every note (the engine's live overflow), so
//! a lost note-off can never leave a note hanging.

use engine::host::{Host, StreamQueue};
use engine::msg::{ControlMsg, Header, MsgError, Tag};
use engine::time::SampleRate;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Frames rendered per chunk; a longer callback is rendered in several chunks.
pub const CHUNK_FRAMES: usize = engine::msg::limits::MAX_BLOCK_FRAMES;

/// How often the control thread pushes what waited and drops the garbage.
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(5);

/// Values the audio thread publishes for the UI.
#[derive(Default)]
pub struct Meter {
    /// Peak since the last read, as `f32` bits (positive floats order like their bits).
    peak_bits: AtomicU32,
    voices: AtomicU32,
    /// Frames per callback, 0 until the first callback.
    callback_frames: AtomicU32,
    /// Delay between the callback and the moment its first sample is played, as the
    /// driver reports it, in microseconds (0 when the backend reports nothing).
    output_delay_us: AtomicU32,
    /// The engine's report region (MO-5), set by [`audio_core`]; a rebuilt core replaces it.
    report: Mutex<Option<Arc<[AtomicU32]>>>,
}

impl Meter {
    /// Returns the peak since the last call and resets it, plus the voice count (the
    /// synth's voices sounding, releasing ones included, after the last callback).
    pub fn take(&self) -> (f32, u32) {
        let peak = f32::from_bits(self.peak_bits.swap(0, Ordering::Relaxed));
        (peak, self.voices.load(Ordering::Relaxed))
    }

    /// Copies the engine's latest whole report into `out` (the bytes of `engine::msg::Feedback`).
    /// `false` before the first report or while one is being written. Not for the audio thread.
    pub fn copy_report(&self, out: &mut Vec<u8>) -> bool {
        let region = self.report.lock().ok().and_then(|r| r.clone());
        region.is_some_and(|r| engine::host::copy_feedback(&r, out))
    }

    pub fn callback_frames(&self) -> Option<u32> {
        match self.callback_frames.load(Ordering::Relaxed) {
            0 => None,
            n => Some(n),
        }
    }

    /// Records the driver's output delay. Allocation-free: called from the audio callback.
    pub fn set_output_delay(&self, delay: Duration) {
        let us = delay.as_micros().min(u128::from(u32::MAX)) as u32;
        self.output_delay_us.store(us, Ordering::Relaxed);
    }

    /// Forgets the output that went away: its report, its callback size and its delay, so that
    /// no stale report or latency reaches a later session (#525).
    pub fn reset(&self) {
        if let Ok(mut report) = self.report.lock() {
            *report = None;
        }
        self.forget_output();
    }

    /// Forgets the output that went away but keeps the engine's report region: the engine is
    /// kept for the next output (#577), whose callback size and delay are not known yet.
    pub fn forget_output(&self) {
        self.peak_bits.store(0, Ordering::Relaxed);
        self.voices.store(0, Ordering::Relaxed);
        self.callback_frames.store(0, Ordering::Relaxed);
        self.output_delay_us.store(0, Ordering::Relaxed);
    }

    /// Estimated output latency in ms: one callback buffer (an event waits for the next
    /// callback) plus the driver's output delay. An estimate from what the driver
    /// reports, never a measurement. `None` before the first callback.
    pub fn estimated_latency_ms(&self, sample_rate: f32) -> Option<f32> {
        let frames = self.callback_frames()?;
        let buffer_ms = frames as f32 * 1000.0 / sample_rate;
        let delay_ms = self.output_delay_us.load(Ordering::Relaxed) as f32 / 1000.0;
        Some(buffer_ms + delay_ms)
    }
}

/// A stream the engine stopped (anything but a busy engine, which only waits): the code of
/// its error (`MsgError::code`) and the byte of the message that stopped it, in the piece the
/// engine was given (a stream sent in pieces counts from the rest being retried; informative).
/// The engine is behind the page's project until the page sends a snapshot. `epoch` is the engine's at that moment: the page's snapshot goes in a newer one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: i32,
    pub at: u32,
    pub epoch: u16,
}

/// The prepare side of the helper's engine behind its queue of streams. Owned behind a mutex
/// by non-real-time threads only.
pub struct EventSender {
    /// The prepare side behind its queue of streams (#245).
    queue: StreamQueue,
    /// The first refusal found at a flush and not yet told to the page (a later one says
    /// nothing more: the page answers any with a snapshot). A refusal at a submit is returned
    /// to the caller instead, which answers the page at once.
    refused: Option<Refusal>,
    /// Streams submitted so far (tests wait on it).
    received: u64,
    /// The epoch the engine is in once what waits is taken: the last `Reset` queued, or the
    /// engine's own. A page stream must start in it (or, with a `Reset`, in a newer one).
    epoch: u16,
}

impl EventSender {
    fn new(queue: StreamQueue) -> Self {
        let epoch = queue.prepare().preparer().epoch();
        EventSender {
            queue,
            refused: None,
            received: 0,
            epoch,
        }
    }

    /// Submits a stream of the page, unchanged. What the busy engine does not take yet waits
    /// for the flushes. A stream it stops is refused: returned, for the caller to tell the page.
    ///
    /// A stream whose first message is not of the engine's epoch (a `Reset` that is not newer,
    /// a page's change made for another engine) is refused whole as a stale epoch, before the
    /// engine sees it: the engine would skip every message of it silently, and the page would
    /// never learn that the helper plays something else (a snapshot overtaken by another
    /// page's, say). The page answers with a snapshot newer than the refusal's epoch.
    pub fn submit(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
        self.received += 1;
        if let Err(error @ MsgError::StaleEpoch { .. }) = Header::decode(bytes, self.epoch) {
            return Err(self.refuse_at(error, 0));
        }
        let reset = Header::decode(bytes, self.epoch)
            .ok()
            .filter(|h| h.tag == Tag::Reset)
            .map(|h| h.epoch);
        let sent = self.queue.send(bytes);
        if let Some(epoch) = reset
            && sent.is_ok()
        {
            self.epoch = epoch;
        }
        sent.map_err(|error| self.refuse(error))
    }

    /// Submits a stream the helper encoded for a note of its own MIDI input (#598,
    /// `crate::live`): behind what waits, like a page's stream, but not counted as one. An
    /// error is kept for the ticker to tell the page, like one found at a flush: it may be a
    /// waiting page stream's, which this call drained.
    pub fn submit_live(&mut self, bytes: &[u8]) {
        if let Err(error) = self.queue.send(bytes) {
            let refusal = self.refuse(error);
            self.refused.get_or_insert(refusal);
        }
    }

    /// Pushes what waited on the rt ring, then the streams that waited for the engine, and
    /// drops the garbage. A waiting stream stopped for another reason than a busy engine is
    /// recorded as a refusal.
    pub fn flush(&mut self) {
        if let Err(error) = self.queue.flush() {
            let refusal = self.refuse(error);
            self.refused.get_or_insert(refusal);
        }
    }

    fn refuse(&mut self, error: MsgError) -> Refusal {
        let at = self.queue.prepare().preparer().stopped_at().unwrap_or(0);
        // A `Reset` the engine took before the stream stopped moved it on: the newer of the
        // two is the epoch the page's snapshot must pass.
        let engine = self.queue.prepare().preparer().epoch();
        if engine.wrapping_sub(self.epoch) as i16 > 0 {
            self.epoch = engine;
        }
        self.refuse_at(error, at)
    }

    fn refuse_at(&self, error: MsgError, at: usize) -> Refusal {
        crate::log!("the engine refused a stream: {error:?}");
        Refusal {
            code: error.code(),
            at: u32::try_from(at).unwrap_or(u32::MAX),
            epoch: self.epoch,
        }
    }

    /// The refusal not yet told to the page, forgotten once taken.
    pub fn take_refusal(&mut self) -> Option<Refusal> {
        self.refused.take()
    }

    /// The page is gone (or the helper quits): what waits is dropped, then the transport
    /// stops and every track the engine holds releases its notes, in the engine's current
    /// epoch. The project stays loaded until the next page's snapshot replaces it. With the
    /// notes of its MIDI input ([`EventSender::submit_live`]), the only stream the helper
    /// writes itself.
    pub fn release_all(&mut self) {
        self.queue.clear();
        self.refused = None;
        let preparer = self.queue.prepare().preparer();
        // What waited is gone, a `Reset` among it too: the engine's own epoch is the one.
        let epoch = preparer.epoch();
        self.epoch = epoch;
        // The notes first: live events, they go straight to the live queue even when the
        // engine is busy; the stop may wait for room behind what the engine still prepares.
        let mut msgs: Vec<ControlMsg> = preparer
            .track_slots()
            .map(|track| ControlMsg::AllNotesOff { track })
            .collect();
        msgs.push(ControlMsg::Stop);
        let sent = engine::msg::encode_stream(epoch, 0, &msgs).and_then(|b| self.queue.send(&b));
        if let Err(error) = sent {
            crate::log!("the notes could not be released: {error:?}");
        }
    }

    /// The epoch of the engine once what waits is taken (its last `Reset`): a page's snapshot
    /// must be newer.
    pub fn epoch(&self) -> u16 {
        self.epoch
    }

    /// Messages the engine rejected so far.
    pub fn rejected(&self) -> u32 {
        self.queue.prepare().preparer().rejected()
    }

    /// Changes the engine's sample rate to `hz`, keeping its playhead and its project (#577):
    /// every device is built again for it off the audio thread
    /// (`StreamQueue::set_sample_rate`). A call at the current rate does nothing; nothing
    /// changes on an error.
    pub fn set_sample_rate(&mut self, hz: f32) -> Result<(), MsgError> {
        // A rate the driver gives is a whole number of hertz; 0 is refused here, any other
        // rate below the devices' lowest by the prepare side.
        let rate = SampleRate::new(hz as u32).ok_or(MsgError::LimitExceeded {
            what: "sample rate of zero",
            limit: 1,
            requested: 0,
        })?;
        self.queue.set_sample_rate(rate)
    }

    /// Streams waiting for the engine to take them.
    pub fn waiting(&self) -> usize {
        self.queue.waiting()
    }

    /// Streams submitted so far.
    pub fn received(&self) -> u64 {
        self.received
    }
}

/// The engine of the open audio output, if any (#525): the prepare side and the rate it runs
/// at. The server, the control thread and Quit reach the engine through it; the output
/// supervisor (`crate::output`) fills it when an output opens and empties it when the output
/// goes away. While it is empty, streams go nowhere: nothing can sound. Each engine installed
/// gets a new generation, so that a session opened on one never feeds the next (BO-1).
/// Non-real-time threads only.
#[derive(Default)]
pub struct EngineSlot {
    running: Mutex<Option<Running>>,
}

struct Running {
    sender: EventSender,
    sample_rate: f32,
    generation: u64,
}

/// Generations of installed engines, across slots (0 is never one).
static GENERATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl EngineSlot {
    /// Runs `f` on the sender of the open output; `None` when there is none. A lock poisoned
    /// by a panic is taken anyway (a note off must never be skipped).
    pub fn with<R>(&self, f: impl FnOnce(&mut EventSender) -> R) -> Option<R> {
        let mut running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        running.as_mut().map(|r| f(&mut r.sender))
    }

    /// Runs `f` on the sender of the open output when its engine is of `generation`; `None`
    /// otherwise.
    pub fn with_generation<R>(
        &self,
        generation: u64,
        f: impl FnOnce(&mut EventSender) -> R,
    ) -> Option<R> {
        let mut running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        running
            .as_mut()
            .filter(|r| r.generation == generation)
            .map(|r| f(&mut r.sender))
    }

    /// The generation and the epoch of the open output's engine; `None` when there is none.
    pub fn current(&self) -> Option<(u64, u16)> {
        let running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        running.as_ref().map(|r| (r.generation, r.sender.epoch()))
    }

    /// Submits a page's stream to the engine of generation `generation`: `None` when that
    /// engine is not the open one any more (dropped), else what the engine made of it.
    pub fn submit(&self, generation: u64, bytes: &[u8]) -> Option<Result<(), Refusal>> {
        let mut running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        running
            .as_mut()
            .filter(|r| r.generation == generation)
            .map(|r| r.sender.submit(bytes))
    }

    /// Stops the transport and releases every note of the open output's engine
    /// ([`EventSender::release_all`]).
    pub fn release_all(&self) {
        self.with(EventSender::release_all);
    }

    /// Pushes what waited on the rt ring and drops the garbage, when an output is open.
    pub fn flush(&self) {
        self.with(EventSender::flush);
    }

    /// The rate of the open output; `None` when there is none.
    pub fn sample_rate(&self) -> Option<f32> {
        let running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        running.as_ref().map(|r| r.sample_rate)
    }

    pub fn is_open(&self) -> bool {
        self.sample_rate().is_some()
    }

    /// Makes `sender` the engine of the output just opened at `sample_rate`.
    pub fn install(&self, sender: EventSender, sample_rate: f32) {
        let generation = GENERATIONS.fetch_add(1, Ordering::Relaxed);
        *self.running.lock().unwrap_or_else(|p| p.into_inner()) = Some(Running {
            sender,
            sample_rate,
            generation,
        });
    }

    /// Changes the rate of the engine in the slot to `hz` ([`EventSender::set_sample_rate`],
    /// #577), for the output it is about to play on. `NoSuchTarget` when the slot is empty;
    /// nothing changes on an error.
    pub fn set_sample_rate(&self, hz: f32) -> Result<(), MsgError> {
        let mut running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        let running = running.as_mut().ok_or(MsgError::NoSuchTarget)?;
        running.sender.set_sample_rate(hz)?;
        running.sample_rate = hz;
        Ok(())
    }

    /// The output went away: drops its engine's prepare side.
    pub fn remove(&self) {
        let old = self
            .running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        drop(old);
    }
}

/// The audio thread's side.
pub struct AudioCore {
    host: Host,
    meter: Arc<Meter>,
}

/// Creates the audio core and the sender that feeds it: an empty engine in epoch 0, silent
/// until a page sends its snapshot (BO-1: the helper holds no project).
pub fn audio_core(
    sample_rate: f32,
    meter: Arc<Meter>,
) -> Result<(AudioCore, EventSender), MsgError> {
    let (prepare, host) = engine::host::channel(sample_rate)?;
    let sender = EventSender::new(StreamQueue::new(prepare));
    if let Ok(mut report) = meter.report.lock() {
        *report = Some(host.feedback_shared());
    }
    Ok((AudioCore { host, meter }, sender))
}

/// The control thread: flushes the engine of the open output every [`FLUSH_INTERVAL`] until
/// the slot is dropped.
pub fn spawn_flusher(slot: &Arc<EngineSlot>) -> std::thread::JoinHandle<()> {
    let slot: Weak<EngineSlot> = Arc::downgrade(slot);
    std::thread::spawn(move || {
        while let Some(slot) = slot.upgrade() {
            slot.flush();
            drop(slot);
            std::thread::sleep(FLUSH_INTERVAL);
        }
    })
}

impl AudioCore {
    /// The engine this core renders, for tests that read its clock.
    #[cfg(test)]
    pub(crate) fn engine(&self) -> &engine::engine::Engine {
        self.host.engine()
    }

    /// The values this core publishes for the UI.
    pub fn meter(&self) -> &Meter {
        &self.meter
    }

    /// Fills an interleaved output buffer of `channels` channels.
    /// Channel 0 gets the left signal, channel 1 the right one, others silence.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let frames = out.len() / channels;
        self.meter
            .callback_frames
            .store(frames as u32, Ordering::Relaxed);
        let mut peak = 0.0f32;
        let mut done = 0;
        while done < frames {
            // The CPU load (#486): each chunk's render timed with the monotonic clock (a
            // user-mode counter read on the supported systems; no allocation, no lock), its
            // time added with its frames. The copy below is not counted.
            let start = Instant::now();
            let n = self.host.render((frames - done).min(CHUNK_FRAMES));
            self.host
                .add_render_time(start.elapsed().as_secs_f32(), n as u32);
            let (left, right) = (self.host.left(), self.host.right());
            for i in 0..n {
                let frame = &mut out[(done + i) * channels..(done + i + 1) * channels];
                for (c, sample) in frame.iter_mut().enumerate() {
                    *sample = match c {
                        0 => left[i],
                        1 => right[i],
                        _ => 0.0,
                    };
                }
                peak = peak.max(left[i].abs());
            }
            done += n.max(1);
        }
        // Samples past the last whole frame (malformed buffer) are silenced.
        for sample in &mut out[frames * channels..] {
            *sample = 0.0;
        }
        self.meter
            .peak_bits
            .fetch_max(peak.to_bits(), Ordering::Relaxed);
        self.meter
            .voices
            .store(self.host.voices(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests;
