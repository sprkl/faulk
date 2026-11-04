//! `view.*`: the part of the view state that is a command.
//!
//! Decision (CT-4): zoom, scroll and the displayed view are not commands; the page keeps
//! them in the session file itself, since they change no model and an agent gains
//! nothing from them. The selected track is a command, `live`, because it decides where
//! live MIDI goes and an agent must be able to set it and read it. The theme (#687) is a
//! command so that an agent can set it like the user: it is the user's preference, kept by
//! the browser for every project, so the page answers it and this crate only declares it.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use project::TrackId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The theme of the interface the user chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Follows the system's light or dark setting, also when it changes during the session.
    System,
    /// The dark theme, the default.
    Dark,
    /// The light theme.
    Light,
}

domain! {
    domain = "view", schema = ("ViewCommand", "ViewOutput");

    /// Select a track, where live MIDI and the keyboard go; `null` deselects. Session
    /// state: not undoable, not saved with the project. Refused with `not_found` when the
    /// track does not exist.
    "view.select_track" (kind = "live", undo = "none", destructive = false)
    SelectTrack {
        /// Required: `null` deselects, a missing field is an error.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(required, transform = crate::contract::allow_null)]
        track: Option<TrackId>
    } => {
        /// The track selected before the command.
        previous: Option<TrackId>,
    };

    /// Set the theme of the interface: `dark` (the default), `light`, or `system`, which
    /// follows the system's light or dark setting, also when it changes later. The user's
    /// preference, kept by the browser for every project: it changes nothing in the
    /// project or its session, is not undoable and works before audio starts.
    "view.set_theme" (kind = "live", undo = "none", destructive = false)
    SetTheme {
        theme: Theme
    } => {};
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::SelectTrack { track } => {
            if let Some(id) = track
                && !state.project.tracks.contains_key(id)
            {
                return Err(CommandError::NotFound {
                    entity: "track".into(),
                    id: id.to_string(),
                });
            }
            let previous = std::mem::replace(&mut state.session.selected_track, track.clone());
            Ok(Outcome {
                output: Output::SelectTrack { previous },
                undo: Undo::None,
                events: vec![ProjectEvent::TrackSelected {
                    track: track.clone(),
                }],
                control: vec![],
            })
        }
        // The page applies it (the browser keeps the choice); the project and its session stay.
        Command::SetTheme { .. } => Ok(Outcome {
            output: Output::SetTheme {},
            undo: Undo::None,
            events: vec![],
            control: vec![],
        }),
    }
}
