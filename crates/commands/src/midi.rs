//! `midi.*`: MIDI inputs: list and select. PR-2 migrates `midi.list_inputs` and
//! `midi.select_input` of v1, unchanged in name and payload.
//!
//! The inputs themselves belong to the host (the helper's MIDI ports, or Web MIDI in
//! the page): the host lists them and connects the chosen one. The choice is session
//! state ([`project::SessionState::midi_input`]), never in the project.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;

domain! {
    domain = "midi", schema = ("MidiCommand", "MidiOutput");

    /// List the MIDI inputs available to the engine. The host answers with its
    /// `midi.inputs` message: the inputs and the one selected.
    "midi.list_inputs" (kind = "query", undo = "none", destructive = false)
    ListInputs {} => {};

    /// Choose the MIDI input that plays the selected track. `id` is an id from
    /// `midi.inputs`; `null` disconnects MIDI input. Session state: not undoable, not
    /// saved with the project.
    "midi.select_input" (kind = "live", undo = "none", destructive = false)
    SelectInput { id: Option<String> } => {
        /// The input chosen before the command.
        previous: Option<String>,
    };
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::ListInputs {} => Ok(Outcome {
            output: Output::ListInputs {},
            undo: Undo::None,
            events: vec![],
            control: vec![],
        }),
        Command::SelectInput { id } => {
            let previous = std::mem::replace(&mut state.session.midi_input, id.clone());
            let events = if previous == *id {
                vec![]
            } else {
                vec![ProjectEvent::MidiInputSelected { id: id.clone() }]
            };
            Ok(Outcome {
                output: Output::SelectInput { previous },
                undo: Undo::None,
                events,
                control: vec![],
            })
        }
    }
}
