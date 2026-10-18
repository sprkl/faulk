//! Real-time side of the helper: the audio callback and what feeds it (MO-2).
//!
//! The helper runs the same engine channel as the web build (`engine::host`): the commands
//! it receives (from the page, from the MIDI input) are mapped by the command layer
//! (`commands::engine_msgs`) and encoded into the same `ControlMsg` byte stream the page
//! sends to its prepare Worker, then submitted to the prepare side ([`EventSender`], behind
//! a mutex, non-real-time threads only); a control thread flushes it every few
//! milliseconds ([`spawn_flusher`]). The rt messages and the live notes reach the audio
//! callback through the engine's lock-free rings.
//!
//! Everything in [`AudioCore::process`] runs on the audio thread: it never allocates,
//! locks or blocks. A full live queue releases every note (the engine's live overflow), so
//! a lost note-off can never leave a note hanging.
//!
//! The page still sends commands, not engine bytes, over the local WebSocket: moving the
//! encoding to the page side of that link is BO-1's.

use commands::Command;
use commands::engine_msgs::Synced;
use engine::host::{Host, Prepare};
use engine::msg::MsgError;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// Frames rendered per chunk; a longer callback is rendered in several chunks.
pub const CHUNK_FRAMES: usize = engine::msg::limits::MAX_BLOCK_FRAMES;

/// How often the control thread pushes what waited and drops the garbage.
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(5);

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
    prepare: Prepare,
    /// The one-synth project the commands run on, and the engine's stream (PR-3).
    stream: Synced,
}

impl EventSender {
    /// Runs a command on the project and sends its engine messages, encoded by the
    /// command layer, as bytes: the web engine's path. A command the project refuses,
    /// or a stream the engine stops (out of memory, engine busy), is logged.
    pub fn send(&mut self, command: &Command) {
        let bytes = match self.stream.run(command) {
            Ok((_, bytes)) => bytes,
            Err(error) => {
                crate::log!("the command was refused: {error:?}");
                return;
            }
        };
        if let Err(error) = self.submit(&bytes) {
            crate::log!("the engine refused a command: {error:?}");
        }
    }

    fn submit(&mut self, bytes: &[u8]) -> Result<(), MsgError> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.prepare.submit(bytes)
    }

    /// Pushes what waited on the rt ring and drops the garbage.
    pub fn flush(&mut self) {
        self.prepare.flush();
    }

    /// Messages the engine rejected so far.
    pub fn rejected(&self) -> u32 {
        self.prepare.preparer().rejected()
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
    let (prepare, host) = engine::host::channel(sample_rate)?;
    let mut sender = EventSender {
        prepare,
        stream: Synced::single_synth(EPOCH),
    };
    if let Ok(mut report) = meter.report.lock() {
        *report = Some(host.feedback_shared());
    }
    let setup = sender.stream.setup()?;
    sender.submit(&setup)?;
    Ok((AudioCore { host, meter }, sender))
}

/// The control thread: flushes the sender every [`FLUSH_INTERVAL`] until it is dropped.
pub fn spawn_flusher(sender: &Arc<Mutex<EventSender>>) -> std::thread::JoinHandle<()> {
    let sender: Weak<Mutex<EventSender>> = Arc::downgrade(sender);
    std::thread::spawn(move || {
        while let Some(sender) = sender.upgrade() {
            if let Ok(mut sender) = sender.lock() {
                sender.flush();
            }
            drop(sender);
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

    #[test]
    fn the_control_thread_flushes_until_the_sender_is_dropped() {
        let (_core, sender, _) = core();
        let sender = Arc::new(Mutex::new(sender));
        let flusher = spawn_flusher(&sender);
        std::thread::sleep(FLUSH_INTERVAL * 3);
        drop(sender);
        flusher.join().unwrap();
    }
}
