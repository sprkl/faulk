//! CL-1: the clip commands beyond the harness: the project stays valid after each
//! command and each undo, every refusal leaves the state intact, a batch is one history
//! entry, and 100 000 notes go through in one command.

mod clip_examples;

use ::project::{ClipContent, Note, PlacementId, Tick, TickLoop};
use clip_examples::{ClipFx, after, c, examples, fx, n};
use commands::Context;
use commands::clip::{self, ClipSnapshot, ClipTarget, NoteChange, PlacementSnapshot};
use commands::testing::{check, check_refusal, context};
use commands::{Command, CommandError, Output, ProjectEvent, State, Undo, run};
use std::collections::BTreeSet;

fn notes_of(state: &State, fx: &ClipFx, clip: &::project::ClipId) -> BTreeSet<Note> {
    match &state.project.tracks[&fx.track].clips[clip].content {
        ClipContent::Midi { notes, .. } => notes.clone(),
        ClipContent::Audio { .. } => panic!("audio"),
    }
}

fn refused(fx: &ClipFx, command: clip::Command) -> CommandError {
    check_refusal(&c(command), &fx.state, &context()).unwrap()
}

#[test]
fn the_project_is_valid_after_every_example_and_after_its_undo_and_redo() {
    for (command, from) in examples() {
        let mut state = from.clone();
        let mut ctx = context();
        let outcome = run(&command, &mut state, &mut ctx).unwrap();
        state
            .project
            .validate()
            .unwrap_or_else(|v| panic!("{}: {v:?}", command.name()));
        let Undo::Inverse(inverse) = outcome.undo else {
            panic!("{}: no inverse", command.name())
        };
        let redo = inverse.apply(&mut state, &mut ctx).unwrap();
        assert_eq!(state.project, from.project, "{}", command.name());
        state.project.validate().unwrap();
        let Undo::Inverse(redo) = redo.undo else {
            panic!("no redo")
        };
        redo.apply(&mut state, &mut ctx).unwrap();
        state.project.validate().unwrap();
    }
}

#[test]
fn the_inverses_are_the_documented_commands() {
    let f = fx();
    let inverse = |command: clip::Command| after(f.state.clone(), &c(command)).1;
    let name = |command: &Command| command.name();
    assert_eq!(
        name(&inverse(clip::Command::Delete {
            clip: f.clip.clone()
        })),
        "clip.restore"
    );
    // Nothing cut: the plain `set_length`; something cut: `set_content`.
    let grow = inverse(clip::Command::SetLength {
        placements: None,
        clip: f.clip.clone(),
        length: Tick(7680),
        trim: false,
    });
    assert_eq!(name(&grow), "clip.set_length");
    let cut = inverse(clip::Command::SetLength {
        placements: None,
        clip: f.clip.clone(),
        length: Tick(1200),
        trim: true,
    });
    assert_eq!(name(&cut), "clip.set_content");
    assert_eq!(
        name(&inverse(clip::Command::Create {
            track: f.track.clone(),
            at: ClipTarget::Arrangement { start: Tick(0) },
            length: None,
            name: None,
        })),
        "clip.delete"
    );
}

#[test]
fn create_in_the_arrangement_places_the_clip_and_defaults_to_one_bar() {
    let f = fx();
    let mut state = f.state.clone();
    let out = run(
        &c(clip::Command::Create {
            track: f.track.clone(),
            at: ClipTarget::Arrangement { start: Tick(0) },
            length: None,
            name: None,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let Output::Clip(clip::Output::Create { clip, placement }) = out.output else {
        panic!()
    };
    let placement = placement.expect("a placement");
    let track = &state.project.tracks[&f.track];
    assert_eq!(track.clips[&clip].slot, None);
    let ClipContent::Midi { length, .. } = track.clips[&clip].content else {
        panic!()
    };
    assert_eq!(length, Tick(3840), "one bar of 4/4");
    assert_eq!(track.placements[&placement].clip, clip);
    assert!(matches!(
        &out.events[0],
        ProjectEvent::ClipCreated { placements, .. } if *placements == [placement]
    ));
}

#[test]
fn create_refusals_leave_the_state_intact() {
    let f = fx();
    let create = |at, length: Option<Tick>, track| clip::Command::Create {
        track,
        at,
        length,
        name: None,
    };
    let arrangement = |start| ClipTarget::Arrangement { start: Tick(start) };
    let cases = [
        // Overlaps the placement at [7680, 11520).
        create(arrangement(7000), Some(Tick(1000)), f.track.clone()),
        create(arrangement(-1), None, f.track.clone()),
        create(arrangement(0), Some(Tick(0)), f.track.clone()),
        create(arrangement(0), Some(Tick(-5)), f.track.clone()),
        create(arrangement(i64::MAX), Some(Tick(10)), f.track.clone()),
        create(arrangement(0), Some(Tick(i64::MAX)), f.track.clone()),
        create(arrangement(0), Some(Tick((1 << 40) + 1)), f.track.clone()),
        // The slot of `clip` is taken.
        create(
            ClipTarget::Slot {
                scene: f.scene_a.clone(),
            },
            None,
            f.track.clone(),
        ),
        // An audio track takes audio clips from import only.
        create(arrangement(0), None, f.audio_track.clone()),
    ];
    for command in cases {
        let err = refused(&f, command.clone());
        assert_eq!(err.code(), "invalid", "{command:?}: {err}");
    }
    let mut ids = ::project::IdGenerator::new(1, 1);
    let unknown_scene = ::project::SceneId::generate(&mut ids);
    let err = refused(
        &f,
        create(
            ClipTarget::Slot {
                scene: unknown_scene,
            },
            None,
            f.track.clone(),
        ),
    );
    assert_eq!(err.code(), "not_found");
    let err = refused(
        &f,
        create(arrangement(0), None, ::project::TrackId::generate(&mut ids)),
    );
    assert_eq!(err.code(), "not_found");
    let err = refused(
        &f,
        clip::Command::Create {
            track: f.track.clone(),
            at: arrangement(0),
            length: None,
            name: Some("x".repeat(201)),
        },
    );
    assert_eq!(err.code(), "invalid");
    // Touching placements do not overlap.
    let touching = c(create(arrangement(3840), Some(Tick(3840)), f.track.clone()));
    check(&touching, &f.state, &context()).unwrap();
}

#[test]
fn delete_removes_the_placements_and_restore_brings_back_the_same_identifiers() {
    let f = fx();
    let mut state = f.state.clone();
    let out = run(
        &c(clip::Command::Delete {
            clip: f.placed.clone(),
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::Delete { placements: 1 })
    );
    let track = &state.project.tracks[&f.track];
    assert!(!track.clips.contains_key(&f.placed) && track.placements.is_empty());
    assert!(matches!(
        &out.events[0],
        ProjectEvent::ClipDeleted { placements, .. } if *placements == [f.placement.clone()]
    ));
    let Undo::Inverse(restore) = out.undo else {
        panic!()
    };
    restore.apply(&mut state, &mut context()).unwrap();
    assert_eq!(state.project, f.state.project);
}

#[test]
fn restore_refuses_a_collision_a_bad_snapshot_and_a_bad_track() {
    let f = fx();
    let (deleted, restore) = after(
        f.state.clone(),
        &c(clip::Command::Delete {
            clip: f.placed.clone(),
        }),
    );
    let Command::Clip(clip::Command::Restore {
        track,
        clip,
        snapshot,
        placements,
    }) = restore
    else {
        panic!()
    };
    let try_restore = |state: &State,
                       track: &::project::TrackId,
                       clip: &::project::ClipId,
                       snapshot: ClipSnapshot,
                       placements: Vec<PlacementSnapshot>| {
        check_refusal(
            &c(clip::Command::Restore {
                track: track.clone(),
                clip: clip.clone(),
                snapshot,
                placements,
            }),
            state,
            &context(),
        )
        .unwrap()
    };
    // The identifier of a clip, a track, a scene, a placement that exists.
    for taken in [&f.clip.0, &f.track.0, &f.scene_a.0] {
        let id = ::project::ClipId::parse(taken.as_str()).unwrap();
        let err = try_restore(&deleted, &track, &id, snapshot.clone(), vec![]);
        assert_eq!(err.code(), "invalid", "{taken}");
    }
    let mut placement_taken = placements.clone();
    placement_taken[0].id = PlacementId::parse(f.track.0.as_str()).unwrap();
    assert_eq!(
        try_restore(&deleted, &track, &clip, snapshot.clone(), placement_taken).code(),
        "invalid"
    );
    let mut twice = placements.clone();
    twice.push(twice[0].clone());
    assert_eq!(
        try_restore(&deleted, &track, &clip, snapshot.clone(), twice).code(),
        "invalid"
    );
    // The clip's own identifier as a placement's.
    let mut own = placements.clone();
    own[0].id = PlacementId::parse(clip.0.as_str()).unwrap();
    assert_eq!(
        try_restore(&deleted, &track, &clip, snapshot.clone(), own).code(),
        "invalid"
    );
    // Not a MIDI track, an unknown track.
    assert_eq!(
        try_restore(&deleted, &f.audio_track, &clip, snapshot.clone(), vec![]).code(),
        "invalid"
    );
    let mut ids = ::project::IdGenerator::new(5, 5);
    assert_eq!(
        try_restore(
            &deleted,
            &::project::TrackId::generate(&mut ids),
            &clip,
            snapshot.clone(),
            vec![]
        )
        .code(),
        "not_found"
    );
    // A note outside the length, a duplicate note, a loop outside, a bad slot.
    let mut bad = snapshot.clone();
    bad.notes.push(n(3800, 60, 100, 100));
    assert_eq!(
        try_restore(&deleted, &track, &clip, bad, vec![]).code(),
        "invalid"
    );
    let mut bad = snapshot.clone();
    bad.notes.push(bad.notes[0]);
    assert_eq!(
        try_restore(&deleted, &track, &clip, bad, vec![]).code(),
        "invalid"
    );
    let mut bad = snapshot.clone();
    bad.looping = Some(TickLoop {
        start: Tick(0),
        end: Tick(99_999),
    });
    assert_eq!(
        try_restore(&deleted, &track, &clip, bad, vec![]).code(),
        "invalid"
    );
    let mut bad = snapshot.clone();
    bad.slot = Some(f.scene_a.clone());
    assert_eq!(
        try_restore(&deleted, &track, &clip, bad, vec![]).code(),
        "invalid"
    );
    let mut bad = snapshot.clone();
    bad.slot = Some(::project::SceneId::generate(&mut ids));
    assert_eq!(
        try_restore(&deleted, &track, &clip, bad, vec![]).code(),
        "not_found"
    );
    // A placement outside the timeline, an offset past the clip.
    let mut outside = placements.clone();
    outside[0].start = Tick(-1);
    assert_eq!(
        try_restore(&deleted, &track, &clip, snapshot.clone(), outside).code(),
        "invalid"
    );
    let mut offset = placements.clone();
    offset[0].offset = snapshot.length;
    assert_eq!(
        try_restore(&deleted, &track, &clip, snapshot.clone(), offset).code(),
        "invalid"
    );
}

#[test]
fn duplicate_is_independent_and_audio_clips_are_refused() {
    let f = fx();
    let mut state = f.state.clone();
    let out = run(
        &c(clip::Command::Duplicate {
            clip: f.clip.clone(),
            slot: Some(f.scene_free.clone()),
            track: None,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let Output::Clip(clip::Output::Duplicate { clip: copy }) = out.output else {
        panic!()
    };
    assert_ne!(copy, f.clip);
    run(
        &c(clip::Command::NotesRemove {
            clip: copy.clone(),
            notes: vec![n(0, 60, 480, 100)],
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!(
        notes_of(&state, &f, &f.clip).len(),
        3,
        "the original is untouched"
    );
    assert_eq!(notes_of(&state, &f, &copy).len(), 2);
    // The slot of the copy is taken, or the clip is audio.
    for command in [
        clip::Command::Duplicate {
            clip: f.clip.clone(),
            slot: Some(f.scene_a.clone()),
            track: None,
        },
        clip::Command::Duplicate {
            clip: f.audio_clip.clone(),
            slot: None,
            track: None,
        },
        clip::Command::Delete {
            clip: f.audio_clip.clone(),
        },
        clip::Command::SetLoop {
            clip: f.audio_clip.clone(),
            looping: None,
        },
        clip::Command::NotesAdd {
            clip: f.audio_clip.clone(),
            notes: vec![n(0, 60, 10, 100)],
        },
    ] {
        assert_eq!(refused(&f, command).code(), "invalid");
    }
}

#[test]
fn rename_trims_clears_and_refuses_a_long_name() {
    let f = fx();
    let mut state = f.state.clone();
    let rename = |name: &str| {
        c(clip::Command::Rename {
            clip: f.clip.clone(),
            name: name.into(),
        })
    };
    run(&rename("  Lead  "), &mut state, &mut context()).unwrap();
    assert_eq!(state.project.tracks[&f.track].clips[&f.clip].name, "Lead");
    let again = run(&rename("Lead"), &mut state, &mut context()).unwrap();
    assert_eq!(again.undo, Undo::None);
    run(&rename(""), &mut state, &mut context()).unwrap();
    state.project.validate().unwrap();
    assert_eq!(
        refused(
            &f,
            clip::Command::Rename {
                clip: f.clip.clone(),
                name: "é".repeat(201)
            }
        )
        .code(),
        "invalid"
    );
    check(&rename(&"é".repeat(200)), &f.state, &context()).unwrap();
}

#[test]
fn set_length_refuses_a_cut_without_trim_and_a_placement_past_the_end() {
    let f = fx();
    let set = |clip: &::project::ClipId, length: i64, trim: bool| clip::Command::SetLength {
        placements: None,
        clip: clip.clone(),
        length: Tick(length),
        trim,
    };
    // A note ends at 3840, the loop at 1920.
    assert_eq!(refused(&f, set(&f.clip, 3000, false)).code(), "invalid");
    assert_eq!(refused(&f, set(&f.clip, 0, true)).code(), "invalid");
    assert_eq!(
        refused(&f, set(&f.clip, (1 << 40) + 1, true)).code(),
        "invalid"
    );
    // The placement of `placed` has offset 480.
    assert_eq!(refused(&f, set(&f.placed, 480, true)).code(), "invalid");
    assert_eq!(refused(&f, set(&f.placed, 100, true)).code(), "invalid");
    check(&c(set(&f.placed, 481, true)), &f.state, &context()).unwrap();
    // The same length changes nothing.
    let mut state = f.state.clone();
    let out = run(&c(set(&f.clip, 3840, false)), &mut state, &mut context()).unwrap();
    assert_eq!((out.undo, out.events.len()), (Undo::None, 0));
}

#[test]
fn trim_cuts_notes_and_the_loop_exactly_and_merges_notes_that_become_equal() {
    let f = fx();
    let mut state = f.state.clone();
    // Two notes that differ only by length become one at the new end; the loop that
    // starts at the new end disappears.
    run(
        &c(clip::Command::SetContent {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(3840),
            looping: Some(TickLoop {
                start: Tick(1000),
                end: Tick(2000),
            }),
            notes: vec![
                n(0, 60, 200, 100),
                n(0, 60, 100, 100),
                n(50, 62, 500, 100),
                n(100, 62, 50, 100),
            ],
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let before = state.clone();
    let out = run(
        &c(clip::Command::SetLength {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(100),
            trim: true,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!(
        notes_of(&state, &f, &f.clip),
        BTreeSet::from([n(0, 60, 100, 100), n(50, 62, 50, 100)])
    );
    let ClipContent::Midi { looping, .. } = &state.project.tracks[&f.track].clips[&f.clip].content
    else {
        panic!()
    };
    assert_eq!(*looping, None);
    state.project.validate().unwrap();
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut context()).unwrap();
    assert_eq!(state.project, before.project);
    // A loop that crosses the end is clamped.
    let mut state = f.state.clone();
    run(
        &c(clip::Command::SetLength {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(1500),
            trim: true,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let ClipContent::Midi { looping, .. } = &state.project.tracks[&f.track].clips[&f.clip].content
    else {
        panic!()
    };
    assert_eq!(
        *looping,
        Some(TickLoop {
            start: Tick(0),
            end: Tick(1500)
        })
    );
}

#[test]
fn set_loop_refuses_what_is_outside_the_clip() {
    let f = fx();
    for (start, end) in [(-1, 100), (100, 100), (200, 100), (0, 3841)] {
        let err = refused(
            &f,
            clip::Command::SetLoop {
                clip: f.clip.clone(),
                looping: Some(TickLoop {
                    start: Tick(start),
                    end: Tick(end),
                }),
            },
        );
        assert_eq!(err.code(), "invalid", "{start}..{end}");
    }
    let mut state = f.state.clone();
    let same = c(clip::Command::SetLoop {
        clip: f.clip.clone(),
        looping: Some(TickLoop {
            start: Tick(0),
            end: Tick(1920),
        }),
    });
    assert_eq!(
        run(&same, &mut state, &mut context()).unwrap().undo,
        Undo::None
    );
}

#[test]
fn set_content_refuses_what_the_note_commands_refuse() {
    let f = fx();
    let set = |length: i64, notes: Vec<Note>| clip::Command::SetContent {
        placements: None,
        clip: f.clip.clone(),
        length: Tick(length),
        looping: None,
        notes,
    };
    for command in [
        set(100, vec![n(0, 60, 101, 100)]),
        set(0, vec![]),
        set(100, vec![n(-1, 60, 10, 100)]),
        set(100, vec![n(0, 128, 10, 100)]),
        // The placement of `placed` starts at offset 480.
        clip::Command::SetContent {
            placements: None,
            clip: f.placed.clone(),
            length: Tick(480),
            looping: None,
            notes: vec![],
        },
    ] {
        assert_eq!(refused(&f, command).code(), "invalid");
    }
    // Empty, with the same length: the clip is emptied in one entry, then nothing.
    let mut state = f.state.clone();
    let empty = c(set(3840, vec![]));
    let loop_clear = c(clip::Command::SetContent {
        placements: None,
        clip: f.clip.clone(),
        length: Tick(3840),
        looping: None,
        notes: vec![],
    });
    run(&empty, &mut state, &mut context()).unwrap();
    run(&loop_clear, &mut state, &mut context()).unwrap();
    let again = run(&loop_clear, &mut state, &mut context()).unwrap();
    assert_eq!(again.undo, Undo::None);
}

#[test]
fn notes_outside_the_clip_or_out_of_range_refuse_the_whole_batch() {
    let f = fx();
    let good = n(0, 60, 100, 100);
    let bad = [
        n(3841, 60, 10, 100),
        n(3830, 60, 11, 100),
        n(-1, 60, 10, 100),
        n(0, 60, 0, 100),
        n(0, 60, -1, 100),
        n(i64::MAX, 60, 10, 100),
        n(10, 60, i64::MAX, 100),
        n(0, 128, 10, 100),
        n(0, 60, 10, 0),
        n(0, 60, 10, 128),
        Note {
            cents: 51,
            ..n(0, 60, 10, 100)
        },
        Note {
            chance: 101,
            ..n(0, 60, 10, 100)
        },
        Note {
            repeat: 0,
            ..n(0, 60, 10, 100)
        },
        Note {
            repeat: 17,
            ..n(0, 60, 10, 100)
        },
    ];
    for note in bad {
        let err = refused(
            &f,
            clip::Command::NotesAdd {
                clip: f.clip.clone(),
                notes: vec![good, note],
            },
        );
        assert_eq!(err.code(), "invalid", "{note:?}");
        let err = refused(
            &f,
            clip::Command::NotesUpdate {
                clip: f.clip.clone(),
                changes: vec![NoteChange {
                    from: n(0, 60, 480, 100),
                    to: note,
                }],
            },
        );
        assert_eq!(err.code(), "invalid", "{note:?}");
    }
    // The last note of the clip, ending exactly at its length, is fine.
    check(
        &c(clip::Command::NotesAdd {
            clip: f.clip.clone(),
            notes: vec![n(3830, 60, 10, 100)],
        }),
        &f.state,
        &context(),
    )
    .unwrap();
}

#[test]
fn add_and_remove_ignore_what_is_already_so_and_report_the_count() {
    let f = fx();
    let mut state = f.state.clone();
    let existing = n(0, 60, 480, 100);
    let out = run(
        &c(clip::Command::NotesAdd {
            clip: f.clip.clone(),
            notes: vec![existing, n(10, 61, 10, 100), n(10, 61, 10, 100)],
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesAdd { added: 1 })
    );
    // The inverse removes only the note actually added.
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut context()).unwrap();
    assert_eq!(state.project, f.state.project);
    let nothing = run(
        &c(clip::Command::NotesAdd {
            clip: f.clip.clone(),
            notes: vec![existing],
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!((nothing.undo, nothing.events.len()), (Undo::None, 0));
    let out = run(
        &c(clip::Command::NotesRemove {
            clip: f.clip.clone(),
            notes: vec![existing, existing, n(9, 9, 9, 9)],
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesRemove { removed: 1 })
    );
    let nothing = run(
        &c(clip::Command::NotesRemove {
            clip: f.clip.clone(),
            notes: vec![existing],
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!(nothing.undo, Undo::None);
}

#[test]
fn update_refusals_and_the_net_no_change_cases() {
    let f = fx();
    let a = n(0, 60, 480, 100);
    let b = n(960, 64, 960, 90);
    let moved = n(5, 60, 480, 100);
    let change = |from, to| NoteChange { from, to };
    let update = |changes| clip::Command::NotesUpdate {
        clip: f.clip.clone(),
        changes,
    };
    // A note that is not in the clip.
    assert_eq!(
        refused(&f, update(vec![change(moved, a)])).code(),
        "not_found"
    );
    // Two changes with the same `from`, or the same `to`.
    assert_eq!(
        refused(
            &f,
            update(vec![change(a, moved), change(a, n(6, 60, 480, 100))])
        )
        .code(),
        "invalid"
    );
    assert_eq!(
        refused(&f, update(vec![change(a, moved), change(b, moved)])).code(),
        "invalid"
    );
    // A `to` that is a note which stays.
    assert_eq!(refused(&f, update(vec![change(a, b)])).code(), "invalid");
    // A swap and an identity end with the same notes: nothing changes, nothing is
    // recorded.
    for changes in [vec![change(a, b), change(b, a)], vec![change(a, a)], vec![]] {
        let mut state = f.state.clone();
        let out = run(&c(update(changes)), &mut state, &mut context()).unwrap();
        assert_eq!(state, f.state);
        assert_eq!((out.undo, out.events.len()), (Undo::None, 0));
        assert!(state.history.undo.len() == f.state.history.undo.len());
    }
    // An identity next to a real change is skipped by the inverse.
    let mut state = f.state.clone();
    let out = run(
        &c(update(vec![change(a, moved), change(b, b)])),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesUpdate { updated: 1 })
    );
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut context()).unwrap();
    assert_eq!(state.project, f.state.project);
}

#[test]
fn a_batch_is_one_history_entry_and_undo_redo_follow_it() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let before = state.history.undo.len();
    let batch: Vec<Note> = (0..1000).map(|i| n(i, 60, 1, 100)).collect();
    run(
        &c(clip::Command::NotesAdd {
            clip: f.clip.clone(),
            notes: batch,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(state.history.undo.len(), before + 1);
    assert_eq!(notes_of(&state, &f, &f.clip).len(), 1003);
    let undo = Command::History(commands::history::Command::Undo {});
    let redo = Command::History(commands::history::Command::Redo {});
    run(&undo, &mut state, &mut ctx).unwrap();
    assert_eq!(state.project, f.state.project);
    run(&redo, &mut state, &mut ctx).unwrap();
    assert_eq!(notes_of(&state, &f, &f.clip).len(), 1003);
}

#[test]
fn a_hundred_thousand_notes_in_one_command_undo_and_redo() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    run(
        &c(clip::Command::SetLength {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(100_000 * 10),
            trim: false,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let batch: Vec<Note> = (0..100_000)
        .map(|i| n(i * 10, (i % 128) as u8, 10, 1 + (i % 127) as u8))
        .collect();
    let entries = state.history.undo.len();
    let out = run(
        &c(clip::Command::NotesAdd {
            clip: f.clip.clone(),
            notes: batch.clone(),
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(state.history.undo.len(), entries + 1, "one entry");
    let added = match out.output {
        Output::Clip(clip::Output::NotesAdd { added }) => added,
        other => panic!("{other:?}"),
    };
    // The three notes of the fixture are in the batch or not: none is lost.
    assert!((99_997..=100_000).contains(&added), "{added}");
    assert!(notes_of(&state, &f, &f.clip).len() >= 100_000);
    state.project.validate().unwrap();
    run(
        &Command::History(commands::history::Command::Undo {}),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(notes_of(&state, &f, &f.clip).len(), 3);
    run(
        &Command::History(commands::history::Command::Redo {}),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert!(notes_of(&state, &f, &f.clip).len() >= 100_000);
    // Update the whole lot in one command, then remove it.
    let changes: Vec<NoteChange> = batch
        .iter()
        .map(|b| NoteChange {
            from: *b,
            to: Note { cents: 7, ..*b },
        })
        .collect();
    run(
        &c(clip::Command::NotesUpdate {
            clip: f.clip.clone(),
            changes,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    run(
        &c(clip::Command::NotesRemove {
            clip: f.clip.clone(),
            notes: notes_of(&state, &f, &f.clip).into_iter().collect(),
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert!(notes_of(&state, &f, &f.clip).is_empty());
}

#[test]
fn the_notes_per_clip_and_project_limits_refuse_with_a_limit_error() {
    use ::project::limits::MAX_NOTES_PER_CLIP;
    let f = fx();
    let mut state = f.state.clone();
    // Fill the clip to its limit in one command (set_content), then one more is refused.
    let length = (MAX_NOTES_PER_CLIP as i64 + 1) * 2;
    let notes: Vec<Note> = (0..MAX_NOTES_PER_CLIP as i64)
        .map(|i| n(i * 2, 60, 1, 100))
        .collect();
    run(
        &c(clip::Command::SetLength {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(length),
            trim: false,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    run(
        &c(clip::Command::SetContent {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(length),
            looping: None,
            notes,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let add = c(clip::Command::NotesAdd {
        clip: f.clip.clone(),
        notes: vec![n(1, 61, 1, 100)],
    });
    let err = check_refusal(&add, &state, &context()).unwrap();
    assert_eq!(err.code(), "limit", "{err}");
    // An exact refill does not count twice: set_content with the same size is fine.
    let ClipContent::Midi { notes, .. } = &state.project.tracks[&f.track].clips[&f.clip].content
    else {
        panic!()
    };
    let same: Vec<Note> = notes.iter().copied().collect();
    let out = run(
        &c(clip::Command::SetContent {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(length),
            looping: None,
            notes: same,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!(out.undo, Undo::None);
}

#[test]
fn the_commands_round_trip_through_json_with_optional_note_fields() {
    let f = fx();
    let plain = r#"{"type":"clip.notes.add","clip":"CLIP","notes":[{"start":0,"pitch":60,"length":10,"velocity":100}]}"#
        .replace("CLIP", f.clip.0.as_str());
    let command: Command = serde_json::from_str(&plain).unwrap();
    let Command::Clip(clip::Command::NotesAdd { notes, .. }) = &command else {
        panic!()
    };
    assert_eq!(notes[0], n(0, 60, 10, 100));
    assert_eq!(serde_json::to_string(&command).unwrap(), plain);
    let full = plain.replace(
        r#""velocity":100}"#,
        r#""velocity":100,"cents":-25,"chance":40,"repeat":3}"#,
    );
    let command: Command = serde_json::from_str(&full).unwrap();
    let Command::Clip(clip::Command::NotesAdd { notes, .. }) = &command else {
        panic!()
    };
    assert_eq!(
        (notes[0].cents, notes[0].chance, notes[0].repeat),
        (-25, 40, 3)
    );
    assert_eq!(serde_json::to_string(&command).unwrap(), full);
    // `trim` is optional; the target is tagged.
    let create = format!(
        r#"{{"type":"clip.create","track":"{}","at":{{"at":"slot","scene":"{}"}}}}"#,
        f.track.0, f.scene_free.0
    );
    assert!(serde_json::from_str::<Command>(&create).is_ok());
    let length = format!(
        r#"{{"type":"clip.set_length","clip":"{}","length":10}}"#,
        f.clip.0
    );
    assert!(serde_json::from_str::<Command>(&length).is_ok());
}

#[test]
fn deleting_a_clip_whose_placements_overlap_can_be_undone() {
    // The project does not forbid overlapping placements (a merge can leave some): the
    // undo of a delete must not fail on them.
    let f = fx();
    let mut state = f.state.clone();
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let mut ids = ::project::IdGenerator::new(1_670_000_000_000, 9);
    let extra = PlacementId::generate(&mut ids);
    let mut again = track.placements[&f.placement].clone();
    again.start = Tick(8000);
    track.placements.insert(extra, again);
    state.project.validate().unwrap();
    let before = state.project.clone();
    let out = run(
        &c(clip::Command::Delete {
            clip: f.placed.clone(),
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let Undo::Inverse(restore) = out.undo else {
        panic!()
    };
    restore.apply(&mut state, &mut context()).unwrap();
    assert_eq!(state.project, before);
}

#[test]
fn update_reports_the_notes_that_changed_not_the_changes() {
    let f = fx();
    let a = n(0, 60, 480, 100);
    let b = n(960, 64, 960, 90);
    let c_note = n(3360, 67, 480, 80);
    let moved = n(3000, 67, 480, 80);
    let command = c(clip::Command::NotesUpdate {
        clip: f.clip.clone(),
        changes: vec![
            NoteChange { from: a, to: b },
            NoteChange { from: b, to: a },
            NoteChange {
                from: c_note,
                to: moved,
            },
        ],
    });
    let mut state = f.state.clone();
    let out = run(&command, &mut state, &mut context()).unwrap();
    assert_eq!(
        out.output,
        Output::Clip(clip::Output::NotesUpdate { updated: 1 })
    );
    check(&command, &f.state, &context()).unwrap();
}

#[test]
fn every_refusal_of_the_clip_commands_leaves_the_state_intact() {
    let f = fx();
    let mut ids = ::project::IdGenerator::new(1_680_000_000_000, 3);
    let ghost = ::project::ClipId::generate(&mut ids);
    let ghost_scene = ::project::SceneId::generate(&mut ids);
    let note = n(0, 60, 10, 100);
    // An unknown clip, for every command that addresses one.
    for command in [
        clip::Command::Delete {
            clip: ghost.clone(),
        },
        clip::Command::Duplicate {
            clip: ghost.clone(),
            slot: None,
            track: None,
        },
        clip::Command::Move {
            clip: ghost.clone(),
            track: None,
            slot: Some(f.scene_free.clone()),
        },
        clip::Command::Rename {
            clip: ghost.clone(),
            name: "x".into(),
        },
        clip::Command::SetLength {
            placements: None,
            clip: ghost.clone(),
            length: Tick(10),
            trim: true,
        },
        clip::Command::SetLoop {
            clip: ghost.clone(),
            looping: None,
        },
        clip::Command::SetContent {
            placements: None,
            clip: ghost.clone(),
            length: Tick(10),
            looping: None,
            notes: vec![],
        },
        clip::Command::NotesAdd {
            clip: ghost.clone(),
            notes: vec![note],
        },
        clip::Command::NotesRemove {
            clip: ghost.clone(),
            notes: vec![note],
        },
        clip::Command::NotesUpdate {
            clip: ghost.clone(),
            changes: vec![],
        },
    ] {
        assert_eq!(refused(&f, command).code(), "not_found");
    }
    // An audio clip, for the commands that need a MIDI one.
    for command in [
        clip::Command::SetLength {
            placements: None,
            clip: f.audio_clip.clone(),
            length: Tick(10),
            trim: true,
        },
        clip::Command::SetContent {
            placements: None,
            clip: f.audio_clip.clone(),
            length: Tick(10),
            looping: None,
            notes: vec![],
        },
        clip::Command::NotesRemove {
            clip: f.audio_clip.clone(),
            notes: vec![note],
        },
        clip::Command::NotesUpdate {
            clip: f.audio_clip.clone(),
            changes: vec![],
        },
    ] {
        assert_eq!(refused(&f, command).code(), "invalid");
    }
    // Duplicating into a scene that does not exist.
    let into_ghost = clip::Command::Duplicate {
        clip: f.clip.clone(),
        slot: Some(ghost_scene),
        track: None,
    };
    assert_eq!(refused(&f, into_ghost).code(), "not_found");
    // A bad name or length in a snapshot.
    let (deleted, restore) = after(
        f.state.clone(),
        &c(clip::Command::Delete {
            clip: f.clip.clone(),
        }),
    );
    let Command::Clip(clip::Command::Restore {
        track,
        clip,
        snapshot,
        placements,
    }) = restore
    else {
        panic!()
    };
    for bad in [
        ClipSnapshot {
            name: " padded".into(),
            ..snapshot.clone()
        },
        ClipSnapshot {
            name: "x".repeat(201),
            ..snapshot.clone()
        },
        ClipSnapshot {
            length: Tick(0),
            ..snapshot.clone()
        },
    ] {
        let command = c(clip::Command::Restore {
            track: track.clone(),
            clip: clip.clone(),
            snapshot: bad,
            placements: placements.clone(),
        });
        let err = check_refusal(&command, &deleted, &context()).unwrap();
        assert_eq!(err.code(), "invalid");
    }
}

#[test]
fn the_clips_and_placements_per_track_limits_refuse_with_a_limit_error() {
    use ::project::limits::{MAX_CLIPS_PER_TRACK, MAX_PLACEMENTS_PER_TRACK};
    let f = fx();
    let mut state = f.state.clone();
    let mut ids = ::project::IdGenerator::new(1_690_000_000_000, 11);
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let template = track.clips[&f.clip].clone();
    while track.clips.len() < MAX_CLIPS_PER_TRACK {
        let mut copy = template.clone();
        copy.slot = None;
        track
            .clips
            .insert(::project::ClipId::generate(&mut ids), copy);
    }
    let create = c(clip::Command::Create {
        track: f.track.clone(),
        at: ClipTarget::Slot {
            scene: f.scene_free.clone(),
        },
        length: None,
        name: None,
    });
    let duplicate = c(clip::Command::Duplicate {
        clip: f.clip.clone(),
        slot: None,
        track: None,
    });
    for command in [&create, &duplicate] {
        let err = check_refusal(command, &state, &context()).unwrap();
        assert_eq!(err.code(), "limit", "{err}");
    }
    // The placements: fill the track's arrangement, then the next one is refused.
    let mut state = f.state.clone();
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let mut start = 100_000;
    while track.placements.len() < MAX_PLACEMENTS_PER_TRACK {
        track.placements.insert(
            PlacementId::generate(&mut ids),
            ::project::Placement {
                clip: f.placed.clone(),
                start: Tick(start),
                span: ::project::Span::Ticks {
                    length: Tick(10),
                    offset: Tick(0),
                },
            },
        );
        start += 10;
    }
    let at_start = c(clip::Command::Create {
        track: f.track.clone(),
        at: ClipTarget::Arrangement { start: Tick(0) },
        length: Some(Tick(10)),
        name: None,
    });
    let err = check_refusal(&at_start, &state, &context()).unwrap();
    assert_eq!(err.code(), "limit", "{err}");
}

#[test]
fn duplicate_without_a_slot_takes_the_next_free_one_below_or_is_refused() {
    let f = fx();
    // `clip` sits in scene_a; scene_b is taken by `placed`, so the copy goes to scene_free.
    let dup = c(clip::Command::Duplicate {
        clip: f.clip.clone(),
        slot: None,
        track: None,
    });
    check(&dup, &f.state, &context()).unwrap();
    let mut state = f.state.clone();
    let out = run(&dup, &mut state, &mut context()).unwrap();
    let Output::Clip(clip::Output::Duplicate { clip: copy }) = out.output else {
        panic!()
    };
    assert_eq!(
        state.project.tracks[&f.track].clips[&copy].slot,
        Some(f.scene_free.clone())
    );
    state.project.validate().unwrap();
    // Every slot is now taken: the next duplicate is refused and changes nothing.
    let err = check_refusal(&dup, &state, &context()).unwrap();
    assert_eq!(err.code(), "invalid");
    // The copy of the clip in the last scene has nothing below it.
    let last = c(clip::Command::Duplicate {
        clip: copy.clone(),
        slot: None,
        track: None,
    });
    assert_eq!(
        check_refusal(&last, &state, &context()).unwrap().code(),
        "invalid"
    );
    // Undo removes the copy and frees the slot again.
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut context()).unwrap();
    assert_eq!(state.project, f.state.project);
    // A source above a free slot that is not directly below it skips the taken one.
    let mut gap = f.state.clone();
    gap.project
        .tracks
        .get_mut(&f.track)
        .unwrap()
        .clips
        .get_mut(&f.placed)
        .unwrap()
        .slot = None;
    let out = run(&dup, &mut gap, &mut context()).unwrap();
    let Output::Clip(clip::Output::Duplicate { clip: copy }) = out.output else {
        panic!()
    };
    assert_eq!(
        gap.project.tracks[&f.track].clips[&copy].slot,
        Some(f.scene_b.clone())
    );
}

#[test]
fn move_changes_the_slot_or_the_track_keeping_the_clip_and_is_refused_where_it_cannot() {
    let f = fx();
    let (two, other) = clip_examples::with_other_track(&f);
    let ghost_scene = {
        let mut ids = ::project::IdGenerator::new(1_680_000_000_000, 9);
        ::project::SceneId::generate(&mut ids)
    };
    let mv = |clip: &::project::ClipId,
              track: Option<&::project::TrackId>,
              slot: Option<&::project::SceneId>| {
        c(clip::Command::Move {
            clip: clip.clone(),
            track: track.cloned(),
            slot: slot.cloned(),
        })
    };
    // Same track, free slot: the clip keeps its identifier and its content.
    let mut state = two.clone();
    let before = state.project.tracks[&f.track].clips[&f.clip].clone();
    let out = run(
        &mv(&f.clip, None, Some(&f.scene_free)),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let Output::Clip(clip::Output::Move {
        previous_track,
        previous_slot,
    }) = out.output
    else {
        panic!("not a move output")
    };
    assert_eq!(previous_track, f.track);
    assert_eq!(previous_slot, Some(f.scene_a.clone()));
    let moved = &state.project.tracks[&f.track].clips[&f.clip];
    assert_eq!(moved.slot, Some(f.scene_free.clone()));
    assert_eq!(moved.content, before.content);
    assert_eq!(moved.name, before.name);
    // The engine hears the clip leave and come back (a playing clip stops).
    assert!(matches!(out.events[0], ProjectEvent::ClipDeleted { .. }));
    assert!(matches!(out.events[1], ProjectEvent::ClipCreated { .. }));
    // Across tracks: the clip leaves the first track for the other one.
    let mut state = two.clone();
    run(
        &mv(&f.clip, Some(&other), Some(&f.scene_a)),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert!(!state.project.tracks[&f.track].clips.contains_key(&f.clip));
    assert_eq!(
        state.project.tracks[&other].clips[&f.clip].slot,
        Some(f.scene_a.clone())
    );
    state.project.validate().unwrap();
    // A move that changes nothing leaves no history entry.
    let mut state = two.clone();
    let out = run(
        &mv(&f.clip, None, Some(&f.scene_a)),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert!(matches!(out.undo, Undo::None));
    assert!(out.events.is_empty());
    assert_eq!(state, two);
    // Refusals, each leaving the state intact.
    let refuse = |command: Command, code: &str| {
        let err = check_refusal(&command, &two, &context()).unwrap();
        assert_eq!(err.code(), code, "{err}");
    };
    refuse(mv(&f.clip, None, Some(&f.scene_b)), "invalid"); // slot taken
    refuse(mv(&f.clip, None, Some(&ghost_scene)), "not_found"); // no such scene
    refuse(mv(&f.clip, None, None), "invalid"); // out of the grid with no placement
    refuse(mv(&f.placed, Some(&other), Some(&f.scene_free)), "invalid"); // placed clips stay
    refuse(
        mv(&f.clip, Some(&f.audio_track), Some(&f.scene_free)),
        "invalid",
    ); // audio track
    refuse(mv(&f.audio_clip, None, Some(&f.scene_free)), "invalid"); // audio clip
    // Out of the grid is fine for a placed clip, and undo puts it back.
    let mut state = two.clone();
    let out = run(&mv(&f.placed, None, None), &mut state, &mut context()).unwrap();
    assert_eq!(state.project.tracks[&f.track].clips[&f.placed].slot, None);
    let Undo::Inverse(inverse) = out.undo else {
        panic!("no inverse")
    };
    inverse.apply(&mut state, &mut context()).unwrap();
    assert_eq!(state.project, two.project);
}

#[test]
fn duplicate_into_another_track_copies_the_clip_there_and_the_source_stays() {
    let f = fx();
    let (two, other) = clip_examples::with_other_track(&f);
    let dup = |slot: Option<&::project::SceneId>, track: &::project::TrackId| {
        c(clip::Command::Duplicate {
            clip: f.clip.clone(),
            slot: slot.cloned(),
            track: Some(track.clone()),
        })
    };
    let mut state = two.clone();
    let out = run(&dup(Some(&f.scene_a), &other), &mut state, &mut context()).unwrap();
    let Output::Clip(clip::Output::Duplicate { clip: copy }) = out.output else {
        panic!("not a duplicate output")
    };
    assert_eq!(
        state.project.tracks[&other].clips[&copy].slot,
        Some(f.scene_a.clone())
    );
    assert_eq!(
        state.project.tracks[&f.track].clips[&f.clip],
        two.project.tracks[&f.track].clips[&f.clip]
    );
    // Without a slot, the first free one from the top (the other track is empty).
    let mut state = two.clone();
    let out = run(&dup(None, &other), &mut state, &mut context()).unwrap();
    let Output::Clip(clip::Output::Duplicate { clip: copy }) = out.output else {
        panic!("not a duplicate output")
    };
    assert_eq!(
        state.project.tracks[&other].clips[&copy].slot,
        Some(f.scene_a.clone())
    );
    // Refused: an unknown track, in a move and in a copy.
    let mut ids = ::project::IdGenerator::new(1_680_000_000_000, 11);
    let ghost_track = ::project::TrackId::generate(&mut ids);
    let err = check_refusal(&dup(None, &ghost_track), &two, &context()).unwrap();
    assert_eq!(err.code(), "not_found");
    let to_ghost = c(clip::Command::Move {
        clip: f.clip.clone(),
        track: Some(ghost_track),
        slot: Some(f.scene_free.clone()),
    });
    assert_eq!(
        check_refusal(&to_ghost, &two, &context()).unwrap().code(),
        "not_found"
    );
    // Refused: an audio track, a taken slot.
    let err = check_refusal(&dup(None, &f.audio_track), &two, &context()).unwrap();
    assert_eq!(err.code(), "invalid");
    let mut taken = two.clone();
    run(&dup(Some(&f.scene_a), &other), &mut taken, &mut context()).unwrap();
    let err = check_refusal(&dup(Some(&f.scene_a), &other), &taken, &context()).unwrap();
    assert_eq!(err.code(), "invalid");
}

fn clip_name(state: &State, clip: &::project::ClipId) -> String {
    state
        .project
        .tracks
        .values()
        .find_map(|t| t.clips.get(clip))
        .expect("the clip exists")
        .name
        .clone()
}

fn create_unnamed(
    state: &mut State,
    ctx: &mut Context,
    f: &ClipFx,
    start: i64,
) -> ::project::ClipId {
    let out = run(
        &c(clip::Command::Create {
            track: f.track.clone(),
            at: ClipTarget::Arrangement { start: Tick(start) },
            length: None,
            name: None,
        }),
        state,
        ctx,
    )
    .unwrap();
    let Output::Clip(clip::Output::Create { clip, .. }) = out.output else {
        panic!()
    };
    clip
}

/// The default name is stored at creation, the first free number of the project, and
/// stays when other clips are deleted, copied or moved (#316).
#[test]
fn a_new_clip_gets_the_first_free_clip_number_and_keeps_it() {
    let f = fx();
    let mut ctx = context();
    let mut state = f.state.clone();
    let one = create_unnamed(&mut state, &mut ctx, &f, 20_000);
    let two = create_unnamed(&mut state, &mut ctx, &f, 24_000);
    let three = create_unnamed(&mut state, &mut ctx, &f, 28_000);
    assert_eq!(
        [&one, &two, &three].map(|id| clip_name(&state, id)),
        ["Clip 1", "Clip 2", "Clip 3"]
    );
    // Deleting the first leaves the others as they are, and its number is free again.
    run(
        &c(clip::Command::Delete { clip: one }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(clip_name(&state, &two), "Clip 2");
    assert_eq!(clip_name(&state, &three), "Clip 3");
    let again = create_unnamed(&mut state, &mut ctx, &f, 32_000);
    assert_eq!(clip_name(&state, &again), "Clip 1");
    // A name the user gave, however it looks, wins; a spelling that is not the default
    // one does not take a number.
    let out = run(
        &c(clip::Command::Create {
            track: f.track.clone(),
            at: ClipTarget::Arrangement {
                start: Tick(36_000),
            },
            length: None,
            name: Some("Clip 04".into()),
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let Output::Clip(clip::Output::Create { clip: named, .. }) = out.output else {
        panic!()
    };
    assert_eq!(clip_name(&state, &named), "Clip 04");
    let next = create_unnamed(&mut state, &mut ctx, &f, 40_000);
    assert_eq!(clip_name(&state, &next), "Clip 4");
    state.project.validate().unwrap();
}

/// Numbers are counted over the whole project, not per track.
#[test]
fn the_clip_number_is_free_in_the_whole_project() {
    let f = fx();
    let mut ctx = context();
    let mut state = f.state.clone();
    state
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .clips
        .get_mut(&f.audio_clip)
        .unwrap()
        .name = "Clip 1".into();
    let made = create_unnamed(&mut state, &mut ctx, &f, 20_000);
    assert_eq!(clip_name(&state, &made), "Clip 2");
}

/// A copy takes the next name by the rule of the track copies.
#[test]
fn a_copy_takes_the_next_name() {
    let f = fx();
    let mut ctx = context();
    let mut state = f.state.clone();
    // Room for the copies: a few more scenes.
    for _ in 0..4 {
        let last = state
            .project
            .scenes_in_order()
            .last()
            .map(|(_, s)| s.order.clone());
        let order = ::project::OrderKey::between(last.as_ref(), None).unwrap();
        let id = ::project::SceneId::generate(&mut ::project::IdGenerator::new(
            1_661_000_000_000,
            state.project.scenes.len() as u128 + 1,
        ));
        state.project.scenes.insert(
            id,
            ::project::Scene {
                name: String::new(),
                order,
            },
        );
    }
    let duplicate = |state: &mut State,
                     ctx: &mut Context,
                     source: &::project::ClipId,
                     slot: Option<::project::SceneId>| {
        let out = run(
            &c(clip::Command::Duplicate {
                clip: source.clone(),
                slot,
                track: None,
            }),
            state,
            ctx,
        )
        .unwrap();
        let Output::Clip(clip::Output::Duplicate { clip }) = out.output else {
            panic!()
        };
        clip
    };
    // "Riff" has no number: the copy is "Riff 2", the next one "Riff 3".
    let first = duplicate(&mut state, &mut ctx, &f.clip, Some(f.scene_free.clone()));
    assert_eq!(clip_name(&state, &first), "Riff 2");
    let second = duplicate(&mut state, &mut ctx, &f.clip, None);
    assert_eq!(clip_name(&state, &second), "Riff 3");
    // "Clip 2" gives "Clip 3", and skips a number that is taken.
    let made = create_unnamed(&mut state, &mut ctx, &f, 20_000);
    let two = create_unnamed(&mut state, &mut ctx, &f, 24_000);
    assert_eq!(clip_name(&state, &made), "Clip 1");
    assert_eq!(clip_name(&state, &two), "Clip 2");
    let copy = duplicate(&mut state, &mut ctx, &two, None);
    assert_eq!(clip_name(&state, &copy), "Clip 3");
    let copy = duplicate(&mut state, &mut ctx, &two, None);
    assert_eq!(clip_name(&state, &copy), "Clip 4");
    state.project.validate().unwrap();
}

/// A duplicated track brings clips with names of their own, none shared with the source.
#[test]
fn a_duplicated_track_renames_its_copied_clips() {
    let f = fx();
    let mut ctx = context();
    let mut state = f.state.clone();
    let out = run(
        &Command::Track(commands::track::Command::Duplicate {
            track: f.track.clone(),
            name: None,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let Output::Track(commands::track::Output::Duplicate { track }) = out.output else {
        panic!()
    };
    let names = |id: &::project::TrackId| -> BTreeSet<String> {
        state.project.tracks[id]
            .clips
            .values()
            .map(|c| c.name.clone())
            .collect()
    };
    let (source, copy) = (names(&f.track), names(&track));
    assert_eq!(
        copy,
        BTreeSet::from(["Riff 2".to_string(), "Placed 2".to_string()])
    );
    assert!(source.is_disjoint(&copy));
}

/// An empty name asks for the default one on a MIDI clip; an audio clip keeps being
/// able to go without.
#[test]
fn renaming_to_nothing_gives_a_midi_clip_the_default_name() {
    let f = fx();
    let mut ctx = context();
    let mut state = f.state.clone();
    let rename = |state: &mut State, ctx: &mut Context, clip: &::project::ClipId, name: &str| {
        run(
            &c(clip::Command::Rename {
                clip: clip.clone(),
                name: name.into(),
            }),
            state,
            ctx,
        )
        .unwrap()
    };
    let out = rename(&mut state, &mut ctx, &f.clip, "");
    assert_eq!(clip_name(&state, &f.clip), "Clip 1");
    // Undo brings the old name back.
    let Undo::Inverse(inverse) = out.undo else {
        panic!("a rename has an inverse")
    };
    run(&inverse, &mut state, &mut ctx).unwrap();
    assert_eq!(clip_name(&state, &f.clip), "Riff");
    rename(&mut state, &mut ctx, &f.audio_clip, "");
    assert_eq!(clip_name(&state, &f.audio_clip), "");
}
