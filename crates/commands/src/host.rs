//! What a host of the project core tells the page (PR-4): the reports of running a
//! command and of an engine snapshot, and the page's view of the project.
//!
//! The page owns the project in both modes (spec § 2.1): `crates/project-wasm` runs these
//! on the page's main thread and returns them as JSON; their schema is in the bundle the
//! web app generates its types from (`protocol::schema_bundle`, `project_core`).
//!
//! **Opening** a project is the command `project.open` (SV-4): the host stages the folder
//! in the command context and the command reads it with `project::file` (SV-1).
//!
//! **Saving** is the page's storage applying the frames `crates/project-wasm` computes with
//! `project::file::save` (SV-2): nothing of it is a command report.

use std::collections::BTreeMap;

use project::{EngineSlots, Project, SceneId, SessionState, TrackId};
use schemars::JsonSchema;
use serde::Serialize;

use crate::{CommandError, Output, ProjectEvent};

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

/// What preparing an export (`project.export_audio`, #297) gave: the render's plan, with the
/// engine bytes returned beside it (the setup stream, then the stream that stops the
/// transport, `setup_len` bytes then the rest).
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExportReport {
    Ready {
        plan: crate::export::ExportPlan,
        setup_len: u32,
    },
    /// The project cannot be exported as asked (nothing to render, no loop region...).
    Refused { error: CommandError },
    /// The messages did not encode (`MsgError` code).
    Failed { stream_error: i32 },
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
