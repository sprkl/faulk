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
    /// The settings of MIDI recording (RE-2): they belong to the machine (its latency), not
    /// to the music. A session saved before recording existed has the defaults.
    #[serde(default)]
    pub record: RecordSettings,
}

/// The settings of MIDI recording, in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordSettings {
    /// A manual latency offset, in milliseconds, -500 to 500: every recorded note is moved
    /// this much earlier, on top of the output latency the host estimates (a negative
    /// value moves them later). For an audio interface whose driver reports its latency
    /// wrong.
    #[serde(default)]
    pub offset_ms: i32,
    /// Replace mode (#500): a take removes the notes the clip already holds in the range
    /// it recorded, instead of adding to them (overdub, the default).
    #[serde(default)]
    pub replace: bool,
    /// The grid the notes of a take are quantized to when the take is written (#500);
    /// [`InputGrid::None`] (the default) keeps them as played.
    #[serde(default)]
    pub quantize_on_input: InputGrid,
    /// How far the quantization moves a note toward the grid, 0 to 100 percent.
    #[serde(default = "full_strength")]
    pub quantize_strength: u8,
}

fn full_strength() -> u8 {
    100
}

impl Default for RecordSettings {
    fn default() -> Self {
        RecordSettings {
            offset_ms: 0,
            replace: false,
            quantize_on_input: InputGrid::None,
            quantize_strength: full_strength(),
        }
    }
}

/// The grids a take can be quantized to on input: the ones of the piano roll's quantize
/// field (`clip.notes.quantize`), plus none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum InputGrid {
    /// The notes stay as played.
    #[default]
    None,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    EighthTriplet,
    SixteenthTriplet,
}

impl InputGrid {
    /// The grid step in ticks; `None` for [`InputGrid::None`].
    pub fn ticks(self) -> Option<i64> {
        let ppq = crate::PPQ;
        match self {
            InputGrid::None => None,
            InputGrid::Quarter => Some(ppq),
            InputGrid::Eighth => Some(ppq / 2),
            InputGrid::Sixteenth => Some(ppq / 4),
            InputGrid::ThirtySecond => Some(ppq / 8),
            InputGrid::EighthTriplet => Some(ppq / 3),
            InputGrid::SixteenthTriplet => Some(ppq / 6),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_saved_before_the_replace_and_quantize_settings_loads_with_their_defaults() {
        let old: RecordSettings = serde_json::from_str(r#"{"offset_ms": -12}"#).unwrap();
        assert_eq!(
            old,
            RecordSettings {
                offset_ms: -12,
                ..RecordSettings::default()
            }
        );
        assert!(!old.replace);
        assert_eq!(old.quantize_on_input, InputGrid::None);
        assert_eq!(old.quantize_strength, 100);
        let none: RecordSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(none, RecordSettings::default());
    }
}
