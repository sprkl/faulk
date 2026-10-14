//! `live.*`: notes played live, to the selected or armed track (`live.note_on { track, .. }`), and the panic button. PR-2 migrates `note.on`, `note.off` and `note.all_off` from v1.
//!
//! Empty skeleton (CT-4): the tasks named above add their commands here, one
//! `domain!` block each, and implement them in [`apply`]. A command is declared only
//! once it is implemented: the test rule refuses a declared command without an example.

use crate::v2::contract::{CommandError, Context, Outcome, State};
use crate::v2::macros::domain;

domain! {
    domain = "live", schema = ("LiveCommand", "LiveOutput");
}

/// Applies a command of this domain. No command yet.
pub fn apply(
    command: &Command,
    _state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match *command {}
}
