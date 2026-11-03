//! MIDI inputs: listing, hot-plug detection by polling, and the selected connection.
//!
//! midir has no hot-plug event, so the host polls [`MidiManager::refresh`]. The MIDI
//! callback runs on midir's thread (not the audio thread). The helper holds no project
//! (BO-1): the page decides which tracks a note plays and tells the helper as a table
//! (`live.routing`, #598). The callback plays each note of the selected input on the helper's
//! engine at once with that table ([`crate::live::LiveInput`]), then forwards the message, as
//! received, to the page's session with whether it was played ([`MidiSink`]); the page runs it
//! in its project core (keys shown, held notes, recording) and sends engine bytes only for
//! what the helper did not play (control changes, a note before any table).

use crate::live::LiveInput;
use midir::{MidiInput, MidiInputConnection};
use protocol::MidiInput as InputInfo;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

/// A message of the selected input, for the page, and whether the helper's engine played it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received {
    pub data: Vec<u8>,
    pub played: bool,
}

/// Where the selected input's messages go: first the helper's engine (the notes,
/// [`LiveInput`]), then the server, which forwards them to the current session.
#[derive(Clone)]
pub struct MidiSink {
    live: Arc<LiveInput>,
    out: UnboundedSender<Received>,
}

impl MidiSink {
    pub fn new(live: Arc<LiveInput>, out: UnboundedSender<Received>) -> Self {
        MidiSink { live, out }
    }

    /// The helper's live input, which the session gives its table.
    pub fn live(&self) -> &Arc<LiveInput> {
        &self.live
    }

    /// A message of the input: played if it is a note the helper can play, then forwarded
    /// when it is a channel message.
    pub fn receive(&self, message: &[u8]) {
        if !forwarded(message) {
            return;
        }
        let played = self.live.receive(message);
        let _ = self.out.send(Received {
            data: message.to_vec(),
            played,
        });
    }
}

/// "All notes off" (control change 123): what the input "sends" when it goes away (unplugged,
/// another one selected): the helper releases the notes it played, and the page the rest, as
/// Web MIDI releases on unplug.
pub const ALL_NOTES_OFF: [u8; 3] = [0xB0, 123, 0];

/// Whether a message is forwarded to the page: the channel messages (note on and off,
/// control change and the others the page may decode), not the system ones (clock, active
/// sensing, sysex), which arrive many times a second and that nothing reads.
pub fn forwarded(message: &[u8]) -> bool {
    message
        .first()
        .is_some_and(|status| (0x80..0xF0).contains(status))
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

/// Whether this system has a MIDI backend the helper can open (`hello`'s capabilities).
pub fn available() -> bool {
    MidiInput::new("faulk-helper-probe").is_ok()
}

pub struct MidiManager {
    sink: MidiSink,
    connection: Option<MidiInputConnection<()>>,
    selected: Option<String>,
    inputs: Vec<InputInfo>,
}

impl MidiManager {
    pub fn new(sink: MidiSink) -> Self {
        Self {
            sink,
            connection: None,
            selected: None,
            inputs: Vec::new(),
        }
    }

    /// The helper's live input (`crate::live`).
    pub fn live(&self) -> Arc<LiveInput> {
        self.sink.live().clone()
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
        let sink = self.sink.clone();
        let connection = input
            .connect(
                &port,
                "faulk-helper-in",
                move |_timestamp, message, _| sink.receive(message),
                (),
            )
            .map_err(|e| e.to_string())?;
        self.connection = Some(connection);
        self.selected = Some(id.to_owned());
        Ok(())
    }

    /// Closes the connection; the notes the helper played are released, and the page is told
    /// to release the notes it was playing.
    pub fn disconnect(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.close();
            self.sink.receive(&ALL_NOTES_OFF);
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
    fn forwards_the_channel_messages_only() {
        for message in [
            &[0x90, 60, 100][..],
            &[0x8F, 60, 0],
            &[0xB0, 123, 0],
            &[0xB5, 1, 0x7F],
            &[0xE0, 0, 64],
        ] {
            assert!(forwarded(message), "{message:?}");
        }
        for message in [&[][..], &[0xF8], &[0xFE], &[0xF0, 1, 0xF7], &[0x40, 1]] {
            assert!(!forwarded(message), "{message:?}");
        }
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

    #[test]
    fn disconnecting_with_no_connection_tells_the_page_nothing() {
        let (out, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let live = Arc::new(LiveInput::new(Arc::default()));
        let mut midi = MidiManager::new(MidiSink::new(live, out));
        midi.disconnect();
        assert!(rx.try_recv().is_err());
        assert_eq!(midi.selected(), None);
    }
}
