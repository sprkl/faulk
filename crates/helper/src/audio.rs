//! Real-time side of the helper: the audio callback and what feeds it.
//!
//! Everything in [`AudioCore::process`] runs on the audio thread: it never allocates,
//! locks or blocks. Events reach it through one `rtrb` ring; when the ring is full the
//! sender raises a flag that the callback turns into "all notes off", so a lost
//! note-off can never leave a note hanging.

use engine::{Param, Synth};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Events small enough to cross the ring without allocation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EngineEvent {
    NoteOn { note: u8, velocity: u8 },
    NoteOff { note: u8 },
    AllNotesOff,
    SetParam { param: Param, value: f32 },
}

/// Capacity of the event ring. Far above what a player or a page sends between two
/// audio callbacks.
pub const RING_CAPACITY: usize = 1024;
/// Frames rendered per chunk; a longer callback is rendered in several chunks.
pub const CHUNK_FRAMES: usize = 1024;

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
}

impl Meter {
    /// Returns the peak since the last call and resets it, plus the voice count.
    pub fn take(&self) -> (f32, u32) {
        let peak = f32::from_bits(self.peak_bits.swap(0, Ordering::Relaxed));
        (peak, self.voices.load(Ordering::Relaxed))
    }

    pub fn callback_frames(&self) -> Option<u32> {
        match self.callback_frames.load(Ordering::Relaxed) {
            0 => None,
            n => Some(n),
        }
    }

    /// Records the driver's output delay. Allocation-free: called from the audio callback.
    pub fn set_output_delay(&self, delay: std::time::Duration) {
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

/// The sending side of the ring. Owned behind a mutex by non-real-time threads only.
pub struct EventSender {
    producer: Producer<EngineEvent>,
    overflow: Arc<AtomicBool>,
}

impl EventSender {
    /// Sends an event; if the ring is full, asks the audio thread to release every note.
    pub fn send(&mut self, event: EngineEvent) {
        if self.producer.push(event).is_err() {
            self.overflow.store(true, Ordering::Release);
        }
    }
}

pub struct AudioCore {
    synth: Synth,
    events: Consumer<EngineEvent>,
    overflow: Arc<AtomicBool>,
    meter: Arc<Meter>,
    left: Box<[f32; CHUNK_FRAMES]>,
    right: Box<[f32; CHUNK_FRAMES]>,
}

/// Creates the audio core and the sender that feeds it.
pub fn audio_core(sample_rate: f32, meter: Arc<Meter>) -> (AudioCore, EventSender) {
    let (producer, consumer) = RingBuffer::new(RING_CAPACITY);
    let overflow = Arc::new(AtomicBool::new(false));
    let core = AudioCore {
        synth: Synth::new(sample_rate),
        events: consumer,
        overflow: overflow.clone(),
        meter,
        left: Box::new([0.0; CHUNK_FRAMES]),
        right: Box::new([0.0; CHUNK_FRAMES]),
    };
    (core, EventSender { producer, overflow })
}

impl AudioCore {
    /// The values this core publishes for the UI.
    pub fn meter(&self) -> &Meter {
        &self.meter
    }

    /// Fills an interleaved output buffer of `channels` channels.
    /// Channel 0 gets the left signal, channel 1 the right one, others silence.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        while let Ok(event) = self.events.pop() {
            match event {
                EngineEvent::NoteOn { note, velocity } => self.synth.note_on(note, velocity),
                EngineEvent::NoteOff { note } => self.synth.note_off(note),
                EngineEvent::AllNotesOff => self.synth.all_notes_off(),
                EngineEvent::SetParam { param, value } => self.synth.set_param(param, value),
            }
        }
        // After draining, so that the note-ons queued before a lost note-off are released too.
        if self.overflow.swap(false, Ordering::Acquire) {
            self.synth.all_notes_off();
        }

        let channels = channels.max(1);
        let frames = out.len() / channels;
        self.meter
            .callback_frames
            .store(frames as u32, Ordering::Relaxed);
        let mut peak = 0.0f32;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CHUNK_FRAMES);
            self.synth.render(&mut self.left[..n], &mut self.right[..n]);
            for i in 0..n {
                let frame = &mut out[(done + i) * channels..(done + i + 1) * channels];
                for (c, sample) in frame.iter_mut().enumerate() {
                    *sample = match c {
                        0 => self.left[i],
                        1 => self.right[i],
                        _ => 0.0,
                    };
                }
                peak = peak.max(self.left[i].abs());
            }
            done += n;
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
            .store(self.synth.active_voices() as u32, Ordering::Relaxed);
    }
}

/// Converts a command for the engine into a ring event. Every live note and every
/// synth parameter goes to the one synth, whatever the track and device ids (the page
/// owns the project and the helper plays what it sends, until PR-3 gives the engine
/// messages). Other commands give `None`: MIDI device management is the host's, and
/// the rest changes only the page's project. A parameter the synth does not have gives
/// `None` too.
pub fn engine_event(command: &commands::Command) -> Option<EngineEvent> {
    use commands::{Command, device, live};
    Some(match command {
        Command::Live(live::Command::NoteOn { note, velocity, .. }) => EngineEvent::NoteOn {
            note: (*note).min(127),
            velocity: (*velocity).min(127),
        },
        Command::Live(live::Command::NoteOff { note, .. }) => EngineEvent::NoteOff {
            note: (*note).min(127),
        },
        Command::Live(live::Command::AllNotesOff {}) => EngineEvent::AllNotesOff,
        Command::Device(device::Command::SetParam { param, value, .. }) => {
            EngineEvent::SetParam {
                param: device::SynthParam::from_name(param)?.to_engine(),
                // The engine clamps to the parameter's range.
                value: *value as f32,
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;
    use commands::Command;

    fn core() -> (AudioCore, EventSender, Arc<Meter>) {
        let meter = Arc::new(Meter::default());
        let (core, sender) = audio_core(48_000.0, meter.clone());
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
        sender.send(EngineEvent::NoteOn {
            note: 60,
            velocity: 100,
        });
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
        sender.send(EngineEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
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
        meter.set_output_delay(std::time::Duration::from_micros(12_500));
        assert_eq!(meter.estimated_latency_ms(48_000.0), Some(22.5));
    }

    #[test]
    fn full_ring_releases_every_note() {
        let (mut core, mut sender, _) = core();
        sender.send(EngineEvent::NoteOn {
            note: 60,
            velocity: 100,
        });
        let mut out = vec![0.0; 256];
        core.process(&mut out, 2);
        // Fill the ring with note-ons: the note-off that follows is lost.
        for _ in 0..RING_CAPACITY {
            sender.send(EngineEvent::NoteOn {
                note: 72,
                velocity: 100,
            });
        }
        sender.send(EngineEvent::NoteOff { note: 60 });
        // The overflow flag releases everything, so after the release time all is silent.
        for _ in 0..200 {
            core.process(&mut out, 2);
        }
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn commands_map_to_engine_events() {
        use commands::{device, history, live, midi};
        let track = || project::TrackId::parse(protocol::single_synth::TRACK).unwrap();
        let synth = || project::DeviceId::parse(protocol::single_synth::DEVICE).unwrap();
        assert_eq!(
            engine_event(&Command::Device(device::Command::SetParam {
                device: synth(),
                param: "gain".into(),
                value: 0.5
            })),
            Some(EngineEvent::SetParam {
                param: Param::Gain,
                value: 0.5
            })
        );
        assert_eq!(
            engine_event(&Command::Device(device::Command::SetParam {
                device: synth(),
                param: "cutoff".into(),
                value: 0.5
            })),
            None,
            "a parameter the synth does not have"
        );
        assert_eq!(
            engine_event(&Command::Live(live::Command::NoteOn {
                track: track(),
                note: 60,
                velocity: 100
            })),
            Some(EngineEvent::NoteOn {
                note: 60,
                velocity: 100
            })
        );
        assert_eq!(
            engine_event(&Command::Live(live::Command::NoteOff {
                track: track(),
                note: 200
            })),
            Some(EngineEvent::NoteOff { note: 127 })
        );
        assert_eq!(
            engine_event(&Command::Live(live::Command::AllNotesOff {})),
            Some(EngineEvent::AllNotesOff)
        );
        assert_eq!(
            engine_event(&Command::Midi(midi::Command::ListInputs {})),
            None
        );
        assert_eq!(
            engine_event(&Command::Midi(midi::Command::SelectInput { id: None })),
            None
        );
        assert_eq!(
            engine_event(&Command::History(history::Command::Undo {})),
            None
        );
    }
}
