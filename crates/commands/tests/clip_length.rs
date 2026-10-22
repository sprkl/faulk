//! #280: `clip.set_length` (and `clip.set_content`) make the placements that play the
//! whole clip follow its length, in the same history entry; the others are left alone.

mod clip_examples;

use ::project::{ClipId, IdGenerator, PlacementId, Project, Span, Tick};
use clip_examples::{ClipFx, c, fx};
use commands::clip::{self, ClipTarget, PlacementLength};
use commands::testing::{check_refusal, context};
use commands::{Command, CommandError, Context, Output, State, Undo, run};

/// A one-bar clip placed in the arrangement of the fixture's MIDI track at `start`.
fn placed_bar(
    state: &mut State,
    ctx: &mut Context,
    f: &ClipFx,
    start: i64,
) -> (ClipId, PlacementId) {
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
    let Output::Clip(clip::Output::Create { clip, placement }) = out.output else {
        panic!()
    };
    (clip, placement.unwrap())
}

fn set_length(state: &mut State, clip: &ClipId, length: i64) -> commands::Outcome {
    run(
        &c(clip::Command::SetLength {
            placements: None,
            clip: clip.clone(),
            length: Tick(length),
            trim: true,
        }),
        state,
        &mut context(),
    )
    .unwrap()
}

fn length_in(project: &Project, f: &ClipFx, placement: &PlacementId) -> i64 {
    match project.tracks[&f.track].placements[placement].span {
        Span::Ticks { length, .. } => length.get(),
        Span::Frames { .. } => panic!("frames"),
    }
}

#[test]
fn the_placement_that_plays_the_whole_clip_follows_grow_and_shrink() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let (clip, placement) = placed_bar(&mut state, &mut ctx, &f, 0);
    set_length(&mut state, &clip, 7680);
    assert_eq!(length_in(&state.project, &f, &placement), 7680);
    state.project.validate().unwrap();
    set_length(&mut state, &clip, 1920);
    assert_eq!(length_in(&state.project, &f, &placement), 1920);
    state.project.validate().unwrap();
}

#[test]
fn a_growing_placement_stops_at_the_next_one_and_undo_restores_the_exact_lengths() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let (clip, placement) = placed_bar(&mut state, &mut ctx, &f, 0);
    let (_, next) = placed_bar(&mut state, &mut ctx, &f, 3840);
    let before = state.project.clone();
    let out = set_length(&mut state, &clip, 3840 * 3);
    assert_eq!(length_in(&state.project, &f, &placement), 3840, "clamped");
    assert_eq!(length_in(&state.project, &f, &next), 3840);
    state.project.validate().unwrap();
    let after = state.project.clone();
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    let redo = inverse.apply(&mut state, &mut context()).unwrap();
    assert_eq!(state.project, before, "the clamped placement is restored");
    let Undo::Inverse(redo) = redo.undo else {
        panic!()
    };
    redo.apply(&mut state, &mut context()).unwrap();
    assert_eq!(state.project, after);
}

#[test]
fn a_placement_that_shows_part_of_the_clip_or_repeats_it_is_left_alone() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let (clip, whole) = placed_bar(&mut state, &mut ctx, &f, 0);
    let mut ids = IdGenerator::new(1_670_000_000_000, 11);
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let mut add = |start: i64, length: i64, offset: i64| {
        let id = PlacementId::generate(&mut ids);
        let mut p = track.placements[&whole].clone();
        p.start = Tick(start);
        p.span = Span::Ticks {
            length: Tick(length),
            offset: Tick(offset),
        };
        track.placements.insert(id.clone(), p);
        id
    };
    let partial = add(20_000, 3840, 480);
    let repeats = add(30_000, 7680, 0);
    let short = add(40_000, 1920, 0);
    state.project.validate().unwrap();
    let before = state.project.clone();
    let out = set_length(&mut state, &clip, 7680);
    assert_eq!(length_in(&state.project, &f, &whole), 7680);
    for other in [&partial, &repeats, &short] {
        assert_eq!(
            length_in(&state.project, &f, other),
            length_in(&before, &f, other)
        );
    }
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut context()).unwrap();
    assert_eq!(
        state.project, before,
        "the 2-bar repeat does not shrink on undo"
    );
}

#[test]
fn one_history_entry_undoes_and_redoes_clip_and_placement() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let (clip, placement) = placed_bar(&mut state, &mut ctx, &f, 0);
    let before = state.project.clone();
    let entries = state.history.undo.len();
    run(
        &c(clip::Command::SetLength {
            placements: None,
            clip: clip.clone(),
            length: Tick(7680),
            trim: false,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(state.history.undo.len(), entries + 1);
    let undo = Command::History(commands::history::Command::Undo {});
    let redo = Command::History(commands::history::Command::Redo {});
    run(&undo, &mut state, &mut ctx).unwrap();
    assert_eq!(state.project, before);
    run(&redo, &mut state, &mut ctx).unwrap();
    assert_eq!(length_in(&state.project, &f, &placement), 7680);
}

#[test]
fn set_content_follows_too_and_exact_lengths_refuse_a_bad_length_a_twin_or_a_foreign_placement() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let (clip, placement) = placed_bar(&mut state, &mut ctx, &f, 0);
    let (other, next) = placed_bar(&mut state, &mut ctx, &f, 3840);
    assert_ne!(other, clip);
    run(
        &c(clip::Command::SetContent {
            placements: None,
            clip: clip.clone(),
            length: Tick(1920),
            looping: None,
            notes: vec![],
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert_eq!(length_in(&state.project, &f, &placement), 1920);
    let exact = |placement: &PlacementId, length: i64| {
        c(clip::Command::SetLength {
            placements: Some(vec![PlacementLength {
                placement: placement.clone(),
                length: Tick(length),
            }]),
            clip: clip.clone(),
            length: Tick(1920),
            trim: false,
        })
    };
    let refusal = |command: Command| check_refusal(&command, &state, &context()).unwrap();
    let field = |e: CommandError| match e {
        CommandError::Invalid { field, .. } => field,
        other => panic!("{other:?}"),
    };
    assert_eq!(field(refusal(exact(&placement, 1 << 41))), "placements");
    assert_eq!(field(refusal(exact(&placement, 0))), "placements");
    assert!(matches!(
        refusal(exact(&next, 100)),
        CommandError::NotFound { .. }
    ));
    let twice = c(clip::Command::SetLength {
        placements: Some(vec![
            PlacementLength {
                placement: placement.clone(),
                length: Tick(100),
            };
            2
        ]),
        clip: clip.clone(),
        length: Tick(1920),
        trim: false,
    });
    assert_eq!(field(refusal(twice)), "placements");
}

#[test]
fn a_grow_never_shrinks_a_placement_and_adjacent_copies_stay_as_they_are() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let (clip, first) = placed_bar(&mut state, &mut ctx, &f, 0);
    let (_, second) = placed_bar(&mut state, &mut ctx, &f, 3840);
    // The next placement starts inside the first one (a merge can leave that): it stays.
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let mut early = track.placements[&first].clone();
    early.start = Tick(100);
    early.span = Span::Ticks {
        length: Tick(100),
        offset: Tick(0),
    };
    let mut ids = IdGenerator::new(1_670_000_000_000, 12);
    track
        .placements
        .insert(PlacementId::generate(&mut ids), early);
    set_length(&mut state, &clip, 7680);
    assert_eq!(length_in(&state.project, &f, &first), 3840, "not shrunk");
    assert_eq!(
        length_in(&state.project, &f, &second),
        3840,
        "the last copy"
    );
}

#[test]
fn the_undo_of_a_length_change_never_fails_on_overlaps_the_project_already_holds() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let (clip, _) = placed_bar(&mut state, &mut ctx, &f, 0);
    // Two unrelated placements that overlap each other, far from the clip's.
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let mut ids = IdGenerator::new(1_670_000_000_000, 13);
    for start in [30_000, 31_000] {
        let id = PlacementId::generate(&mut ids);
        let mut p = track.placements[&f.placement].clone();
        p.start = Tick(start);
        track.placements.insert(id, p);
    }
    state.project.validate().unwrap();
    let before = state.project.clone();
    let out = set_length(&mut state, &clip, 7680);
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut ctx).unwrap();
    assert_eq!(state.project, before);
}

#[test]
fn shortening_a_clip_out_of_an_overlap_can_be_undone() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let (clip, whole) = placed_bar(&mut state, &mut ctx, &f, 0);
    set_length(&mut state, &clip, 7680);
    // A placement that starts inside the whole one: the project allows the overlap.
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let mut ids = IdGenerator::new(1_670_000_000_000, 14);
    let mut inside = track.placements[&whole].clone();
    inside.start = Tick(3840);
    inside.span = Span::Ticks {
        length: Tick(100),
        offset: Tick(0),
    };
    track
        .placements
        .insert(PlacementId::generate(&mut ids), inside);
    state.project.validate().unwrap();
    let before = state.project.clone();
    let out = set_length(&mut state, &clip, 3840);
    assert_eq!(length_in(&state.project, &f, &whole), 3840);
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut ctx).unwrap();
    assert_eq!(state.project, before);
}
