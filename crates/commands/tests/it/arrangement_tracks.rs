//! #491: `arrangement.move_to_track`: the clip that goes with a placement, what stays, the
//! carve in the target, the refusals, the no-op and the exact undo.

use crate::arrangement_examples;
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
        if_occupied: arrangement::IfOccupied::Replace,
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

// --- #805: a move or a copy never replaces a placement unless it is told to ---------------------

fn with_occupied(
    mut command: arrangement::Command,
    how: arrangement::IfOccupied,
) -> arrangement::Command {
    match &mut command {
        arrangement::Command::MoveToTrack { if_occupied, .. }
        | arrangement::Command::Duplicate { if_occupied, .. } => *if_occupied = how,
        _ => panic!("not a move or a copy"),
    }
    command
}

/// Three MIDI tracks in display order, the first the fixture's, the other two copies of its
/// layout (so the same places are taken).
fn three_tracks(f: &arrangement_examples::ArFx) -> (State, [TrackId; 3]) {
    let (mut state, _) = with_second_track(f);
    run(
        &commands::Command::Track(commands::track::Command::Duplicate {
            track: f.track.clone(),
            name: None,
        }),
        &mut state,
        &mut commands::Context::new(
            commands::Author::User,
            ::project::IdGenerator::new(1_685_000_000_000, 0x00DC_0000_0000_0000_0000),
        ),
    )
    .unwrap();
    let midi: Vec<TrackId> = state
        .project
        .tracks_in_order()
        .into_iter()
        .filter(|(_, t)| t.kind == ::project::TrackKind::Midi)
        .map(|(id, _)| id.clone())
        .collect();
    assert_eq!(midi.len(), 3);
    let [x, y, z]: [TrackId; 3] = midi.try_into().unwrap();
    assert_eq!(x, f.track, "the fixture's own track comes first");
    (state, [x, y, z])
}

fn invalid_if_occupied(err: &CommandError) -> bool {
    matches!(err, CommandError::Invalid { field, .. } if field == "if_occupied")
}

#[test]
fn a_refused_move_to_an_occupied_place_replaces_nothing_and_changes_nothing() {
    let f = fx();
    let (state, second) = with_second_track(&f);
    let command = with_occupied(
        to_track(&[(&f.c, &second)], Some(-7680)),
        arrangement::IfOccupied::Refuse,
    );
    let mut after = state.clone();
    let err = run(&a(command), &mut after, &mut fresh_context()).unwrap_err();
    assert!(invalid_if_occupied(&err));
    assert_eq!(after, state);
    // The default, `replace`, still carves, as a drag does.
    let mut replaced = state.clone();
    let (out, _) = ok(&mut replaced, to_track(&[(&f.c, &second)], Some(-7680)));
    let arrangement::Output::MoveToTrack { overwritten, .. } = out else {
        panic!()
    };
    assert!(overwritten >= 1);
}

#[test]
fn a_refusing_move_onto_free_ground_is_a_plain_move_and_undoes_exactly() {
    let f = fx();
    let (mut state, second) = with_second_track(&f);
    let before = state.clone();
    let mut ctx = fresh_context();
    let command = with_occupied(
        to_track(&[(&f.c, &second)], Some(100_000)),
        arrangement::IfOccupied::Refuse,
    );
    let outcome = run(&a(command), &mut state, &mut ctx).unwrap();
    assert!(overlap_free(&state, &second));
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut ctx).unwrap();
    assert_eq!(state.project, before.project);
}

#[test]
fn a_skipping_move_lands_on_the_next_free_track_and_undoes_exactly() {
    let f = fx();
    let (mut state, [first, second, third]) = three_tracks(&f);
    state
        .project
        .tracks
        .get_mut(&third)
        .unwrap()
        .placements
        .clear();
    let before = state.clone();
    let mut ctx = fresh_context();
    // The second track holds the same layout: its place is taken, so the move goes on.
    let command = with_occupied(
        to_track(&[(&f.a, &second)], None),
        arrangement::IfOccupied::Skip,
    );
    let outcome = run(&a(command), &mut state, &mut ctx).unwrap();
    state.project.validate().unwrap();
    let Output::Arrangement(arrangement::Output::MoveToTrack {
        placements,
        overwritten,
    }) = outcome.output
    else {
        panic!()
    };
    assert_eq!(overwritten, 0);
    assert!(
        state.project.tracks[&third]
            .placements
            .contains_key(&placements[0])
    );
    assert!(!state.project.tracks[&first].placements.contains_key(&f.a));
    assert_eq!(
        state.project.tracks[&second].placements, before.project.tracks[&second].placements,
        "the occupied track is left as it was"
    );
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut ctx).unwrap();
    assert_eq!(state.project, before.project);
}

#[test]
fn a_skipping_move_to_a_free_place_does_not_skip() {
    let f = fx();
    let (mut state, [_, second, third]) = three_tracks(&f);
    state
        .project
        .tracks
        .get_mut(&second)
        .unwrap()
        .placements
        .clear();
    let command = with_occupied(
        to_track(&[(&f.a, &second)], None),
        arrangement::IfOccupied::Skip,
    );
    let (out, _) = ok(&mut state, command);
    let arrangement::Output::MoveToTrack { placements, .. } = out else {
        panic!()
    };
    assert!(
        state.project.tracks[&second]
            .placements
            .contains_key(&placements[0])
    );
    assert!(
        !state.project.tracks[&third]
            .placements
            .contains_key(&placements[0])
    );
}

#[test]
fn a_skipping_move_with_no_free_track_left_is_refused_and_changes_nothing() {
    let f = fx();
    let (state, [_, second, _]) = three_tracks(&f);
    // Every track below holds the layout: nowhere to go.
    let command = with_occupied(
        to_track(&[(&f.a, &second)], None),
        arrangement::IfOccupied::Skip,
    );
    let mut after = state.clone();
    let err = run(&a(command), &mut after, &mut fresh_context()).unwrap_err();
    assert!(invalid_if_occupied(&err));
    assert_eq!(after, state);
}

#[test]
fn a_skipping_move_goes_up_past_taken_tracks_and_is_refused_at_the_first() {
    let f = fx();
    let (state, [first, second, third]) = three_tracks(&f);
    let on_third = state.project.tracks[&third]
        .placements
        .keys()
        .next()
        .unwrap()
        .clone();
    let up = |state: &State, from: &PlacementId, to: &TrackId| {
        let command = with_occupied(to_track(&[(from, to)], None), arrangement::IfOccupied::Skip);
        let mut after = state.clone();
        let result = run(&a(command), &mut after, &mut fresh_context());
        (result, after)
    };
    // The track above is taken and so is the first: nowhere to go, nothing changed.
    let (result, after) = up(&state, &on_third, &second);
    assert!(invalid_if_occupied(&result.unwrap_err()));
    assert_eq!(after, state);
    // With the first track free, the move skips the second and lands there.
    let mut freed = state.clone();
    freed
        .project
        .tracks
        .get_mut(&first)
        .unwrap()
        .placements
        .clear();
    let (result, after) = up(&freed, &on_third, &second);
    let Output::Arrangement(arrangement::Output::MoveToTrack { placements, .. }) =
        result.unwrap().output
    else {
        panic!()
    };
    assert!(
        after.project.tracks[&first]
            .placements
            .contains_key(&placements[0])
    );
    assert!(
        !after.project.tracks[&third]
            .placements
            .contains_key(&on_third)
    );
    assert_eq!(
        after.project.tracks[&second].placements,
        freed.project.tracks[&second].placements
    );
}

#[test]
fn a_skipping_move_needs_one_shift_of_tracks_for_every_move() {
    let f = fx();
    let (state, [_, second, third]) = three_tracks(&f);
    let on_second = state.project.tracks[&second]
        .placements
        .keys()
        .next()
        .unwrap()
        .clone();
    // One placement one track down, another one track down too is a shift; one down and one
    // two down is not.
    let command = with_occupied(
        to_track(
            &[(&f.a, &second), (&on_second, &third), (&f.b, &third)],
            None,
        ),
        arrangement::IfOccupied::Skip,
    );
    let mut after = state.clone();
    let err = run(&a(command), &mut after, &mut fresh_context()).unwrap_err();
    assert!(invalid_if_occupied(&err));
    assert_eq!(after, state);
}

#[test]
fn a_refused_copy_onto_the_next_placement_replaces_nothing() {
    let f = fx();
    let copy = |how| {
        a(with_occupied(
            arrangement::Command::Duplicate {
                placements: vec![f.a.clone()],
                linked: false,
                by: None,
                if_occupied: arrangement::IfOccupied::Replace,
            },
            how,
        ))
    };
    // `a` lasts 3840 ticks and `b` starts right after it: its copy lands on `b`.
    let mut refused = f.state.clone();
    let err = run(
        &copy(arrangement::IfOccupied::Refuse),
        &mut refused,
        &mut fresh_context(),
    )
    .unwrap_err();
    assert!(invalid_if_occupied(&err));
    assert_eq!(refused, f.state);
    // Told to replace (the default), the copy carves `b`.
    let mut replaced = f.state.clone();
    let outcome = run(
        &copy(arrangement::IfOccupied::Replace),
        &mut replaced,
        &mut fresh_context(),
    )
    .unwrap();
    let Output::Arrangement(arrangement::Output::Duplicate { overwritten, .. }) = outcome.output
    else {
        panic!()
    };
    assert!(overwritten >= 1);
}

#[test]
fn a_refusing_copy_onto_free_ground_is_a_plain_copy_and_undoes_exactly() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = fresh_context();
    let command = a(arrangement::Command::Duplicate {
        placements: vec![f.c.clone()],
        linked: false,
        by: Some(Tick(40_000)),
        if_occupied: arrangement::IfOccupied::Refuse,
    });
    let outcome = run(&command, &mut state, &mut ctx).unwrap();
    assert_eq!(
        state.project.tracks[&f.track].placements.len(),
        f.state.project.tracks[&f.track].placements.len() + 1
    );
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut ctx).unwrap();
    assert_eq!(state.project, f.state.project);
}

#[test]
fn a_copy_cannot_skip() {
    let f = fx();
    let command = a(arrangement::Command::Duplicate {
        placements: vec![f.c.clone()],
        linked: false,
        by: Some(Tick(40_000)),
        if_occupied: arrangement::IfOccupied::Skip,
    });
    let mut state = f.state.clone();
    let err = run(&command, &mut state, &mut fresh_context()).unwrap_err();
    assert!(invalid_if_occupied(&err));
    assert_eq!(state, f.state);
}

#[test]
fn the_default_is_not_written_and_a_missing_field_means_replace() {
    let f = fx();
    let plain = to_track(&[(&f.a, &f.track)], None);
    let json = serde_json::to_value(&plain).unwrap();
    assert!(json.get("if_occupied").is_none());
    let skip = with_occupied(plain, arrangement::IfOccupied::Skip);
    let json = serde_json::to_value(&skip).unwrap();
    assert_eq!(json["if_occupied"], "skip");
    let back: arrangement::Command = serde_json::from_value(json).unwrap();
    assert_eq!(back, skip);
}

/// Runs `refusing` and checks that it is refused for `if_occupied` and changed nothing, then
/// that `replacing` (the default, a drag's and the agents' behaviour) carves.
fn refused_when_it_carves(
    state: &State,
    refusing: arrangement::Command,
    replacing: arrangement::Command,
) {
    let mut refused = state.clone();
    let err = run(&a(refusing), &mut refused, &mut fresh_context()).unwrap_err();
    assert!(invalid_if_occupied(&err), "{err:?}");
    assert_eq!(&refused, state, "a refused command changes nothing");
    let mut replaced = state.clone();
    run(&a(replacing), &mut replaced, &mut fresh_context()).unwrap();
    assert_ne!(&replaced, state, "the default still carves");
}

#[test]
fn a_placement_from_the_pool_refuses_an_occupied_place_and_carves_by_default() {
    let f = fx();
    // `c` plays 7680 to 11520: the clip put down at 7000 for 1000 ticks reaches into it.
    let place = |how| arrangement::Command::Place {
        clip: f.looped.clone(),
        start: Tick(7000),
        length: Some(Tick(1000)),
        offset: None,
        if_occupied: how,
    };
    refused_when_it_carves(
        &f.state,
        place(arrangement::IfOccupied::Refuse),
        place(arrangement::IfOccupied::Replace),
    );
    // On free ground it is a plain placement, whatever `if_occupied` says.
    let mut state = f.state.clone();
    let free = arrangement::Command::Place {
        clip: f.looped.clone(),
        start: Tick(40_000),
        length: None,
        offset: None,
        if_occupied: arrangement::IfOccupied::Refuse,
    };
    run(&a(free), &mut state, &mut fresh_context()).unwrap();
    assert_eq!(
        state.project.tracks[&f.track].placements.len(),
        f.state.project.tracks[&f.track].placements.len() + 1
    );
}

#[test]
fn an_audio_placement_refuses_an_occupied_place_too() {
    let f = fx();
    let mut state = f.state.clone();
    let place = |how| arrangement::Command::Place {
        clip: f.audio_clip.clone(),
        start: Tick(3840),
        length: None,
        offset: None,
        if_occupied: how,
    };
    run(
        &a(place(arrangement::IfOccupied::Replace)),
        &mut state,
        &mut fresh_context(),
    )
    .unwrap();
    // Placed again on itself: the first placement would be carved.
    refused_when_it_carves(
        &state,
        place(arrangement::IfOccupied::Refuse),
        place(arrangement::IfOccupied::Replace),
    );
}

#[test]
fn a_grid_clip_sent_to_the_arrangement_refuses_an_occupied_place() {
    let f = fx();
    let state = f.grid_state();
    let send = |how| arrangement::Command::PlaceFromSlot {
        clip: f.looped.clone(),
        start: Tick(1000),
        linked: false,
        if_occupied: how,
    };
    refused_when_it_carves(
        &state,
        send(arrangement::IfOccupied::Refuse),
        send(arrangement::IfOccupied::Replace),
    );
}

#[test]
fn a_move_in_time_refuses_to_land_on_a_placement_that_stays() {
    let f = fx();
    // `a` (0 to 3840) one step later lands on `b` (3840 to 5760).
    let step = |how| arrangement::Command::Move {
        placements: vec![f.a.clone()],
        by: Tick(1920),
        if_occupied: how,
    };
    refused_when_it_carves(
        &f.state,
        step(arrangement::IfOccupied::Refuse),
        step(arrangement::IfOccupied::Replace),
    );
    // Moving onto the place of a placement that moves too is not an overlap, and the plain
    // move undoes exactly.
    let mut state = f.state.clone();
    let together = arrangement::Command::Move {
        placements: vec![f.a.clone(), f.b.clone()],
        by: Tick(1920),
        if_occupied: arrangement::IfOccupied::Refuse,
    };
    let outcome = run(&a(together), &mut state, &mut fresh_context()).unwrap();
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut fresh_context()).unwrap();
    assert_eq!(state.project, f.state.project);
}

#[test]
fn a_repeat_refuses_to_cover_the_placements_after_it() {
    let f = fx();
    // `a` repeated up to the end of `b` covers `b`.
    let fill = |how| arrangement::Command::Repeat {
        placement: f.a.clone(),
        until: Tick(3840 * 2),
        if_occupied: how,
    };
    refused_when_it_carves(
        &f.state,
        fill(arrangement::IfOccupied::Refuse),
        fill(arrangement::IfOccupied::Replace),
    );
    // Over free ground it repeats.
    let mut state = f.state.clone();
    let free = arrangement::Command::Repeat {
        placement: f.c.clone(),
        until: Tick(3840 * 5),
        if_occupied: arrangement::IfOccupied::Refuse,
    };
    let outcome = run(&a(free), &mut state, &mut fresh_context()).unwrap();
    assert!(matches!(
        outcome.output,
        Output::Arrangement(arrangement::Output::Repeat { overwritten: 0, .. })
    ));
}

#[test]
fn only_a_move_to_another_track_can_skip() {
    let f = fx();
    let skip = arrangement::IfOccupied::Skip;
    let commands = [
        arrangement::Command::Place {
            clip: f.looped.clone(),
            start: Tick(40_000),
            length: None,
            offset: None,
            if_occupied: skip,
        },
        arrangement::Command::Move {
            placements: vec![f.c.clone()],
            by: Tick(40_000),
            if_occupied: skip,
        },
        arrangement::Command::Repeat {
            placement: f.c.clone(),
            until: Tick(3840 * 5),
            if_occupied: skip,
        },
        arrangement::Command::PlaceFromSlot {
            clip: f.looped.clone(),
            start: Tick(40_000),
            linked: false,
            if_occupied: skip,
        },
    ];
    for command in commands {
        let mut state = f.grid_state();
        let before = state.clone();
        let err = run(&a(command), &mut state, &mut fresh_context()).unwrap_err();
        assert!(invalid_if_occupied(&err), "{err:?}");
        assert_eq!(state, before);
    }
}

#[test]
fn the_default_of_the_new_commands_is_not_written() {
    let f = fx();
    let place = arrangement::Command::Place {
        clip: f.looped.clone(),
        start: Tick(0),
        length: None,
        offset: None,
        if_occupied: Default::default(),
    };
    assert!(
        serde_json::to_value(&place)
            .unwrap()
            .get("if_occupied")
            .is_none()
    );
    let json = serde_json::json!({
        "type": "arrangement.repeat", "placement": f.a, "until": 9000, "if_occupied": "refuse"
    });
    let back: arrangement::Command = serde_json::from_value(json).unwrap();
    assert!(matches!(
        back,
        arrangement::Command::Repeat {
            if_occupied: arrangement::IfOccupied::Refuse,
            ..
        }
    ));
}
