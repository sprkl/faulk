//! What a host of the project core tells the page (PR-4): the reports of running a
//! command, of an engine snapshot and of loading a project, the page's view of the
//! project, and the project's files.
//!
//! The page owns the project in both modes (spec § 2.1): `crates/project-wasm` runs these
//! on the page's main thread and returns them as JSON; their schema is in the bundle the
//! web app generates its types from (`protocol::schema_bundle`, `project_core`).
//!
//! **Interim files** until the folder format of SV-1 (#93): [`PROJECT_FILE`] holds the
//! whole serde [`Project`] (its `format` field included) and [`SESSION_FILE`] the
//! [`SessionState`]. SV-1 replaces [`save_files`] and [`load_files`] with its canonical
//! writer and reader (one file per track, TSV, blobs) behind the same `load(files)` and
//! `save()` of the page.

use std::collections::BTreeMap;

use project::{EngineSlots, FORMAT_VERSION, FormatProbe, Project, SceneId, SessionState, TrackId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{CommandError, Output, ProjectEvent, State};

/// The file of the project document (interim: the whole project, see the module docs).
pub const PROJECT_FILE: &str = "project.json";
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

/// What loading a project did.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum LoadReport {
    /// The project replaced the previous one, with an empty history; the engine bytes
    /// beside this report reset the engine to it (`snapshot`).
    /// `session_reset` when the session file was unreadable and the default session was
    /// used instead: the session is machine-local and never blocks opening a project.
    Loaded {
        snapshot: SnapshotReport,
        session_reset: bool,
    },
    /// The files were refused: nothing changed.
    Refused { error: LoadError },
}

/// Why project files were refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum LoadError {
    /// A required file is missing.
    MissingFile { file: String },
    /// The project's format version is not the one this build reads (newer, or older
    /// without a migration).
    Format { found: u32, supported: u32 },
    /// A file is not valid JSON of its type; `message` is for developers.
    Parse { file: String, message: String },
    /// The project breaks its invariants (`Project::validate`), one line per violation.
    Invalid { violations: Vec<String> },
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

/// A project read from files.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// The project and its session, with an empty history.
    pub state: State,
    /// The session file was there but unreadable: the default session was used.
    pub session_reset: bool,
}

/// A state read from project files, with an empty history. The session file is
/// optional: absent or unreadable (another build's, a hand edit), the default session
/// is used, since it is machine-local and never versioned; a session that names tracks
/// the project no longer has forgets them.
pub fn load_files(files: &ProjectFiles) -> Result<Loaded, LoadError> {
    let text = files
        .get(PROJECT_FILE)
        .ok_or_else(|| LoadError::MissingFile {
            file: PROJECT_FILE.to_owned(),
        })?;
    // The format first: a newer project fails as such, not as an opaque parse error.
    let probe: FormatProbe = serde_json::from_str(text).map_err(|e| parse(PROJECT_FILE, &e))?;
    if probe.format != FORMAT_VERSION {
        return Err(LoadError::Format {
            found: probe.format,
            supported: FORMAT_VERSION,
        });
    }
    let project: Project = serde_json::from_str(text).map_err(|e| parse(PROJECT_FILE, &e))?;
    project
        .validate()
        .map_err(|violations| LoadError::Invalid {
            violations: violations.iter().map(ToString::to_string).collect(),
        })?;
    let (mut session, session_reset) = match files.get(SESSION_FILE) {
        Some(text) => match serde_json::from_str::<SessionState>(text) {
            Ok(session) => (session, false),
            Err(_) => (SessionState::default(), true),
        },
        None => (SessionState::default(), false),
    };
    session.forget_missing_tracks(&project);
    let mut state = State::new(project);
    state.session = session;
    Ok(Loaded {
        state,
        session_reset,
    })
}

fn parse(file: &str, error: &serde_json::Error) -> LoadError {
    LoadError::Parse {
        file: file.to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_msgs::single_synth;

    fn files(project: &str) -> ProjectFiles {
        ProjectFiles::from([(PROJECT_FILE.to_owned(), project.to_owned())])
    }

    fn saved(state: &State) -> ProjectFiles {
        save_files(state).expect("a valid state serializes")
    }

    #[test]
    fn save_then_load_gives_the_state_and_the_same_files() {
        let mut state = single_synth::state();
        state.session.selected_track = Some(single_synth::track());
        let files = saved(&state);
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            [SESSION_FILE, PROJECT_FILE]
        );
        let loaded = load_files(&files).expect("its own files load");
        assert!(!loaded.session_reset);
        assert_eq!(loaded.state.project, state.project);
        assert_eq!(loaded.state.session, state.session);
        assert_eq!(saved(&loaded.state), files);
    }

    #[test]
    fn a_missing_or_unreadable_session_is_the_default_one() {
        let text = saved(&single_synth::state())[PROJECT_FILE].clone();
        let loaded = load_files(&files(&text)).unwrap();
        assert_eq!(loaded.state.session, SessionState::default());
        assert!(!loaded.session_reset, "absent is not unreadable");
        for bad in ["[]", "not json", r#"{"view":{}}"#] {
            let mut with_session = files(&text);
            with_session.insert(SESSION_FILE.to_owned(), bad.to_owned());
            let loaded = load_files(&with_session).expect("a session never blocks the project");
            assert_eq!(loaded.state.session, SessionState::default());
            assert!(loaded.session_reset, "{bad}");
        }
    }

    #[test]
    fn a_session_forgets_tracks_the_project_does_not_have() {
        let mut files = saved(&single_synth::state());
        files.insert(
            SESSION_FILE.to_owned(),
            r#"{"selected_track":"00000000000000000000000009","solo":["00000000000000000000000009"],"midi_input":null}"#.to_owned(),
        );
        let loaded = load_files(&files).unwrap();
        assert_eq!(loaded.state.session, SessionState::default());
        assert!(!loaded.session_reset);
    }

    #[test]
    fn refusals_are_typed() {
        assert!(matches!(
            load_files(&ProjectFiles::new()),
            Err(LoadError::MissingFile { file }) if file == PROJECT_FILE
        ));
        let text = saved(&single_synth::state())[PROJECT_FILE].clone();
        let newer = text.replacen(
            &format!("\"format\": {FORMAT_VERSION}"),
            &format!("\"format\": {}", FORMAT_VERSION + 1),
            1,
        );
        assert_ne!(newer, text);
        assert!(matches!(
            load_files(&files(&newer)),
            Err(LoadError::Format { found, supported })
                if found == FORMAT_VERSION + 1 && supported == FORMAT_VERSION
        ));
        assert!(matches!(
            load_files(&files("not json")),
            Err(LoadError::Parse { file, .. }) if file == PROJECT_FILE
        ));
        assert!(matches!(
            load_files(&files(&text.replacen("\"name\"", "\"nom\"", 1))),
            Err(LoadError::Parse { .. })
        ));
        let empty_name = text.replacen("\"name\": \"Untitled\"", "\"name\": \"\"", 1);
        assert_ne!(empty_name, text);
        assert!(matches!(
            load_files(&files(&empty_name)),
            Err(LoadError::Invalid { violations }) if !violations.is_empty()
        ));
    }

    #[test]
    fn a_value_that_does_not_serialize_is_an_error_not_a_panic() {
        let mut state = single_synth::state();
        state.project.master.pan = f64::NAN;
        assert!(save_files(&state).is_err());
    }
}
