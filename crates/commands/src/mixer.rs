//! `mixer.*`: gain, pan and mute of a track and of the master (`edit`, in the project),
//! and solo (`live`, in the session state, founder decision 10) (epic 10, MX).
//!
//! Contract of the values (MX-2):
//!
//! - **Gain** is in dB, `null` meaning silence (`-inf`, as in the document). A finite
//!   gain outside [`MIN_GAIN_DB`], [`MAX_GAIN_DB`] is refused, never clamped: an agent
//!   asking for +40 dB must hear that it is not allowed. Below the floor, say `null`.
//! - **Pan** is from -1 (left) to 1 (right); a value outside is refused.
//! - A fader gesture is a burst of `set_volume` or `set_pan` inside one history group
//!   (`history.begin_group` / `history.end_group`, or the context's group): the history
//!   merges them into one entry, so one undo undoes the whole gesture.
//! - A command that changes nothing returns no inverse and no event.
//! - **Solo** lives in [`project::SessionState`]: it never enters the history nor the
//!   project's serialization. **Mute** is a field of the project and is versioned.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use project::{Mixer, TrackId};

/// The lowest finite gain a command accepts, in dB. Below it, use `null` (silence).
pub const MIN_GAIN_DB: f64 = -96.0;
/// The highest gain a command accepts, in dB.
pub const MAX_GAIN_DB: f64 = 6.0;

domain! {
    domain = "mixer", schema = ("MixerCommand", "MixerOutput");

    /// Set a track's volume in dB, from -96 to +6; `null` is silence. A value outside
    /// the range or not a number is refused. A fader gesture sent in one history group
    /// is undone in one step. Undo: set the previous volume back.
    "mixer.set_volume" (kind = "edit", undo = "inverse", destructive = false)
    SetVolume {
        track: TrackId,
        /// Required: `null` is silence, a missing field is an error.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(required, schema_with = "nullable_gain")]
        gain_db: Option<f64>,
    } => {
        /// The volume before the command, in dB; `null` was silence.
        previous: Option<f64>,
    };

    /// Set a track's pan, from -1 (left) to 1 (right). A value outside the range or
    /// not a number is refused. A pan gesture sent in one history group is undone in
    /// one step. Undo: set the previous pan back.
    "mixer.set_pan" (kind = "edit", undo = "inverse", destructive = false)
    SetPan {
        track: TrackId,
        #[schemars(range(min = -1.0, max = 1.0))]
        pan: f64,
    } => {
        /// The pan before the command.
        previous: f64,
    };

    /// Mute or unmute a track. Mute is saved with the project and versioned. Undo: set
    /// the previous state back.
    "mixer.set_mute" (kind = "edit", undo = "inverse", destructive = false)
    SetMute {
        track: TrackId,
        mute: bool,
    } => {
        /// Whether the track was muted before the command.
        previous: bool,
    };

    /// Set the master's volume in dB, from -96 to +6; `null` is silence. A value
    /// outside the range or not a number is refused. Undo: set the previous volume
    /// back.
    "mixer.set_master_volume" (kind = "edit", undo = "inverse", destructive = false)
    SetMasterVolume {
        /// Required: `null` is silence, a missing field is an error.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(required, schema_with = "nullable_gain")]
        gain_db: Option<f64>,
    } => {
        /// The volume before the command, in dB; `null` was silence.
        previous: Option<f64>,
    };

    /// Solo or unsolo a track: while any track is soloed, only the soloed tracks are
    /// heard. Session state: not undoable, not in the history, not saved with the
    /// project (it is restored with the session).
    "mixer.set_solo" (kind = "live", undo = "none", destructive = false)
    SetSolo {
        track: TrackId,
        solo: bool,
    } => {
        /// Whether the track was soloed before the command.
        previous: bool,
    };
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::SetVolume { track, gain_db } => {
            let db = to_db(*gain_db)?;
            let mixer = &mut track_mut(state, track)?.mixer;
            check_current(mixer)?;
            let previous = std::mem::replace(&mut mixer.gain_db, db);
            Ok(gain_outcome(
                previous,
                db,
                |previous| Output::SetVolume { previous },
                |gain_db| Command::SetVolume {
                    track: track.clone(),
                    gain_db,
                },
                Some(track.clone()),
                mixer,
            ))
        }
        Command::SetMasterVolume { gain_db } => {
            let db = to_db(*gain_db)?;
            let mixer = &mut state.project.master;
            check_current(mixer)?;
            let previous = std::mem::replace(&mut mixer.gain_db, db);
            Ok(gain_outcome(
                previous,
                db,
                |previous| Output::SetMasterVolume { previous },
                |gain_db| Command::SetMasterVolume { gain_db },
                None,
                mixer,
            ))
        }
        Command::SetPan { track, pan } => {
            if !pan.is_finite() || !(-1.0..=1.0).contains(pan) {
                return Err(CommandError::Invalid {
                    field: "pan".into(),
                    message: format!("a pan is from -1 to 1, not {pan}"),
                });
            }
            let mixer = &mut track_mut(state, track)?.mixer;
            let previous = mixer.pan;
            let output = Output::SetPan { previous };
            if previous == *pan {
                return Ok(unchanged(output));
            }
            mixer.pan = *pan;
            Ok(edited(
                output,
                Command::SetPan {
                    track: track.clone(),
                    pan: previous,
                },
                Some(track.clone()),
                mixer,
            ))
        }
        Command::SetMute { track, mute } => {
            let mixer = &mut track_mut(state, track)?.mixer;
            let previous = mixer.mute;
            let output = Output::SetMute { previous };
            if previous == *mute {
                return Ok(unchanged(output));
            }
            mixer.mute = *mute;
            Ok(edited(
                output,
                Command::SetMute {
                    track: track.clone(),
                    mute: previous,
                },
                Some(track.clone()),
                mixer,
            ))
        }
        Command::SetSolo { track, solo } => {
            if !state.project.tracks.contains_key(track) {
                return Err(not_found(track));
            }
            let previous = state.session.solo.contains(track);
            let changed = if *solo {
                state.session.solo.insert(track.clone())
            } else {
                state.session.solo.remove(track)
            };
            Ok(Outcome {
                output: Output::SetSolo { previous },
                undo: Undo::None,
                events: if changed {
                    vec![ProjectEvent::SoloChanged {
                        track: track.clone(),
                        solo: *solo,
                    }]
                } else {
                    vec![]
                },
                control: vec![],
            })
        }
    }
}

fn not_found(track: &TrackId) -> CommandError {
    CommandError::NotFound {
        entity: "track".into(),
        id: track.to_string(),
    }
}

fn track_mut<'a>(
    state: &'a mut State,
    track: &TrackId,
) -> Result<&'a mut project::Track, CommandError> {
    state
        .project
        .tracks
        .get_mut(track)
        .ok_or_else(|| not_found(track))
}

/// The document's gain (`-inf` for silence) for a command's (`null` for silence), after
/// checking the range.
fn to_db(gain: Option<f64>) -> Result<f64, CommandError> {
    match gain {
        None => Ok(f64::NEG_INFINITY),
        Some(g) if g.is_finite() && (MIN_GAIN_DB..=MAX_GAIN_DB).contains(&g) => Ok(g),
        Some(g) => Err(CommandError::Invalid {
            field: "gain_db".into(),
            message: format!(
                "a gain is from {MIN_GAIN_DB} to {MAX_GAIN_DB} dB, or null for silence, not {g}"
            ),
        }),
    }
}

/// A command's gain for the document's one.
fn from_db(db: f64) -> Option<f64> {
    (db != f64::NEG_INFINITY).then_some(db)
}

fn unchanged(output: Output) -> Outcome<Command, Output> {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

fn edited(
    output: Output,
    inverse: Command,
    track: Option<TrackId>,
    mixer: &Mixer,
) -> Outcome<Command, Output> {
    Outcome {
        output,
        undo: Undo::Inverse(Box::new(inverse)),
        events: vec![mixer_changed(track, mixer)],
        control: vec![],
    }
}

fn mixer_changed(track: Option<TrackId>, mixer: &Mixer) -> ProjectEvent {
    ProjectEvent::MixerChanged {
        track,
        gain_db: from_db(mixer.gain_db),
        pan: mixer.pan,
        mute: mixer.mute,
    }
}

/// The outcome of a gain edit. An inverse must restore the state exactly and a command
/// cannot write a gain outside the range, so [`check_current`] has refused the edit
/// when the current gain is outside it: here the previous gain is always restorable.
fn gain_outcome(
    previous: f64,
    db: f64,
    output: impl FnOnce(Option<f64>) -> Output,
    inverse: impl FnOnce(Option<f64>) -> Command,
    track: Option<TrackId>,
    mixer: &Mixer,
) -> Outcome<Command, Output> {
    let before = from_db(previous);
    let output = output(before);
    if previous == db {
        return unchanged(output);
    }
    Outcome {
        output,
        undo: Undo::Inverse(Box::new(inverse(before))),
        events: vec![mixer_changed(track, mixer)],
        control: vec![],
    }
}

/// Refuses to edit a gain that is outside the range (a document edited by hand: the
/// project accepts any finite gain): the edit could not be undone exactly.
fn check_current(mixer: &Mixer) -> Result<(), CommandError> {
    to_db(from_db(mixer.gain_db))
        .map(|_| ())
        .map_err(|_| CommandError::Invalid {
            field: "gain_db".into(),
            message: format!(
                "the current gain, {} dB, is outside {MIN_GAIN_DB} to {MAX_GAIN_DB} dB:                  a command could not undo the edit exactly",
                mixer.gain_db
            ),
        })
}

/// The schema of a gain: a number from [`MIN_GAIN_DB`] to [`MAX_GAIN_DB`], or `null` for
/// silence (the derived schema of an optional field drops the `null`).
fn nullable_gain(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["number", "null"],
        "format": "double",
        "minimum": MIN_GAIN_DB,
        "maximum": MAX_GAIN_DB,
        "description": "Required: `null` is silence, a missing field is an error."
    })
}
