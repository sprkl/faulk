//! AR-1: the arrangement commands beyond the harness: the overlap rule case by case, the
//! offsets through a clip's loop, groups, independent and linked copies, the clips an
//! edit leaves unused, exact undo and redo (one history entry for a selection), every
//! refusal, and a sweep of random edits that keeps the project valid and overlap-free.

mod arrangement_examples;
mod clip_examples;

use ::project::{ClipId, Placement, PlacementId, Span, Tick, TrackId};
use arrangement_examples::{ArFx, a, after, examples, fx};
use commands::arrangement::{self, Edge};
use commands::testing::{check, check_refusal, context};
use commands::{Command, CommandError, Output, ProjectEvent, State, Undo, history, run};

/// `(start, length, offset)` of every placement of the MIDI track, by start.
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

/// A context whose identifiers differ from every other call's, so that two commands run
/// in a row on one state never generate the same identifier.
fn fresh_context() -> commands::Context {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    commands::Context::new(
        commands::Author::User,
        ::project::IdGenerator::new(
            1_690_000_000_000 + n,
            0x00AA_0000_0000_0000_0000 + u128::from(n),
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

fn refused(fx: &ArFx, command: arrangement::Command) -> CommandError {
    check_refusal(&a(command), &fx.state, &context()).unwrap()
}

fn place(fx: &ArFx, clip: &ClipId, start: i64, length: i64) -> arrangement::Command {
    let _ = fx;
    arrangement::Command::Place {
        clip: clip.clone(),
        start: Tick(start),
        length: Some(Tick(length)),
        offset: None,
    }
}

#[test]
fn every_example_meets_the_contract_keeps_the_project_valid_and_undoes_exactly() {
    for (command, from) in examples() {
        check(&command, &from, &context()).unwrap();
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
        let Undo::Inverse(redo) = redo.undo else {
            panic!("no redo")
        };
        redo.apply(&mut state, &mut ctx).unwrap();
        state.project.validate().unwrap();
    }
}

/// The state with a gain lane on every track and one on the master, to see what a command
/// does to them (#390, #394).
fn with_lanes(from: &State) -> State {
    use ::project::{AutomationLane, AutomationPoint, AutomationTarget, IdGenerator, LaneId};
    use ::project::{MixerParam, Shape};
    let mut ids = IdGenerator::new(1_680_000_000_000, 0x00AA_0000_0000_0000_0000);
    let mut state = from.clone();
    let lane = |ids: &mut IdGenerator| {
        let points = [(0, -6.0), (960, 0.0)]
            .into_iter()
            .map(|(tick, value)| {
                (
                    Tick(tick),
                    AutomationPoint {
                        value,
                        shape: Shape::Linear,
                    },
                )
            })
            .collect();
        (
            LaneId::generate(ids),
            AutomationLane {
                target: AutomationTarget::Mixer {
                    param: MixerParam::Gain,
                },
                points,
            },
        )
    };
    for track in state.project.tracks.values_mut() {
        let (id, l) = lane(&mut ids);
        track.automation.insert(id, l);
    }
    let (id, l) = lane(&mut ids);
    state.project.master_automation.insert(id, l);
    state.project.validate().expect("the lanes are valid");
    state
}

/// The lanes of the whole project, by owner (`None` is the master).
type OwnedLanes = Vec<(
    Option<TrackId>,
    std::collections::BTreeMap<::project::LaneId, ::project::AutomationLane>,
)>;

fn all_lanes(state: &State) -> OwnedLanes {
    let mut all: OwnedLanes = state
        .project
        .tracks
        .iter()
        .map(|(id, t)| (Some(id.clone()), t.automation.clone()))
        .collect();
    all.push((None, state.project.master_automation.clone()));
    all
}

#[test]
fn no_arrangement_command_touches_a_lane() {
    // The nine commands of the module, each from its examples, with lanes on every track
    // and on the master: the lanes, the events and the undo leave them as they are
    // (#390, #394; the spec of AM-1 lists these names).
    let mut seen = std::collections::BTreeSet::new();
    for (command, from) in examples() {
        // The time commands move automation points on purpose (documented exception, #390,
        // #396); `tests/arrangement_time.rs` covers them.
        if matches!(
            command.name(),
            "arrangement.insert_time" | "arrangement.delete_time" | "arrangement.restore_time"
        ) {
            continue;
        }
        let from = with_lanes(&from);
        let before = all_lanes(&from);
        let mut state = from.clone();
        let mut ctx = context();
        let outcome = run(&command, &mut state, &mut ctx).unwrap();
        seen.insert(command.name());
        assert_eq!(all_lanes(&state), before, "{}", command.name());
        assert!(
            !outcome.events.iter().any(|e| matches!(
                e,
                ProjectEvent::AutomationLaneCreated { .. }
                    | ProjectEvent::AutomationLaneDeleted { .. }
                    | ProjectEvent::AutomationPointsChanged { .. }
            )),
            "{}: {:?}",
            command.name(),
            outcome.events
        );
        let Undo::Inverse(inverse) = outcome.undo else {
            panic!("{}: no inverse", command.name())
        };
        inverse.apply(&mut state, &mut ctx).unwrap();
        assert_eq!(all_lanes(&state), before, "{} undone", command.name());
    }
    let expected: std::collections::BTreeSet<&str> = [
        "arrangement.place",
        "arrangement.move",
        "arrangement.resize",
        "arrangement.split",
        "arrangement.delete",
        "arrangement.duplicate",
        "arrangement.place_from_slot",
        "arrangement.place_scene",
        "arrangement.restore",
    ]
    .into_iter()
    .collect();
    assert_eq!(seen, expected, "the examples cover the whole module");
}

#[test]
fn a_placement_on_free_ground_carves_nothing() {
    let f = fx();
    let mut s = f.state.clone();
    let (out, events) = ok(&mut s, place(&f, &f.looped, 11520, 100));
    let arrangement::Output::Place {
        overwritten,
        placement,
        ..
    } = out
    else {
        panic!()
    };
    assert_eq!(overwritten, 0);
    assert!(
        s.project.tracks[&f.track]
            .placements
            .contains_key(&placement)
    );
    assert_eq!(spans(&s, &f.track).len(), 4);
    assert_eq!(
        events[0],
        ProjectEvent::ArrangementChanged {
            track: f.track.clone()
        }
    );
}

#[test]
fn the_last_one_put_down_wins_case_by_case() {
    let f = fx();
    // A: [0, 3840) looped clip, offset 0; B: [3840, 5760) offset 960; C: [7680, 11520)
    // of the straight clip, offset 480.
    let base = spans(&f.state, &f.track);
    assert_eq!(base, [(0, 3840, 0), (3840, 1920, 960), (7680, 3840, 480)]);
    let run_place = |start, length| {
        let mut s = f.state.clone();
        ok(&mut s, place(&f, &f.straight, start, length));
        spans(&s, &f.track)
    };
    // Fully covered: A and B go.
    assert_eq!(run_place(0, 6000), [(0, 6000, 0), (7680, 3840, 480)]);
    // Contained: A is split. Its right part resumes at 1580 in the clip (the loop is
    // 0..1920: 3500 ticks in, 1580 into the second pass).
    assert_eq!(
        run_place(3000, 500),
        [
            (0, 3000, 0),
            (3000, 500, 0),
            (3500, 340, 1580),
            (3840, 1920, 960),
            (7680, 3840, 480)
        ]
    );
    // Starts before and ends inside the next: A is shortened, B trimmed at the front
    // (960 + 160 = 1120, still before the loop's end).
    assert_eq!(
        run_place(3000, 1000),
        [
            (0, 3000, 0),
            (3000, 1000, 0),
            (4000, 1760, 1120),
            (7680, 3840, 480)
        ]
    );
    // Trimmed through the loop's end: 960 + 1000 = 1960 -> 40.
    assert_eq!(
        run_place(3840, 1000),
        [
            (0, 3840, 0),
            (3840, 1000, 0),
            (4840, 920, 40),
            (7680, 3840, 480)
        ]
    );
    // A non-looping clip's placement trimmed at the front: 480 + 320.
    assert_eq!(
        run_place(7000, 1000),
        [
            (0, 3840, 0),
            (3840, 1920, 960),
            (7000, 1000, 0),
            (8000, 3520, 800)
        ]
    );
}

#[test]
fn a_front_trim_with_nothing_left_of_a_clip_without_a_loop_drops_the_placement() {
    let f = fx();
    let mut s = f.state.clone();
    // `straight` is 3840 ticks without a loop: a placement of 8000 ticks plays it once and
    // is silent after. Trimming its first 4000 ticks leaves nothing to play.
    let (out, _) = ok(
        &mut s,
        arrangement::Command::Place {
            clip: f.straight.clone(),
            start: Tick(20_000),
            length: Some(Tick(8000)),
            offset: Some(Tick(0)),
        },
    );
    let arrangement::Output::Place { placement, .. } = out else {
        panic!()
    };
    ok(&mut s, place(&f, &f.looped, 20_000, 4000));
    assert!(
        !s.project.tracks[&f.track]
            .placements
            .contains_key(&placement),
        "nothing of the clip is left past tick 24000"
    );
    s.project.validate().unwrap();
}

#[test]
fn moving_a_group_lifts_it_first_and_may_land_on_its_own_old_place() {
    let f = fx();
    let mut s = f.state.clone();
    // A and B by one A-length: A lands on B's old place, B further, carving C.
    let (out, events) = ok(
        &mut s,
        arrangement::Command::Move {
            placements: vec![f.a.clone(), f.b.clone()],
            by: Tick(3840),
        },
    );
    assert_eq!(
        out,
        arrangement::Output::Move {
            overwritten: 1,
            clips_removed: 0
        }
    );
    assert_eq!(
        spans(&s, &f.track),
        [
            (3840, 3840, 0),
            (7680, 1920, 960),
            // C trimmed at the front by 1920: 480 + 1920.
            (9600, 1920, 2400)
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ProjectEvent::ArrangementChanged { .. }))
            .count(),
        1,
        "one event for the track"
    );
    // One history entry: the undo of a state with a selection moved is one undo.
    let mut state = f.state.clone();
    let mut ctx = context();
    let before = state.history.undo.len();
    run(
        &a(arrangement::Command::Move {
            placements: vec![f.a.clone(), f.b.clone()],
            by: Tick(3840),
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(state.history.undo.len(), before + 1);
    run(
        &Command::History(history::Command::Undo {}),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(state.project, f.state.project);
    run(
        &Command::History(history::Command::Redo {}),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(spans(&state, &f.track), spans(&s, &f.track));
}

#[test]
fn a_move_of_nothing_changes_nothing_and_leaves_no_history() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let outcome = run(
        &a(arrangement::Command::Move {
            placements: vec![f.a.clone()],
            by: Tick(0),
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    assert!(matches!(outcome.undo, Undo::None));
    assert!(outcome.events.is_empty());
    assert_eq!(state, f.state);
}

#[test]
fn moves_cross_tracks_in_one_command_and_one_undo() {
    // Two MIDI tracks, a placement on each, moved together.
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    let Output::Track(commands::track::Output::Duplicate { track: second }) = run(
        &Command::Track(commands::track::Command::Duplicate {
            track: f.track.clone(),
            name: None,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap()
    .output
    else {
        panic!()
    };
    // The duplicate carries its own placements with new identifiers.
    let mine: Vec<PlacementId> = state.project.tracks[&second]
        .placements
        .keys()
        .cloned()
        .collect();
    assert!(!mine.is_empty());
    let mut ids = vec![f.a.clone()];
    ids.push(mine[0].clone());
    let before = state.project.clone();
    let outcome = run(
        &a(arrangement::Command::Move {
            placements: ids,
            by: Tick(100_000),
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let moved = |track: &TrackId, s: &State| {
        s.project.tracks[track]
            .placements
            .values()
            .filter(|p| p.start.get() >= 100_000)
            .count()
    };
    assert_eq!(moved(&f.track, &state), 1);
    assert_eq!(moved(&second, &state), 1);
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut ctx).unwrap();
    assert_eq!(state.project, before);
}

#[test]
fn resizing_the_end_and_the_start_keeps_the_content_where_it_is() {
    let f = fx();
    let resize = |placement: &PlacementId, edge, to| {
        let mut s = f.state.clone();
        ok(
            &mut s,
            arrangement::Command::Resize {
                placement: placement.clone(),
                edge,
                to: Tick(to),
            },
        );
        spans(&s, &f.track)
    };
    // The end of A grows over B: B is trimmed at the front, through the loop.
    assert_eq!(
        resize(&f.a, Edge::End, 5000),
        [(0, 5000, 0), (5000, 760, 200), (7680, 3840, 480)]
    );
    // The end shrinks: nothing else moves.
    assert_eq!(
        resize(&f.a, Edge::End, 1000),
        [(0, 1000, 0), (3840, 1920, 960), (7680, 3840, 480)]
    );
    // The start of B moves right by 960: its offset follows through the loop (960 + 960 =
    // 1920 -> back to the loop's start, 0).
    assert_eq!(
        resize(&f.b, Edge::Start, 4800),
        [(0, 3840, 0), (4800, 960, 0), (7680, 3840, 480)]
    );
    // The start of B moves left into A: A is shortened, B shows earlier content (960 - 480).
    assert_eq!(
        resize(&f.b, Edge::Start, 3360),
        [(0, 3360, 0), (3360, 2400, 480), (7680, 3840, 480)]
    );
    // Moving the edge to where it is changes nothing.
    let mut state = f.state.clone();
    let outcome = run(
        &a(arrangement::Command::Resize {
            placement: f.a.clone(),
            edge: Edge::End,
            to: Tick(3840),
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert!(matches!(outcome.undo, Undo::None));
}

#[test]
fn a_split_gives_the_right_part_its_offset_through_the_loop() {
    let f = fx();
    let mut s = f.state.clone();
    let (out, _) = ok(
        &mut s,
        arrangement::Command::Split {
            placement: f.a.clone(),
            at: Tick(2500),
        },
    );
    let arrangement::Output::Split { placement } = out else {
        panic!()
    };
    // 2500 ticks in, the loop (0..1920) is 580 into its second pass.
    assert_eq!(
        spans(&s, &f.track),
        [
            (0, 2500, 0),
            (2500, 1340, 580),
            (3840, 1920, 960),
            (7680, 3840, 480)
        ]
    );
    let right = &s.project.tracks[&f.track].placements[&placement];
    assert_eq!(right.clip, f.looped, "the right part is a linked placement");
    // The left part kept its identifier.
    assert_eq!(s.project.tracks[&f.track].placements[&f.a].start, Tick(0));
}

#[test]
fn an_independent_copy_is_a_new_clip_and_a_linked_copy_is_a_new_placement() {
    let f = fx();
    let mut s = f.state.clone();
    let clips = s.project.tracks[&f.track].clips.len();
    let (out, events) = ok(
        &mut s,
        arrangement::Command::Duplicate {
            placements: vec![f.a.clone(), f.b.clone()],
            linked: false,
            by: None,
        },
    );
    let arrangement::Output::Duplicate {
        placements,
        clips: created,
        overwritten,
        clips_removed,
    } = out
    else {
        panic!()
    };
    // A and B share a clip: one copy of it, shared by both copies. The group spans
    // [0, 5760): the copies start at 5760 and 9600; the first carves C at the front, the
    // second covers what is left of it.
    assert_eq!(
        (placements.len(), created.len(), overwritten, clips_removed),
        (2, 1, 2, 1),
        "the report says the covered clip went"
    );
    // The copy joined the clips; C, covered whole, took its clip with it (no grid slot).
    assert_eq!(s.project.tracks[&f.track].clips.len(), clips);
    assert!(!s.project.tracks[&f.track].clips.contains_key(&f.straight));
    let track = &s.project.tracks[&f.track];
    let copy = &track.clips[&created[0]];
    assert_eq!(copy.slot, None, "a copy is not in the launch grid");
    let source = &track.clips[&track.placements[&f.a].clip].name;
    assert_ne!(&copy.name, source, "a copy has a name of its own");
    assert_eq!(track.placements[&placements[0]].clip, created[0]);
    assert_eq!(track.placements[&placements[1]].clip, created[0]);
    assert_eq!(
        spans(&s, &f.track),
        [
            (0, 3840, 0),
            (3840, 1920, 960),
            (5760, 3840, 0),
            (9600, 1920, 960)
        ]
    );
    // Editing the copy leaves the original alone.
    let mut state = s.clone();
    run(
        &Command::Clip(commands::clip::Command::NotesAdd {
            clip: created[0].clone(),
            notes: vec![clip_examples::n(5, 1, 5, 5)],
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    let notes =
        |state: &State, clip: &ClipId| match &state.project.tracks[&f.track].clips[clip].content {
            ::project::ClipContent::Midi { notes, .. } => notes.len(),
            _ => panic!(),
        };
    assert_eq!(notes(&state, &created[0]), notes(&s, &f.looped) + 1);
    assert_eq!(notes(&state, &f.looped), notes(&s, &f.looped));
    // The events: the clip removed, the clip created, then the track.
    assert!(matches!(events[0], ProjectEvent::ClipDeleted { .. }));
    assert!(matches!(events[1], ProjectEvent::ClipCreated { .. }));
    assert!(matches!(events[2], ProjectEvent::ArrangementChanged { .. }));

    // Linked: no clip, the same clip twice.
    let mut l = f.state.clone();
    let (out, _) = ok(
        &mut l,
        arrangement::Command::Duplicate {
            placements: vec![f.a.clone()],
            linked: true,
            by: Some(Tick(20_000)),
        },
    );
    let arrangement::Output::Duplicate {
        placements,
        clips: created,
        ..
    } = out
    else {
        panic!()
    };
    assert!(created.is_empty());
    assert_eq!(l.project.tracks[&f.track].clips.len(), clips);
    let Placement { clip, start, .. } = &l.project.tracks[&f.track].placements[&placements[0]];
    assert_eq!((clip, *start), (&f.looped, Tick(20_000)));
}

#[test]
fn a_clip_left_without_placement_goes_with_it_unless_it_sits_in_the_grid() {
    let f = fx();
    let scene = f.state.project.scenes.keys().next().unwrap().clone();
    // A clip in the launch grid keeps living without a placement.
    let mut s = f.state.clone();
    s.project
        .tracks
        .get_mut(&f.track)
        .unwrap()
        .clips
        .get_mut(&f.straight)
        .unwrap()
        .slot = Some(scene);
    ok(
        &mut s,
        arrangement::Command::Delete {
            placements: vec![f.c.clone()],
        },
    );
    assert!(s.project.tracks[&f.track].clips.contains_key(&f.straight));
    // A clip of the arrangement only goes with its last placement, and the undo brings it
    // back with its notes and its identifier.
    let mut g = f.state.clone();
    let mut ctx = context();
    let outcome = run(
        &a(arrangement::Command::Delete {
            placements: vec![f.c.clone()],
        }),
        &mut g,
        &mut ctx,
    )
    .unwrap();
    assert!(!g.project.tracks[&f.track].clips.contains_key(&f.straight));
    assert!(matches!(
        outcome.events[0],
        ProjectEvent::ClipDeleted { .. }
    ));
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!()
    };
    inverse.apply(&mut g, &mut ctx).unwrap();
    assert_eq!(g.project, f.state.project);
    // Only the last placement counts: A and B share a clip.
    let mut h = f.state.clone();
    ok(
        &mut h,
        arrangement::Command::Delete {
            placements: vec![f.a.clone()],
        },
    );
    assert!(h.project.tracks[&f.track].clips.contains_key(&f.looped));
    ok(
        &mut h,
        arrangement::Command::Delete {
            placements: vec![f.b.clone()],
        },
    );
    assert!(!h.project.tracks[&f.track].clips.contains_key(&f.looped));
}

#[test]
fn restoring_brings_back_what_the_rule_carved_with_the_same_identifiers() {
    let f = fx();
    for command in [
        place(&f, &f.straight, 3000, 500),
        place(&f, &f.straight, 3000, 1000),
        place(&f, &f.straight, 0, 20_000),
    ] {
        let (state, inverse) = after(f.state.clone(), &a(command));
        let mut back = state.clone();
        let mut ctx = context();
        run(&inverse, &mut back, &mut ctx).unwrap();
        assert_eq!(back.project, f.state.project);
    }
}

#[test]
fn audio_is_refused_and_so_is_everything_that_leaves_the_timeline() {
    let f = fx();
    use arrangement::Command as C;
    let cases = [
        // A clip of an audio track.
        C::Place {
            clip: f.audio_clip.clone(),
            start: Tick(0),
            length: None,
            offset: None,
        },
    ];
    for command in cases {
        assert!(matches!(refused(&f, command), CommandError::Invalid { .. }));
    }
    // A clip of the launch grid (AR-5 copies or links it).
    let mut gridded = fx();
    let scene = gridded.state.project.scenes.keys().next().unwrap().clone();
    gridded
        .state
        .project
        .tracks
        .get_mut(&gridded.track)
        .unwrap()
        .clips
        .get_mut(&gridded.looped)
        .unwrap()
        .slot = Some(scene);
    assert!(matches!(
        refused(
            &gridded,
            C::Place {
                clip: gridded.looped.clone(),
                start: Tick(20_000),
                length: None,
                offset: None,
            }
        ),
        CommandError::Invalid { .. }
    ));
    let unknown = PlacementId::generate(&mut ::project::IdGenerator::for_tests());
    let invalid = |c: C| {
        assert!(
            matches!(refused(&f, c.clone()), CommandError::Invalid { .. }),
            "{c:?}"
        )
    };
    // Place: an offset past the clip, a start before 0, a length of 0, past 2^40.
    let mut s = f.state.clone();
    let (out, _) = ok(
        &mut s,
        C::Duplicate {
            placements: vec![f.b.clone()],
            linked: false,
            by: Some(Tick(30_000)),
        },
    );
    let arrangement::Output::Duplicate { clips, .. } = out else {
        panic!()
    };
    let free = clips[0].clone();
    let check_g = |c: C| {
        let e = check_refusal(&a(c.clone()), &s, &context()).unwrap();
        assert!(
            matches!(e, CommandError::Invalid { .. } | CommandError::Limit { .. }),
            "{c:?}: {e}"
        );
    };
    for command in [
        C::Place {
            clip: free.clone(),
            start: Tick(0),
            length: None,
            offset: Some(Tick(3840)),
        },
        C::Place {
            clip: free.clone(),
            start: Tick(-1),
            length: None,
            offset: None,
        },
        C::Place {
            clip: free.clone(),
            start: Tick(0),
            length: Some(Tick(0)),
            offset: None,
        },
        C::Place {
            clip: free.clone(),
            start: Tick(1 << 40),
            length: None,
            offset: None,
        },
    ] {
        check_g(command);
    }
    invalid(C::Move {
        placements: vec![],
        by: Tick(1),
    });
    invalid(C::Move {
        placements: vec![f.a.clone(), f.a.clone()],
        by: Tick(1),
    });
    invalid(C::Move {
        placements: vec![f.a.clone()],
        by: Tick(-1),
    });
    invalid(C::Move {
        placements: vec![f.c.clone()],
        by: Tick(1 << 40),
    });
    invalid(C::Move {
        placements: vec![f.a.clone()],
        by: Tick(i64::MAX),
    });
    assert!(matches!(
        refused(
            &f,
            C::Move {
                placements: vec![unknown.clone()],
                by: Tick(1)
            }
        ),
        CommandError::NotFound { .. }
    ));
    invalid(C::Delete { placements: vec![] });
    invalid(C::Duplicate {
        placements: vec![],
        linked: false,
        by: None,
    });
    invalid(C::Duplicate {
        placements: vec![f.a.clone()],
        linked: false,
        by: Some(Tick(-1)),
    });
    // Resize: the end before the start, the start past the end, before the clip's start.
    invalid(C::Resize {
        placement: f.a.clone(),
        edge: Edge::End,
        to: Tick(0),
    });
    invalid(C::Resize {
        placement: f.a.clone(),
        edge: Edge::Start,
        to: Tick(3840),
    });
    invalid(C::Resize {
        placement: f.a.clone(),
        edge: Edge::End,
        to: Tick(1 << 41),
    });
    // B starts 960 into the clip: its start can go left by 960 but not 961.
    invalid(C::Resize {
        placement: f.b.clone(),
        edge: Edge::Start,
        to: Tick(3840 - 961),
    });
    // C's clip has no loop: its start cannot go past the clip's end (480 + 3360 = 3840).
    invalid(C::Resize {
        placement: f.c.clone(),
        edge: Edge::Start,
        to: Tick(7680 + 3360),
    });
    // Split: on the edges, outside, and past the end of a clip without a loop.
    for at in [0, 3840, 5000, -1] {
        invalid(C::Split {
            placement: f.a.clone(),
            at: Tick(at),
        });
    }
    invalid(C::Split {
        placement: f.c.clone(),
        at: Tick(7680 + 3360),
    });
    // Restore: an unknown clip, an offset outside the clip, a removed clip still placed,
    // an identifier in use, two edits of a track.
    let record =
        |id: &PlacementId, clip: &ClipId, start, length, offset| arrangement::PlacementRecord {
            id: id.clone(),
            clip: clip.clone(),
            start: Tick(start),
            length: Tick(length),
            offset: Tick(offset),
        };
    let edit = |put: Vec<arrangement::PlacementRecord>,
                remove: Vec<PlacementId>,
                remove_clips: Vec<ClipId>| arrangement::TrackEdit {
        track: f.track.clone(),
        put,
        remove,
        clips: vec![],
        remove_clips,
    };
    let fresh = PlacementId::generate(&mut ::project::IdGenerator::for_tests());
    invalid(C::Restore { edits: vec![] });
    assert!(matches!(
        refused(
            &f,
            C::Restore {
                edits: vec![edit(
                    vec![record(
                        &fresh,
                        &ClipId::generate(&mut ::project::IdGenerator::for_tests()),
                        0,
                        1,
                        0
                    )],
                    vec![],
                    vec![],
                )],
            }
        ),
        CommandError::NotFound { .. }
    ));
    invalid(C::Restore {
        edits: vec![edit(
            vec![record(&fresh, &f.straight, 20_000, 10, 3840)],
            vec![],
            vec![],
        )],
    });
    invalid(C::Restore {
        edits: vec![edit(vec![], vec![], vec![f.straight.clone()])],
    });
    invalid(C::Restore {
        edits: vec![edit(vec![], vec![unknown.clone()], vec![])],
    });
    invalid(C::Restore {
        edits: vec![edit(vec![], vec![], vec![]), edit(vec![], vec![], vec![])],
    });
    invalid(C::Restore {
        edits: vec![edit(
            vec![
                record(&f.b, &f.straight, 0, 10, 0),
                record(&f.b, &f.straight, 0, 10, 0),
            ],
            vec![],
            vec![],
        )],
    });
    // Unknown placements, and a repeated one, in every command that takes a list or one.
    invalid(C::Delete {
        placements: vec![f.a.clone(), f.a.clone()],
    });
    invalid(C::Duplicate {
        placements: vec![f.a.clone(), f.a.clone()],
        linked: true,
        by: None,
    });
    for command in [
        C::Delete {
            placements: vec![unknown.clone()],
        },
        C::Duplicate {
            placements: vec![unknown.clone()],
            linked: false,
            by: None,
        },
        C::Resize {
            placement: unknown.clone(),
            edge: Edge::End,
            to: Tick(10),
        },
        C::Split {
            placement: unknown.clone(),
            at: Tick(10),
        },
    ] {
        assert!(matches!(
            refused(&f, command),
            CommandError::NotFound { .. }
        ));
    }
    // An extreme edge never overflows.
    for to in [i64::MIN, i64::MAX, -1] {
        invalid(C::Resize {
            placement: f.b.clone(),
            edge: Edge::Start,
            to: Tick(to),
        });
        invalid(C::Resize {
            placement: f.b.clone(),
            edge: Edge::End,
            to: Tick(to),
        });
    }
    // Restore: an identifier in use (a placement of another track would do the same).
    invalid(C::Restore {
        edits: vec![edit(
            vec![record(&unknown_in_use(&f), &f.straight, 20_000, 10, 0)],
            vec![],
            vec![],
        )],
    });
    // Restore `clips`: every rule of the clip commands.
    let snap = || commands::clip::ClipSnapshot {
        name: "x".into(),
        color: None,
        slot: None,
        launch_quantize: None,
        length: Tick(960),
        looping: None,
        notes: vec![clip_examples::n(0, 60, 480, 100)],
    };
    let clip_record = |id: &ClipId, snapshot: commands::clip::ClipSnapshot| C::Restore {
        edits: vec![arrangement::TrackEdit {
            track: f.track.clone(),
            put: vec![],
            remove: vec![],
            clips: vec![arrangement::ClipRecord {
                id: id.clone(),
                snapshot,
            }],
            remove_clips: vec![],
        }],
    };
    let new_clip = ClipId::generate(&mut ::project::IdGenerator::for_tests());
    let mut state = f.state.clone();
    run(
        &a(clip_record(&new_clip, snap())),
        &mut state,
        &mut fresh_context(),
    )
    .unwrap();
    // The same clip is fine once, and its identifier is then in use.
    invalid(clip_record(&f.straight, snap()));
    let bad: Vec<commands::clip::ClipSnapshot> = vec![
        commands::clip::ClipSnapshot {
            name: "x".repeat(201),
            ..snap()
        },
        commands::clip::ClipSnapshot {
            length: Tick(0),
            ..snap()
        },
        commands::clip::ClipSnapshot {
            looping: Some(::project::TickLoop {
                start: Tick(500),
                end: Tick(100),
            }),
            ..snap()
        },
        commands::clip::ClipSnapshot {
            notes: vec![clip_examples::n(900, 60, 480, 100)],
            ..snap()
        },
        commands::clip::ClipSnapshot {
            notes: vec![
                clip_examples::n(0, 60, 480, 100),
                clip_examples::n(0, 60, 480, 100),
            ],
            ..snap()
        },
        commands::clip::ClipSnapshot {
            slot: Some(f.state.project.scenes.keys().next().unwrap().clone()),
            ..snap()
        },
    ];
    // The last one is valid (a free slot); the others are not.
    for (i, snapshot) in bad.into_iter().enumerate() {
        let id = ClipId::generate(&mut ::project::IdGenerator::for_tests());
        let result = check_refusal(&a(clip_record(&id, snapshot)), &f.state, &context());
        if i < 5 {
            assert!(
                matches!(result.unwrap(), CommandError::Invalid { .. }),
                "{i}"
            );
        } else {
            assert!(result.is_err(), "a free slot is accepted");
        }
    }
    // Two clips in one slot.
    let scene = f.state.project.scenes.keys().next().unwrap().clone();
    let (c1, c2) = (
        ClipId::generate(&mut ::project::IdGenerator::for_tests()),
        ClipId::generate(&mut ::project::IdGenerator::new(1_650_000_000_000, 77)),
    );
    let two = |slot: Option<::project::SceneId>| C::Restore {
        edits: vec![arrangement::TrackEdit {
            track: f.track.clone(),
            put: vec![],
            remove: vec![],
            clips: [&c1, &c2]
                .into_iter()
                .map(|id| arrangement::ClipRecord {
                    id: id.clone(),
                    snapshot: commands::clip::ClipSnapshot {
                        slot: slot.clone(),
                        ..snap()
                    },
                })
                .collect(),
            remove_clips: vec![],
        }],
    };
    invalid(two(Some(scene)));
    // An audio track has no arrangement here.
    invalid(C::Restore {
        edits: vec![arrangement::TrackEdit {
            track: f.audio_track.clone(),
            put: vec![],
            remove: vec![],
            clips: vec![],
            remove_clips: vec![],
        }],
    });
}

/// The identifier of a placement that exists (in the fixture's track).
fn unknown_in_use(fx: &ArFx) -> PlacementId {
    // An identifier of a clip is in use too: placements and clips share the id space.
    PlacementId(fx.straight.0.clone())
}

#[test]
fn the_placement_limit_of_a_track_is_refused_before_anything_changes() {
    let f = fx();
    let mut state = f.state.clone();
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let mut ids = ::project::IdGenerator::new(1_680_000_000_000, 0x00DD_0000_0000_0000_0001);
    let mut at = 20_000;
    while track.placements.len() < ::project::limits::MAX_PLACEMENTS_PER_TRACK {
        track.placements.insert(
            PlacementId::generate(&mut ids),
            Placement {
                clip: f.straight.clone(),
                start: Tick(at),
                span: Span::Ticks {
                    length: Tick(1),
                    offset: Tick(0),
                },
            },
        );
        at += 2;
    }
    let error = check_refusal(
        &a(arrangement::Command::Duplicate {
            placements: vec![f.a.clone()],
            linked: true,
            by: Some(Tick(1 << 30)),
        }),
        &state,
        &context(),
    )
    .unwrap();
    assert!(matches!(error, CommandError::Limit { .. }), "{error}");
    // A split adds one placement: refused at the limit as well, state intact.
    let error = check_refusal(
        &a(arrangement::Command::Split {
            placement: f.a.clone(),
            at: Tick(100),
        }),
        &state,
        &context(),
    )
    .unwrap();
    assert!(matches!(error, CommandError::Limit { .. }), "{error}");
}

/// A deterministic generator for the sweep.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

fn overlap_free(state: &State, track: &TrackId) -> bool {
    let list = spans(state, track);
    list.windows(2).all(|w| w[0].0 + w[0].1 <= w[1].0)
}

#[test]
fn random_edits_keep_the_project_valid_overlap_free_and_exactly_undoable() {
    let f = fx();
    // Start from the arrangement fixture with its overlaps resolved (it has none).
    let mut state = f.state.clone();
    let mut ctx = context();
    let mut rng = Lcg(77);
    for step in 0..400 {
        let ids: Vec<PlacementId> = state.project.tracks[&f.track]
            .placements
            .keys()
            .cloned()
            .collect();
        let pick = |rng: &mut Lcg| ids[rng.next(ids.len() as u64) as usize].clone();
        let clips: Vec<ClipId> = state.project.tracks[&f.track]
            .clips
            .keys()
            .cloned()
            .collect();
        let tick = |rng: &mut Lcg| Tick(rng.next(30) as i64 * 480);
        let command = match rng.next(7) {
            0 if !ids.is_empty() => {
                // Only a clip of the arrangement (no grid slot) can be placed.
                let free: Vec<&ClipId> = clips
                    .iter()
                    .filter(|c| state.project.tracks[&f.track].clips[*c].slot.is_none())
                    .collect();
                let Some(clip) = free.get(rng.next(free.len().max(1) as u64) as usize) else {
                    continue;
                };
                arrangement::Command::Place {
                    clip: (*clip).clone(),
                    start: tick(&mut rng),
                    length: Some(Tick(1 + rng.next(6000) as i64)),
                    offset: None,
                }
            }
            1 if !ids.is_empty() => arrangement::Command::Move {
                placements: vec![pick(&mut rng)],
                by: Tick(rng.next(8000) as i64 - 2000),
            },
            2 if !ids.is_empty() => arrangement::Command::Resize {
                placement: pick(&mut rng),
                edge: if rng.next(2) == 0 {
                    Edge::Start
                } else {
                    Edge::End
                },
                to: tick(&mut rng),
            },
            3 if !ids.is_empty() => arrangement::Command::Split {
                placement: pick(&mut rng),
                at: Tick(rng.next(30_000) as i64),
            },
            4 if ids.len() > 1 => arrangement::Command::Delete {
                placements: vec![pick(&mut rng)],
            },
            5 if !ids.is_empty() => arrangement::Command::Duplicate {
                placements: vec![pick(&mut rng)],
                linked: rng.next(2) == 0,
                by: Some(Tick(rng.next(12_000) as i64 - 3000)),
            },
            _ if !ids.is_empty() => {
                // A group of two.
                let (p, q) = (pick(&mut rng), pick(&mut rng));
                if p == q {
                    continue;
                }
                arrangement::Command::Move {
                    placements: vec![p, q],
                    by: Tick(rng.next(6000) as i64 - 1000),
                }
            }
            _ => continue,
        };
        // Seed the overlap-free start: the first edits resolve the fixture's own state.
        let before = state.clone();
        let outcome = match run(&a(command.clone()), &mut state, &mut ctx) {
            Ok(outcome) => outcome,
            Err(_) => {
                assert_eq!(state, before, "step {step}: a refusal changes nothing");
                continue;
            }
        };
        state
            .project
            .validate()
            .unwrap_or_else(|v| panic!("step {step} {command:?}: {v:?}"));
        assert!(overlap_free(&state, &f.track), "step {step} {command:?}");
        if let Undo::Inverse(inverse) = outcome.undo {
            let mut undone = state.clone();
            inverse.apply(&mut undone, &mut ctx).unwrap();
            assert_eq!(undone.project, before.project, "step {step} {command:?}");
        } else {
            assert_eq!(state.project, before.project);
        }
    }
}

#[test]
fn a_project_edited_by_the_commands_survives_the_folder_format() {
    use ::project::file::{MemFs, OpenOptions, SaveOptions, SaveState, open, save};
    let f = fx();
    let mut state = f.state.clone();
    ok(
        &mut state,
        arrangement::Command::Split {
            placement: f.a.clone(),
            at: Tick(2500),
        },
    );
    ok(
        &mut state,
        arrangement::Command::Duplicate {
            placements: vec![f.b.clone(), f.c.clone()],
            linked: false,
            by: Some(Tick(30_000)),
        },
    );
    ok(&mut state, place(&f, &f.looped, 3000, 1000));
    // Opening stores a colour on a track that has none (#479).
    state.project.color_uncolored_tracks();
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &state.project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.project, state.project);
}
