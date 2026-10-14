//! `transport.*`: playback and the saved transport settings: play, stop, position, tempo, loop region, metronome, launch quantization (epic 3, TR).
//!
//! Empty skeleton (CT-4): the tasks named above add their commands here, one
//! `domain!` block each, and implement them in [`apply`]. A command is declared only
//! once it is implemented: the test rule refuses a declared command without an example.

use crate::contract::{CommandError, Context, Outcome, State};
use crate::macros::domain;

domain! {
    domain = "transport", schema = ("TransportCommand", "TransportOutput");
}

/// Applies a command of this domain. No command yet.
pub fn apply(
    command: &Command,
    _state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match *command {}
}
