//! The session state: what belongs to the screen or the machine, not to the music
//! (founder's decision 10 of 2026-10-14).

use crate::ids::TrackId;
use crate::model::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The session state. Written apart from the project, to `.faulk/session.json` (SV-1),
/// ignored by git, restored on reopening and never versioned: no field of [`Project`]
/// holds it, so changing it never changes the project's text. Commands change it like
/// the project; `live` commands change only it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionState {
    /// The selected track: where live MIDI goes. `None` when no track is selected.
    pub selected_track: Option<TrackId>,
    /// The soloed tracks. Solo is session state; mute is in the project.
    pub solo: BTreeSet<TrackId>,
    /// The name of the chosen MIDI input port; `None` when none is chosen.
    pub midi_input: Option<String>,
    /// The armed tracks (RE-1): live input reaches them besides the selected track, and
    /// recording (RE-2) captures them. A session saved before arming existed has none.
    #[serde(default)]
    pub armed: BTreeSet<TrackId>,
    /// The tracks that monitor the live input through their chain (RE-1). Kept for the
    /// audio input; a MIDI track's instrument always sounds what is played.
    #[serde(default)]
    pub monitor: BTreeSet<TrackId>,
}

impl SessionState {
    /// Drops the references to tracks the project no longer has (a session restored
    /// against a project edited elsewhere, or a track deleted). Returns whether anything
    /// was dropped.
    pub fn forget_missing_tracks(&mut self, project: &Project) -> bool {
        let before = self.solo.len() + self.armed.len() + self.monitor.len();
        self.solo.retain(|t| project.tracks.contains_key(t));
        self.armed.retain(|t| project.tracks.contains_key(t));
        self.monitor.retain(|t| project.tracks.contains_key(t));
        let mut changed = self.solo.len() + self.armed.len() + self.monitor.len() != before;
        if self
            .selected_track
            .as_ref()
            .is_some_and(|t| !project.tracks.contains_key(t))
        {
            self.selected_track = None;
            changed = true;
        }
        changed
    }
}
