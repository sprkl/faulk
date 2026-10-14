//! `launch.*`: launching and stopping clips and scenes, `live`: the playing state is never saved (epic 6, GR).
//!
//! Empty skeleton (CT-4): the tasks named above add their commands here, one
//! `domain!` block each, and implement them in [`apply`]. A command is declared only
//! once it is implemented: the test rule refuses a declared command without an example.

use crate::contract::{CommandError, Context, Outcome, State};
use crate::macros::domain;

domain! {
    domain = "launch", schema = ("LaunchCommand", "LaunchOutput");
}

/// Applies a command of this domain. No command yet.
pub fn apply(
    command: &Command,
    _state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match *command {}
}
