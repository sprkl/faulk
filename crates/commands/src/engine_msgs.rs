//! Commands → engine messages, for the one synth the page plays today (MO-2).
//!
//! The single mapping from a [`Command`] to the engine's [`ControlMsg`] stream, shared by
//! both targets: the page encodes through `crates/project-wasm` and sends the bytes to the
//! web engine's prepare Worker; the helper encodes the commands it receives and submits
//! the same bytes to its prepare side. Nothing maps commands to the engine in TypeScript.
//!
//! Interim until PR-3 gives every [`crate::Outcome`] its engine messages and the project
//! its id → slot table: the page has one track with one synth (`protocol::single_synth`),
//! which lives in engine slot [`SYNTH_TRACK`], device slot [`SYNTH_DEVICE`]; every live
//! note and every synth parameter goes there whatever the ids, as the hosts did before.

use engine::OSC_SYNTH;
use engine::msg::{ControlMsg, DeviceSlot, MsgError, ParamId, TrackKind, TrackSlot};

use crate::{Command, device, live};

/// The engine slot of the one synth's track.
pub const SYNTH_TRACK: TrackSlot = TrackSlot(0);
/// The engine slot of the synth in that track's chain.
pub const SYNTH_DEVICE: DeviceSlot = DeviceSlot(0);

/// What a fresh engine needs before the commands: a `Reset` to `epoch`, the track, the
/// synth with its default parameters (the page then sends its parameter values as
/// `device.set_param` commands).
pub fn single_synth_setup(epoch: u16) -> [ControlMsg; 3] {
    [
        ControlMsg::Reset { epoch },
        ControlMsg::TrackCreate {
            track: SYNTH_TRACK,
            kind: TrackKind::Midi,
        },
        ControlMsg::DeviceInsert {
            track: SYNTH_TRACK,
            device: SYNTH_DEVICE,
            kind: OSC_SYNTH.kind_id,
            state: Vec::new(),
        },
    ]
}

/// The engine message of a command, `None` for a command the engine does not take: MIDI
/// device management (the host's), a parameter the synth does not have, and every
/// project edit until PR-3. Note numbers and velocities above 127 are clamped, as the
/// engine always did.
pub fn to_control(command: &Command) -> Option<ControlMsg> {
    Some(match command {
        Command::Live(live::Command::NoteOn { note, velocity, .. }) => ControlMsg::NoteOn {
            track: SYNTH_TRACK,
            pitch: (*note).min(live::MIDI_MAX),
            velocity: (*velocity).min(live::MIDI_MAX),
        },
        Command::Live(live::Command::NoteOff { note, .. }) => ControlMsg::NoteOff {
            track: SYNTH_TRACK,
            pitch: (*note).min(live::MIDI_MAX),
        },
        Command::Live(live::Command::AllNotesOff {}) => {
            ControlMsg::AllNotesOff { track: SYNTH_TRACK }
        }
        Command::Device(device::Command::SetParam { param, value, .. }) => {
            let param = device::SynthParam::from_name(param)?;
            ControlMsg::ParamSet {
                track: SYNTH_TRACK,
                device: SYNTH_DEVICE,
                param: ParamId(param.to_engine().id()),
                // The device clamps to the parameter's range; a non-finite value is
                // refused by the codec (`BadPayload`), never reaches the synth.
                value: *value as f32,
            }
        }
        _ => return None,
    })
}

/// The encoder of one engine channel's stream: its epoch and the next sequence number
/// (+1 per message, wrapping), the header fields of `engine::msg`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineStream {
    epoch: u16,
    seq: u32,
}

impl EngineStream {
    /// A stream for a fresh engine, in `epoch` (the setup's `Reset` adopts it).
    pub fn new(epoch: u16) -> Self {
        EngineStream { epoch, seq: 0 }
    }

    pub fn epoch(&self) -> u16 {
        self.epoch
    }

    /// The bytes of `msgs`, in this stream's epoch, numbered after the previous ones.
    pub fn encode(&mut self, msgs: &[ControlMsg]) -> Result<Vec<u8>, MsgError> {
        let bytes = engine::msg::encode_stream(self.epoch, self.seq, msgs)?;
        self.seq = self.seq.wrapping_add(msgs.len() as u32);
        Ok(bytes)
    }

    /// The setup of [`single_synth_setup`], encoded.
    pub fn setup(&mut self) -> Result<Vec<u8>, MsgError> {
        self.encode(&single_synth_setup(self.epoch))
    }

    /// The bytes of a command (empty when the engine does not take it).
    pub fn command(&mut self, command: &Command) -> Result<Vec<u8>, MsgError> {
        match to_control(command) {
            Some(msg) => self.encode(&[msg]),
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{history, midi};
    use engine::msg::{HEADER_BYTES, Header};

    fn track() -> project::TrackId {
        project::TrackId::parse(protocol_ids::TRACK).unwrap()
    }

    fn synth() -> project::DeviceId {
        project::DeviceId::parse(protocol_ids::DEVICE).unwrap()
    }

    /// The single-synth ids (`protocol::single_synth`, which depends on this crate).
    mod protocol_ids {
        pub const TRACK: &str = "00000000000000000000000001";
        pub const DEVICE: &str = "00000000000000000000000002";
    }

    #[test]
    fn every_engine_command_maps_and_the_others_do_not() {
        let note_on = Command::Live(live::Command::NoteOn {
            track: track(),
            note: 200,
            velocity: 255,
        });
        assert_eq!(
            to_control(&note_on),
            Some(ControlMsg::NoteOn {
                track: SYNTH_TRACK,
                pitch: 127,
                velocity: 127
            })
        );
        let note_off = Command::Live(live::Command::NoteOff {
            track: track(),
            note: 60,
        });
        assert_eq!(
            to_control(&note_off),
            Some(ControlMsg::NoteOff {
                track: SYNTH_TRACK,
                pitch: 60
            })
        );
        assert_eq!(
            to_control(&Command::Live(live::Command::AllNotesOff {})),
            Some(ControlMsg::AllNotesOff { track: SYNTH_TRACK })
        );
        for param in device::SynthParam::ALL {
            let command = Command::Device(device::Command::SetParam {
                device: synth(),
                param: param.name().into(),
                value: 0.25,
            });
            assert_eq!(
                to_control(&command),
                Some(ControlMsg::ParamSet {
                    track: SYNTH_TRACK,
                    device: SYNTH_DEVICE,
                    param: ParamId(param.to_engine().id()),
                    value: 0.25
                }),
                "{}",
                param.name()
            );
        }
        let unknown = Command::Device(device::Command::SetParam {
            device: synth(),
            param: "cutoff".into(),
            value: 0.5,
        });
        assert_eq!(to_control(&unknown), None);
        for other in [
            Command::Midi(midi::Command::ListInputs {}),
            Command::Midi(midi::Command::SelectInput { id: None }),
            Command::History(history::Command::Undo {}),
        ] {
            assert_eq!(to_control(&other), None, "{other:?}");
        }
    }

    #[test]
    fn the_stream_numbers_its_messages_and_keeps_its_epoch() {
        let mut stream = EngineStream::new(7);
        let setup = stream.setup().unwrap();
        let headers: Vec<Header> = {
            let mut at = 0;
            let mut out = Vec::new();
            let mut epoch = 6;
            while at < setup.len() {
                let h = Header::decode(&setup[at..], epoch).unwrap();
                epoch = h.epoch;
                at += Header::frame_len(h.len);
                out.push(h);
            }
            out
        };
        assert_eq!(headers.len(), 3);
        assert!(headers.iter().all(|h| h.epoch == 7));
        assert_eq!(headers.iter().map(|h| h.seq).collect::<Vec<_>>(), [0, 1, 2]);
        let note = stream
            .command(&Command::Live(live::Command::AllNotesOff {}))
            .unwrap();
        assert_eq!(Header::decode(&note, 7).unwrap().seq, 3);
        assert!(note.len() >= HEADER_BYTES);
        let none = stream
            .command(&Command::Midi(midi::Command::ListInputs {}))
            .unwrap();
        assert!(none.is_empty());
        assert_eq!(
            Header::decode(
                &stream
                    .command(&Command::Live(live::Command::AllNotesOff {}))
                    .unwrap(),
                7
            )
            .unwrap()
            .seq,
            4,
            "a command the engine does not take uses no number"
        );
    }

    #[test]
    fn the_setup_and_the_commands_play_the_synth_through_the_channel() {
        let (mut prepare, mut host) = engine::host::channel(48_000.0).unwrap();
        let mut stream = EngineStream::new(1);
        prepare.submit(&stream.setup().unwrap()).unwrap();
        for command in [
            Command::Device(device::Command::SetParam {
                device: synth(),
                param: "gain".into(),
                value: 0.5,
            }),
            Command::Live(live::Command::NoteOn {
                track: track(),
                note: 69,
                velocity: 100,
            }),
        ] {
            prepare.submit(&stream.command(&command).unwrap()).unwrap();
        }
        assert_eq!(prepare.preparer().rejected(), 0);
        let mut peak = 0.0f32;
        for _ in 0..40 {
            let n = host.render(128);
            peak = host.left()[..n].iter().fold(peak, |p, s| p.max(s.abs()));
        }
        assert!(peak > 0.05, "the note sounds: {peak}");
        assert_eq!(host.engine().skipped(), 0);
    }
}
