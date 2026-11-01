//! #396: inserting and deleting time across all tracks: placements (split, move, cut,
//! remove), the points of the tracks' and the master's automation lanes, the loop region,
//! one history entry, exact undo and redo, every refusal, and a random sweep.

mod arrangement_examples;
mod clip_examples;

use ::project::{
    AutomationLane, AutomationPoint, AutomationTarget, IdGenerator, LaneId, LoopRegion, MixerParam,
    Shape, Span, Tick, TickLoop, TrackId,
};
use arrangement_examples::{ArFx, a, fx};
use commands::arrangement::{self, LanePoints, LoopState};
use commands::testing::{check_refusal, context};
use commands::{Command, CommandError, Output, ProjectEvent, State, Undo, history, run};
use std::collections::BTreeMap;

const BAR: i64 = 3840;

/// A loop as `(start, end, enabled)`.
type LoopSeen = (i64, i64, bool);

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

fn insert(at: i64, length: i64) -> Command {
    a(arrangement::Command::InsertTime {
        at: Tick(at),
        length: Tick(length),
    })
}

fn delete(start: i64, length: i64) -> Command {
    a(arrangement::Command::DeleteTime {
        from: Tick(start),
        to: Tick(start + length),
    })
}

fn ok(state: &mut State, command: &Command) -> (arrangement::Output, Vec<ProjectEvent>) {
    let outcome = run(command, state, &mut context()).unwrap();
    state.project.validate().expect("the project stays valid");
    let Output::Arrangement(output) = outcome.output else {
        panic!("not an arrangement output")
    };
    (output, outcome.events)
}

fn lane_points(table: &[(i64, f64)]) -> BTreeMap<Tick, AutomationPoint> {
    table
        .iter()
        .map(|(t, v)| {
            (
                Tick(*t),
                AutomationPoint {
                    value: *v,
                    shape: Shape::Linear,
                },
            )
        })
        .collect()
}

/// Adds a gain lane to a track (`Some`) or the master (`None`).
fn add_lane(state: &mut State, owner: Option<&TrackId>, table: &[(i64, f64)]) -> LaneId {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut ids = IdGenerator::new(
        1_700_000_000_000 + n,
        0x00BB_0000_0000_0000_0000 + u128::from(n),
    );
    let id = LaneId::generate(&mut ids);
    let lane = AutomationLane {
        target: AutomationTarget::Mixer {
            param: MixerParam::Gain,
        },
        points: lane_points(table),
    };
    state
        .project
        .lanes_mut(owner)
        .unwrap()
        .insert(id.clone(), lane);
    state.project.validate().expect("the lane is valid");
    id
}

fn ticks(state: &State, owner: Option<&TrackId>, lane: &LaneId) -> Vec<i64> {
    state.project.lanes(owner).unwrap()[lane]
        .points
        .keys()
        .map(|t| t.get())
        .collect()
}

fn set_loop(state: &mut State, start: i64, end: i64, enabled: bool) {
    state.project.transport.loop_region = LoopRegion::new(Tick(start), Tick(end));
    state.project.transport.loop_enabled = enabled;
}

fn loop_of(state: &State) -> Option<(i64, i64, bool)> {
    state.project.transport.loop_region.map(|r| {
        (
            r.start().get(),
            r.end().get(),
            state.project.transport.loop_enabled,
        )
    })
}

/// The fixture: A `[0, 3840)` and B `[3840, 5760)` of the looped clip (loop 0..1920, length
/// 3840), C `[7680, 11520)` offset 480 of the straight clip (3840, no loop).
fn base() -> ArFx {
    let f = fx();
    assert_eq!(
        spans(&f.state, &f.track),
        [(0, 3840, 0), (3840, 1920, 960), (7680, 3840, 480)]
    );
    f
}

#[test]
fn an_insertion_at_a_boundary_moves_what_starts_there_and_splits_nothing() {
    let f = base();
    let mut s = f.state.clone();
    let (out, _) = ok(&mut s, &insert(3840, 960));
    assert_eq!(
        out,
        arrangement::Output::InsertTime {
            split: 0,
            shifted: 2,
            points_shifted: 0,
            loop_changed: false
        }
    );
    assert_eq!(
        spans(&s, &f.track),
        [(0, 3840, 0), (4800, 1920, 960), (8640, 3840, 480)]
    );
}

#[test]
fn an_insertion_inside_a_placement_splits_it_and_resumes_the_clip_through_its_loop() {
    let f = base();
    let mut s = f.state.clone();
    // Inside the first pass of the loop: the right part resumes at 1000.
    let (out, _) = ok(&mut s, &insert(1000, 960));
    assert!(matches!(
        out,
        arrangement::Output::InsertTime {
            split: 1,
            shifted: 3,
            ..
        }
    ));
    assert_eq!(
        spans(&s, &f.track),
        [
            (0, 1000, 0),
            (1960, 2840, 1000),
            (4800, 1920, 960),
            (8640, 3840, 480)
        ]
    );
    // The two parts are the same clip: linked.
    let clips: Vec<_> = s.project.tracks[&f.track]
        .placements
        .values()
        .map(|p| p.clip.clone())
        .collect();
    assert_eq!(clips.iter().filter(|c| **c == f.looped).count(), 3);
    // Past the loop's end (1920): 2000 ticks in is 80 into the second pass.
    let mut s = f.state.clone();
    ok(&mut s, &insert(2000, 100));
    assert_eq!(spans(&s, &f.track)[..2], [(0, 2000, 0), (2100, 1840, 80)]);
}

#[test]
fn a_clip_without_a_loop_that_ended_leaves_no_right_part() {
    let f = base();
    // C: offset 480, the clip is 3840 long: its content ends 3360 ticks in.
    let mut s = f.state.clone();
    ok(&mut s, &insert(11000, 100));
    assert_eq!(
        spans(&s, &f.track)[2..],
        [(7680, 3320, 480), (11100, 520, 3800)]
    );
    let mut s = f.state.clone();
    ok(&mut s, &insert(11500, 100));
    assert_eq!(spans(&s, &f.track)[2..], [(7680, 3820, 480)]);
}

#[test]
fn an_insertion_at_or_after_the_end_of_everything_changes_nothing() {
    let f = base();
    for at in [11520, 20_000] {
        let mut state = f.state.clone();
        let outcome = run(&insert(at, BAR), &mut state, &mut context()).unwrap();
        assert!(matches!(outcome.undo, Undo::None), "at {at}");
        assert!(outcome.events.is_empty());
        assert_eq!(state, f.state);
    }
}

#[test]
fn a_deletion_cuts_both_edges_removes_the_inside_and_closes_the_gap() {
    let f = base();
    // Inside A only: A is split, the right part resumes at 2000 -> 80.
    let mut s = f.state.clone();
    let (out, _) = ok(&mut s, &delete(1000, 1000));
    assert_eq!(
        out,
        arrangement::Output::DeleteTime {
            removed: 0,
            cut: 1,
            shifted: 3,
            points_removed: 0,
            points_shifted: 0,
            loop_changed: false,
            clips_removed: 0
        }
    );
    assert_eq!(
        spans(&s, &f.track),
        [
            (0, 1000, 0),
            (1000, 1840, 80),
            (2840, 1920, 960),
            (6680, 3840, 480)
        ]
    );
    // From the end of A into B: A shortened, B trimmed at the front (960 + 960).
    let mut s = f.state.clone();
    let (out, _) = ok(&mut s, &delete(3000, 1800));
    assert!(matches!(
        out,
        arrangement::Output::DeleteTime {
            removed: 0,
            cut: 2,
            shifted: 2,
            ..
        }
    ));
    assert_eq!(
        spans(&s, &f.track),
        [(0, 3000, 0), (3000, 960, 0), (5880, 3840, 480)]
    );
    // Exactly B.
    let mut s = f.state.clone();
    let (out, _) = ok(&mut s, &delete(3840, 1920));
    assert!(matches!(
        out,
        arrangement::Output::DeleteTime {
            removed: 1,
            cut: 0,
            shifted: 1,
            clips_removed: 0,
            ..
        }
    ));
    assert_eq!(spans(&s, &f.track), [(0, 3840, 0), (5760, 3840, 480)]);
}

#[test]
fn a_deletion_that_removes_the_last_placement_of_a_clip_removes_the_clip_and_the_undo_returns_it() {
    let f = base();
    let mut state = f.state.clone();
    let mut ctx = context();
    let outcome = run(&delete(7000, 5000), &mut state, &mut ctx).unwrap();
    let Output::Arrangement(arrangement::Output::DeleteTime { clips_removed, .. }) = outcome.output
    else {
        panic!()
    };
    assert_eq!(clips_removed, 1);
    assert!(
        !state.project.tracks[&f.track]
            .clips
            .contains_key(&f.straight)
    );
    let Undo::Inverse(inverse) = outcome.undo else {
        panic!()
    };
    inverse.apply(&mut state, &mut ctx).unwrap();
    assert_eq!(state.project, f.state.project);
}

#[test]
fn automation_points_of_tracks_and_master_follow() {
    let f = base();
    let mut s = f.state.clone();
    let on_track = add_lane(
        &mut s,
        Some(&f.track),
        &[(0, 0.0), (3000, -6.0), (5000, -12.0), (9000, 0.0)],
    );
    let on_master = add_lane(&mut s, None, &[(1000, 0.0), (4000, -3.0)]);
    let before = s.clone();

    let (out, events) = ok(&mut s, &insert(4000, 1000));
    assert!(matches!(
        out,
        arrangement::Output::InsertTime {
            points_shifted: 3,
            ..
        }
    ));
    assert_eq!(
        ticks(&s, Some(&f.track), &on_track),
        [0, 3000, 6000, 10_000]
    );
    assert_eq!(ticks(&s, None, &on_master), [1000, 5000]);
    // Values and shapes travel with the points.
    assert_eq!(
        s.project.tracks[&f.track].automation[&on_track].points[&Tick(6000)].value,
        -12.0
    );
    assert!(events.contains(&ProjectEvent::AutomationPointsChanged {
        track: None,
        lane: on_master.clone()
    }));
    assert!(events.contains(&ProjectEvent::AutomationPointsChanged {
        track: Some(f.track.clone()),
        lane: on_track.clone()
    }));

    // Delete [3000, 6000) from the original: the points inside go, the later ones close up
    // so that the last point before the cut links to the first one after it.
    let mut s = before.clone();
    let (out, _) = ok(&mut s, &delete(3000, 3000));
    assert!(matches!(
        out,
        arrangement::Output::DeleteTime {
            points_removed: 3,
            points_shifted: 1,
            ..
        }
    ));
    assert_eq!(ticks(&s, Some(&f.track), &on_track), [0, 6000]);
    assert_eq!(ticks(&s, None, &on_master), [1000]);
}

#[test]
fn an_insertion_and_a_deletion_move_the_loop_region_and_the_undo_gives_it_back() {
    let f = base();
    let cases: Vec<(Command, Option<LoopSeen>)> = vec![
        // Inside the loop: it grows.
        (insert(5000, 1000), Some((2000, 9000, true))),
        // An end exactly at the point stays.
        (insert(8000, 1000), Some((2000, 8000, true))),
        // A start exactly at the point moves with the end.
        (insert(2000, 1000), Some((3000, 9000, true))),
        // Before the loop: it moves.
        (insert(0, 1000), Some((3000, 9000, true))),
        // Inside: it shrinks.
        (delete(3000, 2000), Some((2000, 6000, true))),
        // A start inside the range falls to its start.
        (delete(1000, 2000), Some((1000, 6000, true))),
        // The loop is swallowed: the region is cleared and the loop is off.
        (delete(1000, 8000), None),
        // After the loop: nothing moves.
        (delete(9000, 1000), Some((2000, 8000, true))),
    ];
    for (command, expected) in cases {
        let mut state = f.state.clone();
        set_loop(&mut state, 2000, 8000, true);
        let before = state.clone();
        let outcome = run(&command, &mut state, &mut context()).unwrap();
        state.project.validate().unwrap();
        assert_eq!(loop_of(&state), expected, "{command:?}");
        if let Undo::Inverse(inverse) = outcome.undo {
            inverse.apply(&mut state, &mut context()).unwrap();
            assert_eq!(state.project, before.project, "undo of {command:?}");
        }
    }
    // A disabled loop stays disabled.
    let mut state = f.state.clone();
    set_loop(&mut state, 2000, 8000, false);
    ok(&mut state, &insert(5000, 1000));
    assert_eq!(loop_of(&state), Some((2000, 9000, false)));
}

#[test]
fn the_loop_change_is_reported_to_the_interface() {
    let f = base();
    let mut state = f.state.clone();
    set_loop(&mut state, 2000, 8000, true);
    let (_, events) = ok(&mut state, &insert(5000, 1000));
    assert!(events.contains(&ProjectEvent::LoopChanged {
        region: Some(TickLoop {
            start: Tick(2000),
            end: Tick(9000)
        }),
        enabled: true
    }));
}

#[test]
fn one_history_entry_undoes_and_redoes_everything_exactly() {
    let f = base();
    let mut state = f.state.clone();
    let lane = add_lane(&mut state, Some(&f.track), &[(0, 0.0), (5000, -6.0)]);
    set_loop(&mut state, 2000, 8000, true);
    let start = state.clone();
    let mut ctx = context();
    for command in [insert(1000, 960), delete(2000, 3000)] {
        let before = state.history.undo.len();
        run(&command, &mut state, &mut ctx).unwrap();
        assert_eq!(state.history.undo.len(), before + 1);
    }
    let end = state.clone();
    assert_ne!(end.project, start.project);
    // Editing time never touches the tempo map nor the time signature.
    assert_eq!(end.project.tempo, start.project.tempo);
    assert_eq!(end.project.meter, start.project.meter);
    for _ in 0..2 {
        run(
            &Command::History(history::Command::Undo {}),
            &mut state,
            &mut ctx,
        )
        .unwrap();
    }
    assert_eq!(state.project, start.project);
    assert_eq!(state.project.tempo, start.project.tempo);
    assert_eq!(state.project.meter, start.project.meter);
    for _ in 0..2 {
        run(
            &Command::History(history::Command::Redo {}),
            &mut state,
            &mut ctx,
        )
        .unwrap();
    }
    assert_eq!(state.project, end.project);
    assert!(
        state.project.tracks[&f.track]
            .automation
            .contains_key(&lane)
    );
}

#[test]
fn what_would_leave_the_timeline_is_refused_and_changes_nothing() {
    let f = base();
    let max = 1i64 << 40;
    for command in [
        insert(0, 0),
        insert(0, -5),
        insert(-1, 10),
        insert(max, 1),
        insert(0, max + 1),
        delete(0, 0),
        delete(-1, 10),
        delete(max - 5, 10),
        delete(i64::MIN, i64::MAX),
        delete(-1, i64::MAX),
    ] {
        let error = check_refusal(&command, &f.state, &context()).unwrap();
        assert!(matches!(error, CommandError::Invalid { .. }), "{command:?}");
    }
    // A placement, a point and the loop that would pass 2^40.
    let mut s = f.state.clone();
    let lane = add_lane(&mut s, None, &[(max - 10, 0.0)]);
    let error = check_refusal(&insert(0, 100), &s, &context()).unwrap();
    assert!(matches!(error, CommandError::Invalid { .. }));
    s.project.master_automation.remove(&lane);
    set_loop(&mut s, 0, max - 10, false);
    let error = check_refusal(&insert(0, 100), &s, &context()).unwrap();
    assert!(matches!(error, CommandError::Invalid { .. }));
    set_loop(&mut s, 0, 100, false);
    let c = s.project.tracks.get_mut(&f.track).unwrap();
    let last = c.placements.values_mut().last().unwrap();
    last.start = Tick(max - 3840);
    last.span = Span::Ticks {
        length: Tick(3840),
        offset: Tick(0),
    };
    // The last placement now touches 2^40: any insertion before it overflows.
    let error = check_refusal(&insert(0, 1), &s, &context()).unwrap();
    assert!(matches!(error, CommandError::Invalid { .. }));
}

#[test]
fn restore_time_refuses_what_is_not_a_valid_restoration() {
    let f = base();
    let mut s = f.state.clone();
    let lane = add_lane(&mut s, Some(&f.track), &[(0, 0.0)]);
    let point = |tick, value| commands::automation::Point {
        tick: Tick(tick),
        value,
        shape: Shape::Linear,
    };
    let lanes = |l: Vec<LanePoints>| {
        a(arrangement::Command::RestoreTime {
            edits: vec![],
            lanes: l,
            transport: None,
        })
    };
    let named = |points: Vec<commands::automation::Point>| LanePoints {
        lane: lane.clone(),
        points,
    };
    let unknown = LanePoints {
        lane: LaneId::generate(&mut IdGenerator::new(1_700_000_000_001, 7)),
        points: vec![],
    };
    for command in [
        lanes(vec![]),
        lanes(vec![unknown]),
        lanes(vec![named(vec![point(0, 0.0)]), named(vec![point(0, 0.0)])]),
        lanes(vec![named(vec![point(0, 100.0)])]),
        lanes(vec![named(vec![point(0, f64::NAN)])]),
        lanes(vec![named(vec![point(-1, 0.0)])]),
        lanes(vec![named(vec![point(5, 0.0), point(5, 1.0)])]),
        a(arrangement::Command::RestoreTime {
            edits: vec![],
            lanes: vec![],
            transport: Some(LoopState {
                loop_region: None,
                loop_enabled: true,
            }),
        }),
        a(arrangement::Command::RestoreTime {
            edits: vec![],
            lanes: vec![],
            transport: Some(LoopState {
                loop_region: Some(TickLoop {
                    start: Tick(10),
                    end: Tick(10),
                }),
                loop_enabled: false,
            }),
        }),
    ] {
        let error = check_refusal(&command, &s, &context()).unwrap();
        assert!(
            matches!(
                error,
                CommandError::Invalid { .. } | CommandError::NotFound { .. }
            ),
            "{command:?}: {error:?}"
        );
    }
}

#[test]
fn a_random_sweep_keeps_the_project_valid_free_of_overlaps_and_undoable() {
    let f = base();
    let mut state = f.state.clone();
    add_lane(
        &mut state,
        Some(&f.track),
        &[(0, 0.0), (3000, -6.0), (9000, -12.0)],
    );
    add_lane(&mut state, None, &[(500, 0.0), (7000, -3.0)]);
    set_loop(&mut state, 1000, 9000, true);
    let start = state.clone();
    let mut ctx = context();
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = |n: i64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as i64
    };
    let mut done = 0;
    for _ in 0..120 {
        let at = next(14_000);
        let length = 1 + next(4000);
        let command = if next(2) == 0 {
            insert(at, length)
        } else {
            delete(at, length)
        };
        let outcome = run(&command, &mut state, &mut ctx).unwrap();
        state.project.validate().unwrap();
        if matches!(outcome.undo, Undo::Inverse(_)) {
            done += 1;
        }
        let spans = spans(&state, &f.track);
        for pair in spans.windows(2) {
            assert!(pair[0].0 + pair[0].1 <= pair[1].0, "overlap in {spans:?}");
        }
    }
    assert!(done > 8, "the sweep changes the project: {done}");
    for _ in 0..done {
        run(
            &Command::History(history::Command::Undo {}),
            &mut state,
            &mut ctx,
        )
        .unwrap();
    }
    assert_eq!(state.project, start.project);
}

#[test]
fn an_audio_placement_anywhere_refuses_the_whole_command_and_nothing_moves() {
    let f = base();
    let mut s = f.state.clone();
    let mut ids = IdGenerator::new(1_700_000_000_500, 0x00DD_0000_0000_0000_0000);
    s.project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .placements
        .insert(
            ::project::PlacementId::generate(&mut ids),
            ::project::Placement {
                clip: f.audio_clip.clone(),
                start: Tick(0),
                span: Span::Frames {
                    length: ::project::Frame::new(44_100),
                    offset: ::project::Frame::new(0),
                },
            },
        );
    s.project.validate().unwrap();
    for command in [insert(100, 960), delete(100, 960)] {
        let error = check_refusal(&command, &s, &context()).unwrap();
        // The refusal names the field and says why, so that an agent or a person knows what to do.
        let CommandError::Invalid { field, message } = &error else {
            panic!("{error:?}")
        };
        assert_eq!(field, "track");
        assert!(message.contains("audio"), "{message}");
        assert!(message.contains("insert or delete time"), "{message}");
    }
}

#[test]
fn the_placement_limit_refuses_with_a_limit_error_and_changes_nothing() {
    let f = base();
    let mut s = f.state.clone();
    let mut ids = IdGenerator::new(1_700_000_000_600, 0x00EE_0000_0000_0000_0000);
    let max = ::project::limits::MAX_PLACEMENTS_PER_TRACK;
    let track = s.project.tracks.get_mut(&f.track).unwrap();
    track.placements.clear();
    // Full to the limit, the first placement straddles tick 2: splitting it needs one more.
    for i in 0..max {
        let start = 10 * i64::try_from(i).unwrap();
        track.placements.insert(
            ::project::PlacementId::generate(&mut ids),
            ::project::Placement {
                clip: f.looped.clone(),
                start: Tick(start),
                span: Span::Ticks {
                    length: Tick(5),
                    offset: Tick(0),
                },
            },
        );
    }
    s.project.validate().unwrap();
    let error = check_refusal(&insert(2, 960), &s, &context()).unwrap();
    assert!(matches!(error, CommandError::Limit { .. }), "{error:?}");
}
