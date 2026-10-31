//! AM-1: the automation commands. A lane is validated against the device's descriptor or the
//! mixer, values are rounded to the declared step, a lane holds one point per tick, a batch
//! is one history entry, undo gives the very points back, a device or a track takes its
//! lanes with it and brings them back, a dense lane goes to a blob and is read back as the
//! same table, and the engine follows.

use ::project::file::{FileSystem, MemFs, OpenOptions, SaveOptions, SaveState, open, save};
use ::project::{
    AutomationLane, AutomationPoint, AutomationTarget, DeviceId, LaneId, MixerParam, Shape, Tick,
    TrackId,
};
use commands::engine_msgs::{Synced, resync, snapshot};
use commands::testing::{check, check_refusal, context};
use commands::{
    Command, CommandError, Context, ControlMsg, Output, ProjectEvent, State, Undo, automation,
    device, history, run, track,
};
use std::collections::BTreeMap;

mod automation_examples;
mod clip_examples;
mod device_examples;

use automation_examples::{
    a, add, create, device_target, fx, gain_target, more_context, pan_target, pt,
};
use device_examples::d;

fn refused(command: &Command, state: &State) -> CommandError {
    check_refusal(command, state, &context()).unwrap()
}

fn code(command: &Command, state: &State) -> &'static str {
    refused(command, state).code()
}

fn undo() -> Command {
    Command::History(history::Command::Undo {})
}

fn redo() -> Command {
    Command::History(history::Command::Redo {})
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Runs a command through the history, with ids of its own.
fn go(state: &mut State, command: &Command) -> commands::Outcome {
    let mut ctx = Context::new(
        commands::Author::User,
        ::project::IdGenerator::new(
            1_700_000_100_000 + NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            0x00DE_0000_0000_0000_0000,
        ),
    );
    run(command, state, &mut ctx).unwrap()
}

/// The events of an outcome without the history's own.
fn events(outcome: &commands::Outcome) -> Vec<ProjectEvent> {
    outcome
        .events
        .iter()
        .filter(|e| !matches!(e, ProjectEvent::HistoryChanged { .. }))
        .cloned()
        .collect()
}

fn lane<'a>(state: &'a State, track: &TrackId, lane: &LaneId) -> &'a AutomationLane {
    &state.project.tracks[track].automation[lane]
}

fn values(state: &State, track: &TrackId, id: &LaneId) -> Vec<(i64, f64, Shape)> {
    lane(state, track, id)
        .points
        .iter()
        .map(|(t, p)| (t.get(), p.value, p.shape))
        .collect()
}

fn add_cmd(lane: &LaneId, points: Vec<automation::Point>) -> Command {
    a(automation::Command::PointsAdd {
        lane: lane.clone(),
        points,
    })
}

fn create_cmd(track: &TrackId, target: AutomationTarget) -> Command {
    a(automation::Command::LaneCreate {
        track: Some(track.clone()),
        master: false,
        target,
    })
}

fn linear(tick: i64, value: f64) -> automation::Point {
    pt(tick, value, Shape::Linear)
}

// ---------------------------------------------------------------- lanes

#[test]
fn a_lane_is_created_on_a_parameter_the_track_has() {
    let f = fx();
    let c = &f.chain;
    let mut s = f.state.clone();
    // Pan, a parameter of an effect (a stepped one), both with the contract's checks.
    check(&create_cmd(&f.track, pan_target()), &s, &context()).unwrap();
    check(
        &create_cmd(&f.track, device_target(&c.b, "mute")),
        &s,
        &context(),
    )
    .unwrap();
    let out = go(&mut s, &create_cmd(&f.track, pan_target()));
    let created = events(&out);
    let Output::Automation(automation::Output::LaneCreate { lane: id }) = out.output else {
        panic!()
    };
    assert!(lane(&s, &f.track, &id).points.is_empty());
    assert_eq!(
        created,
        [ProjectEvent::AutomationLaneCreated {
            track: Some(f.track.clone()),
            lane: id.clone()
        }]
    );
    s.project.validate().unwrap();
    // Undo deletes it, redo brings the very lane back.
    go(&mut s, &undo());
    assert!(!s.project.tracks[&f.track].automation.contains_key(&id));
    go(&mut s, &redo());
    assert!(s.project.tracks[&f.track].automation.contains_key(&id));
}

#[test]
fn a_target_is_checked_against_the_track_and_the_descriptor() {
    let f = fx();
    let c = &f.chain;
    let missing = TrackId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let code_of = |track: &TrackId, target| code(&create_cmd(track, target), &f.state);
    // The track, the device and the parameter must exist.
    assert_eq!(code_of(&missing, pan_target()), "not_found");
    assert_eq!(
        code_of(
            &f.track,
            device_target(
                &DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
                "gain"
            )
        ),
        "not_found"
    );
    assert_eq!(code_of(&f.track, device_target(&c.b, "nope")), "not_found");
    // A device of another track is not this track's.
    assert_eq!(
        code_of(&c.audio_track, device_target(&c.b, "gain")),
        "not_found"
    );
    // The clip player has no parameter; a master device is not in a track's chain.
    assert_eq!(
        code_of(&c.audio_track, device_target(&c.player, "gain")),
        "not_found"
    );
    assert_eq!(code_of(&f.track, device_target(&c.ma, "gain")), "not_found");
    // One lane per parameter and track.
    assert_eq!(code_of(&f.track, gain_target()), "invalid");
    assert_eq!(code_of(&f.track, device_target(&c.a, "gain")), "invalid");
    // The same parameter of another track is another lane.
    let mut s = f.state.clone();
    go(&mut s, &create_cmd(&c.audio_track, gain_target()));
    s.project.validate().unwrap();
}

#[test]
fn deleting_a_lane_and_undoing_it_gives_it_back_whole() {
    let f = fx();
    let delete = a(automation::Command::LaneDelete {
        lane: f.gain_lane.clone(),
    });
    check(&delete, &f.state, &context()).unwrap();
    let mut s = f.state.clone();
    let out = go(&mut s, &delete);
    assert!(
        !s.project.tracks[&f.track]
            .automation
            .contains_key(&f.gain_lane)
    );
    assert!(matches!(
        events(&out).as_slice(),
        [ProjectEvent::AutomationLaneDeleted { .. }]
    ));
    go(&mut s, &undo());
    assert_eq!(s.project, f.state.project);
    go(&mut s, &redo());
    assert!(
        !s.project.tracks[&f.track]
            .automation
            .contains_key(&f.gain_lane)
    );
    // An unknown lane.
    let none = LaneId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(
        code(&a(automation::Command::LaneDelete { lane: none }), &f.state),
        "not_found"
    );
}

#[test]
fn a_lane_restore_is_refused_when_it_does_not_fit() {
    let f = fx();
    let snapshot = lane(&f.state, &f.track, &f.gain_lane).clone();
    let fresh = LaneId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let restore = |track: &TrackId, id: &LaneId, snapshot: AutomationLane| {
        a(automation::Command::LaneRestore {
            track: Some(track.clone()),
            master: false,
            lane: id.clone(),
            snapshot,
        })
    };
    // The identifier is in use.
    assert_eq!(
        code(&restore(&f.track, &f.gain_lane, snapshot.clone()), &f.state),
        "invalid"
    );
    // A lane on the same parameter is fine: a restore gives back what was there, even after
    // a merge that left two lanes on one parameter.
    let mut s = f.state.clone();
    go(&mut s, &restore(&f.track, &fresh, snapshot.clone()));
    s.project.validate().unwrap();
    // The track, the device and the parameter must exist.
    let missing = TrackId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAW").unwrap();
    assert_eq!(
        code(&restore(&missing, &fresh, snapshot.clone()), &f.state),
        "not_found"
    );
    let mut bad = snapshot.clone();
    bad.target = device_target(&f.chain.player, "gain");
    assert_eq!(code(&restore(&f.track, &fresh, bad), &f.state), "not_found");
    // A device parameter keeps its range: the effect's `gain` is 0 to 2.
    let mut high = lane(&f.state, &f.track, &f.effect_lane).clone();
    high.points.insert(
        Tick(5),
        AutomationPoint {
            value: 9.0,
            shape: Shape::Step,
        },
    );
    assert_eq!(code(&restore(&f.track, &fresh, high), &f.state), "invalid");
    // A point the project would refuse leaves the state untouched.
    for (value, tick) in [(99.0, 0), (f64::NAN, 0), (0.0, -1), (0.0, (1 << 40) + 1)] {
        let mut bad = snapshot.clone();
        bad.points.insert(
            Tick(tick),
            AutomationPoint {
                value,
                shape: Shape::Step,
            },
        );
        assert_eq!(
            code(&restore(&f.track, &fresh, bad), &f.state),
            "invalid",
            "{value} at {tick}"
        );
    }
}

// ---------------------------------------------------------------- points

#[test]
fn a_value_is_rounded_to_the_step_of_its_parameter() {
    let f = fx();
    let c = &f.chain;
    let mut s = f.state.clone();
    let mut ctx = more_context();
    // The test effect: `mute` is 0 or 1 (step 1), `gain` 0 to 2 (continuous).
    let mute = create(&mut s, &mut ctx, &f.track, device_target(&c.b, "mute"));
    add(
        &mut s,
        &mut ctx,
        &mute,
        vec![
            linear(0, 0.6),
            linear(1, 0.4),
            linear(2, 7.0),
            linear(3, -3.0),
        ],
    );
    assert_eq!(
        values(&s, &f.track, &mute),
        [
            (0, 1.0, Shape::Linear),
            (1, 0.0, Shape::Linear),
            (2, 1.0, Shape::Linear),
            (3, 0.0, Shape::Linear)
        ]
    );
    // A continuous parameter is clamped to its range.
    add(&mut s, &mut ctx, &f.effect_lane, vec![linear(5, 9.0)]);
    assert_eq!(
        lane(&s, &f.track, &f.effect_lane).points[&Tick(5)].value,
        2.0
    );
    // Gain: 0.01 dB; the value is the nearest decimal, never `-5.000000000000001`.
    add(
        &mut s,
        &mut ctx,
        &f.gain_lane,
        vec![
            linear(10, -5.004),
            linear(11, -5.006),
            linear(12, -0.0),
            linear(13, 0.1 + 0.2),
        ],
    );
    let g = lane(&s, &f.track, &f.gain_lane);
    assert_eq!(g.points[&Tick(10)].value, -5.0);
    assert_eq!(g.points[&Tick(11)].value, -5.01);
    assert_eq!(g.points[&Tick(12)].value.to_bits(), 0.0f64.to_bits());
    assert_eq!(g.points[&Tick(13)].value, 0.3);
    // Pan: 0.01, within -1..1.
    let pan = create(&mut s, &mut ctx, &f.track, pan_target());
    add(
        &mut s,
        &mut ctx,
        &pan,
        vec![linear(0, 0.333), linear(1, -1.0)],
    );
    assert_eq!(values(&s, &f.track, &pan)[0].1, 0.33);
    s.project.validate().unwrap();
}

#[test]
fn a_gain_or_a_pan_outside_its_range_a_tick_outside_the_timeline_and_a_non_number_are_refused() {
    let f = fx();
    let gain = |tick, value| add_cmd(&f.gain_lane, vec![linear(tick, value)]);
    for value in [6.01, -96.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(code(&gain(5, value), &f.state), "invalid", "{value}");
    }
    // The bounds themselves are fine.
    check(&gain(5, 6.0), &f.state, &context()).unwrap();
    check(&gain(5, -96.0), &f.state, &context()).unwrap();
    for tick in [-1, (1 << 40) + 1, i64::MIN, i64::MAX] {
        assert_eq!(code(&gain(tick, 0.0), &f.state), "invalid", "{tick}");
    }
    check(&gain(1 << 40, 0.0), &f.state, &context()).unwrap();
    // A non-number for a device parameter, a pan out of range.
    assert_eq!(
        code(
            &add_cmd(&f.effect_lane, vec![linear(5, f64::NAN)]),
            &f.state
        ),
        "invalid"
    );
    let mut s = f.state.clone();
    let mut ctx = more_context();
    let pan = create(&mut s, &mut ctx, &f.track, pan_target());
    assert_eq!(code(&add_cmd(&pan, vec![linear(0, 1.01)]), &s), "invalid");
    // All or nothing: a good point before a bad one changes nothing (check_refusal also
    // compares the whole state).
    assert_eq!(
        code(
            &add_cmd(&f.gain_lane, vec![linear(5, 0.0), linear(6, 99.0)]),
            &f.state
        ),
        "invalid"
    );
}

#[test]
fn a_lane_holds_one_point_per_tick() {
    let f = fx();
    // Two points on one tick in a batch, even equal ones.
    assert_eq!(
        code(
            &add_cmd(&f.gain_lane, vec![linear(5, 0.0), linear(5, 0.0)]),
            &f.state
        ),
        "invalid"
    );
    // A tick that holds a different point: `points.update` changes it.
    let e = refused(&add_cmd(&f.gain_lane, vec![linear(960, -1.0)]), &f.state);
    assert_eq!(e.code(), "invalid");
    assert!(e.to_string().contains("automation.points.update"), "{e}");
    // The same point is already there: nothing changes, no event, no history entry.
    let mut s = f.state.clone();
    let before = s.history.undo.len();
    let out = go(
        &mut s,
        &add_cmd(&f.gain_lane, vec![pt(960, 0.0, Shape::Step)]),
    );
    assert_eq!(
        out.output,
        Output::Automation(automation::Output::PointsAdd { added: 0 })
    );
    assert!(events(&out).is_empty());
    assert_eq!(s.history.undo.len(), before);
    assert_eq!(s.project, f.state.project);
    // A mix of new and existing points adds the new ones only, and its undo removes only those.
    let out = go(
        &mut s,
        &add_cmd(
            &f.gain_lane,
            vec![pt(960, 0.0, Shape::Step), pt(2000, -1.0, Shape::Step)],
        ),
    );
    assert_eq!(
        out.output,
        Output::Automation(automation::Output::PointsAdd { added: 1 })
    );
    go(&mut s, &undo());
    assert_eq!(s.project, f.state.project);
}

#[test]
fn a_batch_is_one_history_entry_whatever_its_size() {
    let f = fx();
    let mut s = f.state.clone();
    let entries = s.history.undo.len();
    let points: Vec<_> = (0..2000)
        .map(|i| linear(10_000 + i * 10, -(i as f64 % 90.0)))
        .collect();
    go(&mut s, &add_cmd(&f.gain_lane, points));
    assert_eq!(s.history.undo.len(), entries + 1);
    assert_eq!(lane(&s, &f.track, &f.gain_lane).points.len(), 2003);
    let edited = s.project.clone();
    go(&mut s, &undo());
    assert_eq!(s.project, f.state.project);
    go(&mut s, &redo());
    assert_eq!(s.project, edited);
    // The same for remove, update and clear.
    let ticks: Vec<Tick> = (0..2000).map(|i| Tick(10_000 + i * 10)).collect();
    let n = s.history.undo.len();
    go(
        &mut s,
        &a(automation::Command::PointsRemove {
            lane: f.gain_lane.clone(),
            ticks: ticks.clone(),
        }),
    );
    assert_eq!(s.history.undo.len(), n + 1);
    go(&mut s, &undo());
    assert_eq!(s.project, edited);
    let update: Vec<_> = (0..2000)
        .map(|i| pt(10_000 + i * 10, -1.0, Shape::Step))
        .collect();
    go(
        &mut s,
        &a(automation::Command::PointsUpdate {
            lane: f.gain_lane.clone(),
            points: update,
        }),
    );
    assert_eq!(s.history.undo.len(), n + 1);
    go(&mut s, &undo());
    assert_eq!(s.project, edited);
    go(
        &mut s,
        &a(automation::Command::LaneClear {
            lane: f.gain_lane.clone(),
        }),
    );
    assert_eq!(s.history.undo.len(), n + 1);
    assert!(lane(&s, &f.track, &f.gain_lane).points.is_empty());
    go(&mut s, &undo());
    assert_eq!(s.project, edited);
}

#[test]
fn remove_ignores_absent_ticks_and_update_needs_present_ones() {
    let f = fx();
    let mut s = f.state.clone();
    let out = go(
        &mut s,
        &a(automation::Command::PointsRemove {
            lane: f.gain_lane.clone(),
            ticks: vec![Tick(5), Tick(960), Tick(960), Tick(-4)],
        }),
    );
    assert_eq!(
        out.output,
        Output::Automation(automation::Output::PointsRemove { removed: 1 })
    );
    assert_eq!(lane(&s, &f.track, &f.gain_lane).points.len(), 2);
    // Nothing to remove: no event, no entry.
    let n = s.history.undo.len();
    let out = go(
        &mut s,
        &a(automation::Command::PointsRemove {
            lane: f.gain_lane.clone(),
            ticks: vec![Tick(7)],
        }),
    );
    assert!(events(&out).is_empty());
    assert_eq!(s.history.undo.len(), n);
    // Update: a tick with no point, a tick twice.
    let update = |points| {
        a(automation::Command::PointsUpdate {
            lane: f.gain_lane.clone(),
            points,
        })
    };
    assert_eq!(
        code(&update(vec![linear(960, 0.0), linear(7, 0.0)]), &f.state),
        "not_found"
    );
    assert_eq!(
        code(&update(vec![linear(960, 0.0), linear(960, 1.0)]), &f.state),
        "invalid"
    );
    // The shape changes with the value; a point that ends equal is skipped.
    let out = go(
        &mut s,
        &update(vec![linear(0, -6.0), pt(1920, -12.0, Shape::Step)]),
    );
    assert_eq!(
        out.output,
        Output::Automation(automation::Output::PointsUpdate { updated: 1 })
    );
    assert_eq!(
        lane(&s, &f.track, &f.gain_lane).points[&Tick(1920)].shape,
        Shape::Step
    );
    // The position cannot change: update names the tick it addresses.
    check(
        &update(vec![pt(960, -1.0, Shape::Linear)]),
        &f.state,
        &context(),
    )
    .unwrap();
    // An unknown lane.
    let none = LaneId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(
        code(&add_cmd(&none, vec![linear(0, 0.0)]), &f.state),
        "not_found"
    );
}

#[test]
fn clearing_an_empty_lane_changes_nothing() {
    let f = fx();
    let mut s = f.state.clone();
    let mut ctx = more_context();
    let empty = create(&mut s, &mut ctx, &f.track, pan_target());
    let n = s.history.undo.len();
    let out = go(&mut s, &a(automation::Command::LaneClear { lane: empty }));
    assert!(events(&out).is_empty());
    assert!(matches!(out.undo, Undo::None));
    assert_eq!(s.history.undo.len(), n);
}

#[test]
fn undo_gives_back_the_very_points_even_when_they_are_off_the_step() {
    // A lane read from a file or merged may hold values the commands would round (a hand
    // edit, a descriptor that changed): remove, update and clear must undo to the same bytes.
    let f = fx();
    let mut s = f.state.clone();
    // `mute` has a step of 1: 0.3 is off it.
    let mute = create(
        &mut s,
        &mut more_context(),
        &f.track,
        device_target(&f.chain.b, "mute"),
    );
    {
        let l = s
            .project
            .tracks
            .get_mut(&f.track)
            .unwrap()
            .automation
            .get_mut(&f.gain_lane)
            .unwrap();
        l.points.insert(
            Tick(100),
            AutomationPoint {
                value: -3.333,
                shape: Shape::Linear,
            },
        );
        let e = s.project.tracks.get_mut(&f.track).unwrap();
        e.automation.get_mut(&mute).unwrap().points.insert(
            Tick(100),
            AutomationPoint {
                value: 0.3,
                shape: Shape::Step,
            },
        );
    }
    let before = s.project.clone();
    let commands = [
        a(automation::Command::LaneClear {
            lane: f.gain_lane.clone(),
        }),
        a(automation::Command::PointsRemove {
            lane: f.gain_lane.clone(),
            ticks: vec![Tick(100)],
        }),
        a(automation::Command::PointsUpdate {
            lane: f.gain_lane.clone(),
            points: vec![linear(100, -1.0)],
        }),
        a(automation::Command::PointsUpdate {
            lane: mute.clone(),
            points: vec![linear(100, 1.0)],
        }),
        a(automation::Command::LaneDelete { lane: mute.clone() }),
    ];
    for command in commands {
        go(&mut s, &command);
        assert_ne!(s.project, before, "{command:?}");
        go(&mut s, &undo());
        assert_eq!(s.project, before, "{command:?}");
        go(&mut s, &redo());
        go(&mut s, &undo());
        assert_eq!(s.project, before, "{command:?} again");
    }
}

#[test]
fn a_points_restore_puts_points_back_as_they_are_and_checks_what_it_must() {
    let f = fx();
    let restore = |points, remove| {
        a(automation::Command::PointsRestore {
            lane: f.gain_lane.clone(),
            points,
            remove,
        })
    };
    // Not rounded, but a tick, a number and a range are still checked.
    let mut s = f.state.clone();
    go(&mut s, &restore(vec![linear(10, -3.333)], vec![]));
    assert_eq!(
        lane(&s, &f.track, &f.gain_lane).points[&Tick(10)].value,
        -3.333
    );
    go(&mut s, &undo());
    assert_eq!(s.project, f.state.project);
    // A device parameter keeps its range too: the effect's `gain` is 0 to 2.
    let out_of_range = a(automation::Command::PointsRestore {
        lane: f.effect_lane.clone(),
        points: vec![linear(10, 9.0)],
        remove: vec![],
    });
    assert_eq!(code(&out_of_range, &f.state), "invalid");
    for bad in [
        restore(vec![linear(10, 99.0)], vec![]),
        restore(vec![linear(10, f64::NAN)], vec![]),
        restore(vec![linear(-1, 0.0)], vec![]),
        restore(vec![linear(10, 0.0), linear(10, 1.0)], vec![]),
        restore(vec![linear(10, 0.0)], vec![Tick(10)]),
        restore(vec![], vec![Tick(960), Tick(960)]),
    ] {
        assert_eq!(code(&bad, &f.state), "invalid", "{bad:?}");
    }
    // Replaces, adds and removes at once; its inverse does the opposite, exactly.
    let mixed = restore(
        vec![linear(960, -1.0), linear(30, -2.0)],
        vec![Tick(1920), Tick(77)],
    );
    check(&mixed, &f.state, &context()).unwrap();
    let out = go(&mut s, &mixed);
    assert_eq!(
        values(&s, &f.track, &f.gain_lane),
        [
            (0, -6.0, Shape::Linear),
            (30, -2.0, Shape::Linear),
            (960, -1.0, Shape::Linear)
        ]
    );
    assert!(events(&out).len() == 1);
    // Nothing changes: no event.
    let n = s.history.undo.len();
    go(&mut s, &restore(vec![linear(30, -2.0)], vec![Tick(1920)]));
    assert_eq!(s.history.undo.len(), n);
}

// ---------------------------------------------------------------- devices and tracks

#[test]
fn removing_a_device_takes_its_lanes_and_undo_brings_them_back() {
    let f = fx();
    let c = &f.chain;
    let remove = d(device::Command::Remove {
        device: c.a.clone(),
    });
    check(&remove, &f.state, &context()).unwrap();
    let mut s = f.state.clone();
    let out = go(&mut s, &remove);
    let tr = &s.project.tracks[&f.track];
    assert!(!tr.automation.contains_key(&f.effect_lane));
    assert!(tr.automation.contains_key(&f.gain_lane), "other lanes stay");
    assert_eq!(
        events(&out),
        [
            ProjectEvent::AutomationLaneDeleted {
                track: Some(f.track.clone()),
                lane: f.effect_lane.clone()
            },
            ProjectEvent::DeviceChainChanged {
                track: Some(f.track.clone())
            }
        ]
    );
    s.project.validate().unwrap();
    go(&mut s, &undo());
    assert_eq!(s.project, f.state.project);
    go(&mut s, &redo());
    assert!(
        !s.project.tracks[&f.track]
            .automation
            .contains_key(&f.effect_lane)
    );
    go(&mut s, &undo());
    assert_eq!(s.project, f.state.project);
    // A device without a lane carries none: the wire form leaves `lanes` out.
    let mut plain = f.state.clone();
    let out = go(
        &mut plain,
        &d(device::Command::Remove {
            device: c.b.clone(),
        }),
    );
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    let json = serde_json::to_value(&*inverse).unwrap();
    assert!(json.get("lanes").is_none(), "{json}");
    // A restore written before lanes existed still reads, without `lanes`.
    let back: Command = serde_json::from_value(json).unwrap();
    assert_eq!(back, *inverse);
}

#[test]
fn a_device_restore_checks_the_lanes_it_brings_and_takes_them_back_out_when_refused() {
    let f = fx();
    let c = &f.chain;
    let mut s = f.state.clone();
    let Undo::Inverse(inverse) = go(
        &mut s,
        &d(device::Command::Remove {
            device: c.a.clone(),
        }),
    )
    .undo
    else {
        panic!()
    };
    let Command::Device(device::Command::Restore {
        track,
        master,
        device,
        snapshot,
        lanes,
    }) = *inverse
    else {
        panic!()
    };
    assert_eq!(lanes.len(), 1);
    let (lane_id, good) = lanes
        .iter()
        .next()
        .map(|(k, v)| (k.clone(), v.clone()))
        .unwrap();
    let restore = |lanes: BTreeMap<LaneId, AutomationLane>| {
        d(device::Command::Restore {
            track: track.clone(),
            master,
            device: device.clone(),
            snapshot: snapshot.clone(),
            lanes,
        })
    };
    // The good one applies (and `check_refusal` compares the state after each refusal below).
    check(&restore(lanes.clone()), &s, &context()).unwrap();
    let with = |l: AutomationLane| restore(BTreeMap::from([(lane_id.clone(), l)]));
    // A lane on another device, on the mixer, on a parameter the device lacks.
    let mut other = good.clone();
    other.target = device_target(&c.b, "gain");
    assert_eq!(code(&with(other), &s), "invalid");
    let mut mixer = good.clone();
    mixer.target = AutomationTarget::Mixer {
        param: MixerParam::Pan,
    };
    assert_eq!(code(&with(mixer), &s), "invalid");
    let mut lacking = good.clone();
    lacking.target = device_target(&c.a, "nope");
    assert_eq!(code(&with(lacking), &s), "invalid");
    // A value the parameter cannot hold (`gain` is 0 to 2) is refused too.
    let mut high = good.clone();
    high.points.insert(
        Tick(5),
        AutomationPoint {
            value: 9.0,
            shape: Shape::Step,
        },
    );
    assert_eq!(code(&with(high), &s), "invalid");
    // A point the project refuses: the device and the lanes go back out.
    let mut nan = good.clone();
    nan.points.insert(
        Tick(5),
        AutomationPoint {
            value: f64::NAN,
            shape: Shape::Step,
        },
    );
    assert_eq!(code(&with(nan), &s), "invalid");
    // A lane identifier in use, or one that is the device's own.
    let mut taken = BTreeMap::new();
    taken.insert(f.gain_lane.clone(), good.clone());
    assert_eq!(code(&restore(taken), &s), "invalid");
    let own = ::project::LaneId(device.0.clone());
    assert_eq!(
        code(&restore(BTreeMap::from([(own, good.clone())])), &s),
        "invalid"
    );
    // The same restore into the master's chain brings the lanes to the master (#378).
    let master_restore = d(device::Command::Restore {
        track: None,
        master: true,
        device: device.clone(),
        snapshot: snapshot.clone(),
        lanes: lanes.clone(),
    });
    let mut in_master = s.clone();
    go(&mut in_master, &master_restore);
    assert_eq!(in_master.project.master_automation.len(), 1);
    assert!(in_master.project.master_devices.contains_key(&device));
    in_master.project.validate().unwrap();
    // Without lanes the same restore is fine.
    check(&restore(BTreeMap::new()), &s, &context()).unwrap();
}

#[test]
fn deleting_a_track_and_undoing_it_keeps_its_lanes() {
    let f = fx();
    let mut s = f.state.clone();
    let delete = Command::Track(track::Command::Delete {
        track: f.track.clone(),
    });
    go(&mut s, &delete);
    assert!(!s.project.tracks.contains_key(&f.track));
    go(&mut s, &undo());
    assert_eq!(s.project, f.state.project);
    assert_eq!(s.project.tracks[&f.track].automation.len(), 2);
}

// ---------------------------------------------------------------- the file

#[test]
fn a_dense_lane_is_a_blob_in_the_folder_and_a_table_when_read_back() {
    let f = fx();
    let mut s = f.state.clone();
    let n: i64 = 100_001;
    let points: Vec<_> = (0..n)
        .map(|i| {
            pt(
                10_000 + i * 7,
                -((i % 9000) as f64) / 100.0,
                if i % 5 == 0 {
                    Shape::Step
                } else {
                    Shape::Linear
                },
            )
        })
        .collect();
    // One batch, one history entry.
    let entries = s.history.undo.len();
    go(&mut s, &add_cmd(&f.gain_lane, points));
    assert_eq!(s.history.undo.len(), entries + 1);
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &s.project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    let header = String::from_utf8(
        fs.read(&format!(
            "tracks/{}/automation/{}.tsv",
            f.track, f.gain_lane
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(header.contains("\tblob="), "{header}");
    assert!(header.contains(&format!("\tpoints={}", n + 3)), "{header}");
    assert_eq!(header.lines().count(), 1, "the points are not listed");
    // The other lane, small, stays text.
    let small = String::from_utf8(
        fs.read(&format!(
            "tracks/{}/automation/{}.tsv",
            f.track, f.effect_lane
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(!small.contains("blob="), "{small}");
    // Read back as the same table, and the commands keep working on it.
    let mut opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.project, s.project);
    let mut reopened = State::new(opened.project.clone());
    go(
        &mut reopened,
        &a(automation::Command::PointsRemove {
            lane: f.gain_lane.clone(),
            ticks: vec![Tick(10_007)],
        }),
    );
    assert_eq!(
        lane(&reopened, &f.track, &f.gain_lane).points.len(),
        (n + 2) as usize
    );
    // Down to a few points, the lane is text again.
    let mut p = opened.project.clone();
    p.tracks
        .get_mut(&f.track)
        .unwrap()
        .automation
        .get_mut(&f.gain_lane)
        .unwrap()
        .points
        .retain(|t, _| t.get() < 70);
    save(
        &mut fs,
        &p,
        &mut opened.state,
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    let text = String::from_utf8(
        fs.read(&format!(
            "tracks/{}/automation/{}.tsv",
            f.track, f.gain_lane
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(!text.contains("blob="), "{text}");
}

// ---------------------------------------------------------------- the engine

/// The lanes an engine holds, replayed from control messages. A lane is identified by its
/// content (its track, its target and its points), not by its slot, since a slot depends on
/// the order lanes came and went.
#[derive(Default, Debug)]
struct Lanes(BTreeMap<u32, String>);

impl Lanes {
    fn apply(&mut self, msgs: &[ControlMsg]) {
        for msg in msgs {
            match msg {
                ControlMsg::Reset { .. } => self.0.clear(),
                ControlMsg::AutomationLaneUpload {
                    lane,
                    track,
                    target,
                    first,
                    total,
                    points,
                } => {
                    assert_eq!((*first, *total as usize), (0, points.len()), "one chunk");
                    self.0
                        .insert(lane.0, format!("{track:?} {target:?} {points:?}"));
                }
                ControlMsg::AutomationLaneRemove { lane } => {
                    assert!(
                        self.0.remove(&lane.0).is_some(),
                        "{msg:?}: the slot is empty"
                    );
                }
                _ => {}
            }
        }
    }

    fn content(&self) -> Vec<&String> {
        let mut all: Vec<_> = self.0.values().collect();
        all.sort();
        all
    }
}

struct World {
    s: Synced,
    lanes: Lanes,
}

impl World {
    fn new(state: State) -> Self {
        let mut ctx = Context::new(
            commands::Author::User,
            ::project::IdGenerator::new(1_690_000_000_000, 0x00AB_0000_0000_0000_0000),
        );
        let mut lanes = Lanes::default();
        lanes.apply(&resync(&state, &mut ctx, 1));
        let mut s = Synced::new(state, ctx, 1);
        s.setup().unwrap();
        World { s, lanes }
    }

    /// Runs a command; the engine's lanes must then be the ones a snapshot of the project gives.
    fn exec(&mut self, command: &Command) -> commands::Outcome {
        let (outcome, _) = self
            .s
            .run(command)
            .unwrap_or_else(|e| panic!("{command:?}: {e:?}"));
        self.lanes.apply(&outcome.control);
        let (msgs, _) = snapshot(&self.s.state.project, &self.s.state.session);
        let mut fresh = Lanes::default();
        fresh.apply(&msgs);
        assert_eq!(
            self.lanes.content(),
            fresh.content(),
            "the engine's lanes drifted from the project after {command:?}"
        );
        outcome
    }
}

#[test]
fn the_engine_holds_the_lanes_of_the_project_through_every_lane_command_and_its_undo() {
    let f = fx();
    let c = &f.chain;
    let mut w = World::new(f.state.clone());
    assert_eq!(
        w.lanes.0.len(),
        2,
        "the snapshot uploads the lanes of the project"
    );
    let mut count = 0;
    let mut step = |w: &mut World, command: Command| {
        w.exec(&command);
        count += 1;
    };
    let new_lane = |w: &World| {
        w.s.state.project.tracks[&f.track]
            .automation
            .keys()
            .find(|k| **k != f.gain_lane && **k != f.effect_lane)
            .cloned()
            .unwrap()
    };
    step(&mut w, create_cmd(&f.track, pan_target()));
    let pan = new_lane(&w);
    for command in [
        add_cmd(&pan, vec![linear(0, 0.5), pt(480, -0.5, Shape::Step)]),
        add_cmd(&f.gain_lane, vec![linear(5000, -1.0)]),
        a(automation::Command::PointsUpdate {
            lane: f.gain_lane.clone(),
            points: vec![pt(960, -2.0, Shape::Linear)],
        }),
        a(automation::Command::PointsRemove {
            lane: f.gain_lane.clone(),
            ticks: vec![Tick(0)],
        }),
        a(automation::Command::LaneClear {
            lane: f.effect_lane.clone(),
        }),
        a(automation::Command::LaneDelete { lane: pan.clone() }),
        // The device a lane targets goes, and comes back by undo.
        d(device::Command::Remove {
            device: c.a.clone(),
        }),
        // A device ahead of the one a lane targets shifts it: the lane follows its slot.
        create_cmd(&f.track, device_target(&c.b, "mute")),
        d(device::Command::Insert {
            track: Some(f.track.clone()),
            master: false,
            kind: device_examples::EFFECT.into(),
            index: 0,
        }),
        Command::Track(track::Command::Delete {
            track: f.track.clone(),
        }),
    ] {
        step(&mut w, command);
    }
    // Undo everything, then redo everything: the engine follows each time.
    for _ in 0..count {
        w.exec(&undo());
    }
    assert_eq!(w.s.state.project, f.state.project);
    for _ in 0..count {
        w.exec(&redo());
    }
}

/// The `ParamSet` messages of a command's engine messages, as (device slot, value).
fn param_sets(outcome: &commands::Outcome) -> Vec<(u16, f32)> {
    outcome
        .control
        .iter()
        .filter_map(|m| match m {
            ControlMsg::ParamSet { device, value, .. } => Some((device.0, *value)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_cleared_or_deleted_device_lane_gives_its_parameter_back_to_the_stored_value() {
    // The engine keeps the last value a lane sent (AM-2): when the lane stops driving the
    // parameter, the project's value is sent again (#394).
    let f = fx();
    let c = &f.chain;
    let mut w = World::new(f.state.clone());
    // The stored value differs from what the lane sent last.
    w.exec(&d(device::Command::SetParam {
        device: c.a.clone(),
        param: "gain".into(),
        value: 0.75,
    }));
    // Clear: the lane leaves the engine (it does nothing without a point), then the stored value follows.
    let out = w.exec(&a(automation::Command::LaneClear {
        lane: f.effect_lane.clone(),
    }));
    assert_eq!(param_sets(&out), vec![(1, 0.75)], "{:?}", out.control);
    // Undo brings the points back and the lane plays again: no ParamSet is needed.
    let out = w.exec(&undo());
    assert!(param_sets(&out).is_empty(), "{:?}", out.control);
    // Delete: the slot is released, then the stored value follows.
    let out = w.exec(&a(automation::Command::LaneDelete {
        lane: f.effect_lane.clone(),
    }));
    assert_eq!(param_sets(&out), vec![(1, 0.75)], "{:?}", out.control);
    assert!(
        !w.s.ctx.engine.lane_targets.contains_key(&f.effect_lane),
        "the cached target is dropped with the lane"
    );
    // A mixer lane is given back by the engine itself: no ParamSet is sent for it.
    let out = w.exec(&a(automation::Command::LaneDelete {
        lane: f.gain_lane.clone(),
    }));
    assert!(param_sets(&out).is_empty(), "{:?}", out.control);
}

#[test]
fn the_stored_value_sent_back_follows_the_slot_of_a_device_that_shifted() {
    let f = fx();
    let c = &f.chain;
    let mut w = World::new(f.state.clone());
    let before = w.s.ctx.engine.track[&f.track].devices.get(&c.a).unwrap() as u16;
    // A device inserted ahead of `a` shifts it by one slot: the lane follows.
    w.exec(&d(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: device_examples::EFFECT.into(),
        index: 0,
    }));
    let out = w.exec(&a(automation::Command::LaneClear {
        lane: f.effect_lane.clone(),
    }));
    let sets = param_sets(&out);
    assert_eq!(sets.len(), 1, "{:?}", out.control);
    let slot_of_a = w.s.ctx.engine.track[&f.track].devices.get(&c.a).unwrap() as u16;
    assert_eq!(slot_of_a, before + 1, "the insert shifted `a`");
    assert_eq!(sets[0].0, slot_of_a);
}

#[test]
fn a_parameter_the_project_does_not_hold_goes_back_to_its_default() {
    let f = fx();
    let c = &f.chain;
    let mut state = f.state.clone();
    // The key is absent from the project: the engine built the device at its default.
    state
        .project
        .tracks
        .get_mut(&f.track)
        .unwrap()
        .devices
        .get_mut(&c.a)
        .unwrap()
        .params
        .remove("gain");
    let mut w = World::new(state);
    let out = w.exec(&a(automation::Command::LaneClear {
        lane: f.effect_lane.clone(),
    }));
    let default = commands::device::registry()
        .iter()
        .filter(|b| b.descriptor.kind == device_examples::EFFECT)
        .flat_map(|b| b.descriptor.params.iter())
        .find(|p| p.key == "gain")
        .map(|p| p.default as f32)
        .unwrap();
    assert_eq!(param_sets(&out), vec![(1, default)], "{:?}", out.control);
}

#[test]
fn removing_a_device_or_a_track_sends_no_stored_value_for_the_lanes_it_takes() {
    // The device is gone from the project and from the engine: there is nothing to restore.
    let f = fx();
    let c = &f.chain;
    let mut w = World::new(f.state.clone());
    let out = w.exec(&d(device::Command::Remove {
        device: c.a.clone(),
    }));
    assert!(param_sets(&out).is_empty(), "{:?}", out.control);
    assert!(!w.s.ctx.engine.lane_targets.contains_key(&f.effect_lane));
    let out = w.exec(&Command::Track(track::Command::Delete {
        track: f.track.clone(),
    }));
    assert!(param_sets(&out).is_empty(), "{:?}", out.control);
    assert!(
        w.s.ctx.engine.lane_targets.is_empty(),
        "{:?}",
        w.s.ctx.engine.lane_targets
    );
}

/// The `AutomationLaneRemove` and `AutomationLaneUpload` messages of a command.
fn lane_traffic(outcome: &commands::Outcome) -> (usize, usize) {
    let count = |f: fn(&ControlMsg) -> bool| outcome.control.iter().filter(|m| f(m)).count();
    (
        count(|m| matches!(m, ControlMsg::AutomationLaneRemove { .. })),
        count(|m| matches!(m, ControlMsg::AutomationLaneUpload { .. })),
    )
}

#[test]
fn a_lane_with_no_point_leaves_the_engine_and_comes_back_whole_with_its_points() {
    // A lane with no point does nothing (AM-1): clearing it takes it out of the engine, like a
    // delete, so the `ParamSet` that restores the stored value holds nothing and the points
    // that come back (undo) play at once (#394).
    let f = fx();
    let mut w = World::new(f.state.clone());
    let out = w.exec(&a(automation::Command::LaneClear {
        lane: f.effect_lane.clone(),
    }));
    assert_eq!(lane_traffic(&out), (1, 0), "{:?}", out.control);
    assert_eq!(w.lanes.0.len(), 1, "only the gain lane is left");
    let out = w.exec(&undo());
    assert_eq!(lane_traffic(&out), (0, 1), "{:?}", out.control);
    assert_eq!(w.lanes.0.len(), 2);
    // A lane created with no point is not uploaded, nor is anything restored for it.
    let out = w.exec(&create_cmd(&f.track, pan_target()));
    assert_eq!(lane_traffic(&out), (0, 0), "{:?}", out.control);
    assert!(param_sets(&out).is_empty(), "{:?}", out.control);
    // A mixer lane that is cleared is dropped as well: the engine gives the strip back to
    // its fader when it forgets the lane.
    let out = w.exec(&a(automation::Command::LaneClear {
        lane: f.gain_lane.clone(),
    }));
    assert_eq!(lane_traffic(&out), (1, 0), "{:?}", out.control);
    assert!(param_sets(&out).is_empty(), "{:?}", out.control);
}

#[test]
fn deleting_a_device_lane_after_a_shift_restores_the_device_that_moved() {
    let f = fx();
    let c = &f.chain;
    let mut w = World::new(f.state.clone());
    let before = w.s.ctx.engine.track[&f.track].devices.get(&c.a).unwrap() as u16;
    w.exec(&d(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: device_examples::EFFECT.into(),
        index: 0,
    }));
    let out = w.exec(&a(automation::Command::LaneDelete {
        lane: f.effect_lane.clone(),
    }));
    let sets = param_sets(&out);
    let slot_of_a = w.s.ctx.engine.track[&f.track].devices.get(&c.a).unwrap() as u16;
    assert_eq!(slot_of_a, before + 1, "the insert shifted `a`");
    assert_eq!(sets.len(), 1, "{:?}", out.control);
    assert_eq!(sets[0].0, slot_of_a);
}

#[test]
fn a_master_device_lane_is_restored_on_the_master() {
    let f = fx();
    let c = &f.chain;
    let mut state = f.state.clone();
    let mut ctx = more_context();
    commands::run(
        &master_create(device_target(&c.mb, "gain")),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let lane = state
        .project
        .master_automation
        .keys()
        .next()
        .cloned()
        .unwrap();
    commands::run(&add_cmd(&lane, vec![linear(7, 0.5)]), &mut state, &mut ctx).unwrap();
    let mut w = World::new(state);
    let out = w.exec(&a(automation::Command::LaneClear { lane }));
    let master_sets: Vec<_> = out
        .control
        .iter()
        .filter_map(|m| match m {
            ControlMsg::ParamSet { track, value, .. } => Some((track.is_master(), *value)),
            _ => None,
        })
        .collect();
    assert_eq!(master_sets.len(), 1, "{:?}", out.control);
    assert!(master_sets[0].0, "{:?}", out.control);
}

#[test]
fn deleting_or_clearing_one_of_two_lanes_on_a_parameter_restores_nothing_while_the_other_plays() {
    // A merged project may hold two lanes on one parameter: both play. The stored value sent
    // back for one would hold the other, so it is not sent.
    let f = fx();
    let mut state = f.state.clone();
    let twin = ::project::LaneId::generate(&mut ::project::IdGenerator::new(
        1_680_000_000_000,
        0x00BB_0000_0000_0000_0000,
    ));
    let copy = state.project.tracks[&f.track].automation[&f.effect_lane].clone();
    state
        .project
        .tracks
        .get_mut(&f.track)
        .unwrap()
        .automation
        .insert(twin.clone(), copy);
    let mut w = World::new(state);
    let out = w.exec(&a(automation::Command::LaneDelete {
        lane: f.effect_lane.clone(),
    }));
    assert!(param_sets(&out).is_empty(), "{:?}", out.control);
    // The last one standing, once deleted, restores the stored value.
    let out = w.exec(&a(automation::Command::LaneDelete { lane: twin }));
    assert_eq!(param_sets(&out).len(), 1, "{:?}", out.control);
    // The same by a clear: the lane that is cleared leaves the engine, the twin keeps playing.
    let mut state = f.state.clone();
    let copy = state.project.tracks[&f.track].automation[&f.effect_lane].clone();
    state
        .project
        .tracks
        .get_mut(&f.track)
        .unwrap()
        .automation
        .insert(
            ::project::LaneId::generate(&mut ::project::IdGenerator::new(
                1_680_000_000_000,
                0x00CC_0000_0000_0000_0000,
            )),
            copy,
        );
    let mut w = World::new(state);
    let out = w.exec(&a(automation::Command::LaneClear {
        lane: f.effect_lane.clone(),
    }));
    assert!(param_sets(&out).is_empty(), "{:?}", out.control);
    assert_eq!(lane_traffic(&out), (1, 0), "{:?}", out.control);
}

#[test]
fn replacing_the_instrument_sends_no_stored_value_for_the_lanes_it_takes() {
    // The new instrument takes the old one's slot: the lanes' deletion is compiled before the
    // chain change, when the slot still names a device that is no longer in the project.
    let f = fx();
    let c = &f.chain;
    let mut state = f.state.clone();
    let mut ctx = more_context();
    let lane = create(
        &mut state,
        &mut ctx,
        &f.track,
        device_target(&c.synth, "gain"),
    );
    add(&mut state, &mut ctx, &lane, vec![pt(7, 0.5, Shape::Linear)]);
    let mut w = World::new(state);
    let out = w.exec(&Command::Track(track::Command::SetInstrument {
        track: f.track.clone(),
        instrument: "faulk.drums".into(),
    }));
    assert!(param_sets(&out).is_empty(), "{:?}", out.control);
    assert!(!w.s.ctx.engine.lane_targets.contains_key(&lane));
}

#[test]
fn a_lane_the_engine_cannot_place_is_not_uploaded_and_a_track_it_does_not_know_gets_nothing() {
    let f = fx();
    let mut w = World::new(f.state.clone());
    // Forget the track's slot, as an engine table that was full would.
    w.s.ctx.engine.tracks.release(&f.track);
    w.s.ctx.engine.track.remove(&f.track);
    let (out, _) =
        w.s.run(&add_cmd(&f.gain_lane, vec![linear(5000, -1.0)]))
            .unwrap();
    assert!(out.control.is_empty(), "{:?}", out.control);
    let (out, _) = w.s.run(&create_cmd(&f.track, pan_target())).unwrap();
    assert!(out.control.is_empty(), "{:?}", out.control);
    let (out, _) =
        w.s.run(&a(automation::Command::LaneDelete {
            lane: f.gain_lane.clone(),
        }))
        .unwrap();
    assert!(
        !out.control
            .iter()
            .any(|m| matches!(m, ControlMsg::AutomationLaneUpload { .. })),
        "{:?}",
        out.control
    );
}

#[test]
fn the_schema_and_the_wire_form_of_the_commands() {
    // `shape` is optional on the wire and defaults to linear; `remove` of a restore too.
    let c: Command = serde_json::from_str(
        r#"{"type":"automation.points.add","lane":"01ARZ3NDEKTSV4RRFFQ69G5FAV","points":[{"tick":0,"value":-3}]}"#,
    )
    .unwrap();
    let Command::Automation(automation::Command::PointsAdd { points, .. }) = c else {
        panic!()
    };
    assert_eq!(points[0].shape, Shape::Linear);
    let c: Command = serde_json::from_str(
        r#"{"type":"automation.lane.create","track":"01ARZ3NDEKTSV4RRFFQ69G5FAV","target":{"mixer":{"param":"gain"}}}"#,
    )
    .unwrap();
    assert!(matches!(
        c,
        Command::Automation(automation::Command::LaneCreate { .. })
    ));
    // An unknown field, an unknown shape, a missing tick.
    for bad in [
        r#"{"type":"automation.points.add","lane":"01ARZ3NDEKTSV4RRFFQ69G5FAV","points":[{"tick":0,"value":1,"x":1}]}"#,
        r#"{"type":"automation.points.add","lane":"01ARZ3NDEKTSV4RRFFQ69G5FAV","points":[{"tick":0,"value":1,"shape":"cubic"}]}"#,
        r#"{"type":"automation.points.add","lane":"01ARZ3NDEKTSV4RRFFQ69G5FAV","points":[{"value":1}]}"#,
        r#"{"type":"automation.lane.clear","lane":"01ARZ3NDEKTSV4RRFFQ69G5FAV","extra":1}"#,
    ] {
        assert!(serde_json::from_str::<Command>(bad).is_err(), "{bad}");
    }
}

// ---------------------------------------------------------------- the master's lanes (#378)

fn master_create(target: AutomationTarget) -> Command {
    a(automation::Command::LaneCreate {
        track: None,
        master: true,
        target,
    })
}

fn master_lane<'a>(state: &'a State, id: &LaneId) -> &'a AutomationLane {
    &state.project.master_automation[id]
}

#[test]
fn a_lane_is_created_on_the_master_volume_pan_and_effects_and_names_its_owner() {
    let f = fx();
    let c = &f.chain;
    let mut s = f.state.clone();
    // Volume, pan, and a parameter of each effect of the master's chain, with the contract's
    // checks (schema, undo, determinism).
    for target in [
        gain_target(),
        pan_target(),
        device_target(&c.ma, "gain"),
        device_target(&c.mb, "mute"),
    ] {
        check(&master_create(target), &s, &context()).unwrap();
    }
    let out = go(&mut s, &master_create(gain_target()));
    let Output::Automation(automation::Output::LaneCreate { lane: id }) = out.output.clone() else {
        panic!()
    };
    assert!(master_lane(&s, &id).points.is_empty());
    assert!(
        s.project
            .tracks
            .values()
            .all(|t| !t.automation.contains_key(&id))
    );
    assert_eq!(
        events(&out),
        [ProjectEvent::AutomationLaneCreated {
            track: None,
            lane: id.clone()
        }]
    );
    s.project.validate().unwrap();
    // One lane per parameter on the master, as on a track; another parameter is fine, and
    // so is the track's own lane on the same parameter.
    assert_eq!(code(&master_create(gain_target()), &s), "invalid");
    check(&master_create(pan_target()), &s, &context()).unwrap();
    check(&create_cmd(&f.track, pan_target()), &s, &context()).unwrap();
    // Undo takes the lane back out, redo puts it back.
    go(&mut s, &undo());
    assert!(s.project.master_automation.is_empty());
    go(&mut s, &redo());
    assert!(s.project.master_automation.contains_key(&id));
    // The owner is named, exactly once.
    let neither = a(automation::Command::LaneCreate {
        track: None,
        master: false,
        target: gain_target(),
    });
    assert_eq!(code(&neither, &s), "invalid");
    let both = a(automation::Command::LaneCreate {
        track: Some(f.track.clone()),
        master: true,
        target: gain_target(),
    });
    assert_eq!(code(&both, &s), "invalid");
    // A device of a track is not in the master's chain, nor a master device in a track's.
    assert_eq!(
        code(&master_create(device_target(&c.a, "gain")), &s),
        "not_found"
    );
    assert_eq!(
        code(&master_create(device_target(&c.ma, "nope")), &s),
        "not_found"
    );
    // The command round-trips through JSON without a `track` for the master.
    let json = serde_json::to_value(master_create(gain_target())).unwrap();
    assert_eq!(json["master"], true);
    assert!(json.get("track").is_none());
    assert_eq!(
        serde_json::from_value::<Command>(json).unwrap(),
        master_create(gain_target())
    );
}

#[test]
fn the_points_of_a_master_lane_are_edited_with_the_same_commands_and_undo_gives_them_back() {
    let f = fx();
    let mut s = f.state.clone();
    let out = go(&mut s, &master_create(gain_target()));
    let Output::Automation(automation::Output::LaneCreate { lane }) = out.output else {
        panic!()
    };
    let steps = [
        add_cmd(
            &lane,
            vec![
                linear(0, 0.0),
                pt(3840, -96.0, Shape::Step),
                linear(1000, -12.0),
            ],
        ),
        a(automation::Command::PointsUpdate {
            lane: lane.clone(),
            points: vec![linear(1000, -20.0)],
        }),
        a(automation::Command::PointsRemove {
            lane: lane.clone(),
            ticks: vec![Tick(3840)],
        }),
        a(automation::Command::LaneClear { lane: lane.clone() }),
    ];
    let mut stack = vec![s.project.clone()];
    for step in &steps {
        let out = go(&mut s, step);
        assert_eq!(
            events(&out),
            [ProjectEvent::AutomationPointsChanged {
                track: None,
                lane: lane.clone()
            }]
        );
        stack.push(s.project.clone());
    }
    for want in stack.iter().rev().skip(1) {
        go(&mut s, &undo());
        assert_eq!(&s.project, want);
    }
    // A value out of the master's range is refused like a track's.
    let high = add_cmd(&lane, vec![linear(5, 7.0)]);
    assert_eq!(code(&high, &s), "invalid");
    // Delete and restore bring back the lane under its identifier, in the master.
    let deleted = go(
        &mut s,
        &a(automation::Command::LaneDelete { lane: lane.clone() }),
    );
    assert_eq!(
        deleted.output,
        Output::Automation(automation::Output::LaneDelete { track: None })
    );
    assert!(s.project.master_automation.is_empty());
    go(&mut s, &undo());
    assert!(s.project.master_automation.contains_key(&lane));
    s.project.validate().unwrap();
}

#[test]
fn removing_a_master_effect_takes_its_lanes_and_undo_brings_them_back() {
    let f = fx();
    let c = &f.chain;
    let mut s = f.state.clone();
    let gain = {
        let out = go(&mut s, &master_create(gain_target()));
        let Output::Automation(automation::Output::LaneCreate { lane }) = out.output else {
            panic!()
        };
        lane
    };
    let out = go(&mut s, &master_create(device_target(&c.ma, "gain")));
    let Output::Automation(automation::Output::LaneCreate { lane: effect }) = out.output else {
        panic!()
    };
    go(
        &mut s,
        &add_cmd(&effect, vec![linear(0, 0.5), pt(960, 1.5, Shape::Step)]),
    );
    let with_lanes = s.project.clone();
    let out = go(
        &mut s,
        &d(device::Command::Remove {
            device: c.ma.clone(),
        }),
    );
    assert_eq!(
        events(&out),
        [
            ProjectEvent::AutomationLaneDeleted {
                track: None,
                lane: effect.clone()
            },
            ProjectEvent::DeviceChainChanged { track: None }
        ]
    );
    assert!(!s.project.master_automation.contains_key(&effect));
    assert!(
        s.project.master_automation.contains_key(&gain),
        "the volume lane stays"
    );
    s.project.validate().unwrap();
    // Undo brings the device and the lane back, points included, with the right events.
    let out = go(&mut s, &undo());
    assert_eq!(s.project, with_lanes);
    assert!(events(&out).contains(&ProjectEvent::AutomationLaneCreated {
        track: None,
        lane: effect.clone()
    }));
    // Redo and undo again: stable.
    go(&mut s, &redo());
    go(&mut s, &undo());
    assert_eq!(s.project, with_lanes);
    // The inverse of the removal is a restore into the master's chain, with the lane in it.
    let mut again = s.clone();
    let Undo::Inverse(inverse) = go(
        &mut again,
        &d(device::Command::Remove {
            device: c.ma.clone(),
        }),
    )
    .undo
    else {
        panic!()
    };
    let Command::Device(device::Command::Restore {
        track,
        master,
        lanes,
        ..
    }) = *inverse
    else {
        panic!()
    };
    assert_eq!((track, master), (None, true));
    assert_eq!(lanes.keys().collect::<Vec<_>>(), [&effect]);
}

#[test]
fn the_engine_holds_the_master_lanes_through_every_lane_command_and_its_undo() {
    let f = fx();
    let c = &f.chain;
    let mut state = f.state.clone();
    // Two master lanes in the project that the snapshot uploads (a lane with no point is not
    // uploaded: it does nothing).
    let mut ctx = more_context();
    for target in [gain_target(), device_target(&c.mb, "mute")] {
        commands::run(&master_create(target), &mut state, &mut ctx).unwrap();
    }
    let created: Vec<LaneId> = state.project.master_automation.keys().cloned().collect();
    for lane in &created {
        commands::run(&add_cmd(lane, vec![linear(7, 0.0)]), &mut state, &mut ctx).unwrap();
    }
    let mut w = World::new(state.clone());
    assert_eq!(
        w.lanes.0.len(),
        4,
        "the snapshot uploads the track's lanes and the master's"
    );
    assert_eq!(w.s.ctx.engine.master_lanes.len(), 2);
    let master_ids: Vec<LaneId> =
        w.s.state
            .project
            .master_automation
            .keys()
            .cloned()
            .collect();
    let [gain, effect] = [master_ids[0].clone(), master_ids[1].clone()];
    let effect = if matches!(
        w.s.state.project.master_automation[&effect].target,
        AutomationTarget::Device { .. }
    ) {
        effect
    } else {
        gain.clone()
    };
    let gain = master_ids
        .iter()
        .find(|id| **id != effect)
        .cloned()
        .unwrap();
    let mut count = 0;
    let mut step = |w: &mut World, command: Command| {
        w.exec(&command);
        count += 1;
    };
    step(&mut w, master_create(pan_target()));
    let pan =
        w.s.state
            .project
            .master_automation
            .keys()
            .find(|k| **k != gain && **k != effect)
            .cloned()
            .unwrap();
    for command in [
        add_cmd(&pan, vec![linear(0, 0.5), pt(480, -0.5, Shape::Step)]),
        add_cmd(&gain, vec![linear(0, -6.0), linear(5000, -96.0)]),
        add_cmd(&effect, vec![pt(0, 1.0, Shape::Step)]),
        a(automation::Command::PointsUpdate {
            lane: gain.clone(),
            points: vec![pt(0, -3.0, Shape::Linear)],
        }),
        a(automation::Command::PointsRemove {
            lane: gain.clone(),
            ticks: vec![Tick(5000)],
        }),
        a(automation::Command::LaneClear {
            lane: effect.clone(),
        }),
        a(automation::Command::LaneDelete { lane: pan.clone() }),
        // A master effect ahead of the one a lane targets shifts it: the lane follows its slot.
        d(device::Command::Insert {
            track: None,
            master: true,
            kind: device_examples::EFFECT.into(),
            index: 0,
        }),
        // The device a lane targets goes, with its lane, and comes back by undo.
        d(device::Command::Remove {
            device: c.mb.clone(),
        }),
        // A track lane next to them: the two rows stay apart.
        create_cmd(&f.track, pan_target()),
    ] {
        step(&mut w, command);
    }
    // The track's lanes and the master's are in different rows of the engine.
    let rows: Vec<bool> =
        w.s.ctx
            .engine
            .master_lanes
            .iter()
            .map(|l| w.s.state.project.master_automation.contains_key(l))
            .collect();
    assert!(rows.iter().all(|r| *r));
    // Undo everything, then redo everything: the engine follows each time.
    for _ in 0..count {
        w.exec(&undo());
    }
    assert_eq!(w.s.state.project, state.project);
    for _ in 0..count {
        w.exec(&redo());
    }
}

#[test]
fn the_master_lanes_count_in_the_safety_limits_of_lanes() {
    let f = fx();
    let mut s = f.state.clone();
    let before = s.project.counts().lanes;
    go(&mut s, &master_create(gain_target()));
    assert_eq!(s.project.counts().lanes, before + 1);
}
