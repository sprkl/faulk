//! GR-2: scenes created, filled, renamed, moved, deleted with their clips and brought back
//! by undo, then launched, leave the engine in the state a snapshot of the final project
//! gives; the engine takes the stream whole, natively and in WASM.

use commands::{Command, Output, clip, history, scene, testing};
use project::{ClipId, SceneId};
use serde_json::{Value, json};
use test_support::{EngineView, Session, parity};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

fn created_scene(out: Output) -> SceneId {
    match out {
        Output::Scene(scene::Output::Create { scene }) => scene,
        other => panic!("not a scene creation: {other:?}"),
    }
}

fn created_clip(out: Output) -> ClipId {
    match out {
        Output::Clip(clip::Output::Create { clip, .. }) => clip,
        other => panic!("not a clip creation: {other:?}"),
    }
}

#[test]
fn scene_commands_and_launches_leave_the_same_engine_as_a_snapshot() {
    let mut live = Session::single_synth();
    let synth = testing::fixture().track;
    let intro = created_scene(live.exec(cmd(json!({"type": "scene.create", "name": "Intro"}))));
    let chorus = created_scene(live.exec(cmd(json!({"type": "scene.create"}))));
    let mut clips = Vec::new();
    for scene in [&intro, &chorus] {
        clips.push(created_clip(live.exec(cmd(json!({
            "type": "clip.create", "track": synth, "at": {"at": "slot", "scene": scene}
        })))));
    }
    live.run(cmd(
        json!({"type": "clip.notes.add", "clip": clips[0], "notes": [
            {"start": 0, "pitch": 60, "length": 480, "velocity": 100}
        ]}),
    ));
    live.run(cmd(
        json!({"type": "scene.rename", "scene": chorus, "name": "Chorus"}),
    ));
    live.run(cmd(
        json!({"type": "scene.move", "scene": chorus, "index": 0}),
    ));
    // A deleted scene frees its slot with its clip; undo brings both back; the next scene
    // takes the slot freed by a second delete.
    live.run(cmd(json!({"type": "scene.delete", "scene": intro})));
    live.run(Command::History(history::Command::Undo {}));
    live.run(cmd(json!({"type": "scene.delete", "scene": chorus})));
    let outro = created_scene(live.exec(cmd(json!({"type": "scene.create", "name": "Outro"}))));
    clips.push(created_clip(live.exec(cmd(json!({
        "type": "clip.create", "track": synth, "at": {"at": "slot", "scene": outro}
    })))));
    // Launches: a clip at stop (starts the transport), a scene, a stop, stop all.
    live.run(cmd(json!({"type": "launch.clip", "clip": clips[0]})));
    live.render_secs(0.3);
    live.run(cmd(json!({"type": "launch.scene", "scene": outro})));
    live.run(cmd(json!({"type": "launch.stop_track", "track": synth})));
    live.run(cmd(json!({"type": "launch.stop_all"})));
    live.render_secs(0.3);

    let state = live.state().clone();
    state.project.validate().unwrap();
    let snapshot = Session::from_state(state.clone(), live.ctx().clone());
    let expected = EngineView::of_project(&state.project, &state.session);
    assert_eq!(
        EngineView::of_stream(live.scenario(), &live.ctx().engine),
        expected,
        "the live stream"
    );
    assert_eq!(
        EngineView::of_stream(snapshot.scenario(), &snapshot.ctx().engine),
        expected,
        "the snapshot"
    );
    assert_eq!(
        expected
            .tracks
            .values()
            .map(|t| t.scenes.len())
            .sum::<usize>(),
        2,
        "the clips of intro and outro"
    );
    parity(live.scenario());
}
