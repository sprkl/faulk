//! AR-5: sending clips of the launch grid to the arrangement (`arrangement.place_from_slot`
//! and `arrangement.place_scene`): the independent copy against the linked placement, the
//! overlap rule, a whole scene in one history entry, every refusal.

use crate::arrangement_examples;
use crate::clip_examples;
use ::project::{ClipId, Span, Tick, TrackId};
use arrangement_examples::{a, fx};
use commands::arrangement;
use commands::testing::{check_refusal, context};
use commands::{Command, Output, ProjectEvent, State, Undo, run};

/// `(start, length, offset)` of every placement of the track, by start.
fn spans(state: &State, track: &TrackId) -> Vec<(i64, i64, i64)> {
    let mut out: Vec<_> = state.project.tracks[track]
        .placements
        .values()
        .map(|p| match p.span {
            Span::Ticks { length, offset } => (p.start.get(), length.get(), offset.get()),
            Span::Frames { .. } => panic!("audio"),
        })
        .collect();
    out.sort();
    out
}

fn fresh_context() -> commands::Context {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    commands::Context::new(
        commands::Author::User,
        ::project::IdGenerator::new(
            1_690_000_000_000 + n,
            0x00AB_0000_0000_0000_0000 + u128::from(n),
        ),
    )
}

fn ok(
    state: &mut State,
    command: arrangement::Command,
) -> (arrangement::Output, Vec<ProjectEvent>) {
    let mut ctx = fresh_context();
    let outcome = run(&a(command), state, &mut ctx).unwrap();
    state.project.validate().expect("the project stays valid");
    let Output::Arrangement(output) = outcome.output else {
        panic!("not an arrangement output")
    };
    (output, outcome.events)
}

fn notes_of(state: &State, track: &TrackId, clip: &ClipId) -> usize {
    match &state.project.tracks[track].clips[clip].content {
        ::project::ClipContent::Midi { notes, .. } => notes.len(),
        _ => panic!("not MIDI"),
    }
}

fn add_note(state: &mut State, clip: &ClipId) {
    run(
        &Command::Clip(commands::clip::Command::NotesAdd {
            clip: clip.clone(),
            notes: vec![clip_examples::n(5, 1, 5, 5)],
        }),
        state,
        &mut context(),
    )
    .unwrap();
}

#[test]
fn a_clip_sent_from_the_grid_is_an_independent_copy_and_a_linked_one_follows_the_grid() {
    let f = fx();
    let grid = f.grid_state();
    let base = notes_of(&grid, &f.track, &f.looped);

    // Independent (the default): a new clip outside the grid, the grid clip untouched.
    let mut s = grid.clone();
    let (out, events) = ok(
        &mut s,
        arrangement::Command::PlaceFromSlot {
            clip: f.looped.clone(),
            start: Tick(11520),
            linked: false,
            if_occupied: Default::default(),
        },
    );
    let arrangement::Output::PlaceFromSlot {
        placement,
        clip: Some(copy),
        overwritten,
    } = out
    else {
        panic!("a copy was expected")
    };
    assert_eq!(overwritten, 0);
    let track = &s.project.tracks[&f.track];
    assert_eq!(track.clips[&copy].slot, None, "the copy is not in the grid");
    assert_eq!(track.clips[&f.looped].slot, Some(f.scene_a.clone()));
    assert_eq!(track.placements[&placement].clip, copy);
    // The copy keeps the grid clip's name, as clip.duplicate does (#473).
    let grid_name = track.clips[&f.looped].name.clone();
    assert_eq!(track.clips[&copy].name, grid_name);
    assert_eq!(
        spans(&s, &f.track).last(),
        Some(&(11520, 3840, 0)),
        "the whole clip, from its start"
    );
    assert!(matches!(events[0], ProjectEvent::ClipCreated { .. }));
    // Editing the copy leaves the grid clip alone, and the other way round.
    let mut edited = s.clone();
    add_note(&mut edited, &copy);
    assert_eq!(notes_of(&edited, &f.track, &copy), base + 1);
    assert_eq!(notes_of(&edited, &f.track, &f.looped), base);
    let mut edited = s.clone();
    add_note(&mut edited, &f.looped);
    assert_eq!(notes_of(&edited, &f.track, &f.looped), base + 1);
    assert_eq!(notes_of(&edited, &f.track, &copy), base);

    // Linked: no new clip, the placement plays the grid clip, so an edit reaches both.
    let mut l = grid.clone();
    let clips = l.project.tracks[&f.track].clips.len();
    let (out, _) = ok(
        &mut l,
        arrangement::Command::PlaceFromSlot {
            clip: f.looped.clone(),
            start: Tick(11520),
            linked: true,
            if_occupied: Default::default(),
        },
    );
    let arrangement::Output::PlaceFromSlot {
        placement,
        clip: None,
        ..
    } = out
    else {
        panic!("no copy was expected")
    };
    assert_eq!(l.project.tracks[&f.track].clips.len(), clips);
    assert_eq!(
        l.project.tracks[&f.track].placements[&placement].clip,
        f.looped
    );
    assert_eq!(
        l.project.tracks[&f.track].clips[&f.looped].name,
        grid.project.tracks[&f.track].clips[&f.looped].name,
        "a linked placement keeps the grid clip and its name"
    );
    add_note(&mut l, &f.looped);
    assert_eq!(notes_of(&l, &f.track, &f.looped), base + 1);
    // Deleting the placement keeps the grid clip.
    ok(
        &mut l,
        arrangement::Command::Delete {
            placements: vec![placement],
        },
    );
    assert!(l.project.tracks[&f.track].clips.contains_key(&f.looped));
}

#[test]
fn a_clip_sent_over_placements_carves_them_and_the_undo_gives_everything_back() {
    let f = fx();
    let mut s = f.grid_state();
    let before = s.project.clone();
    let mut ctx = context();
    // Over A (0..3840) and B (3840..5760): the last placed wins.
    let outcome = run(
        &a(arrangement::Command::PlaceFromSlot {
            clip: f.straight.clone(),
            start: Tick(1000),
            linked: false,
            if_occupied: Default::default(),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    s.project.validate().unwrap();
    let Output::Arrangement(arrangement::Output::PlaceFromSlot { overwritten, .. }) =
        outcome.output
    else {
        panic!()
    };
    assert_eq!(overwritten, 2);
    let spans = spans(&s, &f.track);
    assert_eq!(spans[0], (0, 1000, 0));
    assert_eq!(spans[1], (1000, 3840, 0));
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!("no inverse")
    };
    inverse.apply(&mut s, &mut ctx).unwrap();
    assert_eq!(s.project, before);
}

#[test]
fn a_scene_goes_to_the_arrangement_in_one_command_one_undo_and_empty_slots_place_nothing() {
    let f = fx();
    let mut s = f.grid_state();
    let mut ctx = fresh_context();
    // A second MIDI track with a clip in scene A; the first track has `looped` there.
    let Output::Track(commands::track::Output::Create { track: other, .. }) = run(
        &Command::Track(commands::track::Command::Create {
            kind: ::project::TrackKind::Midi,
            instrument: None,
            name: None,
            color: None,
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap()
    .output
    else {
        panic!()
    };
    let Output::Clip(commands::clip::Output::Create { clip: second, .. }) = run(
        &Command::Clip(commands::clip::Command::Create {
            track: other.clone(),
            at: commands::clip::ClipTarget::Slot {
                scene: f.scene_a.clone(),
            },
            length: None,
            name: None,
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap()
    .output
    else {
        panic!()
    };
    let before = s.project.clone();
    let outcome = run(
        &a(arrangement::Command::PlaceScene {
            scene: f.scene_a.clone(),
            start: Tick(20_000),
            linked: false,
            if_occupied: Default::default(),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    s.project.validate().unwrap();
    let Output::Arrangement(arrangement::Output::PlaceScene {
        placements, clips, ..
    }) = outcome.output.clone()
    else {
        panic!()
    };
    assert_eq!((placements.len(), clips.len()), (2, 2), "one per track");
    assert_eq!(spans(&s, &f.track).last().map(|p| p.0), Some(20_000));
    assert_eq!(spans(&s, &other).len(), 1);
    assert_eq!(s.project.tracks[&other].clips.len(), 2);
    // One undo takes the whole scene back.
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!("no inverse")
    };
    inverse.apply(&mut s, &mut ctx).unwrap();
    assert_eq!(s.project, before);

    // Linked: no copy, every placement plays the clip of the grid.
    let outcome = run(
        &a(arrangement::Command::PlaceScene {
            scene: f.scene_a.clone(),
            start: Tick(20_000),
            linked: true,
            if_occupied: Default::default(),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    let Output::Arrangement(arrangement::Output::PlaceScene {
        placements, clips, ..
    }) = outcome.output
    else {
        panic!()
    };
    assert!(clips.is_empty());
    assert_eq!(placements.len(), 2);
    assert!(
        s.project.tracks[&other]
            .placements
            .values()
            .any(|p| p.clip == second)
    );

    // Scene B holds a clip on the first track only: the other track's slot is empty.
    let mut t = f.grid_state();
    let (out, _) = ok(
        &mut t,
        arrangement::Command::PlaceScene {
            scene: f.scene_b.clone(),
            start: Tick(20_000),
            linked: true,
            if_occupied: Default::default(),
        },
    );
    let arrangement::Output::PlaceScene { placements, .. } = out else {
        panic!()
    };
    assert_eq!(placements.len(), 1);
}

#[test]
fn sending_from_the_grid_refuses_what_is_not_in_the_grid_and_what_leaves_the_timeline() {
    let f = fx();
    let grid = f.grid_state();
    let refuse = |state: &State, command: arrangement::Command| {
        check_refusal(&a(command), state, &context()).unwrap()
    };
    // An arrangement-only clip: that is `place` or `duplicate`.
    refuse(
        &f.state,
        arrangement::Command::PlaceFromSlot {
            clip: f.looped.clone(),
            start: Tick(0),
            linked: false,
            if_occupied: Default::default(),
        },
    );
    for start in [-1, 1 << 41] {
        refuse(
            &grid,
            arrangement::Command::PlaceFromSlot {
                clip: f.looped.clone(),
                start: Tick(start),
                linked: true,
                if_occupied: Default::default(),
            },
        );
    }
    // An audio clip, a scene without a clip, an unknown scene, an unknown clip.
    refuse(
        &grid,
        arrangement::Command::PlaceFromSlot {
            clip: f.audio_clip.clone(),
            start: Tick(0),
            linked: false,
            if_occupied: Default::default(),
        },
    );
    refuse(
        &grid,
        arrangement::Command::PlaceScene {
            scene: f.scene_free.clone(),
            start: Tick(0),
            linked: false,
            if_occupied: Default::default(),
        },
    );
    // A scene that holds an audio clip only places it (AU-6), whole, as a copy.
    let mut audio = grid.clone();
    audio
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .clips
        .get_mut(&f.audio_clip)
        .unwrap()
        .slot = Some(f.scene_free.clone());
    let (out, _) = ok(
        &mut audio,
        arrangement::Command::PlaceScene {
            scene: f.scene_free.clone(),
            start: Tick(0),
            linked: false,
            if_occupied: Default::default(),
        },
    );
    let arrangement::Output::PlaceScene {
        placements, clips, ..
    } = out
    else {
        panic!()
    };
    assert_eq!((placements.len(), clips.len()), (1, 1));
    let track = &audio.project.tracks[&f.audio_track];
    let placed = &track.placements[&placements[0]];
    assert_eq!(placed.clip, clips[0]);
    assert_eq!(
        placed.span,
        ::project::Span::Frames {
            length: ::project::Frame::new(44_100),
            offset: ::project::Frame::new(0),
        }
    );
    assert_eq!(
        track.clips[&clips[0]].slot, None,
        "the copy is not in the grid"
    );
    let mut ids = ::project::IdGenerator::new(1_680_000_000_000, 0x00BB_0000_0000_0000_0000);
    refuse(
        &grid,
        arrangement::Command::PlaceScene {
            scene: ::project::SceneId::generate(&mut ids),
            start: Tick(0),
            linked: false,
            if_occupied: Default::default(),
        },
    );
    refuse(
        &grid,
        arrangement::Command::PlaceFromSlot {
            clip: ClipId::generate(&mut ids),
            start: Tick(0),
            linked: false,
            if_occupied: Default::default(),
        },
    );
}
