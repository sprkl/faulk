//! Real-time side of the helper: the audio callback and what feeds it (MO-2).
//!
//! The helper runs the same engine channel as the web build (`engine::host`): the commands
//! it receives (from the page, from the MIDI input) are mapped by the command layer
//! (`commands::engine_msgs`) and encoded into the same `ControlMsg` byte stream the page
//! sends to its prepare Worker, then submitted to the prepare side ([`EventSender`], behind
//! a mutex, non-real-time threads only); a control thread flushes it every few
//! milliseconds ([`spawn_flusher`]). A stream longer than the engine takes at once (a big
//! project's snapshot) waits in the sender's queue and is sent in pieces as the engine
//! consumes it (`engine::host::StreamQueue`, #245). The rt messages and the live notes reach the audio
//! callback through the engine's lock-free rings.
//!
//! Everything in [`AudioCore::process`] runs on the audio thread: it never allocates,
//! locks or blocks. A full live queue releases every note (the engine's live overflow), so
//! a lost note-off can never leave a note hanging.
//!
//! The page still sends commands, not engine bytes, over the local WebSocket: moving the
//! encoding to the page side of that link is BO-1's.

use commands::Command;
use commands::engine_msgs::{SyncError, Synced};
use engine::host::{Host, StreamQueue};
use engine::msg::MsgError;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// Frames rendered per chunk; a longer callback is rendered in several chunks.
pub const CHUNK_FRAMES: usize = engine::msg::limits::MAX_BLOCK_FRAMES;

/// How often the control thread pushes what waited and drops the garbage.
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(5);

/// Flushes between two tries of a resync that could not be sent (about a second).
const RESYNC_RETRY_FLUSHES: u32 = 200;

/// The epoch of the helper's engine: newer than a fresh channel's 0, so the setup's
/// `Reset` is taken.
const EPOCH: u16 = 1;

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

/// The prepare side of the helper's engine and its stream encoder. Owned behind a mutex by
/// non-real-time threads only.
pub struct EventSender {
    /// The prepare side behind its queue of streams (#245).
    queue: StreamQueue,
    /// The one-synth project the commands run on, and the engine's stream (PR-3).
    stream: Synced,
    /// A resync is due: flushes left before it is tried ([`RESYNC_RETRY_FLUSHES`]).
    behind: Option<u32>,
    /// Flushes since the last resync was tried: two resyncs are at least
    /// [`RESYNC_RETRY_FLUSHES`] apart, so that a snapshot the engine keeps refusing does not
    /// make the flusher encode the whole project again at every tick.
    since_resync: u32,
}

impl EventSender {
    /// Runs a command on the project and sends its engine messages, encoded by the
    /// command layer, as bytes: the web engine's path. A command the project refuses is
    /// logged. Messages that did not encode, or a stream the engine stopped (out of memory,
    /// a corrupt stream, too much waiting for an engine that does not consume), leave the
    /// engine behind the project: logged, and the engine is resynced.
    pub fn send(&mut self, command: &Command) {
        let bytes = match self.stream.run(command) {
            Ok((_, bytes)) => bytes,
            Err(SyncError::Refused(error)) => {
                crate::log!("the command was refused: {error:?}");
                return;
            }
            Err(SyncError::Stream(error)) => {
                crate::log!("the command's engine messages did not encode: {error:?}");
                self.fall_behind();
                return;
            }
        };
        self.submit(&bytes);
    }

    /// Sends a command's bytes: a stream the engine refuses is logged and the engine resynced.
    fn submit(&mut self, bytes: &[u8]) {
        if let Err(error) = self.queue.send(bytes) {
            crate::log!("the engine refused a command: {error:?}");
            self.fall_behind();
        }
    }

    /// Pushes what waited on the rt ring, then the streams that waited for the engine, and
    /// drops the garbage. A waiting stream stopped for another reason than a busy engine is
    /// logged and the engine resynced; a resync that could not be sent is tried again.
    pub fn flush(&mut self) {
        self.since_resync = self.since_resync.saturating_add(1);
        if let Some(left) = self.behind {
            if left == 0 {
                self.resync();
            } else {
                self.behind = Some(left - 1);
            }
        }
        if let Err(error) = self.queue.flush() {
            crate::log!("the engine refused a stream: {error:?}");
            self.fall_behind();
        }
    }

    /// The engine is behind the project: resync it now, or once [`RESYNC_RETRY_FLUSHES`]
    /// flushes have passed since the last resync.
    fn fall_behind(&mut self) {
        if self.since_resync >= RESYNC_RETRY_FLUSHES {
            self.resync();
        } else if self.behind.is_none() {
            // Counted down by the flushes to come: the try falls on the one that ends the pause.
            self.behind = Some((RESYNC_RETRY_FLUSHES - self.since_resync).saturating_sub(1));
        }
    }

    /// The engine missed messages: what waits is dropped (the `Reset` would wipe it) and a
    /// `Reset` with the project's snapshot goes in a new epoch, as the page does after a
    /// stream its engine refused.
    fn resync(&mut self) {
        self.since_resync = 0;
        self.queue.clear();
        let sent = self
            .stream
            .resync()
            .and_then(|bytes| self.queue.send(&bytes));
        self.behind = match sent {
            Ok(()) => None,
            Err(error) => {
                crate::log!("the engine could not be resynced: {error:?}");
                Some(RESYNC_RETRY_FLUSHES)
            }
        };
    }

    /// Drops the streams that wait for the engine, so that what is sent next goes in at once:
    /// the helper is quitting and only its note off matters.
    pub fn drop_waiting(&mut self) {
        self.queue.clear();
        self.behind = None;
    }

    /// Messages the engine rejected so far.
    pub fn rejected(&self) -> u32 {
        self.queue.prepare().preparer().rejected()
    }

    /// Streams waiting for the engine to take them.
    pub fn waiting(&self) -> usize {
        self.queue.waiting()
    }
}

/// The engine of the open audio output, if any (#525): the prepare side and the rate it runs
/// at. The server, the MIDI callback, the control thread and Quit reach the engine through it;
/// the output supervisor (`crate::output`) fills it when an output opens and empties it when
/// the output goes away. While it is empty, commands and MIDI notes go nowhere: nothing can
/// sound. Non-real-time threads only.
#[derive(Default)]
pub struct EngineSlot {
    running: Mutex<Option<Running>>,
}

struct Running {
    sender: EventSender,
    sample_rate: f32,
}

impl EngineSlot {
    /// Runs `f` on the sender of the open output; `None` when there is none. A lock poisoned
    /// by a panic is taken anyway (a note off must never be skipped).
    pub fn with<R>(&self, f: impl FnOnce(&mut EventSender) -> R) -> Option<R> {
        let mut running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        running.as_mut().map(|r| f(&mut r.sender))
    }

    /// Sends a command to the engine of the open output; dropped when there is none.
    pub fn send(&self, command: &Command) {
        self.with(|sender| sender.send(command));
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
        *self.running.lock().unwrap_or_else(|p| p.into_inner()) = Some(Running {
            sender,
            sample_rate,
        });
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

/// Creates the audio core and the sender that feeds it, with the one-synth project set up
/// (`commands::engine_msgs::Synced::setup`: a `Reset` and the project's snapshot).
pub fn audio_core(
    sample_rate: f32,
    meter: Arc<Meter>,
) -> Result<(AudioCore, EventSender), MsgError> {
    let mut stream = Synced::single_synth(EPOCH);
    // The helper holds only the one synth's track until BO-1, so its live input (MIDI)
    // has that track selected: the page's selection names tracks the helper does not have.
    stream.state.session.selected_track = Some(commands::engine_msgs::single_synth::track());
    audio_core_for(sample_rate, meter, stream)
}

/// [`audio_core`] for the project of `stream` (a big one in the tests: what BO-1 will send).
/// Its setup goes through the sender's queue: what the engine does not take at once waits
/// for the flusher.
pub(crate) fn audio_core_for(
    sample_rate: f32,
    meter: Arc<Meter>,
    stream: Synced,
) -> Result<(AudioCore, EventSender), MsgError> {
    let (prepare, host) = engine::host::channel(sample_rate)?;
    let mut sender = EventSender {
        queue: StreamQueue::new(prepare),
        stream,
        behind: None,
        since_resync: RESYNC_RETRY_FLUSHES,
    };
    if let Ok(mut report) = meter.report.lock() {
        *report = Some(host.feedback_shared());
    }
    let setup = sender.stream.setup()?;
    sender.queue.send(&setup)?;
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
            let n = self.host.render((frames - done).min(CHUNK_FRAMES));
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
mod tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;
    use commands::{device, history, live, midi};

    fn track() -> project::TrackId {
        project::TrackId::parse(protocol::single_synth::TRACK).unwrap()
    }

    fn note_on(note: u8, velocity: u8) -> Command {
        Command::Live(live::Command::NoteOn {
            track: track(),
            note,
            velocity,
        })
    }

    fn core() -> (AudioCore, EventSender, Arc<Meter>) {
        let meter = Arc::new(Meter::default());
        let (core, sender) = audio_core(48_000.0, meter.clone()).unwrap();
        (core, sender, meter)
    }

    #[test]
    fn process_never_allocates_for_any_buffer_shape() {
        let (mut core, mut sender, _) = core();
        // Buffers allocated before the check; the check covers only `process`.
        let mut buffers: Vec<(Vec<f32>, usize)> =
            [(441, 2), (480, 2), (5_000, 2), (128, 1), (256, 6), (0, 2)]
                .iter()
                .map(|&(frames, ch)| (vec![0.0; frames * ch + 1], ch))
                .collect();
        sender.send(&note_on(60, 100));
        assert_no_alloc(|| {
            for _ in 0..20 {
                for (buf, ch) in buffers.iter_mut() {
                    core.process(buf, *ch);
                }
            }
        });
    }

    #[test]
    fn notes_reach_both_channels_and_extra_channels_stay_silent() {
        let (mut core, mut sender, meter) = core();
        sender.send(&note_on(69, 127));
        let mut out = vec![0.0; 4_800 * 4];
        core.process(&mut out, 4);
        let frame = |i: usize| &out[i * 4..i * 4 + 4];
        assert!(out.chunks(4).any(|f| f[0].abs() > 0.05));
        for i in 0..4_800 {
            assert_eq!(frame(i)[0], frame(i)[1]);
            assert_eq!(frame(i)[2], 0.0);
            assert_eq!(frame(i)[3], 0.0);
        }
        let (peak, voices) = meter.take();
        assert!(peak > 0.05);
        assert_eq!(voices, 1);
        assert_eq!(meter.callback_frames(), Some(4_800));
        assert_eq!(meter.take().0, 0.0, "the peak resets after a read");
        assert_eq!(sender.rejected(), 0, "the setup and the note were taken");
    }

    #[test]
    fn parameters_reach_the_synth() {
        let (mut core, mut sender, meter) = core();
        sender.send(&note_on(69, 127));
        let mut out = vec![0.0; 9_600 * 2];
        core.process(&mut out, 2);
        let loud = meter.take().0;
        sender.send(&Command::Device(device::Command::SetParam {
            device: project::DeviceId::parse(protocol::single_synth::DEVICE).unwrap(),
            param: "gain".into(),
            value: 0.1,
        }));
        for _ in 0..4 {
            core.process(&mut out, 2);
        }
        meter.take();
        core.process(&mut out, 2);
        let quiet = meter.take().0;
        assert!(quiet < loud * 0.5, "gain 0.1: {quiet} against {loud}");
    }

    #[test]
    fn latency_estimate_adds_the_driver_delay_to_one_buffer() {
        let meter = Meter::default();
        assert_eq!(
            meter.estimated_latency_ms(48_000.0),
            None,
            "unknown before the first callback"
        );
        meter.callback_frames.store(480, Ordering::Relaxed);
        assert_eq!(meter.estimated_latency_ms(48_000.0), Some(10.0));
        meter.set_output_delay(Duration::from_micros(12_500));
        assert_eq!(meter.estimated_latency_ms(48_000.0), Some(22.5));
    }

    #[test]
    fn full_live_queue_releases_every_note() {
        let (mut core, mut sender, _) = core();
        sender.send(&note_on(60, 100));
        let mut out = vec![0.0; 256];
        core.process(&mut out, 2);
        // Fill the live queue with note-ons: the note-off that follows is lost.
        for _ in 0..engine::msg::limits::LIVE_QUEUE_CAPACITY {
            sender.send(&note_on(72, 100));
        }
        sender.send(&Command::Live(live::Command::NoteOff {
            track: track(),
            note: 60,
        }));
        // The overflow releases everything, so after the release time all is silent.
        for _ in 0..200 {
            core.process(&mut out, 2);
        }
        assert!(out.iter().all(|s| *s == 0.0));
    }

    /// MIDI bytes as the helper's MIDI callback gets them, sent as it sends them.
    fn midi(sender: &mut EventSender, bytes: &[u8]) {
        sender.send(&crate::midi::decode(bytes).expect("a live command"));
    }

    #[test]
    fn simulated_midi_plays_the_selected_track_and_a_held_note_survives_a_deselect() {
        let (mut core, mut sender, meter) = core();
        let mut out = vec![0.0; 4_800 * 2];
        midi(&mut sender, &[0x90, 69, 100]);
        core.process(&mut out, 2);
        assert!(
            out.iter().any(|s| s.abs() > 0.01),
            "the selected track plays"
        );
        assert_eq!(meter.take().1, 1);
        // The selection goes away while the key is held: the note off still ends the note.
        sender.send(&Command::View(commands::view::Command::SelectTrack {
            track: None,
        }));
        midi(&mut sender, &[0x80, 69, 0]);
        for _ in 0..200 {
            core.process(&mut out, 2);
        }
        assert!(out.iter().all(|s| *s == 0.0), "no note is stuck");
        // With nothing selected nor armed, the input plays nowhere.
        midi(&mut sender, &[0x90, 69, 100]);
        core.process(&mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(sender.rejected(), 0);
    }

    #[test]
    fn an_armed_track_plays_without_being_selected() {
        let (mut core, mut sender, _) = core();
        sender.send(&Command::View(commands::view::Command::SelectTrack {
            track: None,
        }));
        sender.send(&Command::Track(commands::track::Command::Arm {
            track: track(),
            armed: true,
        }));
        let mut out = vec![0.0; 4_800 * 2];
        midi(&mut sender, &[0x90, 60, 100]);
        core.process(&mut out, 2);
        assert!(out.iter().any(|s| s.abs() > 0.01));
        assert_eq!(sender.rejected(), 0);
    }

    #[test]
    fn commands_the_engine_does_not_take_change_nothing() {
        let (mut core, mut sender, meter) = core();
        for command in [
            Command::Midi(midi::Command::ListInputs {}),
            Command::Midi(midi::Command::SelectInput { id: None }),
            Command::History(history::Command::Undo {}),
        ] {
            sender.send(&command);
        }
        let mut out = vec![0.0; 960];
        core.process(&mut out, 2);
        assert_eq!(meter.take(), (0.0, 0));
        assert_eq!(sender.rejected(), 0);
    }

    /// The helper set up with a big project (what BO-1 will send): `tracks * 128` clips,
    /// the last track selected so that the live input plays it.
    fn big_core(tracks: usize) -> (AudioCore, EventSender, Arc<Meter>) {
        let big = test_support::big::big_project(tracks, 64, 64);
        let mut stream = Synced::new(big.state, big.ctx, EPOCH);
        stream.state.session.selected_track = big.tracks.last().cloned();
        let meter = Arc::new(Meter::default());
        let (core, sender) = audio_core_for(48_000.0, meter.clone(), stream).unwrap();
        (core, sender, meter)
    }

    /// Callbacks of 256 frames, each followed by a flush of the control thread, until the
    /// engine has applied everything (nothing waits, nothing is pending, a callback applies
    /// nothing): how many.
    fn until_applied(core: &mut AudioCore, sender: &mut EventSender) -> usize {
        let mut out = vec![0.0; 256 * 2];
        let mut callbacks = 0;
        loop {
            let before = core.host.engine().applied();
            assert_no_alloc(|| core.process(&mut out, 2));
            sender.flush();
            if core.host.engine().applied() == before
                && sender.waiting() == 0
                && sender.queue.prepare().preparer().pending() == 0
            {
                return callbacks;
            }
            callbacks += 1;
            assert!(callbacks < 10_000, "the setup does not drain");
        }
    }

    #[test]
    fn a_big_project_is_set_up_in_pieces_and_the_audio_thread_never_allocates() {
        let (mut core, mut sender, meter) = big_core(48);
        assert_eq!(sender.waiting(), 1, "the setup waits for the engine");
        let callbacks = until_applied(&mut core, &mut sender);
        assert!(callbacks > 1, "{callbacks} callbacks");
        assert_eq!(sender.rejected(), 0);
        assert_eq!(core.host.engine().skipped(), 0);
        assert!(sender.behind.is_none());
        // The live input plays the last track the setup created.
        midi(&mut sender, &[0x90, 69, 100]);
        let mut out = vec![0.0; 4_800 * 2];
        assert_no_alloc(|| core.process(&mut out, 2));
        assert!(out.iter().any(|s| s.abs() > 0.01), "the last track plays");
        assert_eq!(meter.take().1, 1);
    }

    #[test]
    fn a_note_played_while_the_setup_drains_sounds_once_it_is_applied() {
        // In order, as on the web: the note waits behind the snapshot of its track.
        let (mut core, mut sender, _) = big_core(48);
        midi(&mut sender, &[0x90, 69, 100]);
        assert_eq!(sender.waiting(), 2, "the note waits behind the setup");
        let mut out = vec![0.0f32; 256 * 2];
        let mut callbacks = 0;
        while !out.iter().any(|s| s.abs() > 0.01) {
            assert_no_alloc(|| core.process(&mut out, 2));
            sender.flush();
            callbacks += 1;
            assert!(callbacks < 10_000, "the note never sounds");
        }
        assert_eq!(sender.rejected(), 0);
        assert_eq!(core.host.engine().skipped(), 0);
    }

    #[test]
    fn a_stream_the_engine_refuses_resyncs_it_to_the_project() {
        let (mut core, mut sender, _) = big_core(48);
        until_applied(&mut core, &mut sender);
        let epoch = sender.stream.epoch();
        // A corrupt stream (what a lost or damaged message gives): refused, then the
        // engine is brought back to the project by a `Reset` and its snapshot.
        sender.submit(&[1, 2, 3]);
        assert_eq!(sender.stream.epoch(), epoch + 1);
        assert!(sender.waiting() > 0, "the snapshot waits for the engine");
        assert!(sender.behind.is_none());
        until_applied(&mut core, &mut sender);
        assert_eq!(core.host.engine().skipped(), 0);
        assert_eq!(sender.rejected(), 0);
        assert_eq!(
            sender.queue.prepare().preparer().epoch(),
            epoch + 1,
            "the engine took the resync"
        );
    }

    #[test]
    fn a_waiting_stream_the_engine_refuses_at_a_flush_resyncs_it_too() {
        let (mut core, mut sender, _) = big_core(48);
        let epoch = sender.stream.epoch();
        // Behind the setup that waits: kept, refused when its turn comes at a flush.
        sender.queue.send(&[1, 2, 3]).unwrap();
        assert_eq!(sender.waiting(), 2);
        until_applied(&mut core, &mut sender);
        assert_eq!(sender.stream.epoch(), epoch + 1, "resynced at the flush");
        assert_eq!(sender.queue.prepare().preparer().epoch(), epoch + 1);
        assert_eq!(core.host.engine().skipped(), 0);
    }

    #[test]
    fn a_resync_that_could_not_be_sent_is_tried_again_after_a_pause() {
        let (mut core, mut sender, _) = big_core(2);
        until_applied(&mut core, &mut sender);
        let epoch = sender.stream.epoch();
        // As a failed send leaves it: two flushes to wait, then a try.
        sender.behind = Some(2);
        sender.flush();
        sender.flush();
        assert_eq!(sender.stream.epoch(), epoch, "not before its turn");
        sender.flush();
        assert_eq!(sender.stream.epoch(), epoch + 1, "tried again");
        assert!(sender.behind.is_none());
        until_applied(&mut core, &mut sender);
        assert_eq!(sender.queue.prepare().preparer().epoch(), epoch + 1);
        // Quitting forgets a pending try.
        sender.behind = Some(0);
        sender.drop_waiting();
        sender.flush();
        assert_eq!(sender.stream.epoch(), epoch + 1);
    }

    #[test]
    fn two_refusals_in_a_row_resync_once_then_again_after_a_pause() {
        let (mut core, mut sender, _) = big_core(2);
        until_applied(&mut core, &mut sender);
        let epoch = sender.stream.epoch();
        sender.submit(&[1, 2, 3]);
        assert_eq!(
            sender.stream.epoch(),
            epoch + 1,
            "the first refusal resyncs at once"
        );
        // Refused again right after (a snapshot the engine keeps refusing): no new encoding
        // of the whole project at this tick, one after the pause.
        sender.submit(&[1, 2, 3]);
        assert_eq!(sender.stream.epoch(), epoch + 1);
        assert!(sender.behind.is_some());
        for _ in 0..RESYNC_RETRY_FLUSHES {
            sender.flush();
        }
        assert_eq!(sender.stream.epoch(), epoch + 2, "resynced after the pause");
        assert!(sender.behind.is_none());
        until_applied(&mut core, &mut sender);
        assert_eq!(sender.queue.prepare().preparer().epoch(), epoch + 2);
    }

    #[test]
    fn quitting_drops_what_waits_so_the_note_off_goes_in_at_once() {
        let (mut core, mut sender, meter) = big_core(48);
        until_applied(&mut core, &mut sender);
        midi(&mut sender, &[0x90, 69, 100]);
        let mut out = vec![0.0; 256 * 2];
        core.process(&mut out, 2);
        assert_eq!(meter.take().1, 1);
        // A resync waits (a switch, say) when the helper quits.
        sender.resync();
        assert!(sender.waiting() > 0);
        sender.drop_waiting();
        sender.send(&Command::Live(live::Command::AllNotesOff {}));
        assert_eq!(sender.waiting(), 0, "the note off went in at once");
    }

    #[test]
    fn the_control_thread_flushes_until_the_sender_is_dropped() {
        let (_core, sender, _) = core();
        let slot = Arc::new(EngineSlot::default());
        slot.install(sender, 48_000.0);
        let flusher = spawn_flusher(&slot);
        std::thread::sleep(FLUSH_INTERVAL * 3);
        drop(slot);
        flusher.join().unwrap();
    }

    #[test]
    fn an_empty_slot_drops_commands_and_reports_no_rate() {
        let slot = EngineSlot::default();
        assert!(!slot.is_open());
        assert_eq!(slot.sample_rate(), None);
        slot.send(&note_on(60, 100));
        slot.flush();
        assert_eq!(slot.with(|s| s.rejected()), None);
        let (mut core, sender, meter) = core();
        slot.install(sender, 44_100.0);
        assert_eq!(slot.sample_rate(), Some(44_100.0));
        slot.send(&note_on(69, 127));
        let mut out = vec![0.0; 960];
        core.process(&mut out, 2);
        assert_eq!(meter.take().1, 1, "the installed engine plays");
        slot.remove();
        assert!(!slot.is_open());
    }

    #[test]
    fn a_reset_meter_forgets_the_output_that_went_away() {
        let (mut core, mut sender, meter) = core();
        sender.send(&note_on(69, 127));
        let mut out = vec![0.0; 960];
        core.process(&mut out, 2);
        meter.set_output_delay(Duration::from_millis(5));
        assert!(meter.callback_frames().is_some());
        let mut report = Vec::new();
        // The engine writes its report every few blocks.
        for _ in 0..100 {
            if meter.copy_report(&mut report) {
                break;
            }
            core.process(&mut out, 2);
        }
        assert!(meter.copy_report(&mut report), "a report before the reset");
        meter.reset();
        assert_eq!(meter.callback_frames(), None);
        assert_eq!(meter.estimated_latency_ms(48_000.0), None);
        assert_eq!(meter.take(), (0.0, 0));
        assert!(!meter.copy_report(&mut report), "no stale report");
    }
}
