//! `launch.*`: launching and stopping clips and scenes, `live`: the playing state is never
//! saved (epic 6, GR-2).
//!
//! - **Played live**: a launch changes neither the project nor the session and never enters
//!   the history; `apply` only checks that what it names exists. The engine messages come
//!   from `engine_msgs::compile` (PR-3), like `transport.*`.
//! - **Quantized**: a launch or a stop takes effect at the next boundary of the project's
//!   launch quantization (`transport.set_launch_quantize`), never in the past.
//! - **A launch starts the transport**: `launch.clip` and `launch.scene` while the
//!   transport is stopped launch (the engine arms the order) then play from its position,
//!   so that the clip starts at the first boundary (at once when playback starts on a bar
//!   line). The launch comes first so that it never reaches the engine after the playhead
//!   left the position. While it plays, they only launch. A scene with no clip still starts
//!   the transport: it is a launch. `launch.stop_track` and `launch.stop_all` never start it.
//! - One clip per track at a time: launching a clip replaces the one the track plays at
//!   the boundary. Launching a scene launches the clip of each track's slot in that scene
//!   and stops the tracks whose slot is empty.

use crate::contract::{CommandError, Context, Outcome, State};
use crate::macros::domain;
use project::{ClipContent, ClipId, SceneId, TrackId};

domain! {
    domain = "launch", schema = ("LaunchCommand", "LaunchOutput");

    /// Launch a MIDI clip of the launch grid at the next boundary of the launch
    /// quantization; it replaces the clip its track plays. The clip must sit in a slot of
    /// the grid (a clip of the arrangement only cannot be launched) and must not be an
    /// audio clip (not launchable yet). If the transport is stopped, it starts playing
    /// from its position, so the clip starts at the first boundary. Played live: not
    /// undoable, not saved.
    "launch.clip" (kind = "live", undo = "none", destructive = false)
    Clip { clip: ClipId } => {};

    /// Launch every clip of a scene at the same boundary: each track plays the clip of its
    /// slot in the scene, and a track whose slot is empty stops. An empty scene stops every
    /// track that plays. If the transport is stopped, it starts playing from its position.
    /// Played live: not undoable, not saved.
    "launch.scene" (kind = "live", undo = "none", destructive = false)
    Scene { scene: SceneId } => {};

    /// Stop the clip a track plays, or cancel the launch it waits for, at the next boundary
    /// of the launch quantization. Does not start the transport. Played live: not undoable,
    /// not saved.
    "launch.stop_track" (kind = "live", undo = "none", destructive = false)
    StopTrack { track: TrackId } => {};

    /// Stop every track that plays a clip or waits for a launch, at the next boundary of
    /// the launch quantization; it replaces the launches waiting. Does not start the
    /// transport, and does not stop it. Played live: not undoable, not saved.
    "launch.stop_all" (kind = "live", undo = "none", destructive = false)
    StopAll {} => {};

    /// Give every track back to its arrangement: a clip launched from the grid (or a stop
    /// given from it) takes a track until this command. The notes of the grid's clips end
    /// at once, launches still waiting for their boundary are dropped, and each track plays
    /// its arrangement again from the playhead (a note already started on the timeline is
    /// not restarted: it stays silent until its end). Does nothing to a track that follows
    /// its arrangement. Played live: not undoable, not saved.
    "launch.back_to_arrangement" (kind = "live", undo = "none", destructive = false)
    BackToArrangement {} => {};

    /// Give one track back to its arrangement: what `launch.back_to_arrangement` does, for
    /// the track alone; the other tracks are left as they are. Does nothing to a track that
    /// follows its arrangement. Played live: not undoable, not saved.
    "launch.back_track_to_arrangement" (kind = "live", undo = "none", destructive = false)
    BackTrackToArrangement { track: TrackId } => {};
}

type Out = Outcome<Command, Output>;

fn done(output: Output) -> Out {
    Outcome {
        output,
        undo: crate::contract::Undo::None,
        events: vec![],
        control: vec![],
    }
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    _ctx: &mut Context,
) -> Result<Out, CommandError> {
    let project = &state.project;
    match command {
        Command::Clip { clip } => {
            let found = project
                .tracks
                .values()
                .find_map(|t| t.clips.get(clip))
                .ok_or_else(|| CommandError::NotFound {
                    entity: "clip".into(),
                    id: clip.to_string(),
                })?;
            if !matches!(found.content, ClipContent::Midi { .. }) {
                return Err(CommandError::Invalid {
                    field: "clip".into(),
                    message: "an audio clip cannot be launched yet".into(),
                });
            }
            if found.slot.is_none() {
                return Err(CommandError::Invalid {
                    field: "clip".into(),
                    message: "a clip is launched from a slot of the launch grid; this one sits \
                              in the arrangement only"
                        .into(),
                });
            }
            Ok(done(Output::Clip {}))
        }
        Command::Scene { scene } => {
            if !project.scenes.contains_key(scene) {
                return Err(CommandError::NotFound {
                    entity: "scene".into(),
                    id: scene.to_string(),
                });
            }
            Ok(done(Output::Scene {}))
        }
        Command::StopTrack { track } => {
            if !project.tracks.contains_key(track) {
                return Err(CommandError::NotFound {
                    entity: "track".into(),
                    id: track.to_string(),
                });
            }
            Ok(done(Output::StopTrack {}))
        }
        Command::StopAll {} => Ok(done(Output::StopAll {})),
        Command::BackToArrangement {} => Ok(done(Output::BackToArrangement {})),
        Command::BackTrackToArrangement { track } => {
            if !project.tracks.contains_key(track) {
                return Err(CommandError::NotFound {
                    entity: "track".into(),
                    id: track.to_string(),
                });
            }
            Ok(done(Output::BackTrackToArrangement {}))
        }
    }
}
