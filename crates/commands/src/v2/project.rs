//! `project.*`: the project itself: create, open, save and rename (epic 11, SV).
//!
//! `project.rename` is implemented here as the reference command of the contract: an
//! `edit` with its inverse, its event and its output, exercised by the test rule.

use crate::v2::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::v2::macros::domain;

/// Longest project name accepted, in characters.
pub const MAX_NAME_CHARS: usize = 200;

domain! {
    domain = "project", schema = ("ProjectCommand", "ProjectOutput");

    /// Rename the project. The name is trimmed, not empty, at most 200 characters.
    /// Undo: rename it back to its previous name.
    "project.rename" (kind = "edit", undo = "inverse", destructive = false)
    Rename { name: String } => {
        /// The name before the command.
        previous: String,
    };
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::Rename { name } => {
            let name = name.trim();
            if name.is_empty() {
                return Err(CommandError::Invalid {
                    field: "name".into(),
                    message: "a project name is not empty".into(),
                });
            }
            if name.chars().count() > MAX_NAME_CHARS {
                return Err(CommandError::Invalid {
                    field: "name".into(),
                    message: format!("a project name has at most {MAX_NAME_CHARS} characters"),
                });
            }
            let previous = std::mem::replace(&mut state.project.name, name.to_owned());
            Ok(Outcome {
                output: Output::Rename {
                    previous: previous.clone(),
                },
                undo: Undo::Inverse(Box::new(Command::Rename { name: previous })),
                events: vec![ProjectEvent::ProjectRenamed {
                    name: name.to_owned(),
                }],
                control: vec![],
            })
        }
    }
}
