//! `live.*`: notes played live, to a track, and the panic button. PR-2 migrates
//! `note.on`, `note.off` and `note.all_off` of v1; RE-1 routes the live input to the
//! selected and armed tracks.
//!
//! Live notes change neither the project nor the session: they only reach the engine.
//! Until CT-3 and PR-3 give the [`crate::ControlMsg`], the hosts translate them to the
//! engine themselves; `apply` checks them against the project.

use crate::contract::{CommandError, Context, Outcome, State, Undo};
use crate::macros::domain;
use project::TrackId;

/// The highest MIDI note number and velocity.
pub const MIDI_MAX: u8 = 127;

domain! {
    domain = "live", schema = ("LiveCommand", "LiveOutput");

    /// Start a note on a track's instrument. `note` is a MIDI note number, 0 to 127
    /// (60 = middle C); `velocity` 1 to 127, 0 acts as a note off. Played live: not
    /// undoable, not saved.
    "live.note_on" (kind = "live", undo = "none", destructive = false)
    NoteOn {
        track: TrackId,
        #[schemars(range(max = 127))]
        note: u8,
        #[schemars(range(max = 127))]
        velocity: u8,
    } => {};

    /// Release a note started with `live.note_on` on the same track. Played live: not
    /// undoable, not saved.
    "live.note_off" (kind = "live", undo = "none", destructive = false)
    NoteOff {
        track: TrackId,
        #[schemars(range(max = 127))]
        note: u8,
    } => {};

    /// Release every playing note on every track (panic button). Not undoable: it
    /// changes no project data.
    "live.all_notes_off" (kind = "live", undo = "none", destructive = false)
    AllNotesOff {} => {};
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    let output = match command {
        Command::NoteOn {
            track,
            note,
            velocity,
        } => {
            check_track(state, track)?;
            check_midi("note", *note)?;
            check_midi("velocity", *velocity)?;
            Output::NoteOn {}
        }
        Command::NoteOff { track, note } => {
            check_track(state, track)?;
            check_midi("note", *note)?;
            Output::NoteOff {}
        }
        Command::AllNotesOff {} => Output::AllNotesOff {},
    };
    Ok(Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    })
}

fn check_track(state: &State, track: &TrackId) -> Result<(), CommandError> {
    if state.project.tracks.contains_key(track) {
        Ok(())
    } else {
        Err(CommandError::NotFound {
            entity: "track".into(),
            id: track.to_string(),
        })
    }
}

fn check_midi(field: &str, value: u8) -> Result<(), CommandError> {
    if value <= MIDI_MAX {
        Ok(())
    } else {
        Err(CommandError::Invalid {
            field: field.into(),
            message: format!("{field} is 0 to {MIDI_MAX}, not {value}"),
        })
    }
}
