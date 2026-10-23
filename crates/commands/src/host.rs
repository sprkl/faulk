//! What a host of the project core tells the page (PR-4): the reports of running a
//! command and of an engine snapshot, the page's view of the project, and the project's
//! files.
//!
//! The page owns the project in both modes (spec § 2.1): `crates/project-wasm` runs these
//! on the page's main thread and returns them as JSON; their schema is in the bundle the
//! web app generates its types from (`protocol::schema_bundle`, `project_core`).
//!
//! **Opening** a project is the command `project.open` (SV-4): the host stages the folder
//! in the command context and the command reads it with `project::file` (SV-1).
//!
//! **Interim files** for saving, until SV-2 (#95) writes the folder format with the dirty
//! units the commands mark: [`PROJECT_FILE`] holds the whole serde [`Project`] and
//! [`SESSION_FILE`] the [`SessionState`]. They are *not* what `project.open` reads: nothing
//! reads them back, and [`save_files`] is replaced by SV-2's `project::file::save`.

use std::collections::BTreeMap;

use project::{EngineSlots, Project, SceneId, SessionState, TrackId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{CommandError, Output, ProjectEvent, State};

/// The file of the project document (interim: the whole project, see the module docs). Kept under `.faulk/`, apart from the `project.json` of the folder format, which an interim write must never overwrite.
pub const PROJECT_FILE: &str = ".faulk/interim-project.json";
/// The file of the session state: ignored by git, never versioned.
pub const SESSION_FILE: &str = ".faulk/session.json";

/// Files of a project folder: path relative to the folder → text.
pub type ProjectFiles = BTreeMap<String, String>;

/// What to change in the project folder: files to write (path → text) and files to
/// remove. The interim files never remove anything; the folder format of SV-1 removes
/// the files of a deleted track or clip.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectChanges {
    pub write: ProjectFiles,
    pub remove: Vec<String>,
}

impl ProjectChanges {
    pub fn is_empty(&self) -> bool {
        self.write.is_empty() && self.remove.is_empty()
    }
}

/// What asking for the project's changes gave.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SaveReport {
    /// The changes since the last recorded write (empty when the folder is up to date).
    Changes { changes: ProjectChanges },
    /// The project did not serialize (a bug: the commands keep it valid); nothing to
    /// write, and the project in memory is untouched. `message` is for developers.
    Failed { message: String },
}

/// What running a command did.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunReport {
    /// The command applied: its typed output and its events. Its engine messages are
    /// returned beside this report, as bytes; `stream_error` is the `MsgError` code when
    /// they did not encode: the project changed, the engine is behind it until a
    /// snapshot.
    Applied {
        output: Output,
        events: Vec<ProjectEvent>,
        stream_error: Option<i32>,
    },
    /// The project refused the command: nothing changed.
    Refused { error: CommandError },
    /// The JSON is not a command this build knows: nothing changed.
    Invalid { message: String },
}

/// What encoding a snapshot for the engine did: the bytes are returned beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotReport {
    /// The engine stream starts with a `Reset` to `epoch`.
    Encoded { epoch: u16 },
    /// The messages did not encode (`MsgError` code): no bytes.
    Failed { stream_error: i32 },
}

/// The page's view of the project: what it reads after an event that names an entity
/// (`track_created`, `clip_created`), from its own copy of the project.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectView {
    pub project: Project,
    pub session: SessionState,
    /// Where the engine holds the tracks and the scenes: the key of the engine's feedback.
    pub slots: ViewSlots,
}

/// The engine slots of the tracks and scenes that have one. The engine reports the state of
/// a launch-grid cell by slots (its track slot, its scene slot), never by identifier; the
/// launch grid reads this table to find the cell of a report. An entity the engine was never
/// told about is absent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewSlots {
    /// The epoch of the engine stream these slots belong to: a report of another epoch (an
    /// engine behind a snapshot) names other slots and is not read against this table.
    pub epoch: u16,
    pub tracks: BTreeMap<TrackId, u32>,
    pub scenes: BTreeMap<SceneId, u32>,
}

impl ViewSlots {
    /// The slots of `engine`, in the stream's `epoch`.
    pub fn of(engine: &EngineSlots, epoch: u16) -> Self {
        Self {
            epoch,
            tracks: engine
                .tracks
                .iter()
                .map(|(slot, id)| (id.clone(), slot))
                .collect(),
            scenes: engine
                .scenes
                .iter()
                .map(|(slot, id)| (id.clone(), slot))
                .collect(),
        }
    }
}

/// The files of a state (interim, see the module docs); an error when a value does not
/// serialize (the project's writers refuse a non-finite number).
pub fn save_files(state: &State) -> Result<ProjectFiles, serde_json::Error> {
    let mut files = ProjectFiles::new();
    files.insert(PROJECT_FILE.to_owned(), to_text(&state.project)?);
    files.insert(SESSION_FILE.to_owned(), to_text(&state.session)?);
    Ok(files)
}

fn to_text<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_msgs::single_synth;

    #[test]
    fn the_interim_files_are_the_project_and_the_session() {
        let state = single_synth::state();
        let files = save_files(&state).expect("a valid state serializes");
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            [PROJECT_FILE, SESSION_FILE]
        );
        let project: Project = serde_json::from_str(&files[PROJECT_FILE]).unwrap();
        assert_eq!(project, state.project);
    }

    #[test]
    fn a_value_that_does_not_serialize_is_an_error_not_a_panic() {
        let mut state = single_synth::state();
        state.project.master.pan = f64::NAN;
        assert!(save_files(&state).is_err());
    }
}
