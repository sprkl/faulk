//! `clip.*`: clips and their content: create, delete, length, loop, `clip.notes.add` and `clip.notes.remove` (epics 5 and 8, CL and PN). Notes are addressed by value: there is no `NoteId` (CT-4).
//!
//! Empty skeleton (CT-4): the tasks named above add their commands here, one
//! `domain!` block each, and implement them in [`apply`]. A command is declared only
//! once it is implemented: the test rule refuses a declared command without an example.

use crate::v2::contract::{CommandError, Context, Outcome, State};
use crate::v2::macros::domain;

domain! {
    domain = "clip", schema = ("ClipCommand", "ClipOutput");
}

/// Applies a command of this domain. No command yet.
pub fn apply(
    command: &Command,
    _state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match *command {}
}
