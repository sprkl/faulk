//! MIDI inputs: listing, hot-plug detection by polling, and the selected connection.
//!
//! midir has no hot-plug event, so the host polls [`MidiManager::refresh`]. The MIDI
//! callback runs on midir's thread (not the audio thread): it turns each message into a
//! live command and sends it through the shared [`EventSender`] (the engine's prepare
//! side), which lives outside the connection so that closing or failing a connection never
//! loses it.

use crate::audio::EventSender;
use commands::{Command, live};
use midir::{MidiInput, MidiInputConnection};
use protocol::MidiInput as InputInfo;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// The track the MIDI input plays: the one synth's (`protocol::single_synth`), until RE-1
/// routes the live input to the selected and armed tracks.
fn track() -> project::TrackId {
    project::TrackId::parse(protocol::single_synth::TRACK).expect("a valid track id")
}

/// The command that releases every note.
pub fn all_notes_off() -> Command {
    Command::Live(live::Command::AllNotesOff {})
}

/// The note and velocity a command plays, for the MIDI activity light (0: released).
pub fn light(command: &Command) -> Option<(u8, u8)> {
    match command {
        Command::Live(live::Command::NoteOn { note, velocity, .. }) => Some((*note, *velocity)),
        Command::Live(live::Command::NoteOff { note, .. }) => Some((*note, 0)),
        _ => None,
    }
}

fn note_on(note: u8, velocity: u8) -> Command {
    Command::Live(live::Command::NoteOn {
        track: track(),
        note,
        velocity,
    })
}

fn note_off(note: u8) -> Command {
    Command::Live(live::Command::NoteOff {
        track: track(),
        note,
    })
}

/// Decodes a MIDI message into a live command. Channels are merged (omni).
pub fn decode(message: &[u8]) -> Option<Command> {
    let (&status, data) = message.split_first()?;
    let kind = status & 0xF0;
    let byte = |i: usize| data.get(i).map(|b| b & 0x7F);
    match kind {
        0x90 => {
            let note = byte(0)?;
            match byte(1)? {
                0 => Some(note_off(note)),
                velocity => Some(note_on(note, velocity)),
            }
        }
        0x80 => Some(note_off(byte(0)?)),
        // Control change 120 (all sound off) and 123 (all notes off).
        0xB0 if matches!(byte(0)?, 120 | 123) => Some(all_notes_off()),
        _ => None,
    }
}

/// True when the two lists hold different inputs (order ignored).
pub fn inputs_changed(previous: &[InputInfo], current: &[InputInfo]) -> bool {
    previous.len() != current.len() || previous.iter().any(|p| !current.contains(p))
}

/// The selected input after a refresh: kept if still plugged in, else dropped.
pub fn still_selected(selected: Option<&str>, current: &[InputInfo]) -> Option<String> {
    selected
        .filter(|id| current.iter().any(|i| i.id == *id))
        .map(str::to_owned)
}

pub struct MidiManager {
    sender: Arc<Mutex<EventSender>>,
    activity: mpsc::Sender<(u8, u8)>,
    connection: Option<MidiInputConnection<()>>,
    selected: Option<String>,
    inputs: Vec<InputInfo>,
}

impl MidiManager {
    pub fn new(sender: Arc<Mutex<EventSender>>, activity: mpsc::Sender<(u8, u8)>) -> Self {
        Self {
            sender,
            activity,
            connection: None,
            selected: None,
            inputs: Vec::new(),
        }
    }

    pub fn inputs(&self) -> &[InputInfo] {
        &self.inputs
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Re-lists the inputs. Returns true when the list or the selection changed.
    /// A selected input that disappeared is disconnected.
    pub fn refresh(&mut self) -> bool {
        let current = list_inputs();
        let changed = inputs_changed(&self.inputs, &current);
        self.inputs = current;
        let kept = still_selected(self.selected.as_deref(), &self.inputs);
        let lost = kept.is_none() && self.selected.is_some();
        if lost {
            self.disconnect();
        }
        changed || lost
    }

    /// Connects the input with this id, or disconnects with `None`.
    pub fn select(&mut self, id: Option<&str>) -> Result<(), String> {
        self.disconnect();
        let Some(id) = id else { return Ok(()) };
        let input = MidiInput::new("faulk-helper").map_err(|e| e.to_string())?;
        let port = input
            .find_port_by_id(id)
            .ok_or_else(|| format!("no MIDI input with id {id}"))?;
        let sender = self.sender.clone();
        let activity = self.activity.clone();
        let connection = input
            .connect(
                &port,
                "faulk-helper-in",
                move |_timestamp, message, _| {
                    if let Some(command) = decode(message) {
                        if let Ok(mut sender) = sender.lock() {
                            sender.send(&command);
                        }
                        if let Some(light) = light(&command) {
                            let _ = activity.send(light);
                        }
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        self.connection = Some(connection);
        self.selected = Some(id.to_owned());
        Ok(())
    }

    /// Closes the connection and releases the notes it was holding.
    pub fn disconnect(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.close();
            if let Ok(mut sender) = self.sender.lock() {
                sender.send(&all_notes_off());
            }
        }
        self.selected = None;
    }
}

fn list_inputs() -> Vec<InputInfo> {
    let Ok(input) = MidiInput::new("faulk-helper-list") else {
        return Vec::new();
    };
    input
        .ports()
        .iter()
        .map(|port| InputInfo {
            id: port.id(),
            name: input.port_name(port).unwrap_or_else(|_| port.id()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_notes_on_any_channel() {
        assert_eq!(decode(&[0x90, 60, 100]), Some(note_on(60, 100)));
        assert_eq!(decode(&[0x9F, 60, 100]), Some(note_on(60, 100)));
        assert_eq!(decode(&[0x90, 60, 0]), Some(note_off(60)));
        assert_eq!(decode(&[0x83, 61, 40]), Some(note_off(61)));
        assert_eq!(decode(&[0xB0, 123, 0]), Some(all_notes_off()));
        assert_eq!(decode(&[0xB0, 120, 0]), Some(all_notes_off()));
    }

    #[test]
    fn ignores_other_and_malformed_messages() {
        assert_eq!(decode(&[]), None);
        assert_eq!(decode(&[0x90]), None);
        assert_eq!(decode(&[0x90, 60]), None);
        assert_eq!(decode(&[0xB0, 7, 100]), None);
        assert_eq!(decode(&[0xF8]), None); // clock
        assert_eq!(decode(&[0x90, 0xFF, 0xFF]), Some(note_on(127, 127)));
    }

    fn input(id: &str) -> InputInfo {
        InputInfo {
            id: id.into(),
            name: id.into(),
        }
    }

    #[test]
    fn detects_plug_and_unplug() {
        let a = [input("a")];
        let ab = [input("a"), input("b")];
        let ba = [input("b"), input("a")];
        assert!(!inputs_changed(&a, &a));
        assert!(!inputs_changed(&ab, &ba), "order does not matter");
        assert!(inputs_changed(&a, &ab), "plugged");
        assert!(inputs_changed(&ab, &a), "unplugged");
        assert!(inputs_changed(&a, &[input("c")]), "swapped");
    }

    #[test]
    fn selection_survives_only_while_plugged() {
        let ab = [input("a"), input("b")];
        assert_eq!(still_selected(Some("a"), &ab), Some("a".into()));
        assert_eq!(still_selected(Some("z"), &ab), None);
        assert_eq!(still_selected(None, &ab), None);
    }
}
