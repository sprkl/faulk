//! Each command's engine messages (PR-3, `engine_msgs::compile`), on a context synced
//! with its engine by a snapshot: the message of every input, an undo's messages exactly
//! once, the rollback of the slot table, chunked uploads, and the playback orders.

use commands::engine_msgs::{CHUNK_ITEMS, Synced, resync, snapshot};
use commands::{Command, ControlMsg, Output, State, testing};
use engine::msg::{ClipSlot, DeviceSlot, ParamId, SceneSlot, TrackSlot};
use engine::time::LoopRegion;
use project::{OrderKey, Scene, SceneId, Tick, TrackId};
use serde_json::{Value, json};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

/// The fixture (one synth track, slot 0) with one scene, synced.
fn synced() -> (Synced, TrackId, SceneId) {
    let f = testing::fixture();
    let mut state: State = f.state.clone();
    let scene = SceneId::parse("0000000000000000000000005A").unwrap();
    state.project.scenes.insert(
        scene.clone(),
        Scene {
            name: "S".into(),
            order: OrderKey::first(),
        },
    );
    let mut s = Synced::new(state, testing::context(), 1);
    s.setup().unwrap();
    (s, f.track, scene)
}

fn run(s: &mut Synced, v: Value) -> (Output, Vec<ControlMsg>) {
    let (outcome, _) = s
        .run(&cmd(v.clone()))
        .unwrap_or_else(|e| panic!("{v}: {e:?}"));
    (outcome.output, outcome.control)
}

fn track_of(out: &Output) -> TrackId {
    match out {
        Output::Track(commands::track::Output::Create { track, .. }) => track.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_snapshot_starts_with_the_settings_then_each_track() {
    let (s, _, _) = synced();
    let (msgs, slots) = snapshot(&s.state.project, &s.state.session);
    assert!(matches!(msgs[0], ControlMsg::SetTransport { .. }));
    assert!(matches!(msgs[1], ControlMsg::SetMetronome { .. }));
    assert!(matches!(
        msgs[2],
        ControlMsg::SetMaster {
            gain: 1.0,
            pan: 0.0,
            mute: false
        }
    ));
    assert!(matches!(
        msgs[3],
        ControlMsg::TrackCreate {
            track: TrackSlot(0),
            ..
        }
    ));
    assert!(matches!(
        msgs[4],
        ControlMsg::TrackSetMix {
            track: TrackSlot(0),
            ..
        }
    ));
    let ControlMsg::DeviceInsert { device, state, .. } = &msgs[5] else {
        panic!("{:?}", msgs[5]);
    };
    assert_eq!(*device, DeviceSlot(0));
    assert_eq!(
        engine::msg::state_pairs(state).count(),
        6,
        "the synth's six parameters travel with it"
    );
    assert_eq!(msgs.len(), 6);
    assert_eq!(slots.tracks.len(), 1);
    assert_eq!(slots.scenes.len(), 1);
    // `resync` puts a `Reset` first and adopts the table.
    let mut ctx = testing::context();
    let all = resync(&s.state, &mut ctx, 9);
    assert_eq!(all[0], ControlMsg::Reset { epoch: 9 });
    assert_eq!(all[1..], msgs[..]);
    assert_eq!(ctx.engine, slots);
}

#[test]
fn each_edit_compiles_to_its_messages() {
    let (mut s, synth, scene) = synced();
    let (out, msgs) = run(&mut s, json!({"type": "track.create", "kind": "midi"}));
    let a = track_of(&out);
    assert!(
        matches!(
            msgs[..],
            [
                ControlMsg::TrackCreate {
                    track: TrackSlot(1),
                    ..
                },
                ControlMsg::TrackSetMix {
                    track: TrackSlot(1),
                    gain: 1.0,
                    pan: 0.0,
                    mute: false
                },
                ControlMsg::DeviceInsert {
                    track: TrackSlot(1),
                    device: DeviceSlot(0),
                    ..
                },
            ]
        ),
        "{msgs:?}"
    );
    let (_, msgs) = run(
        &mut s,
        json!({"type": "mixer.set_volume", "track": a, "gain_db": null}),
    );
    assert_eq!(
        msgs,
        [ControlMsg::TrackSetMix {
            track: TrackSlot(1),
            gain: 0.0,
            pan: 0.0,
            mute: false
        }]
    );
    let (_, msgs) = run(
        &mut s,
        json!({"type": "mixer.set_master_volume", "gain_db": -20.0}),
    );
    assert!(
        matches!(msgs[..], [ControlMsg::SetMaster { gain, mute: false, .. }] if (gain - 0.1).abs() < 1e-6)
    );
    let (_, msgs) = run(&mut s, json!({"type": "mixer.set_master_pan", "pan": -0.5}));
    assert!(
        matches!(msgs[..], [ControlMsg::SetMaster { pan, .. }] if (pan + 0.5).abs() < 1e-6),
        "{msgs:?}"
    );
    let (_, msgs) = run(
        &mut s,
        json!({"type": "mixer.set_solo", "track": a, "solo": true}),
    );
    assert_eq!(
        msgs,
        [ControlMsg::TrackSetSolo {
            track: TrackSlot(1),
            solo: true
        }]
    );
    let (_, msgs) = run(&mut s, json!({"type": "view.select_track", "track": a}));
    assert_eq!(
        msgs,
        [ControlMsg::SelectTrack {
            track: Some(TrackSlot(1))
        }]
    );
    let device = s.state.project.tracks[&a]
        .devices
        .keys()
        .next()
        .unwrap()
        .clone();
    let (_, msgs) = run(
        &mut s,
        json!({"type": "device.set_param", "device": device, "param": "gain", "value": 0.5}),
    );
    let gain = engine::Param::Gain.id();
    assert_eq!(
        msgs,
        [ControlMsg::ParamSet {
            track: TrackSlot(1),
            device: DeviceSlot(0),
            param: ParamId(gain),
            value: 0.5
        }]
    );
    let (_, msgs) = run(&mut s, json!({"type": "transport.set_tempo", "bpm": 100.0}));
    assert!(matches!(msgs[..], [ControlMsg::SetTransport { tempo, .. }] if tempo.bpm() == 100.0));
    let (_, msgs) = run(
        &mut s,
        json!({"type": "transport.set_launch_quantize", "quantize": "bars_4"}),
    );
    assert!(matches!(
        msgs[..],
        [ControlMsg::SetTransport {
            launch_quantize: engine::time::Quantize::Bars4,
            ..
        }]
    ));
    let (_, msgs) = run(
        &mut s,
        json!({"type": "transport.set_loop", "loop_region": {"start": 0, "end": 960}}),
    );
    assert_eq!(
        msgs,
        [ControlMsg::SetLoop { region: None }],
        "a region without the loop on"
    );
    let (_, msgs) = run(&mut s, json!({"type": "transport.toggle_loop"}));
    assert!(matches!(
        msgs[..],
        [ControlMsg::SetLoop { region: Some(_) }]
    ));
    let (_, msgs) = run(
        &mut s,
        json!({"type": "transport.set_metronome", "enabled": true, "gain_db": null}),
    );
    assert_eq!(
        msgs,
        [ControlMsg::SetMetronome {
            enabled: true,
            volume: 0.0,
            count_in_bars: 1
        }]
    );
    // Clips: in a scene, renamed (nothing), changed, deleted.
    let (out, msgs) = run(
        &mut s,
        json!({"type": "clip.create", "track": synth, "at": {"at": "slot", "scene": scene}}),
    );
    let Output::Clip(commands::clip::Output::Create { clip, .. }) = out else {
        panic!()
    };
    assert!(
        matches!(
            msgs[..],
            [
                ControlMsg::ClipNotesUpload {
                    clip: ClipSlot(0),
                    first: 0,
                    total: 0,
                    ..
                },
                ControlMsg::SceneSetSlot {
                    track: TrackSlot(0),
                    scene: SceneSlot(0),
                    clip: Some(ClipSlot(0))
                },
                ControlMsg::TrackPlacementsUpload {
                    track: TrackSlot(0),
                    total: 0,
                    ..
                },
            ]
        ),
        "{msgs:?}"
    );
    let (_, msgs) = run(
        &mut s,
        json!({"type": "clip.rename", "clip": clip, "name": "x"}),
    );
    assert_eq!(msgs, []);
    let (_, msgs) = run(
        &mut s,
        json!({"type": "clip.notes.add", "clip": clip, "notes": [{"start": 0, "pitch": 60, "length": 10, "velocity": 1}]}),
    );
    assert!(matches!(
        msgs[..],
        [ControlMsg::ClipNotesUpload {
            clip: ClipSlot(0),
            total: 1,
            ..
        }]
    ));
    let (_, msgs) = run(&mut s, json!({"type": "clip.delete", "clip": clip}));
    assert!(
        matches!(
            msgs[..],
            [
                ControlMsg::SceneSetSlot {
                    track: TrackSlot(0),
                    scene: SceneSlot(0),
                    clip: None
                },
                ControlMsg::TrackPlacementsUpload {
                    track: TrackSlot(0),
                    total: 0,
                    ..
                },
                ControlMsg::ClipRemove { clip: ClipSlot(0) },
            ]
        ),
        "{msgs:?}"
    );
    // Nothing for the engine.
    for v in [
        json!({"type": "project.rename", "name": "Other"}),
        json!({"type": "track.rename", "track": a, "name": "A"}),
        json!({"type": "track.move", "track": a, "index": 0}),
        json!({"type": "midi.list_inputs"}),
    ] {
        assert_eq!(run(&mut s, v.clone()).1, [], "{v}");
    }
    // Delete: the track and what it holds go, the selection and the solo follow.
    let (_, msgs) = run(&mut s, json!({"type": "track.delete", "track": a}));
    assert_eq!(
        msgs,
        [
            ControlMsg::TrackRemove {
                track: TrackSlot(1)
            },
            ControlMsg::SelectTrack { track: None },
        ]
    );
    assert_eq!(s.ctx.engine.tracks.get(&a), None);
}

#[test]
fn an_undo_compiles_each_message_once() {
    let (mut s, synth, scene) = synced();
    run(
        &mut s,
        json!({"type": "clip.create", "track": synth, "at": {"at": "slot", "scene": scene}}),
    );
    let (_, deleted) = run(&mut s, json!({"type": "track.delete", "track": synth}));
    assert!(
        matches!(
            deleted[..],
            [
                ControlMsg::TrackRemove {
                    track: TrackSlot(0)
                },
                ControlMsg::ClipRemove { clip: ClipSlot(0) }
            ]
        ),
        "{deleted:?}"
    );
    let before_undo = s.ctx.engine.clone();
    let (_, undone) = run(&mut s, json!({"type": "history.undo"}));
    let creates = undone
        .iter()
        .filter(|m| matches!(m, ControlMsg::TrackCreate { .. }))
        .count();
    let uploads = undone
        .iter()
        .filter(|m| matches!(m, ControlMsg::ClipNotesUpload { .. }))
        .count();
    assert_eq!((creates, uploads), (1, 1), "{undone:?}");
    assert_eq!(s.ctx.engine.tracks.len(), 1);
    let (_, redone) = run(&mut s, json!({"type": "history.redo"}));
    assert_eq!(redone, deleted, "redo sends what the delete sent");
    assert_eq!(s.ctx.engine, before_undo);
}

#[test]
fn a_failing_group_undo_rolls_the_slot_table_back() {
    let (mut s, _, _) = synced();
    run(&mut s, json!({"type": "history.begin_group"}));
    let (out, _) = run(&mut s, json!({"type": "track.create", "kind": "midi"}));
    let a = track_of(&out);
    run(&mut s, json!({"type": "track.create", "kind": "midi"}));
    run(&mut s, json!({"type": "history.end_group"}));
    // Undoing the group deletes the second track, then the first. Make the first delete
    // fail by removing that track behind the history's back.
    let mut tampered = s.state.clone();
    tampered.project.tracks.remove(&a);
    s.state = tampered;
    let slots = s.ctx.engine.clone();
    let project = s.state.project.clone();
    let refused = s.run(&cmd(json!({"type": "history.undo"})));
    assert!(refused.is_err());
    assert_eq!(s.state.project, project);
    assert_eq!(s.ctx.engine, slots, "the second track's slot is not freed");
}

#[test]
fn an_entity_the_engine_was_never_told_about_gets_no_message() {
    let f = testing::fixture();
    let mut s = Synced::new(f.state.clone(), testing::context(), 1);
    // No setup: the context's table is empty.
    let (_, msgs) = s
        .run(&cmd(
            json!({"type": "live.note_on", "track": f.track, "note": 60, "velocity": 1}),
        ))
        .unwrap();
    assert!(msgs.is_empty());
    assert_eq!(
        s.run(&cmd(
            json!({"type": "mixer.set_pan", "track": f.track, "pan": 0.5})
        ))
        .unwrap()
        .0
        .control,
        []
    );
}

#[test]
fn large_clips_upload_in_chunks() {
    let (mut s, synth, scene) = synced();
    let (out, _) = run(
        &mut s,
        json!({"type": "clip.create", "track": synth, "at": {"at": "slot", "scene": scene}, "length": 1_000_000}),
    );
    let Output::Clip(commands::clip::Output::Create { clip, .. }) = out else {
        panic!()
    };
    let n = CHUNK_ITEMS + 1;
    let notes: Vec<Value> = (0..n)
        .map(|i| json!({"start": i as i64 * 10, "pitch": 60, "length": 5, "velocity": 1}))
        .collect();
    let length = n as i64 * 10 + 10;
    let (_, msgs) = run(
        &mut s,
        json!({"type": "clip.set_content", "clip": clip, "length": length, "looping": null, "notes": notes}),
    );
    let chunks: Vec<(u32, u32, usize)> = msgs
        .iter()
        .map(|m| match m {
            ControlMsg::ClipNotesUpload {
                first,
                total,
                notes,
                ..
            } => (*first, *total, notes.len()),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        chunks,
        [
            (0, n as u32, CHUNK_ITEMS),
            (CHUNK_ITEMS as u32, n as u32, 1)
        ]
    );
    // The engine takes them whole.
    let (mut prepare, _host) = engine::host::channel(48_000.0).unwrap();
    let mut fresh = s.clone();
    prepare.submit(&fresh.resync().unwrap()).unwrap();
    assert_eq!(prepare.preparer().rejected(), 0);
}

#[test]
fn play_starts_at_the_cue_once_and_seek_moves_the_cue() {
    let (mut s, synth, _) = synced();
    let play = || json!({"type": "transport.play"});
    assert_eq!(run(&mut s, play()).1, [ControlMsg::Play { from: Tick(0) }]);
    assert_eq!(run(&mut s, play()).1, [], "already playing: no jump back");
    assert_eq!(
        run(&mut s, json!({"type": "transport.seek", "tick": 1920})).1,
        [ControlMsg::Seek { tick: Tick(1920) }]
    );
    // The playback started from the cue (the seek above), so the stop brings the head back to it.
    assert_eq!(
        run(&mut s, json!({"type": "transport.stop"})).1,
        [ControlMsg::Stop, ControlMsg::Seek { tick: Tick(1920) }]
    );
    assert_eq!(
        run(&mut s, play()).1,
        [ControlMsg::Play { from: Tick(1920) }]
    );
    // A resync stops and forgets the cue, as the fresh engine does.
    s.resync().unwrap();
    assert_eq!(run(&mut s, play()).1, [ControlMsg::Play { from: Tick(0) }]);
    // Live notes and the panic button.
    assert_eq!(
        run(
            &mut s,
            json!({"type": "live.note_on", "track": synth, "note": 60, "velocity": 90})
        )
        .1,
        [ControlMsg::NoteOn {
            track: TrackSlot(0),
            pitch: 60,
            velocity: 90
        }]
    );
    run(&mut s, json!({"type": "track.create", "kind": "midi"}));
    assert_eq!(
        run(&mut s, json!({"type": "live.all_notes_off"})).1,
        [
            ControlMsg::AllNotesOff {
                track: TrackSlot(0)
            },
            ControlMsg::AllNotesOff {
                track: TrackSlot(1)
            }
        ]
    );
}

#[test]
fn all_sound_off_releases_every_track_then_cuts_and_changes_no_project_data() {
    let (mut s, _, _) = synced();
    run(&mut s, json!({"type": "track.create", "kind": "midi"}));
    let before = s.state.project.clone();
    let (_, control) = run(&mut s, json!({"type": "live.all_sound_off"}));
    assert_eq!(
        control,
        [
            ControlMsg::AllNotesOff {
                track: TrackSlot(0)
            },
            ControlMsg::AllNotesOff {
                track: TrackSlot(1)
            },
            ControlMsg::AllSoundOff
        ]
    );
    assert_eq!(s.state.project, before);
}

#[test]
fn back_to_arrangement_is_one_engine_message_and_changes_nothing_else() {
    let (mut s, _, _) = synced();
    let before = s.state.project.clone();
    let (output, control) = run(&mut s, json!({"type": "launch.back_to_arrangement"}));
    assert_eq!(control, [ControlMsg::BackToArrangement]);
    assert!(matches!(output, Output::Launch(_)));
    assert_eq!(
        s.state.project, before,
        "a live command: the project is untouched"
    );
}

#[test]
fn back_track_to_arrangement_is_one_engine_message_for_that_track() {
    let (mut s, synth, _) = synced();
    let before = s.state.project.clone();
    let (output, control) = run(
        &mut s,
        json!({"type": "launch.back_track_to_arrangement", "track": synth.to_string()}),
    );
    assert_eq!(
        control,
        [ControlMsg::BackTrackToArrangement {
            track: TrackSlot(0)
        }]
    );
    assert!(matches!(output, Output::Launch(_)));
    assert_eq!(s.state.project, before);
}

#[test]
fn a_huge_parameter_value_is_clamped_by_the_command_and_a_huge_stored_one_still_encodes() {
    let (mut s, synth, _) = synced();
    let device = s.state.project.tracks[&synth]
        .devices
        .keys()
        .next()
        .unwrap()
        .clone();
    // The command clamps to the descriptor's range (DV-8): the engine gets the bound.
    let (outcome, bytes) = s
        .run(&cmd(
            json!({"type": "device.set_param", "device": device, "param": "gain", "value": 1e39}),
        ))
        .expect("the command and its message encode");
    assert!(!bytes.is_empty());
    assert!(matches!(outcome.control[..], [ControlMsg::ParamSet { value, .. }] if value == 1.0));
    let (outcome, _) = s
        .run(&cmd(json!({"type": "device.set_param", "device": device, "param": "attack", "value": -1e300})))
        .unwrap();
    assert!(matches!(outcome.control[..], [ControlMsg::ParamSet { value, .. }] if value == 0.002));
    // A project from outside may still hold a value past f32: its snapshot saturates and
    // still encodes.
    s.state
        .project
        .tracks
        .get_mut(&synth)
        .unwrap()
        .devices
        .get_mut(&device)
        .unwrap()
        .params
        .insert("gain".into(), 1e39);
    s.resync().expect("the snapshot encodes");
}

#[test]
fn a_clip_upload_carries_its_loop_and_every_field_of_its_notes() {
    let (mut s, track, scene) = synced();
    let (out, _) = run(
        &mut s,
        json!({"type": "clip.create", "track": track, "at": {"at": "slot", "scene": scene}, "length": 3840}),
    );
    let Output::Clip(commands::clip::Output::Create { clip, .. }) = out else {
        panic!("{out:?}")
    };
    run(
        &mut s,
        json!({"type": "clip.notes.add", "clip": clip, "notes": [
            {"start": 0, "pitch": 60, "length": 10, "velocity": 1},
            {"start": 0, "pitch": 60, "length": 10, "velocity": 1, "cents": -20, "chance": 40, "repeat": 3}
        ]}),
    );
    let (_, msgs) = run(
        &mut s,
        json!({"type": "clip.set_loop", "clip": clip, "looping": {"start": 960, "end": 1920}}),
    );
    let [ControlMsg::ClipNotesUpload { looping, notes, .. }] = &msgs[..] else {
        panic!("{msgs:?}")
    };
    assert_eq!(*looping, LoopRegion::new(Tick(960), Tick(1920)));
    // Two notes that differ only by their optional fields stay two notes.
    let mut fields: Vec<(i8, u8, u8)> = notes
        .iter()
        .map(|n| (n.cents, n.chance, n.repeat))
        .collect();
    fields.sort();
    assert_eq!(fields, [(-20, 40, 3), (0, 100, 1)]);
    let (_, msgs) = run(
        &mut s,
        json!({"type": "clip.set_loop", "clip": clip, "looping": null}),
    );
    assert!(matches!(
        msgs[..],
        [ControlMsg::ClipNotesUpload { looping: None, .. }]
    ));
}

#[test]
fn quantize_transpose_and_duplicate_reach_the_engine_as_one_upload_of_the_new_notes() {
    let (mut s, track, scene) = synced();
    let (out, _) = run(
        &mut s,
        json!({"type": "clip.create", "track": track, "at": {"at": "slot", "scene": scene}, "length": 3840}),
    );
    let Output::Clip(commands::clip::Output::Create { clip, .. }) = out else {
        panic!("{out:?}")
    };
    run(
        &mut s,
        json!({"type": "clip.notes.add", "clip": clip, "notes": [
            {"start": 100, "pitch": 60, "length": 10, "velocity": 90},
            {"start": 110, "pitch": 60, "length": 10, "velocity": 90}
        ]}),
    );
    let starts_and_pitches = |msgs: &[ControlMsg]| -> Vec<(i64, u8)> {
        let [ControlMsg::ClipNotesUpload { notes, .. }] = msgs else {
            panic!("{msgs:?}")
        };
        let mut all: Vec<(i64, u8)> = notes.iter().map(|n| (n.start.get(), n.pitch)).collect();
        all.sort();
        all
    };
    // Quantize merges the two notes into one: the engine gets the merged clip.
    let (_, msgs) = run(
        &mut s,
        json!({"type": "clip.notes.quantize", "clip": clip, "grid": 240, "strength": 100}),
    );
    assert_eq!(starts_and_pitches(&msgs), [(0, 60)]);
    let (_, msgs) = run(
        &mut s,
        json!({"type": "clip.notes.transpose", "clip": clip,
            "notes": [{"start": 0, "pitch": 60, "length": 10, "velocity": 90}], "semitones": 5}),
    );
    assert_eq!(starts_and_pitches(&msgs), [(0, 65)]);
    let (_, msgs) = run(
        &mut s,
        json!({"type": "clip.notes.duplicate", "clip": clip,
            "notes": [{"start": 0, "pitch": 65, "length": 10, "velocity": 90}], "offset": 480}),
    );
    assert_eq!(starts_and_pitches(&msgs), [(0, 65), (480, 65)]);
    // Undo of the merging quantize brings both notes back (the other branch of the inverse).
    for _ in 0..2 {
        run(&mut s, json!({"type": "history.undo"}));
    }
    let (_, msgs) = run(&mut s, json!({"type": "history.undo"}));
    assert_eq!(starts_and_pitches(&msgs), [(100, 60), (110, 60)]);
}

/// The id of a created clip.
fn clip_of(out: &Output) -> project::ClipId {
    match out {
        Output::Clip(commands::clip::Output::Create { clip, .. }) => clip.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_launch_while_stopped_arms_then_plays_once_and_never_while_playing() {
    let (mut s, synth, scene) = synced();
    let (out, _) = run(
        &mut s,
        json!({"type": "clip.create", "track": synth, "at": {"at": "slot", "scene": scene}}),
    );
    let clip = clip_of(&out);
    let launch = ControlMsg::LaunchClip {
        track: TrackSlot(0),
        scene: SceneSlot(0),
    };
    let scene_launch = ControlMsg::LaunchScene {
        scene: SceneSlot(0),
    };
    // Stopped: the engine arms the launch, then the transport plays from the cue. The
    // launch is first, so it never arrives after the playhead left the cue.
    let (_, msgs) = run(&mut s, json!({"type": "launch.clip", "clip": clip}));
    assert_eq!(msgs, [launch.clone(), ControlMsg::Play { from: Tick(0) }]);
    // Playing: only the launch; a second launch does not jump back to the cue.
    let (_, msgs) = run(&mut s, json!({"type": "launch.clip", "clip": clip}));
    assert_eq!(msgs, std::slice::from_ref(&launch));
    let (_, msgs) = run(&mut s, json!({"type": "launch.scene", "scene": scene}));
    assert_eq!(msgs, std::slice::from_ref(&scene_launch));
    // The stops never start the transport, playing or not.
    let (_, msgs) = run(&mut s, json!({"type": "launch.stop_track", "track": synth}));
    assert_eq!(
        msgs,
        [ControlMsg::StopTrack {
            track: TrackSlot(0)
        }]
    );
    let (_, msgs) = run(&mut s, json!({"type": "launch.stop_all"}));
    assert_eq!(msgs, [ControlMsg::StopAll]);
    run(&mut s, json!({"type": "transport.stop"}));
    let (_, msgs) = run(&mut s, json!({"type": "launch.stop_all"}));
    assert_eq!(msgs, [ControlMsg::StopAll], "stopped: no play");
    let (_, msgs) = run(&mut s, json!({"type": "launch.stop_track", "track": synth}));
    assert_eq!(
        msgs,
        [ControlMsg::StopTrack {
            track: TrackSlot(0)
        }]
    );
    // Stopped again, with the cue moved: the scene launch plays from it.
    run(&mut s, json!({"type": "transport.seek", "tick": 3840}));
    let (_, msgs) = run(&mut s, json!({"type": "launch.scene", "scene": scene}));
    assert_eq!(msgs, [scene_launch, ControlMsg::Play { from: Tick(3840) }]);
    // A resync forgets the playback, as the fresh engine does.
    s.resync().unwrap();
    let (_, msgs) = run(&mut s, json!({"type": "launch.clip", "clip": clip}));
    assert_eq!(msgs, [launch, ControlMsg::Play { from: Tick(0) }]);
}

#[test]
fn a_launch_of_what_the_engine_was_never_told_about_sends_nothing_and_starts_nothing() {
    let (mut s, synth, _) = synced();
    // A clip in the arrangement only has no slot: the command refuses it. An audio clip
    // is refused too. A track and a scene the engine holds launch normally.
    let (out, _) = run(
        &mut s,
        json!({"type": "clip.create", "track": synth, "at": {"at": "arrangement", "start": 0}}),
    );
    let clip = clip_of(&out);
    assert!(
        s.run(&cmd(json!({"type": "launch.clip", "clip": clip})))
            .is_err()
    );
    // After a resync that gave the clip no engine slot (a context that never sent it),
    // nothing is sent and the transport stays stopped.
    s.ctx.engine.track.get_mut(&synth).unwrap().clips.clear();
    let slotted = {
        let (out, _) = run(
            &mut s,
            json!({"type": "clip.create", "track": synth, "at": {"at": "slot", "scene": "0000000000000000000000005A"}}),
        );
        clip_of(&out)
    };
    s.ctx.engine.track.get_mut(&synth).unwrap().clips.clear();
    let (_, msgs) = run(&mut s, json!({"type": "launch.clip", "clip": slotted}));
    assert_eq!(
        msgs,
        [],
        "the engine holds no such clip: no launch, no play"
    );
}

#[test]
fn scenes_take_an_engine_slot_when_created_and_give_it_back_when_deleted() {
    let (mut s, synth, first) = synced();
    // The fixture's scene holds slot 0 from the snapshot; a created scene takes slot 1 and
    // sends nothing (the engine has no scene of its own, only rows of clip slots).
    let (out, msgs) = run(&mut s, json!({"type": "scene.create", "name": "B"}));
    assert_eq!(msgs, []);
    let Output::Scene(commands::scene::Output::Create { scene: second }) = out else {
        panic!()
    };
    assert_eq!(s.ctx.engine.scenes.get(&second), Some(1));
    // Its clip goes in row 1; deleting the scene empties the row, frees the slot, and the
    // next scene takes it again. Renaming and moving send nothing.
    let (out, _) = run(
        &mut s,
        json!({"type": "clip.create", "track": synth, "at": {"at": "slot", "scene": second}}),
    );
    let clip = clip_of(&out);
    let (_, msgs) = run(
        &mut s,
        json!({"type": "scene.rename", "scene": second, "name": "C"}),
    );
    assert_eq!(msgs, []);
    let (_, msgs) = run(
        &mut s,
        json!({"type": "scene.move", "scene": second, "index": 0}),
    );
    assert_eq!(msgs, []);
    let (_, msgs) = run(&mut s, json!({"type": "scene.delete", "scene": second}));
    assert!(
        matches!(
            msgs[..],
            [
                ControlMsg::SceneSetSlot {
                    track: TrackSlot(0),
                    scene: SceneSlot(1),
                    clip: None
                },
                ControlMsg::TrackPlacementsUpload { .. },
                ControlMsg::ClipRemove { clip: ClipSlot(0) },
            ]
        ),
        "{msgs:?}"
    );
    assert_eq!(s.ctx.engine.scenes.get(&second), None);
    // The undo brings the scene back into a slot and re-sends its clip into that row.
    let (_, msgs) = run(&mut s, json!({"type": "history.undo"}));
    assert_eq!(s.ctx.engine.scenes.get(&second), Some(1));
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            ControlMsg::SceneSetSlot {
                scene: SceneSlot(1),
                clip: Some(_),
                ..
            }
        )),
        "{msgs:?}"
    );
    run(&mut s, json!({"type": "history.redo"}));
    assert_eq!(s.ctx.engine.scenes.get(&second), None);
    let (out, _) = run(&mut s, json!({"type": "scene.create"}));
    let Output::Scene(commands::scene::Output::Create { scene: third }) = out else {
        panic!()
    };
    assert_eq!(s.ctx.engine.scenes.get(&third), Some(1), "the freed slot");
    assert_eq!(s.ctx.engine.scenes.get(&first), Some(0));
    let _ = clip;
}

#[test]
fn arrangement_commands_send_the_track_table_once_and_new_clips_before_it() {
    let (mut s, synth, _) = synced();
    let (out, _) = run(
        &mut s,
        json!({"type": "clip.create", "track": synth, "at": {"at": "arrangement", "start": 0}}),
    );
    let Output::Clip(commands::clip::Output::Create {
        placement: Some(first),
        ..
    }) = out
    else {
        panic!()
    };
    let uploads = |msgs: &[ControlMsg]| {
        msgs.iter()
            .filter(|m| matches!(m, ControlMsg::TrackPlacementsUpload { .. }))
            .count()
    };
    // A placement-only edit: the table of the track, once.
    let (_, msgs) = run(
        &mut s,
        json!({"type": "arrangement.move", "placements": [first], "by": 960}),
    );
    assert!(
        matches!(
            msgs[..],
            [ControlMsg::TrackPlacementsUpload {
                track: TrackSlot(0),
                total: 1,
                ..
            }]
        ),
        "{msgs:?}"
    );
    // A split: two placements in the table.
    let (_, msgs) = run(
        &mut s,
        json!({"type": "arrangement.split", "placement": first, "at": 1920}),
    );
    assert!(
        matches!(
            msgs[..],
            [ControlMsg::TrackPlacementsUpload { total: 2, .. }]
        ),
        "{msgs:?}"
    );
    // An independent copy: the clip, then the table, once.
    let (_, msgs) = run(
        &mut s,
        json!({"type": "arrangement.duplicate", "placements": [first], "by": 10000}),
    );
    assert!(
        matches!(
            msgs[..],
            [
                ControlMsg::ClipNotesUpload { .. },
                ControlMsg::TrackPlacementsUpload { total: 3, .. }
            ]
        ),
        "{msgs:?}"
    );
    // Its undo: the table without the copy, then the clip goes.
    let (_, msgs) = run(&mut s, json!({"type": "history.undo"}));
    assert_eq!(uploads(&msgs), 1, "{msgs:?}");
    assert!(matches!(msgs.last(), Some(ControlMsg::ClipRemove { .. })));
    // The split and the move are undone with one table each.
    for _ in 0..2 {
        let (_, msgs) = run(&mut s, json!({"type": "history.undo"}));
        assert_eq!(uploads(&msgs), 1, "{msgs:?}");
    }
}

#[test]
fn a_command_that_removes_or_brings_back_many_clips_sends_one_table() {
    let (mut s, synth, _) = synced();
    let mut placements = Vec::new();
    for start in [0, 3840, 7680] {
        let (out, _) = run(
            &mut s,
            json!({"type": "clip.create", "track": synth, "at": {"at": "arrangement", "start": start}}),
        );
        let Output::Clip(commands::clip::Output::Create {
            placement: Some(p), ..
        }) = out
        else {
            panic!()
        };
        placements.push(p);
    }
    let count =
        |msgs: &[ControlMsg], f: fn(&ControlMsg) -> bool| msgs.iter().filter(|m| f(m)).count();
    let table = |m: &ControlMsg| matches!(m, ControlMsg::TrackPlacementsUpload { .. });
    let removal = |m: &ControlMsg| matches!(m, ControlMsg::ClipRemove { .. });
    let upload = |m: &ControlMsg| matches!(m, ControlMsg::ClipNotesUpload { .. });
    // Three independent copies: three clips created, then one table.
    let (_, msgs) = run(
        &mut s,
        json!({"type": "arrangement.duplicate", "placements": placements}),
    );
    assert_eq!(
        (count(&msgs, upload), count(&msgs, table)),
        (3, 1),
        "{msgs:?}"
    );
    // The undo removes the three copies: one table first, then the three slots freed.
    let (_, msgs) = run(&mut s, json!({"type": "history.undo"}));
    assert_eq!(
        (count(&msgs, table), count(&msgs, removal)),
        (1, 3),
        "{msgs:?}"
    );
    let first_removal = msgs.iter().position(removal).unwrap();
    let the_table = msgs.iter().position(table).unwrap();
    assert!(
        the_table < first_removal,
        "the table stops using the clips first"
    );
    // A delete keeps the clips in the pool (#473): one table, no clip removed.
    let (_, msgs) = run(
        &mut s,
        json!({"type": "arrangement.delete", "placements": placements}),
    );
    assert_eq!(
        (count(&msgs, table), count(&msgs, removal)),
        (1, 0),
        "{msgs:?}"
    );
}

#[test]
fn stop_brings_the_head_to_the_cue_and_a_second_stop_to_the_start() {
    let (mut s, _, _) = synced();
    let stop = || json!({"type": "transport.stop"});
    let play = || json!({"type": "transport.play"});
    let seek = |tick: i64| json!({"type": "transport.seek", "tick": tick});
    let to = |tick: i64| ControlMsg::Seek { tick: Tick(tick) };
    // A cue laid on the ruler, playback, stop: the head is back on the cue; play restarts there.
    run(&mut s, seek(7680));
    run(&mut s, play());
    assert_eq!(run(&mut s, stop()).1, [ControlMsg::Stop, to(7680)]);
    assert_eq!(
        run(&mut s, play()).1,
        [ControlMsg::Play { from: Tick(7680) }]
    );
    run(&mut s, stop());
    // The head already on the cue: the second stop goes to the start, and play follows it.
    assert_eq!(run(&mut s, stop()).1, [ControlMsg::Stop, to(0)]);
    assert_eq!(
        run(&mut s, stop()).1,
        [ControlMsg::Stop, to(0)],
        "stays at 0"
    );
    assert_eq!(run(&mut s, play()).1, [ControlMsg::Play { from: Tick(0) }]);
    // A seek while stopped puts the head on the new cue: the next stop leaves it for 0.
    run(&mut s, stop());
    run(&mut s, seek(960));
    assert_eq!(run(&mut s, stop()).1, [ControlMsg::Stop, to(0)]);
    run(&mut s, seek(1234));
    assert_eq!(
        run(&mut s, play()).1,
        [ControlMsg::Play { from: Tick(1234) }]
    );
    // After a resync (the engine keeps its clock, the slot table starts at 0) a stop still
    // brings the head to where play restarts.
    s.resync().unwrap();
    assert_eq!(run(&mut s, stop()).1, [ControlMsg::Stop, to(0)]);
}

#[test]
fn a_pause_keeps_the_marker_so_stop_goes_back_to_it() {
    let (mut s, _, _) = synced();
    let stop = || json!({"type": "transport.stop"});
    let play = || json!({"type": "transport.play"});
    let seek = |tick: i64| json!({"type": "transport.seek", "tick": tick});
    let pause = |tick: i64| json!({"type": "transport.pause", "tick": tick});
    let to = |tick: i64| ControlMsg::Seek { tick: Tick(tick) };
    // Marker at 7680, play, pause at 20000: the head stays there and play resumes from it.
    run(&mut s, seek(7680));
    run(&mut s, play());
    assert_eq!(run(&mut s, pause(20000)).1, [ControlMsg::Stop, to(20000)]);
    assert_eq!(
        run(&mut s, play()).1,
        [ControlMsg::Play { from: Tick(20000) }]
    );
    // Pause, resume, stop: the marker.
    run(&mut s, pause(30000));
    run(&mut s, play());
    assert_eq!(run(&mut s, stop()).1, [ControlMsg::Stop, to(7680)]);
    // Pause then stop: the marker too, not the start; the second stop goes to the start.
    run(&mut s, play());
    run(&mut s, pause(40000));
    assert_eq!(run(&mut s, stop()).1, [ControlMsg::Stop, to(7680)]);
    assert_eq!(run(&mut s, stop()).1, [ControlMsg::Stop, to(0)]);
    // No marker laid: a pause then a stop goes back to the start.
    run(&mut s, play());
    run(&mut s, pause(960));
    assert_eq!(run(&mut s, stop()).1, [ControlMsg::Stop, to(0)]);
    // A pause outside the range is refused like a seek.
    assert!(
        s.run(&cmd(json!({"type": "transport.pause", "tick": -1})))
            .is_err()
    );
}

#[test]
fn insert_time_sends_the_track_table_the_master_lane_and_the_loop_once_and_so_does_its_undo() {
    let (mut s, synth, _) = synced();
    run(
        &mut s,
        json!({"type": "clip.create", "track": synth, "at": {"at": "arrangement", "start": 3840}}),
    );
    let (out, _) = run(
        &mut s,
        json!({"type": "automation.lane.create", "master": true,
               "target": {"mixer": {"param": "gain"}}}),
    );
    let Output::Automation(commands::automation::Output::LaneCreate { lane }) = out else {
        panic!()
    };
    run(
        &mut s,
        json!({"type": "automation.points.add", "lane": lane,
               "points": [{"tick": 0, "value": 0.0}, {"tick": 7680, "value": -6.0}]}),
    );
    run(
        &mut s,
        json!({"type": "transport.set_loop", "loop_region": {"start": 0, "end": 7680}}),
    );
    let counts = |msgs: &[ControlMsg]| {
        (
            msgs.iter()
                .filter(|m| matches!(m, ControlMsg::TrackPlacementsUpload { .. }))
                .count(),
            msgs.iter()
                .filter(|m| matches!(m, ControlMsg::AutomationLaneUpload { .. }))
                .count(),
            msgs.iter()
                .filter(|m| matches!(m, ControlMsg::SetLoop { .. }))
                .count(),
        )
    };
    let (_, msgs) = run(
        &mut s,
        json!({"type": "arrangement.insert_time", "at": 0, "length": 3840}),
    );
    assert_eq!(counts(&msgs), (1, 1, 1), "{msgs:?}");
    let (_, msgs) = run(&mut s, json!({"type": "history.undo"}));
    assert_eq!(counts(&msgs), (1, 1, 1), "{msgs:?}");
}
