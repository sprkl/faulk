//! `scene.*`: scenes of the launch grid: create, delete, rename, reorder (epic 6, GR).
//!
//! Empty skeleton (CT-4): the tasks named above add their commands here, one
//! `domain!` block each, and implement them in [`apply`]. A command is declared only
//! once it is implemented: the test rule refuses a declared command without an example.

use crate::v2::contract::{CommandError, Context, Outcome, State};
use crate::v2::macros::domain;

domain! {
    domain = "scene", schema = ("SceneCommand", "SceneOutput");
}

/// Applies a command of this domain. No command yet.
pub fn apply(
    command: &Command,
    _state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match *command {}
}
