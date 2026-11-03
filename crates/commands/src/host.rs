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

/// The undo history as the history panel shows it (`pw_history`): read when the panel is
/// open, not with every event, since a drag records many.
#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryView {
    /// What `history.undo` undoes, oldest first: the last one goes first.
    pub undo: Vec<HistoryEntryView>,
    /// What `history.redo` redoes, oldest first: the last one goes first.
    pub redo: Vec<HistoryEntryView>,
    /// The settings changed outside the undo history (the loop switch, the metronome),
    /// oldest first: shown among the entries, by id, as not undoable (#473).
    pub activity: Vec<ActivityView>,
}

/// One row of the activity log: a setting changed outside the undo history.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityView {
    /// From the same counter as the entries' ids: ordering both by id gives the order they
    /// happened in.
    pub id: u64,
    /// When it happened, in milliseconds since the Unix epoch (0 when the host gave no clock).
    pub at: u64,
    /// The command's name (`transport.toggle_loop`...).
    pub name: String,
    /// What it acted on, by name, when it says.
    pub subject: Option<crate::describe::Subject>,
    /// Who ran it.
    pub author: crate::Author,
}

/// One entry of the history: one command, or every command of one history group.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryEntryView {
    /// Identifies the entry for as long as it is in the history, whichever stack holds it.
    pub id: u64,
    /// When it was recorded, in milliseconds since the Unix epoch (0 when the host gave no clock).
    pub at: u64,
    /// The name of the entry's first command (`note.move`...).
    pub name: String,
    /// What the entry's commands of that name acted on, by name (#473): the panel's label
    /// names them ("Mute Lead"). `null` when a command does not say.
    pub subject: Option<crate::describe::Subject>,
    /// How many commands of the entry have another name than its first: "and n other edits".
    pub others: u32,
    /// Who ran it.
    pub author: crate::Author,
    /// The history group it belongs to, if any.
    pub group: Option<crate::GroupId>,
}

/// One file of the user's preset library, as the page hands it to the core (`pw_stage_library`,
/// #191): its id (the file's name without `.json`) and its text.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryFile {
    pub id: String,
    pub text: String,
}

/// What staging the library did: how many presets the core now holds, and the files it left
/// out, each with why (a name that is not a preset id, a file that is not a preset, one too
/// many). A file left out is never removed: the page leaves it where it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryReport {
    pub loaded: u32,
    pub skipped: Vec<SkippedPreset>,
}

/// A library file the core left out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkippedPreset {
    pub id: String,
    pub reason: String,
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

/// Where the live input plays now, as the engine knows the tracks (#598): the slots of the
/// tracks `live::input_targets` names (the selected track and the armed MIDI tracks), in the
/// `epoch` of the engine stream those slots belong to. In Boost the page sends it to the
/// helper (`live.routing`), which plays the notes of its own MIDI input with it, without a
/// round trip through the page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LiveRouting {
    pub epoch: u16,
    /// Track slots, in the order of the tracks' ids; a target the engine holds no slot for
    /// is left out.
    pub tracks: Vec<u32>,
}

impl LiveRouting {
    /// The routing of `state`'s live input with the slots of `engine`, in `epoch`.
    pub fn of(state: &crate::State, engine: &EngineSlots, epoch: u16) -> Self {
        Self {
            epoch,
            tracks: crate::live::input_targets(state)
                .iter()
                .filter_map(|track| engine.tracks.get(track))
                .collect(),
        }
    }
}

#[cfg(test)]
mod live_routing_tests {
    use super::LiveRouting;
    use crate::Command;
    use crate::engine_msgs::Synced;
    use project::TrackId;

    fn run(synced: &mut Synced, command: serde_json::Value) -> crate::Output {
        let command: Command = serde_json::from_value(command).expect("a command");
        synced.run(&command).expect("applies").0.output
    }

    fn routing(synced: &Synced) -> LiveRouting {
        LiveRouting::of(&synced.state, &synced.ctx.engine, synced.epoch())
    }

    fn create(synced: &mut Synced, kind: &str) -> TrackId {
        match run(
            synced,
            serde_json::json!({ "type": "track.create", "kind": kind }),
        ) {
            crate::Output::Track(crate::track::Output::Create { track, .. }) => track,
            other => panic!("{other:?}"),
        }
    }

    fn slot(synced: &Synced, track: &TrackId) -> u32 {
        synced.ctx.engine.tracks.get(track).expect("a slot")
    }

    #[test]
    fn the_selected_and_armed_midi_tracks_by_their_slots_in_the_streams_epoch() {
        let mut synced = Synced::single_synth(7);
        synced.setup().unwrap();
        let first = TrackId::parse(crate::engine_msgs::single_synth::TRACK).unwrap();
        assert_eq!(
            routing(&synced),
            LiveRouting {
                epoch: 7,
                tracks: vec![]
            },
            "nothing selected yet"
        );
        run(
            &mut synced,
            serde_json::json!({ "type": "view.select_track", "track": first }),
        );
        assert_eq!(
            routing(&synced),
            LiveRouting {
                epoch: 7,
                tracks: vec![slot(&synced, &first)]
            }
        );
        let second = create(&mut synced, "midi");
        let audio = create(&mut synced, "audio");
        run(
            &mut synced,
            serde_json::json!({ "type": "view.select_track", "track": second }),
        );
        for track in [&first, &audio] {
            run(
                &mut synced,
                serde_json::json!({ "type": "track.arm", "track": track, "armed": true }),
            );
        }
        let mut expected = vec![
            (first.clone(), slot(&synced, &first)),
            (second.clone(), slot(&synced, &second)),
        ];
        expected.sort();
        assert_eq!(
            routing(&synced).tracks,
            expected.into_iter().map(|(_, s)| s).collect::<Vec<_>>(),
            "the selected and the armed MIDI tracks; the armed audio track is left out"
        );
        // Nothing selected, nothing armed: no track.
        run(
            &mut synced,
            serde_json::json!({ "type": "view.select_track", "track": null }),
        );
        for track in [&first, &audio] {
            run(
                &mut synced,
                serde_json::json!({ "type": "track.arm", "track": track, "armed": false }),
            );
        }
        assert_eq!(routing(&synced).tracks, Vec::<u32>::new());
        // A deleted track is no target.
        run(
            &mut synced,
            serde_json::json!({ "type": "view.select_track", "track": second }),
        );
        run(
            &mut synced,
            serde_json::json!({ "type": "track.delete", "track": second }),
        );
        assert_eq!(routing(&synced).tracks, Vec::<u32>::new());
        // A snapshot moves to a newer epoch: the table says so.
        run(
            &mut synced,
            serde_json::json!({ "type": "view.select_track", "track": first }),
        );
        synced.resync().unwrap();
        assert_ne!(synced.epoch(), 7);
        assert_eq!(
            routing(&synced),
            LiveRouting {
                epoch: synced.epoch(),
                tracks: vec![slot(&synced, &first)]
            }
        );
    }
}
