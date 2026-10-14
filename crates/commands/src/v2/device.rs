//! `device.*`: a track's device chain: insert, remove, move, enable, set a parameter (epic 4, DV; `synth.set_param` of v1 becomes `device.set_param` in PR-2).
//!
//! Empty skeleton (CT-4): the tasks named above add their commands here, one
//! `domain!` block each, and implement them in [`apply`]. A command is declared only
//! once it is implemented: the test rule refuses a declared command without an example.

use crate::v2::contract::{CommandError, Context, Outcome, State};
use crate::v2::macros::domain;

domain! {
    domain = "device", schema = ("DeviceCommand", "DeviceOutput");
}

/// Applies a command of this domain. No command yet.
pub fn apply(
    command: &Command,
    _state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match *command {}
}
