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
        matches!(msgs[..], [ControlMsg::SetMaster { gain, mute: false }] if (gain - 0.1).abs() < 1e-6)
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
    assert_eq!(
        run(&mut s, json!({"type": "transport.stop"})).1,
        [ControlMsg::Stop]
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
fn a_huge_parameter_value_saturates_and_still_encodes() {
    let (mut s, synth, _) = synced();
    let device = s.state.project.tracks[&synth]
        .devices
        .keys()
        .next()
        .unwrap()
        .clone();
    let (_, bytes) = s
        .run(&cmd(
            json!({"type": "device.set_param", "device": device, "param": "gain", "value": 1e39}),
        ))
        .expect("the command and its message encode");
    assert!(!bytes.is_empty());
    let (outcome, _) = s
        .run(&cmd(json!({"type": "device.set_param", "device": device, "param": "attack", "value": -1e300})))
        .unwrap();
    assert!(
        matches!(outcome.control[..], [ControlMsg::ParamSet { value, .. }] if value == f32::MIN)
    );
    // The project keeps 1e39; its snapshot still encodes.
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
