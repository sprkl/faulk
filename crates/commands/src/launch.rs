//! `launch.*`: launching and stopping clips and scenes, `live`: the playing state is never saved (epic 6, GR).
//!
//! The tasks of the grid add their commands here, one `domain!` entry each, and implement
//! them in [`apply`]. A command is declared only once it is implemented: the test rule
//! refuses a declared command without an example.
//!
//! **`launch.back_to_arrangement`** (AR-2) is the one command that gives the tracks back to
//! their arrangement: a clip launched from the grid, or a stop given from it, takes a track
//! until then. Like the other launch commands it is `live`: it changes neither the project
//! nor the session and never enters the history; its engine message
//! (`ControlMsg::BackToArrangement`) comes from `engine_msgs::compile`.

use crate::contract::{CommandError, Context, Outcome, State, Undo};
use crate::macros::domain;

domain! {
    domain = "launch", schema = ("LaunchCommand", "LaunchOutput");

    /// Give every track back to its arrangement: a clip launched from the grid (or a stop
    /// given from it) takes a track until this command. The notes of the grid's clips end
    /// at once, launches still waiting for their boundary are dropped, and each track plays
    /// its arrangement again from the playhead (a note already started on the timeline is
    /// not restarted: it stays silent until its end). Does nothing to a track that follows
    /// its arrangement. Played live: not undoable, not saved.
    "launch.back_to_arrangement" (kind = "live", undo = "none", destructive = false)
    BackToArrangement {} => {};
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    _state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::BackToArrangement {} => Ok(Outcome {
            output: Output::BackToArrangement {},
            undo: Undo::None,
            events: vec![],
            control: vec![],
        }),
    }
}
