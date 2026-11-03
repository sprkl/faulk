//! MIDI inputs: listing, hot-plug detection by polling, and the selected connection.
//!
//! midir has no hot-plug event, so the host polls [`MidiManager::refresh`]. The MIDI
//! callback runs on midir's thread (not the audio thread). The helper holds no project
//! (BO-1): it does not decide which track a note plays. It forwards each channel message of
//! the selected input, as received, to the page's session ([`MidiOut`]); the page decodes it
//! with the same decoder as Web MIDI and runs it in its project core (routing to the selected
//! and armed tracks, held notes, recording), and the engine bytes come back like any change.

use midir::{MidiInput, MidiInputConnection};
use protocol::MidiInput as InputInfo;
use tokio::sync::mpsc::UnboundedSender;

/// Where the selected input's messages go: the server forwards them to the current session.
pub type MidiOut = UnboundedSender<Vec<u8>>;

/// "All notes off" (control change 123): what the page gets when the input it played goes
/// away (unplugged, another one selected), as Web MIDI releases on unplug.
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
    out: MidiOut,
    connection: Option<MidiInputConnection<()>>,
    selected: Option<String>,
    inputs: Vec<InputInfo>,
}

impl MidiManager {
    pub fn new(out: MidiOut) -> Self {
        Self {
            out,
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
        let out = self.out.clone();
        let connection = input
            .connect(
                &port,
                "faulk-helper-in",
                move |_timestamp, message, _| {
                    if forwarded(message) {
                        let _ = out.send(message.to_vec());
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        self.connection = Some(connection);
        self.selected = Some(id.to_owned());
        Ok(())
    }

    /// Closes the connection; the page is told to release the notes it was playing.
    pub fn disconnect(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.close();
            let _ = self.out.send(ALL_NOTES_OFF.to_vec());
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
        let mut midi = MidiManager::new(out);
        midi.disconnect();
        assert!(rx.try_recv().is_err());
        assert_eq!(midi.selected(), None);
    }
}
