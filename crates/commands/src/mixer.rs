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
//! - A command that changes nothing returns no inverse and no event, but for a fader or pan
//!   gesture on a control that an automation lane drives, which reports
//!   [`ProjectEvent::GestureUnchanged`] so that the lane gives way to it (#417).
//! - **Solo** lives in [`project::SessionState`]: it never enters the history nor the
//!   project's serialization. **Mute** is a field of the project and is versioned.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::levels::{Level, TrackLevel, level_of, track_levels};
use crate::macros::domain;
use project::{AutomationTarget, Mixer, MixerParam, Project, TrackId};

/// The gain bounds, defined by the project document and shared with the commands (#226).
pub use project::{MAX_GAIN_DB, MIN_GAIN_DB};

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
        #[schemars(required, schema_with = "nullable_number")]
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
        #[schemars(required, schema_with = "nullable_number")]
        previous: Option<f64>,
    };

    /// Set the master's pan, from -1 (left) to 1 (right). A value outside the range or
    /// not a number is refused. A pan gesture sent in one history group is undone in one
    /// step. Undo: set the previous pan back.
    "mixer.set_master_pan" (kind = "edit", undo = "inverse", destructive = false)
    SetMasterPan {
        #[schemars(range(min = -1.0, max = 1.0))]
        pan: f64,
    } => {
        /// The pan before the command.
        previous: f64,
    };

    /// Solo or unsolo a track. The soloed tracks are those in the session's solo set;
    /// the audio engine applies it from MX-1 on (cumulative: several tracks can be soloed).
    /// Session state: saved with the session, never in the project's files; undoable like
    /// a mute (#473). Undo: set the previous state back.
    "mixer.set_solo" (kind = "edit", undo = "inverse", destructive = false)
    SetSolo {
        track: TrackId,
        solo: bool,
    } => {
        /// Whether the track was soloed before the command.
        previous: bool,
    };

    /// Read how loud the project sounds now, per track and for the master, from the
    /// meters of the playing engine: how an agent without ears learns whether a song
    /// sounds. Call it while the transport plays (`transport.play`): the meters keep the
    /// largest peak and the loudest RMS since the previous call (at most the last 10
    /// seconds), so a call right after a passage covers it. That RMS is the loudest of
    /// the engine's short windows (about 30 ms), so it reads higher than the RMS of a
    /// whole passage, which is what `project.analyze_render` gives. Levels are in dBFS (0 is
    /// full scale; the master is measured before the output protection, so it can read
    /// above 0), `null` is silence; a peak is the larger of the two channels, an RMS the
    /// power mean of both. `playing` says whether the transport ran: when it did not (or
    /// the audio is not started, or the engine has stopped reporting), every level is
    /// `null`. Tracks come in display order, after their fader and mute. To measure a
    /// stretch without playing it, use `project.analyze_render`. `silenced_frames` above 0
    /// says the output protection silenced an invalid (non-finite) signal since the page's
    /// audio started, which the levels cannot show. `latency_ms` is the delay the devices add
    /// to what is heard (a look-ahead limiter's): the engine delays what plays beside those
    /// devices (the other tracks, the metronome) to match, so everything stays in time, and
    /// moves recorded takes back by it.
    /// A query: changes nothing and never enters the history.
    "mixer.levels" (kind = "query", undo = "none", destructive = false)
    Levels {} => {
        /// Whether the transport was playing or recording at the last reading.
        playing: bool,
        /// The level of each track, in display order.
        tracks: Vec<TrackLevel>,
        /// The level of the output, after the master and before the output protection.
        master: Level,
        /// Frames the output protection turned to silence because a sample was not a number
        /// (NaN or infinite), since the engine that plays started for this page (in Boost, since
        /// the page's session with the helper began): above 0, a device produced an
        /// invalid signal, which the levels above do not show. Counted whether or not the
        /// transport runs.
        silenced_frames: u64,
        /// The delay the devices add to what is heard, in samples at the engine's rate: the
        /// longest chain of the tracks plus the master's (0 with no such device). The engine
        /// compensates it: the tracks and the metronome stay aligned. Counted whether or not the
        /// transport runs.
        latency_frames: u32,
        /// The same delay in milliseconds (0 when the engine's rate is unknown).
        latency_ms: f64,
    };
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::Levels {} => {
            // Nothing was heard while the transport did not run: the contract is `null` levels.
            let staged = ctx.levels.take();
            // The protection's count holds whether or not the transport ran (#667).
            let silenced_frames = staged.as_ref().map_or(0, |s| s.silenced_frames);
            // The latency holds whether or not the transport ran (#749).
            let (latency_frames, rate) = staged
                .as_ref()
                .map_or((0, 0.0), |s| (s.latency_frames, s.sample_rate));
            let stage = staged.filter(|s| s.playing).unwrap_or_default();
            let tracks = track_levels(
                &state.project,
                |id| ctx.engine.tracks.get(id),
                &stage.tracks,
            );
            Ok(crate::query::answer(Output::Levels {
                playing: stage.playing,
                tracks,
                master: level_of(&stage.master),
                silenced_frames,
                latency_frames,
                latency_ms: crate::levels::latency_ms(latency_frames, rate),
            }))
        }
        Command::SetVolume { track, gain_db } => {
            let db = to_db(*gain_db)?;
            let mixer = &mut track_mut(state, track)?.mixer;
            check_current(mixer)?;
            let previous = std::mem::replace(&mut mixer.gain_db, db);
            let mut outcome = gain_outcome(
                previous,
                db,
                |previous| Output::SetVolume { previous },
                |gain_db| Command::SetVolume {
                    track: track.clone(),
                    gain_db,
                },
                Some(track.clone()),
                mixer,
            );
            if previous == db {
                outcome.events.extend(gesture_unchanged(
                    &state.project,
                    Some(track),
                    MixerParam::Gain,
                ));
            }
            Ok(outcome)
        }
        Command::SetMasterVolume { gain_db } => {
            let db = to_db(*gain_db)?;
            let mixer = &mut state.project.master;
            check_current(mixer)?;
            let previous = std::mem::replace(&mut mixer.gain_db, db);
            let mut outcome = gain_outcome(
                previous,
                db,
                |previous| Output::SetMasterVolume { previous },
                |gain_db| Command::SetMasterVolume { gain_db },
                None,
                mixer,
            );
            if previous == db {
                outcome
                    .events
                    .extend(gesture_unchanged(&state.project, None, MixerParam::Gain));
            }
            Ok(outcome)
        }
        Command::SetMasterPan { pan } => {
            check_pan(*pan)?;
            let mixer = &mut state.project.master;
            let previous = mixer.pan;
            let output = Output::SetMasterPan { previous };
            if previous == *pan {
                let mut outcome = unchanged(output);
                outcome
                    .events
                    .extend(gesture_unchanged(&state.project, None, MixerParam::Pan));
                return Ok(outcome);
            }
            mixer.pan = *pan;
            Ok(edited(
                output,
                Command::SetMasterPan { pan: previous },
                None,
                mixer,
            ))
        }
        Command::SetPan { track, pan } => {
            check_pan(*pan)?;
            let mixer = &mut track_mut(state, track)?.mixer;
            let previous = mixer.pan;
            let output = Output::SetPan { previous };
            if previous == *pan {
                let mut outcome = unchanged(output);
                outcome.events.extend(gesture_unchanged(
                    &state.project,
                    Some(track),
                    MixerParam::Pan,
                ));
                return Ok(outcome);
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
            let previous = state.session.solo.contains(track);
            // A stale id (a track deleted since) can always be unsoloed.
            if !state.project.tracks.contains_key(track) && (*solo || !previous) {
                return Err(not_found(track));
            }
            let changed = if *solo {
                state.session.solo.insert(track.clone())
            } else {
                state.session.solo.remove(track)
            };
            Ok(Outcome {
                output: Output::SetSolo { previous },
                undo: if changed {
                    Undo::Inverse(Box::new(Command::SetSolo {
                        track: track.clone(),
                        solo: previous,
                    }))
                } else {
                    Undo::None
                },
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

fn check_pan(pan: f64) -> Result<(), CommandError> {
    if !pan.is_finite() || !(-1.0..=1.0).contains(&pan) {
        return Err(CommandError::Invalid {
            field: "pan".into(),
            message: format!("a pan is from -1 to 1, not {pan}"),
        });
    }
    Ok(())
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
pub(crate) fn to_db(gain: Option<f64>) -> Result<f64, CommandError> {
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
pub(crate) fn from_db(db: f64) -> Option<f64> {
    (db != f64::NEG_INFINITY).then_some(db)
}

/// The event of a fader or pan gesture that left the value as it was (#417), when a lane
/// drives it; `track` is `None` for the master.
fn gesture_unchanged(
    project: &Project,
    track: Option<&TrackId>,
    param: MixerParam,
) -> Option<ProjectEvent> {
    crate::contract::gesture_unchanged(project, track, AutomationTarget::Mixer { param })
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

/// Refuses to edit a gain that is outside the range: the edit could not be undone exactly.
/// `validate` refuses such a gain when a project is read, so this guards a state built in
/// memory rather than a loaded document.
fn check_current(mixer: &Mixer) -> Result<(), CommandError> {
    to_db(from_db(mixer.gain_db))
        .map(|_| ())
        .map_err(|_| CommandError::Invalid {
            field: "gain_db".into(),
            message: format!(
                "the current gain, {} dB, is outside {MIN_GAIN_DB} to {MAX_GAIN_DB} dB: a command could not undo the edit exactly",
                mixer.gain_db
            ),
        })
}

/// The schema of a gain: a number from [`MIN_GAIN_DB`] to [`MAX_GAIN_DB`], or `null` for
/// silence (the derived schema of an optional field drops the `null`).
pub(crate) fn nullable_gain(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["number", "null"],
        "format": "double",
        "minimum": MIN_GAIN_DB,
        "maximum": MAX_GAIN_DB,
        "description": "Required: `null` is silence, a missing field is an error."
    })
}

/// The schema of a previous gain: a number, or `null` when it was silence.
pub(crate) fn nullable_number(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["number", "null"],
        "format": "double",
        "description": "The volume before the command, in dB; `null` was silence."
    })
}
