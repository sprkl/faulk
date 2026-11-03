//! #491: `arrangement.move_to_track`: the clip that goes with a placement, what stays, the
//! carve in the target, the refusals, the no-op and the exact undo.

mod arrangement_examples;
mod clip_examples;

use ::project::{PlacementId, Span, Tick, TrackId};
use arrangement_examples::{a, fx, with_second_track};
use commands::arrangement;
use commands::testing::{check_refusal, context};
use commands::{CommandError, Output, ProjectEvent, State, Undo, run};

/// A context whose identifiers differ from every other call's.
fn fresh_context() -> commands::Context {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    commands::Context::new(
        commands::Author::User,
        ::project::IdGenerator::new(
            1_691_000_000_000 + n,
            0x00AB_0000_0000_0000_0000 + u128::from(n),
        ),
    )
}

fn to_track(moves: &[(&PlacementId, &TrackId)], by: Option<i64>) -> arrangement::Command {
    arrangement::Command::MoveToTrack {
        moves: moves
            .iter()
            .map(|(p, t)| arrangement::TrackMove {
                placement: (*p).clone(),
                track: (*t).clone(),
            })
            .collect(),
        by: by.map(Tick),
    }
}

fn ok(
    state: &mut State,
    command: arrangement::Command,
) -> (arrangement::Output, Vec<ProjectEvent>) {
    let outcome = run(&a(command), state, &mut fresh_context()).unwrap();
    state.project.validate().expect("the project stays valid");
    let Output::Arrangement(output) = outcome.output else {
        panic!("not an arrangement output")
    };
    (output, outcome.events)
}

fn overlap_free(state: &State, track: &TrackId) -> bool {
    let mut spans: Vec<(i64, i64)> = state.project.tracks[track]
        .placements
        .values()
        .map(|p| match p.span {
            Span::Ticks { length, .. } => (p.start.get(), p.start.get() + length.get()),
            Span::Frames { .. } => panic!("audio"),
        })
        .collect();
    spans.sort();
    spans.windows(2).all(|w| w[0].1 <= w[1].0)
}

#[test]
fn a_placement_moved_to_another_track_takes_a_copy_of_its_clip_and_the_source_keeps_a_shared_one() {
    let f = fx();
    let (mut state, second) = with_second_track(&f);
    let before = state.clone();
    // `a` and `b` share the clip `looped`: moving only `a` leaves the clip to `b`.
    let (out, events) = ok(&mut state, to_track(&[(&f.a, &second)], None));
    let arrangement::Output::MoveToTrack {
        placements,
        overwritten,
    } = out
    else {
        panic!()
    };
    assert_eq!((placements.len(), overwritten), (1, 1));
    let source = &state.project.tracks[&f.track];
    assert!(!source.placements.contains_key(&f.a));
    assert!(source.clips.contains_key(&f.looped));
    let landed = &state.project.tracks[&second].placements[&placements[0]];
    assert!(!source.clips.contains_key(&landed.clip));
    assert_eq!(landed.start.get(), 0);
    let copy = &state.project.tracks[&second].clips[&landed.clip];
    assert_eq!(copy.slot, None);
    assert_eq!(copy.content, source.clips[&f.looped].content);
    assert!(events.iter().any(|e| matches!(
        e,
        ProjectEvent::ArrangementChanged { track } if *track == second
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        ProjectEvent::ClipCreated { track, .. } if *track == second
    )));
    // Both placements go: they stay linked among themselves, and the clip leaves the source.
    let mut state = before.clone();
    let (out, events) = ok(
        &mut state,
        to_track(&[(&f.a, &second), (&f.b, &second)], Some(100)),
    );
    let arrangement::Output::MoveToTrack { placements, .. } = out else {
        panic!()
    };
    let target = &state.project.tracks[&second];
    assert_eq!(
        target.placements[&placements[0]].clip,
        target.placements[&placements[1]].clip
    );
    assert_eq!(target.placements[&placements[0]].start.get(), 100);
    assert!(!state.project.tracks[&f.track].clips.contains_key(&f.looped));
    assert!(events.iter().any(|e| matches!(
        e,
        ProjectEvent::ClipDeleted { track, clip, .. } if *track == f.track && *clip == f.looped
    )));
}

#[test]
fn a_move_to_another_track_is_undone_exactly_and_a_grid_clip_stays() {
    let f = fx();
    let (mut state, second) = with_second_track(&f);
    // `looped` also sits in the launch grid: it stays in its track.
    let scene = state.project.scenes.keys().next().unwrap().clone();
    state
        .project
        .tracks
        .get_mut(&f.track)
        .unwrap()
        .clips
        .get_mut(&f.looped)
        .unwrap()
        .slot = Some(scene);
    let before = state.clone();
    let mut ctx = fresh_context();
    let outcome = run(
        &a(to_track(&[(&f.a, &second), (&f.b, &second)], None)),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert!(state.project.tracks[&f.track].clips.contains_key(&f.looped));
    state.project.validate().unwrap();
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut ctx).unwrap();
    assert_eq!(state.project, before.project);
}

#[test]
fn a_placement_moved_to_another_track_carves_what_it_covers_there() {
    let f = fx();
    let (mut state, second) = with_second_track(&f);
    // The second track holds the same layout: `c` lands over its first placement.
    let (out, _) = ok(&mut state, to_track(&[(&f.c, &second)], Some(-7680)));
    let arrangement::Output::MoveToTrack { overwritten, .. } = out else {
        panic!()
    };
    assert!(overwritten >= 1);
    assert!(overlap_free(&state, &second));
}

#[test]
fn a_move_to_another_track_is_refused_for_audio_unknown_and_repeated_names() {
    let f = fx();
    let (state, second) = with_second_track(&f);
    let unknown = TrackId::generate(&mut ::project::IdGenerator::new(1_690_100_000_000, 7));
    for command in [
        to_track(&[], None),
        to_track(&[(&f.a, &f.audio_track)], None),
        to_track(&[(&f.a, &unknown)], None),
        to_track(&[(&f.a, &second), (&f.a, &second)], None),
        to_track(&[(&f.a, &second)], Some(-1)),
    ] {
        assert!(
            check_refusal(&a(command), &state, &context()).is_ok(),
            "refused"
        );
    }
    let err = check_refusal(
        &a(to_track(&[(&f.a, &f.audio_track)], None)),
        &state,
        &context(),
    )
    .unwrap();
    assert!(matches!(err, CommandError::Invalid { .. }));
}

#[test]
fn a_move_where_everything_stays_changes_nothing_and_makes_no_history() {
    let f = fx();
    let mut state = f.state.clone();
    let outcome = run(
        &a(to_track(&[(&f.a, &f.track), (&f.b, &f.track)], None)),
        &mut state,
        &mut fresh_context(),
    )
    .unwrap();
    assert!(matches!(outcome.undo, Undo::None));
    assert!(outcome.events.is_empty());
    assert_eq!(state, f.state);
}

#[test]
fn a_move_to_another_track_is_refused_for_an_audio_source_and_at_the_placement_limit() {
    let f = fx();
    let (mut state, second) = with_second_track(&f);
    // An audio placement is not moved (audio arrangement comes with the import).
    let mut ids = ::project::IdGenerator::new(1_692_000_000_000, 0x00AC_0000_0000_0000_0001);
    let audio = ::project::PlacementId::generate(&mut ids);
    state
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .placements
        .insert(
            audio.clone(),
            ::project::Placement {
                clip: f.audio_clip.clone(),
                start: Tick(0),
                span: Span::Frames {
                    length: ::project::Frame(100),
                    offset: ::project::Frame(0),
                },
            },
        );
    let err = check_refusal(&a(to_track(&[(&audio, &second)], None)), &state, &context()).unwrap();
    assert!(matches!(err, CommandError::Invalid { .. }), "{err}");
    // The target track is full: refused before anything is written.
    let mut state = with_second_track(&f).0;
    let mut at = 1_000_000;
    let target = state.project.tracks.get_mut(&second).unwrap();
    let clip = target.clips.keys().next().unwrap().clone();
    while target.placements.len() < ::project::limits::MAX_PLACEMENTS_PER_TRACK {
        target.placements.insert(
            PlacementId::generate(&mut ids),
            ::project::Placement {
                clip: clip.clone(),
                start: Tick(at),
                span: Span::Ticks {
                    length: Tick(1),
                    offset: Tick(0),
                },
            },
        );
        at += 2;
    }
    let err = check_refusal(
        &a(to_track(&[(&f.a, &second)], Some(500_000))),
        &state,
        &context(),
    )
    .unwrap();
    assert!(matches!(err, CommandError::Limit { .. }), "{err}");
}
