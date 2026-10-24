//! `live.*`: notes played live, to a track, and the panic button. PR-2 migrates
//! `note.on`, `note.off` and `note.all_off` of v1; RE-1 adds the live input, which goes
//! to the selected and the armed tracks.
//!
//! Live notes change neither the project nor the session: they only reach the engine.
//! `apply` checks them against the project; their engine messages (`NoteOn`, `NoteOff`,
//! `AllNotesOff` on every track the engine has) come from `engine_msgs::compile`.
//!
//! **The live input** (`live.input_note_on`, `live.input_note_off`, `live.control_change`)
//! is what a MIDI controller or the computer keyboard plays: the host decodes the
//! message and runs the command, and the command layer, not the host, decides where it
//! goes, so the page (Web mode) and the helper (Boost mode) route the same way. A note on
//! reaches the selected track and every armed track (MIDI tracks only), as the session
//! stands when it arrives. The engine slots remember where each held note went
//! ([`project::EngineSlots::held`]), and the note off goes to those tracks: selecting
//! another track or arming one while a note is held never leaves it sounding.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use project::{EngineSlots, TrackId, TrackKind};
use std::collections::BTreeSet;

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

    /// Play a note on the live input: it goes to the selected track and to every armed
    /// MIDI track, as the session stands now (`view.select_track`, `track.arm`). `note`
    /// is a MIDI note number, 0 to 127 (60 = middle C); `velocity` 1 to 127, 0 acts as a
    /// note off. Release it with `live.input_note_off`, which reaches the same tracks
    /// whatever was selected or armed meanwhile. The output lists the tracks it went to.
    /// Played live: not undoable, not saved.
    "live.input_note_on" (kind = "live", undo = "none", destructive = false)
    InputNoteOn {
        #[schemars(range(max = 127))]
        note: u8,
        #[schemars(range(max = 127))]
        velocity: u8,
    } => {
        /// The tracks the note is routed to; empty when no MIDI track is selected or armed.
        /// A track the engine has no room for is listed but plays nothing.
        tracks: Vec<TrackId>,
    };

    /// Release a note started with `live.input_note_on`: on the tracks that note went
    /// to, even if the selection or the arming changed since. Played live: not undoable,
    /// not saved.
    "live.input_note_off" (kind = "live", undo = "none", destructive = false)
    InputNoteOff {
        #[schemars(range(max = 127))]
        note: u8,
    } => {
        /// The tracks the note was released on; empty when it was not held.
        tracks: Vec<TrackId>,
    };

    /// A control change (CC) on the live input, channels merged: `controller` 0 to 127,
    /// `value` 0 to 127. It changes no sound by itself: it is reported to the interface
    /// as a `control_change` event, for the automation (AM-5) to take. Controllers 120
    /// (all sound off) and 123 (all notes off) are not sent here: hosts turn them into
    /// `live.all_notes_off`. Played live: not undoable, not saved.
    "live.control_change" (kind = "live", undo = "none", destructive = false)
    ControlChange {
        #[schemars(range(max = 127))]
        controller: u8,
        #[schemars(range(max = 127))]
        value: u8,
    } => {};

    /// Release every playing note on every track (panic button). Not undoable: it
    /// changes no project data.
    "live.all_notes_off" (kind = "live", undo = "none", destructive = false)
    AllNotesOff {} => {};

    /// Cut every sound at once, on every track and the master: the output fades out in about
    /// 5 ms, then every instrument's voices (long releases included) and every effect's
    /// tail (echo repeats, reverb) are cleared, without a click. The transport is not
    /// touched. Use it when a sound rings on or feeds back out of control. Not undoable: it
    /// changes no project data.
    "live.all_sound_off" (kind = "live", undo = "none", destructive = false)
    AllSoundOff {} => {};
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    let mut events = vec![];
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
        Command::AllSoundOff {} => Output::AllSoundOff {},
        Command::InputNoteOn { note, velocity } => {
            check_midi("note", *note)?;
            check_midi("velocity", *velocity)?;
            // Velocity 0 is a note off: it goes where the note is held.
            let tracks = if *velocity == 0 {
                released(&ctx.engine, *note)
            } else {
                input_targets(state)
            };
            Output::InputNoteOn { tracks }
        }
        Command::InputNoteOff { note } => {
            check_midi("note", *note)?;
            Output::InputNoteOff {
                tracks: released(&ctx.engine, *note),
            }
        }
        Command::ControlChange { controller, value } => {
            check_midi("controller", *controller)?;
            check_midi("value", *value)?;
            events.push(ProjectEvent::ControlChange {
                controller: *controller,
                value: *value,
            });
            Output::ControlChange {}
        }
    };
    Ok(Outcome {
        output,
        undo: Undo::None,
        events,
        control: vec![],
    })
}

/// The tracks the live input plays now: the selected track and the armed tracks that
/// exist and are MIDI tracks (an audio track has no instrument to play), in id order.
pub fn input_targets(state: &State) -> Vec<TrackId> {
    let session = &state.session;
    let wanted: BTreeSet<&TrackId> = session
        .armed
        .iter()
        .chain(&session.selected_track)
        .collect();
    wanted
        .into_iter()
        .filter(|id| {
            state
                .project
                .tracks
                .get(*id)
                .is_some_and(|t| t.kind == TrackKind::Midi)
        })
        .cloned()
        .collect()
}

/// The tracks a held live input note is on, in id order.
pub fn released(engine: &EngineSlots, note: u8) -> Vec<TrackId> {
    engine
        .held
        .iter()
        .filter(|(n, _)| *n == note)
        .map(|(_, track)| track.clone())
        .collect()
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
