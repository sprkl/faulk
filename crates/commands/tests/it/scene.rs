//! GR-2: the scene and launch commands. Examples against `validate`, the metadata the
//! issue fixes (`launch.*` live, scenes edit), the scene commands' refusals that leave the
//! state intact, delete with its clips and placements undone through the history, and the
//! launch commands' checks.

use crate::clip_examples;
use crate::grid_examples;
use ::project::{Clip, ClipContent, ClipId, OrderKey, Scene, SceneId, Tick};
use clip_examples::{ClipFx, fx};
use commands::scene::{SceneClip, SceneSnapshot};
use commands::testing::{check_refusal, context};
use commands::{
    Command, CommandError, Kind, Output, ProjectEvent, State, Undo, UndoKind, clip, history,
    launch, run, scene,
};
use grid_examples::{examples, la, sc};

fn undo() -> Command {
    Command::History(history::Command::Undo {})
}

fn redo() -> Command {
    Command::History(history::Command::Redo {})
}

fn refused(command: &Command, state: &State) -> CommandError {
    check_refusal(command, state, &context()).unwrap()
}

fn invalid_field(error: CommandError) -> String {
    match error {
        CommandError::Invalid { field, .. } => field,
        other => panic!("{other:?}"),
    }
}

fn undo_of(undo: Undo<Command>) -> Command {
    match undo {
        Undo::Inverse(c) => *c,
        other => panic!("no inverse: {other:?}"),
    }
}

fn names(state: &State) -> Vec<String> {
    state
        .project
        .scenes_in_order()
        .iter()
        .map(|(_, s)| s.name.clone())
        .collect()
}

#[test]
fn every_example_leaves_a_valid_project() {
    for (command, state) in examples() {
        let mut s = state.clone();
        run(&command, &mut s, &mut context()).unwrap();
        s.project.validate().unwrap();
    }
}

#[test]
fn launch_commands_are_live_and_scenes_are_undoable_edits_none_destructive() {
    let metas: Vec<_> = Command::metas()
        .into_iter()
        .filter(|m| m.name.starts_with("scene.") || m.name.starts_with("launch."))
        .collect();
    let launch: Vec<_> = metas
        .iter()
        .filter(|m| m.name.starts_with("launch."))
        .collect();
    let scenes: Vec<_> = metas
        .iter()
        .filter(|m| m.name.starts_with("scene.") && m.kind != Kind::Query)
        .collect();
    assert_eq!(
        launch.iter().map(|m| m.name).collect::<Vec<_>>(),
        [
            "launch.clip",
            "launch.scene",
            "launch.stop_track",
            "launch.stop_all",
            "launch.back_to_arrangement",
            "launch.back_track_to_arrangement"
        ]
    );
    assert_eq!(
        scenes.iter().map(|m| m.name).collect::<Vec<_>>(),
        [
            "scene.create",
            "scene.delete",
            "scene.restore",
            "scene.rename",
            "scene.move",
            "scene.restore_order"
        ]
    );
    for m in launch {
        assert_eq!(
            (m.kind, m.undo, m.destructive),
            (Kind::Live, UndoKind::None, false)
        );
    }
    for m in scenes {
        assert_eq!(
            (m.kind, m.undo, m.destructive),
            (Kind::Edit, UndoKind::Inverse, false),
            "{}",
            m.name
        );
    }
    // The quantization is a transport edit, not duplicated here.
    let quantize = Command::metas()
        .into_iter()
        .find(|m| m.name == "transport.set_launch_quantize")
        .unwrap();
    assert_eq!(
        (quantize.kind, quantize.undo),
        (Kind::Edit, UndoKind::Inverse)
    );
}

#[test]
fn create_puts_the_scene_last_trims_its_name_and_undoes() {
    let f = fx();
    let mut s = f.state.clone();
    let mut ctx = context();
    let out = sc(scene::Command::Create {
        name: Some("  Outro ".into()),
    })
    .apply(&mut s, &mut ctx)
    .unwrap();
    let Output::Scene(scene::Output::Create { scene: id }) = out.output else {
        panic!()
    };
    assert_eq!(
        out.events,
        vec![ProjectEvent::SceneCreated { scene: id.clone() }]
    );
    assert_eq!(s.project.scenes_in_order().last().unwrap().0, &id);
    assert_eq!(s.project.scenes[&id].name, "Outro");
    s.project.validate().unwrap();
    assert_eq!(undo_of(out.undo), sc(scene::Command::Delete { scene: id }));
    // No name: empty, as a track.
    let out = sc(scene::Command::Create { name: None })
        .apply(&mut s, &mut ctx)
        .unwrap();
    assert!(matches!(
        out.output,
        Output::Scene(scene::Output::Create { .. })
    ));
    assert_eq!(names(&s), ["", "", "", "Outro", ""]);
}

#[test]
fn create_and_rename_refuse_a_name_past_the_limit_and_create_stops_at_the_scene_limit() {
    let f = fx();
    let long = "x".repeat(::project::limits::MAX_NAME_CHARS + 1);
    let e = refused(
        &sc(scene::Command::Create {
            name: Some(long.clone()),
        }),
        &f.state,
    );
    assert_eq!(invalid_field(e), "name");
    let e = refused(
        &sc(scene::Command::Rename {
            scene: f.scene_a.clone(),
            name: long,
        }),
        &f.state,
    );
    assert_eq!(invalid_field(e), "name");
    // At the limit: refused with `limit`, the state untouched.
    let mut full = f.state.clone();
    let mut ids = ::project::IdGenerator::new(1_680_000_000_000, 0x00AB_0000_0000_0000_0000);
    let mut order = full
        .project
        .scenes_in_order()
        .last()
        .unwrap()
        .1
        .order
        .clone();
    while full.project.scenes.len() < ::project::limits::MAX_SCENES {
        order = OrderKey::between(Some(&order), None).unwrap();
        full.project.scenes.insert(
            SceneId::generate(&mut ids),
            Scene {
                name: String::new(),
                order: order.clone(),
            },
        );
    }
    let e = refused(&sc(scene::Command::Create { name: None }), &full);
    assert!(
        matches!(e, CommandError::Limit { max, .. } if max == ::project::limits::MAX_SCENES as u64)
    );
}

#[test]
fn delete_takes_the_clips_of_the_slots_and_their_placements_and_undo_brings_them_back() {
    let f = fx();
    // `scene_a` holds a clip, `scene_b` a clip with a placement.
    for (scene, clip, placements) in [(&f.scene_a, &f.clip, 0), (&f.scene_b, &f.placed, 1)] {
        let mut s = f.state.clone();
        let out = sc(scene::Command::Delete {
            scene: scene.clone(),
        })
        .apply(&mut s, &mut context())
        .unwrap();
        assert_eq!(
            out.output,
            Output::Scene(scene::Output::Delete { clips: 1 })
        );
        // The clips go first, then the scene: the engine's compiler needs the order.
        assert_eq!(out.events.len(), 2);
        assert!(matches!(
            &out.events[0],
            ProjectEvent::ClipDeleted { clip: c, placements: p, .. } if c == clip && p.len() == placements
        ));
        assert_eq!(
            out.events[1],
            ProjectEvent::SceneDeleted {
                scene: scene.clone()
            }
        );
        assert!(!s.project.scenes.contains_key(scene));
        assert!(
            s.project
                .tracks
                .values()
                .all(|t| !t.clips.contains_key(clip)
                    && t.placements.values().all(|p| p.clip != *clip))
        );
        s.project.validate().unwrap();
        // The other scenes and their clips stay.
        assert_eq!(s.project.scenes.len(), 2);
        // Undo through the inverse: everything back, identifiers included.
        let mut back = s.clone();
        let restored = undo_of(out.undo).apply(&mut back, &mut context()).unwrap();
        assert_eq!(back.project, f.state.project);
        assert!(matches!(
            &restored.events[..],
            [ProjectEvent::SceneCreated { scene: sc_id }, ProjectEvent::ClipCreated { clip: c, placements: p, .. }]
                if sc_id == scene && c == clip && p.len() == placements
        ));
    }
}

#[test]
fn delete_goes_through_the_history_and_redo_deletes_again() {
    let f = fx();
    let mut s = f.state.clone();
    let depth = s.history.undo.len();
    run(
        &sc(scene::Command::Delete {
            scene: f.scene_b.clone(),
        }),
        &mut s,
        &mut context(),
    )
    .unwrap();
    assert_eq!(s.history.undo.len(), depth + 1);
    let deleted = s.project.clone();
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, f.state.project);
    run(&redo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, deleted);
}

#[test]
fn delete_of_an_empty_scene_has_no_clip_and_an_unknown_scene_is_refused() {
    let f = fx();
    let mut s = f.state.clone();
    let out = sc(scene::Command::Delete {
        scene: f.scene_free.clone(),
    })
    .apply(&mut s, &mut context())
    .unwrap();
    assert_eq!(
        out.output,
        Output::Scene(scene::Output::Delete { clips: 0 })
    );
    let ghost = SceneId::parse("0000000000000000000000009Z").unwrap();
    for c in [
        scene::Command::Delete {
            scene: ghost.clone(),
        },
        scene::Command::Rename {
            scene: ghost.clone(),
            name: "x".into(),
        },
        scene::Command::Move {
            scene: ghost.clone(),
            index: 0,
        },
        scene::Command::RestoreOrder {
            scene: ghost.clone(),
            order: OrderKey::first(),
        },
    ] {
        let e = refused(&sc(c), &f.state);
        assert!(matches!(e, CommandError::NotFound { ref entity, .. } if entity == "scene"));
    }
}

/// A scene that holds an audio clip (`track.restore` can bring one in today).
fn with_audio_in(f: &ClipFx, scene: &SceneId) -> State {
    let mut s = f.state.clone();
    let track = s.project.tracks.get_mut(&f.audio_track).unwrap();
    track.clips.get_mut(&f.audio_clip).unwrap().slot = Some(scene.clone());
    s.project.validate().unwrap();
    s
}

#[test]
fn a_scene_with_an_audio_clip_is_deleted_and_restored_with_it() {
    let f = fx();
    let state = with_audio_in(&f, &f.scene_free);
    let mut s = state.clone();
    let out = sc(scene::Command::Delete {
        scene: f.scene_free.clone(),
    })
    .apply(&mut s, &mut context())
    .unwrap();
    assert_eq!(
        out.output,
        Output::Scene(scene::Output::Delete { clips: 1 })
    );
    assert!(s.project.tracks[&f.audio_track].clips.is_empty());
    undo_of(out.undo).apply(&mut s, &mut context()).unwrap();
    assert_eq!(s.project, state.project);
}

#[test]
fn restore_refusals_leave_the_state_intact() {
    let f = fx();
    let mut deleted = f.state.clone();
    let out = sc(scene::Command::Delete {
        scene: f.scene_b.clone(),
    })
    .apply(&mut deleted, &mut context())
    .unwrap();
    let Command::Scene(scene::Command::Restore {
        scene: id,
        snapshot,
        clips,
    }) = undo_of(out.undo)
    else {
        panic!("a restore")
    };
    let restore = |scene: &SceneId, snapshot: SceneSnapshot, clips: Vec<SceneClip>| {
        sc(scene::Command::Restore {
            scene: scene.clone(),
            snapshot,
            clips,
        })
    };
    // The scene's identifier is in use (it is not deleted).
    let e = refused(&restore(&f.scene_a, snapshot.clone(), vec![]), &f.state);
    assert_eq!(invalid_field(e), "scene");
    // A clip whose slot is another scene.
    let mut other = clips.clone();
    other[0].clip.slot = Some(f.scene_a.clone());
    let e = refused(&restore(&id, snapshot.clone(), other), &deleted);
    assert_eq!(invalid_field(e), "clips[0].clip.slot");
    // A track that is not in the project.
    let mut lost = clips.clone();
    lost[0].track = ::project::TrackId::parse("0000000000000000000000009Z").unwrap();
    assert!(matches!(
        refused(&restore(&id, snapshot.clone(), lost), &deleted),
        CommandError::NotFound { .. }
    ));
    // A clip identifier in use.
    let mut taken = clips.clone();
    taken[0].id = f.clip.clone();
    for c in &mut taken[0].placements {
        c.placement.clip = f.clip.clone();
    }
    let e = refused(&restore(&id, snapshot.clone(), taken), &deleted);
    assert_eq!(invalid_field(e), "clips[0]");
    // A placement that places another clip.
    let mut stray = clips.clone();
    stray[0].placements[0].placement.clip = f.clip.clone();
    let e = refused(&restore(&id, snapshot.clone(), stray), &deleted);
    assert_eq!(invalid_field(e), "clips[0].placements");
    // The same clip twice: the project would not be valid; nothing is kept.
    let twice = vec![clips[0].clone(), clips[0].clone()];
    let e = refused(&restore(&id, snapshot.clone(), twice), &deleted);
    assert_eq!(invalid_field(e), "snapshot");
    // Two clips in one track's slot: the slot is taken once per track.
    let mut twin = clips[0].clone();
    twin.id = ClipId::parse("0000000000000000000000009Y").unwrap();
    twin.placements.clear();
    let e = refused(
        &restore(&id, snapshot.clone(), vec![clips[0].clone(), twin]),
        &deleted,
    );
    assert_eq!(invalid_field(e), "snapshot");
    // A name past the limit.
    let long = SceneSnapshot {
        name: "x".repeat(::project::limits::MAX_NAME_CHARS + 1),
        order: snapshot.order.clone(),
    };
    let e = refused(&restore(&id, long, clips.clone()), &deleted);
    assert_eq!(invalid_field(e), "snapshot.name");
}

#[test]
fn rename_trims_reports_the_previous_name_and_an_unchanged_one_records_nothing() {
    let f = fx();
    let mut s = f.state.clone();
    let out = sc(scene::Command::Rename {
        scene: f.scene_a.clone(),
        name: "  Verse ".into(),
    })
    .apply(&mut s, &mut context())
    .unwrap();
    assert_eq!(
        out.output,
        Output::Scene(scene::Output::Rename {
            previous: String::new()
        })
    );
    assert_eq!(
        out.events,
        vec![ProjectEvent::SceneChanged {
            scene: f.scene_a.clone(),
            part: commands::contract::ScenePart::Name
        }]
    );
    assert_eq!(s.project.scenes[&f.scene_a].name, "Verse");
    let same = sc(scene::Command::Rename {
        scene: f.scene_a.clone(),
        name: "Verse".into(),
    })
    .apply(&mut s, &mut context())
    .unwrap();
    assert_eq!(same.undo, Undo::None);
    assert!(same.events.is_empty());
    let mut back = s.clone();
    undo_of(out.undo).apply(&mut back, &mut context()).unwrap();
    assert_eq!(back.project, f.state.project);
}

#[test]
fn move_places_the_scene_clamps_the_index_and_undoes_to_the_exact_key() {
    let f = fx();
    let ids = |s: &State| -> Vec<SceneId> {
        s.project
            .scenes_in_order()
            .iter()
            .map(|(id, _)| (*id).clone())
            .collect()
    };
    let (a, b, free) = (&f.scene_a, &f.scene_b, &f.scene_free);
    for (scene, index, expected) in [
        (a, 2, vec![b, free, a]),
        (a, 99, vec![b, free, a]),
        (free, 0, vec![free, a, b]),
        (b, 0, vec![b, a, free]),
        (free, 1, vec![a, free, b]),
    ] {
        let mut s = f.state.clone();
        let out = sc(scene::Command::Move {
            scene: scene.clone(),
            index,
        })
        .apply(&mut s, &mut context())
        .unwrap();
        assert_eq!(
            ids(&s),
            expected.into_iter().cloned().collect::<Vec<_>>(),
            "{index}"
        );
        s.project.validate().unwrap();
        undo_of(out.undo).apply(&mut s, &mut context()).unwrap();
        assert_eq!(s.project, f.state.project);
    }
    // Already there: nothing recorded.
    let mut s = f.state.clone();
    let out = sc(scene::Command::Move {
        scene: a.clone(),
        index: 0,
    })
    .apply(&mut s, &mut context())
    .unwrap();
    assert_eq!(out.undo, Undo::None);
    assert!(out.events.is_empty());
    assert_eq!(
        out.output,
        Output::Scene(scene::Output::Move { previous_index: 0 })
    );
}

#[test]
fn move_refuses_when_scenes_share_an_order_key() {
    let f = fx();
    let mut s = f.state.clone();
    let key = s.project.scenes[&f.scene_a].order.clone();
    s.project.scenes.get_mut(&f.scene_b).unwrap().order = key.clone();
    s.project.scenes.get_mut(&f.scene_free).unwrap().order = key;
    // Moving `free` between `a` and `b`, which share a key, leaves no key between them.
    let e = refused(
        &sc(scene::Command::Move {
            scene: f.scene_free.clone(),
            index: 1,
        }),
        &s,
    );
    assert_eq!(invalid_field(e), "index");
}

#[test]
fn launch_commands_change_nothing_and_enter_no_history() {
    let f = fx();
    for command in [
        la(launch::Command::Clip {
            clip: f.clip.clone(),
        }),
        la(launch::Command::Scene {
            scene: f.scene_a.clone(),
        }),
        la(launch::Command::Scene {
            scene: f.scene_free.clone(),
        }),
        la(launch::Command::StopTrack {
            track: f.track.clone(),
        }),
        la(launch::Command::StopAll {}),
    ] {
        let mut s = f.state.clone();
        let out = command.apply(&mut s, &mut context()).unwrap();
        assert_eq!(s, f.state, "{}", command.name());
        assert!(out.events.is_empty());
        assert_eq!(out.undo, Undo::None);
        run(&command, &mut s, &mut context()).unwrap();
        assert_eq!(s.history, f.state.history, "{}", command.name());
    }
}

#[test]
fn launch_refuses_what_it_cannot_launch_and_leaves_the_state_intact() {
    let f = fx();
    let ghost = SceneId::parse("0000000000000000000000009Z").unwrap();
    let ghost_clip = ClipId::parse("0000000000000000000000009Z").unwrap();
    let ghost_track = ::project::TrackId::parse("0000000000000000000000009Z").unwrap();
    for (command, entity) in [
        (launch::Command::Clip { clip: ghost_clip }, "clip"),
        (launch::Command::Scene { scene: ghost }, "scene"),
        (launch::Command::StopTrack { track: ghost_track }, "track"),
    ] {
        let e = refused(&la(command), &f.state);
        assert!(matches!(e, CommandError::NotFound { entity: ref got, .. } if got == entity));
    }
    // An audio clip in a slot is launched like a MIDI clip (AU-4).
    let mut s = f.state.clone();
    s.project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .clips
        .get_mut(&f.audio_clip)
        .unwrap()
        .slot = Some(f.scene_a.clone());
    s.project.validate().expect("valid");
    la(launch::Command::Clip {
        clip: f.audio_clip.clone(),
    })
    .apply(&mut s, &mut context())
    .expect("an audio clip of the grid is launched");
    // A clip of the arrangement only (no slot).
    let mut s = f.state.clone();
    let out = commands::Command::Clip(clip::Command::Create {
        track: f.track.clone(),
        at: clip::ClipTarget::Arrangement { start: Tick(0) },
        length: None,
        name: None,
    })
    .apply(&mut s, &mut context())
    .unwrap();
    let Output::Clip(clip::Output::Create { clip: arranged, .. }) = out.output else {
        panic!()
    };
    let e = refused(&la(launch::Command::Clip { clip: arranged }), &s);
    assert_eq!(invalid_field(e), "clip");
}

#[test]
fn a_clip_in_a_slot_is_the_one_the_fixture_says() {
    // Guards the fixture the tests above rely on: the MIDI clips sit in scenes a and b.
    let f = fx();
    let slot = |id: &ClipId| -> Option<SceneId> {
        f.state
            .project
            .tracks
            .values()
            .find_map(|t| t.clips.get(id))
            .and_then(|c: &Clip| c.slot.clone())
    };
    assert_eq!(slot(&f.clip), Some(f.scene_a.clone()));
    assert_eq!(slot(&f.placed), Some(f.scene_b.clone()));
    assert!(matches!(
        f.state.project.tracks[&f.audio_track].clips[&f.audio_clip].content,
        ClipContent::Audio { .. }
    ));
}
