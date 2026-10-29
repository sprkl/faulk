//! The project → engine compiler (PR-3) against the engine: commands applied live and the
//! snapshot of the final project leave the engine in the same state and render the same,
//! natively and in WASM; the slot tables reuse freed slots and grow.

use commands::{Command, Output, clip, history, track};
use project::{
    AutomationLane, AutomationPoint, AutomationTarget, ClipId, MixerParam, OrderKey, Scene,
    SceneId, Shape, Tick, TrackId,
};
use serde_json::{Value, json};
use test_support::{EngineView, Session, parity, render_native};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

fn created_track(out: Output) -> TrackId {
    match out {
        Output::Track(track::Output::Create { track, .. })
        | Output::Track(track::Output::Duplicate { track }) => track,
        other => panic!("not a track creation: {other:?}"),
    }
}

fn created_clip(out: Output) -> ClipId {
    match out {
        Output::Clip(clip::Output::Create { clip, .. })
        | Output::Clip(clip::Output::Duplicate { clip }) => clip,
        other => panic!("not a clip creation: {other:?}"),
    }
}

/// The fixture's project with two scenes and two lanes on its synth track (a synth
/// parameter and the track's pan), which no command creates yet.
fn start() -> (Session, TrackId, [SceneId; 2]) {
    let f = commands::testing::fixture();
    let mut state = f.state.clone();
    let scenes = [
        SceneId::parse("0000000000000000000000005A").unwrap(),
        SceneId::parse("0000000000000000000000005B").unwrap(),
    ];
    for (i, s) in scenes.iter().enumerate() {
        state.project.scenes.insert(
            s.clone(),
            Scene {
                name: format!("S{i}"),
                order: OrderKey::parse(["a0", "a1"][i]).unwrap(),
            },
        );
    }
    let t = state.project.tracks.get_mut(&f.track).unwrap();
    let point = |v| AutomationPoint {
        value: v,
        shape: Shape::Linear,
    };
    t.automation.insert(
        project::LaneId::parse("0000000000000000000000005C").unwrap(),
        AutomationLane {
            target: AutomationTarget::Device {
                device: f.device.clone(),
                param: "gain".into(),
            },
            points: [(Tick(0), point(0.2)), (Tick(960), point(0.8))].into(),
        },
    );
    t.automation.insert(
        project::LaneId::parse("0000000000000000000000005D").unwrap(),
        AutomationLane {
            target: AutomationTarget::Mixer {
                param: MixerParam::Pan,
            },
            points: [(Tick(0), point(-0.5))].into(),
        },
    );
    state.project.validate().expect("a valid start");
    let s = Session::from_state(state, commands::testing::context());
    (s, f.track, scenes)
}

/// A session of edits of every kind the compiler maps: tracks created, deleted, brought
/// back by undo, duplicated and moved; an audio track (its clip player is no engine
/// device); clips in scenes and in the arrangement, their notes, loop and length, a
/// duplicate, a delete and its undo; mixer, solo, selection; tempo, signature, loop,
/// metronome; synth parameters.
fn edit(s: &mut Session, synth_track: &TrackId, scenes: &[SceneId; 2]) -> Vec<TrackId> {
    let a = created_track(s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))));
    let b = created_track(s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))));
    let audio = created_track(s.exec(cmd(json!({"type": "track.create", "kind": "audio"}))));
    let c = created_track(s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))));
    // A deleted track frees its slot; the next track takes it.
    s.run(cmd(json!({"type": "track.delete", "track": b})));
    let d = created_track(s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))));
    let in_slot = created_clip(s.exec(cmd(json!({
        "type": "clip.create", "track": a, "at": {"at": "slot", "scene": scenes[0]}
    }))));
    let placed = created_clip(s.exec(cmd(json!({
        "type": "clip.create", "track": a, "at": {"at": "arrangement", "start": 3840}
    }))));
    s.run(cmd(
        json!({"type": "clip.notes.add", "clip": in_slot, "notes": [
            {"start": 0, "pitch": 60, "length": 480, "velocity": 100},
            {"start": 960, "pitch": 64, "length": 480, "velocity": 90, "cents": 10}
        ]}),
    ));
    s.run(cmd(
        json!({"type": "clip.set_loop", "clip": in_slot, "looping": {"start": 0, "end": 1920}}),
    ));
    s.run(cmd(
        json!({"type": "clip.notes.add", "clip": placed, "notes": [
            {"start": 0, "pitch": 48, "length": 960, "velocity": 110}
        ]}),
    ));
    s.run(cmd(
        json!({"type": "clip.set_length", "clip": placed, "length": 7680}),
    ));
    let copy = created_clip(s.exec(cmd(json!({"type": "clip.duplicate", "clip": in_slot}))));
    s.run(cmd(
        json!({"type": "clip.rename", "clip": copy, "name": "Copy"}),
    ));
    s.run(cmd(json!({"type": "clip.delete", "clip": placed})));
    s.run(Command::History(history::Command::Undo {}));
    s.run(cmd(json!({"type": "clip.delete", "clip": copy})));
    // A clip moved to another scene, to another track, and back by undo: its engine slot
    // follows (the clip leaves the old cell and fills the new one).
    s.run(cmd(
        json!({"type": "clip.move", "clip": in_slot, "slot": scenes[1]}),
    ));
    s.run(cmd(
        json!({"type": "clip.move", "clip": in_slot, "track": d, "slot": scenes[0]}),
    ));
    s.run(cmd(
        json!({"type": "clip.move", "clip": in_slot, "track": a, "slot": scenes[0]}),
    ));
    // A clip that also plays in the arrangement: into the grid, out of it, and the undo of a move.
    s.run(cmd(
        json!({"type": "clip.move", "clip": placed, "slot": scenes[1]}),
    ));
    s.run(Command::History(history::Command::Undo {}));
    s.run(cmd(
        json!({"type": "clip.move", "clip": placed, "slot": scenes[1]}),
    ));
    s.run(cmd(
        json!({"type": "clip.move", "clip": placed, "slot": null}),
    ));
    s.run(Command::History(history::Command::Undo {}));
    let on_c = created_clip(s.exec(cmd(json!({
        "type": "clip.create", "track": c, "at": {"at": "arrangement", "start": 0}
    }))));
    s.run(cmd(
        json!({"type": "clip.notes.add", "clip": on_c, "notes": [
            {"start": 0, "pitch": 67, "length": 240, "velocity": 80}
        ]}),
    ));
    // Delete a track with clips, then bring it back.
    s.run(cmd(json!({"type": "track.delete", "track": c})));
    s.run(Command::History(history::Command::Undo {}));
    let dup = created_track(s.exec(cmd(json!({"type": "track.duplicate", "track": a}))));
    s.run(cmd(json!({"type": "track.move", "track": dup, "index": 0})));
    s.run(cmd(
        json!({"type": "track.rename", "track": d, "name": "D"}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_volume", "track": a, "gain_db": -6.0}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_pan", "track": d, "pan": 0.5}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_mute", "track": audio, "mute": true}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_volume", "track": c, "gain_db": null}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_master_volume", "gain_db": -3.0}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_solo", "track": d, "solo": true}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_solo", "track": synth_track, "solo": true}),
    ));
    s.run(cmd(
        json!({"type": "mixer.set_solo", "track": d, "solo": false}),
    ));
    // Arming and monitoring: set, cleared, and on an audio track.
    for (kind, field, track, on) in [
        ("track.arm", "armed", &a, true),
        ("track.arm", "armed", &audio, true),
        ("track.arm", "armed", synth_track, true),
        ("track.arm", "armed", synth_track, false),
        ("track.set_monitor", "monitor", &d, true),
        ("track.set_monitor", "monitor", &a, true),
        ("track.set_monitor", "monitor", &a, false),
    ] {
        s.run(cmd(json!({"type": kind, "track": track, field: on})));
    }
    s.run(cmd(json!({"type": "view.select_track", "track": a})));
    s.run(cmd(json!({"type": "transport.set_tempo", "bpm": 132.5})));
    s.run(cmd(
        json!({"type": "transport.set_time_signature", "numerator": 7, "denominator": 8}),
    ));
    s.run(cmd(
        json!({"type": "transport.set_loop", "loop_region": {"start": 0, "end": 3840}}),
    ));
    s.run(cmd(json!({"type": "transport.toggle_loop"})));
    s.run(cmd(
        json!({"type": "transport.set_metronome", "enabled": true, "gain_db": -12.0}),
    ));
    let device_of = |s: &Session, t: &TrackId| {
        s.state().project.tracks[t]
            .devices
            .keys()
            .next()
            .unwrap()
            .clone()
    };
    for (t, wave, gain) in [(&a, 1.0, 0.3), (&d, 2.0, 0.25), (synth_track, 0.0, 0.4)] {
        let dev = device_of(s, t);
        s.run(cmd(
            json!({"type": "device.set_param", "device": dev, "param": "waveform", "value": wave}),
        ));
        s.run(cmd(
            json!({"type": "device.set_param", "device": dev, "param": "gain", "value": gain}),
        ));
        s.run(cmd(
            json!({"type": "device.set_param", "device": dev, "param": "release", "value": 0.05}),
        ));
    }
    // An edit undone and redone.
    s.run(cmd(json!({"type": "transport.set_tempo", "bpm": 90.0})));
    s.run(Command::History(history::Command::Undo {}));
    s.run(Command::History(history::Command::Redo {}));
    vec![a, c, d, dup, audio]
}

#[test]
fn live_edits_and_the_snapshot_of_the_final_project_leave_the_same_engine() {
    let (mut live, synth, scenes) = start();
    edit(&mut live, &synth, &scenes);
    let state = live.state().clone();
    let snapshot = Session::from_state(state.clone(), live.ctx().clone());

    let expected = EngineView::of_project(&state.project, &state.session);
    let from_live = EngineView::of_stream(live.scenario(), &live.ctx().engine);
    let from_snapshot = EngineView::of_stream(snapshot.scenario(), &snapshot.ctx().engine);
    assert_eq!(from_live, expected, "the live stream");
    assert_eq!(from_snapshot, expected, "the snapshot");
    // What the comparison covers.
    assert_eq!(expected.tracks.len(), 6);
    assert_eq!(expected.clips.len(), 5, "a: 2, c: 1, the copy of a: 2");
    assert_eq!(expected.lanes.len(), 2);
    assert!(expected.tracks.values().any(|t| !t.scenes.is_empty()));
    assert!(expected.tracks.values().any(|t| !t.placements.is_empty()));
    assert!(expected.loop_region.is_some() && expected.selected.is_some());
    assert!(
        expected.tracks.values().any(|t| t.armed) && expected.tracks.values().any(|t| t.monitor)
    );
    // Both streams are accepted whole by the engine, natively and in WASM.
    let mut l = live;
    l.render_blocks(4);
    parity(l.scenario());
}

#[test]
fn live_and_snapshot_render_the_same_natively_and_in_wasm() {
    let (mut live, synth, scenes) = start();
    let tracks = edit(&mut live, &synth, &scenes);
    let mut snapshot = Session::from_state(live.state().clone(), live.ctx().clone());
    // The same playing on both engines: notes on every MIDI track, the transport with the
    // metronome (each stream applies its messages first: 128 per block at most).
    for s in [&mut live, &mut snapshot] {
        // The live session's stopped gestures (the edits) hold until a seek gives the lanes back (AM-2).
        s.run(cmd(json!({"type": "transport.seek", "tick": 0})));
        s.render_secs(0.2);
        s.run(cmd(
            json!({"type": "mixer.set_solo", "track": synth, "solo": false}),
        ));
        for (i, t) in tracks.iter().chain([&synth]).enumerate() {
            if s.state().project.tracks[t].kind == project::TrackKind::Midi {
                s.run(cmd(json!({"type": "live.note_on", "track": t, "note": 57 + 4 * i, "velocity": 100})));
            }
        }
        s.run(cmd(json!({"type": "transport.play"})));
        s.render_secs(0.6);
        s.run(cmd(json!({"type": "transport.seek", "tick": 960})));
        s.run(cmd(json!({"type": "live.all_notes_off"})));
        s.render_secs(0.3);
        s.run(cmd(json!({"type": "transport.stop"})));
        s.render_secs(0.2);
    }
    let a = parity(live.scenario());
    let b = parity(snapshot.scenario());
    a.assert_clean();
    a.assert_no_denormals();
    a.assert_audible(0.25..0.75, 0.01);
    // The tracks sum in slot order, which differs between the two streams.
    a.assert_close(&b, 1e-6);
    a.write_wav("pr3-live");
    b.write_wav("pr3-snapshot");
}

#[test]
fn freed_track_slots_are_reused_and_the_engine_table_grows() {
    let (mut s, _, _) = start();
    let mut tracks = Vec::new();
    for _ in 0..100 {
        tracks.push(created_track(
            s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))),
        ));
    }
    let slot = |s: &Session, t: &TrackId| s.ctx().engine.tracks.get(t).unwrap();
    assert_eq!(
        s.ctx().engine.tracks.span(),
        101,
        "slots 0 to 100: the synth, then 100"
    );
    let freed: Vec<u32> = [10, 50, 3].iter().map(|i| slot(&s, &tracks[*i])).collect();
    for i in [10, 50, 3] {
        s.run(cmd(json!({"type": "track.delete", "track": tracks[i]})));
    }
    let mut reused = Vec::new();
    for _ in 0..4 {
        let t = created_track(s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))));
        reused.push(slot(&s, &t));
    }
    let mut lowest = freed.clone();
    lowest.sort();
    lowest.push(101);
    assert_eq!(
        reused, lowest,
        "the lowest freed slots first, then the next one"
    );
    s.render_blocks(8);
    // Every message accepted (the harness panics on a rejected or skipped one); the engine
    // grew its track table past its initial capacity.
    render_native(s.scenario());
    assert_eq!(
        EngineView::of_stream(s.scenario(), &s.ctx().engine),
        EngineView::of_project(&s.state().project, &s.state().session)
    );
}

#[test]
fn clip_slots_grow_past_a_page_and_freed_ones_are_reused() {
    let (mut s, synth, _) = start();
    let mut clips = Vec::new();
    // 1 100 clips: past the engine's first page of 1 024 clip slots.
    for i in 0..1100i64 {
        clips.push(created_clip(s.exec(cmd(json!({
            "type": "clip.create", "track": synth, "at": {"at": "arrangement", "start": i * 960}, "length": 960
        })))));
    }
    assert_eq!(s.ctx().engine.clips.span(), 1100);
    let freed = s.ctx().engine.clips.get(&clips[7]).unwrap();
    s.run(cmd(json!({"type": "clip.delete", "clip": clips[7]})));
    let again = created_clip(s.exec(cmd(json!({
        "type": "clip.create", "track": synth, "at": {"at": "arrangement", "start": 1100 * 960}, "length": 960
    }))));
    assert_eq!(s.ctx().engine.clips.get(&again), Some(freed));
    s.render_blocks(40);
    render_native(s.scenario());
    assert_eq!(
        EngineView::of_stream(s.scenario(), &s.ctx().engine),
        EngineView::of_project(&s.state().project, &s.state().session)
    );
}
