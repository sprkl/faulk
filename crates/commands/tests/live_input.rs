//! RE-1: arming, monitoring and the live input. `track.arm` and `track.set_monitor` are
//! session state (never in the history nor the project); the live input goes to the
//! selected and the armed MIDI tracks, and a note off to where its note on went.

use commands::engine_msgs::{Synced, snapshot};
use commands::{
    Command, CommandError, ControlMsg, Output, ProjectEvent, State, live, testing, track,
};
use engine::msg::TrackSlot;
use project::TrackId;
use serde_json::{Value, json};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

/// The fixture (one synth track, slot 0), synced, with a second MIDI track (slot 1) and an
/// audio track (slot 2).
fn synced() -> (Synced, [TrackId; 3]) {
    let f = testing::fixture();
    let mut s = Synced::new(f.state.clone(), testing::context(), 1);
    s.setup().unwrap();
    let mut make = |kind: &str| match s
        .run(&cmd(json!({"type": "track.create", "kind": kind})))
        .unwrap()
        .0
        .output
    {
        Output::Track(track::Output::Create { track, .. }) => track,
        other => panic!("{other:?}"),
    };
    let b = make("midi");
    let audio = make("audio");
    (s, [f.track, b, audio])
}

fn run(s: &mut Synced, v: Value) -> (Output, Vec<ProjectEvent>, Vec<ControlMsg>) {
    let (outcome, _) = s
        .run(&cmd(v.clone()))
        .unwrap_or_else(|e| panic!("{v}: {e:?}"));
    (outcome.output, outcome.events, outcome.control)
}

fn note_on(slot: u32, pitch: u8) -> ControlMsg {
    ControlMsg::NoteOn {
        track: TrackSlot(slot),
        pitch,
        velocity: 100,
        at: None,
    }
}

fn note_off(slot: u32, pitch: u8) -> ControlMsg {
    ControlMsg::NoteOff {
        track: TrackSlot(slot),
        pitch,
        at: None,
    }
}

#[test]
fn arm_and_monitor_are_session_state_with_their_events_and_engine_messages() {
    let (mut s, [a, ..]) = synced();
    let history = s.state.history.clone();
    let project = s.state.project.clone();
    let (out, events, msgs) = run(
        &mut s,
        json!({"type": "track.arm", "track": a, "armed": true}),
    );
    assert_eq!(out, Output::Track(track::Output::Arm { previous: false }));
    assert_eq!(
        events,
        [ProjectEvent::ArmChanged {
            track: a.clone(),
            armed: true
        }]
    );
    assert_eq!(
        msgs,
        [ControlMsg::TrackArm {
            track: TrackSlot(0),
            armed: true
        }]
    );
    let (_, _, msgs) = run(
        &mut s,
        json!({"type": "track.set_monitor", "track": a, "monitor": true}),
    );
    assert_eq!(
        msgs,
        [ControlMsg::TrackSetMonitor {
            track: TrackSlot(0),
            monitor: true
        }]
    );
    assert!(s.state.session.armed.contains(&a) && s.state.session.monitor.contains(&a));
    assert_eq!(s.state.history, history, "never in the history");
    assert_eq!(s.state.project, project, "never in the project");

    // Setting the same value again is silent and says what it was.
    let (out, events, msgs) = run(
        &mut s,
        json!({"type": "track.arm", "track": a, "armed": true}),
    );
    assert_eq!(out, Output::Track(track::Output::Arm { previous: true }));
    assert!((events.is_empty()) && msgs.is_empty());

    // A snapshot of the state arms the engine the same way (reopening a session).
    let (msgs, _) = snapshot(&s.state.project, &s.state.session);
    assert!(msgs.contains(&ControlMsg::TrackArm {
        track: TrackSlot(0),
        armed: true
    }));
    assert!(msgs.contains(&ControlMsg::TrackSetMonitor {
        track: TrackSlot(0),
        monitor: true
    }));

    let (_, events, _) = run(
        &mut s,
        json!({"type": "track.arm", "track": a, "armed": false}),
    );
    assert_eq!(events.len(), 1);
    assert!(s.state.session.armed.is_empty());
}

#[test]
fn a_missing_track_cannot_be_armed_but_a_stale_arm_can_be_cleared() {
    let (mut s, [a, ..]) = synced();
    let ghost = TrackId::parse("00000000000000000000000FFF").unwrap();
    for v in [
        json!({"type": "track.arm", "track": ghost, "armed": true}),
        json!({"type": "track.arm", "track": ghost, "armed": false}),
        json!({"type": "track.set_monitor", "track": ghost, "monitor": true}),
    ] {
        let err = s.run(&cmd(v)).unwrap_err();
        assert!(
            matches!(
                err,
                commands::engine_msgs::SyncError::Refused(CommandError::NotFound { .. })
            ),
            "{err:?}"
        );
    }
    // A stale id (a session restored against another project) can be cleared, not set.
    let mut stale = s.state.clone();
    stale.session.armed.insert(ghost.clone());
    stale.session.monitor.insert(ghost.clone());
    let mut ctx = testing::context();
    for v in [
        json!({"type": "track.arm", "track": ghost, "armed": false}),
        json!({"type": "track.set_monitor", "track": ghost, "monitor": false}),
    ] {
        commands::run(&cmd(v), &mut stale, &mut ctx).expect("a stale id can be cleared");
    }
    assert!(stale.session.armed.is_empty() && stale.session.monitor.is_empty());
    // Deleting an armed track clears it, and undoing the delete does not re-arm it.
    run(
        &mut s,
        json!({"type": "track.arm", "track": a, "armed": true}),
    );
    run(
        &mut s,
        json!({"type": "track.set_monitor", "track": a, "monitor": true}),
    );
    let (_, events, _) = run(&mut s, json!({"type": "track.delete", "track": a}));
    assert!(events.contains(&ProjectEvent::ArmChanged {
        track: a.clone(),
        armed: false
    }));
    assert!(events.contains(&ProjectEvent::MonitorChanged {
        track: a.clone(),
        monitor: false
    }));
    assert!(s.state.session.armed.is_empty() && s.state.session.monitor.is_empty());
    run(&mut s, json!({"type": "history.undo"}));
    assert!(s.state.session.armed.is_empty());
}

#[test]
fn the_live_input_goes_to_the_selected_and_armed_midi_tracks() {
    let (mut s, [a, b, audio]) = synced();
    // Nothing selected, nothing armed: it plays nowhere.
    let (out, _, msgs) = run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 60, "velocity": 100}),
    );
    assert_eq!(
        out,
        Output::Live(live::Output::InputNoteOn { tracks: vec![] })
    );
    assert!(msgs.is_empty());

    // The selected track plays.
    run(&mut s, json!({"type": "view.select_track", "track": a}));
    let (_, _, msgs) = run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 62, "velocity": 90}),
    );
    assert_eq!(
        msgs,
        [ControlMsg::NoteOn {
            track: TrackSlot(0),
            pitch: 62,
            velocity: 90,
            at: None,
        }]
    );
    run(&mut s, json!({"type": "live.input_note_off", "note": 62}));

    // Armed tracks play too; an armed audio track has no instrument and is skipped.
    run(
        &mut s,
        json!({"type": "track.arm", "track": b, "armed": true}),
    );
    run(
        &mut s,
        json!({"type": "track.arm", "track": audio, "armed": true}),
    );
    let (out, _, msgs) = run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 64, "velocity": 100}),
    );
    let mut expected = vec![a.clone(), b.clone()];
    expected.sort();
    assert_eq!(
        out,
        Output::Live(live::Output::InputNoteOn { tracks: expected })
    );
    assert_eq!(msgs.len(), 2);
    assert!(msgs.contains(&note_on(0, 64)) && msgs.contains(&note_on(1, 64)));
}

#[test]
fn a_note_off_goes_where_its_note_on_went() {
    let (mut s, [a, b, _]) = synced();
    run(&mut s, json!({"type": "view.select_track", "track": a}));
    run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 60, "velocity": 100}),
    );
    // Selection and arming change while the note is held.
    run(&mut s, json!({"type": "view.select_track", "track": b}));
    run(
        &mut s,
        json!({"type": "track.arm", "track": a, "armed": true}),
    );
    run(
        &mut s,
        json!({"type": "track.arm", "track": a, "armed": false}),
    );
    let (out, _, msgs) = run(&mut s, json!({"type": "live.input_note_off", "note": 60}));
    assert_eq!(
        msgs,
        [note_off(0, 60)],
        "the track that played it, not the selected one"
    );
    assert_eq!(
        out,
        Output::Live(live::Output::InputNoteOff {
            tracks: vec![a.clone()]
        })
    );
    // A second off, or an off of a note never played, releases nothing.
    let (_, _, msgs) = run(&mut s, json!({"type": "live.input_note_off", "note": 60}));
    assert!(msgs.is_empty());
    let (_, _, msgs) = run(&mut s, json!({"type": "live.input_note_off", "note": 12}));
    assert!(msgs.is_empty());

    // Velocity 0 is a note off.
    run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 65, "velocity": 100}),
    );
    run(&mut s, json!({"type": "view.select_track", "track": a}));
    let (_, _, msgs) = run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 65, "velocity": 0}),
    );
    assert_eq!(msgs, [note_off(1, 65)]);

    // The same note held on two tracks is released on both.
    run(
        &mut s,
        json!({"type": "track.arm", "track": b, "armed": true}),
    );
    run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 70, "velocity": 100}),
    );
    run(
        &mut s,
        json!({"type": "track.arm", "track": b, "armed": false}),
    );
    let (_, _, msgs) = run(&mut s, json!({"type": "live.input_note_off", "note": 70}));
    assert_eq!(msgs.len(), 2);
    assert!(msgs.contains(&note_off(0, 70)) && msgs.contains(&note_off(1, 70)));
    assert!(s.ctx.engine.held.is_empty());
}

#[test]
fn held_notes_are_forgotten_with_the_track_the_panic_and_the_engine() {
    let (mut s, [a, b, _]) = synced();
    run(&mut s, json!({"type": "view.select_track", "track": b}));
    run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 60, "velocity": 100}),
    );
    run(&mut s, json!({"type": "track.delete", "track": b}));
    assert!(
        s.ctx.engine.held.is_empty(),
        "a deleted track holds nothing"
    );
    let (_, _, msgs) = run(&mut s, json!({"type": "live.input_note_off", "note": 60}));
    assert!(
        msgs.is_empty(),
        "nothing to release on a slot that may be reused"
    );

    run(&mut s, json!({"type": "view.select_track", "track": a}));
    run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 61, "velocity": 100}),
    );
    run(&mut s, json!({"type": "live.all_notes_off"}));
    assert!(s.ctx.engine.held.is_empty(), "the panic forgets them");

    run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 62, "velocity": 100}),
    );
    run(
        &mut s,
        json!({"type": "live.note_off", "track": a, "note": 62}),
    );
    assert!(s.ctx.engine.held.is_empty(), "an explicit note off too");

    // A resync (a new engine) starts with nothing held.
    run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 63, "velocity": 100}),
    );
    s.resync().unwrap();
    assert!(s.ctx.engine.held.is_empty());
}

#[test]
fn control_changes_are_validated_and_reported_for_the_automation() {
    let (mut s, _) = synced();
    let (_, events, msgs) = run(
        &mut s,
        json!({"type": "live.control_change", "controller": 1, "value": 127}),
    );
    assert_eq!(
        events,
        [ProjectEvent::ControlChange {
            controller: 1,
            value: 127
        }]
    );
    assert!(msgs.is_empty(), "no sound by itself");
    for v in [
        json!({"type": "live.control_change", "controller": 128, "value": 0}),
        json!({"type": "live.control_change", "controller": 0, "value": 200}),
        json!({"type": "live.input_note_on", "note": 128, "velocity": 1}),
        json!({"type": "live.input_note_off", "note": 200}),
    ] {
        // The schema's range is a hint: the command itself refuses what it is not.
        let parsed = serde_json::from_value::<Command>(v);
        if let Ok(c) = parsed {
            assert!(s.run(&c).is_err());
        }
    }
}

#[test]
fn the_new_commands_are_live_and_not_undoable() {
    for name in [
        "track.arm",
        "track.set_monitor",
        "live.input_note_on",
        "live.input_note_off",
        "live.control_change",
    ] {
        let tool = commands::catalog()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap();
        assert_eq!(tool.kind, commands::Kind::Live, "{name}");
        assert_eq!(tool.undo, commands::UndoKind::None, "{name}");
        assert!(!tool.description.is_empty() && !tool.destructive);
    }
    let state: State = testing::fixture().state;
    assert!(state.session.armed.is_empty());
}

/// #499: the time a live note was played (`frame`, a host frame) reaches the engine with
/// every message the note becomes, its note off included (velocity 0 or `input_note_off`);
/// without it the messages carry none, and the JSON of an unstamped command has no `frame`.
#[test]
fn the_frame_of_a_live_note_reaches_its_engine_messages() {
    let (mut s, [a, b, _]) = synced();
    run(&mut s, json!({"type": "view.select_track", "track": a}));
    run(
        &mut s,
        json!({"type": "track.arm", "track": b, "armed": true}),
    );
    let stamp = |slot: u32, at: Option<u64>| ControlMsg::NoteOn {
        track: TrackSlot(slot),
        pitch: 60,
        velocity: 90,
        at,
    };
    let (_, _, msgs) = run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 60, "velocity": 90, "frame": 123_456}),
    );
    assert_eq!(msgs, [stamp(0, Some(123_456)), stamp(1, Some(123_456))]);
    let (_, _, msgs) = run(
        &mut s,
        json!({"type": "live.input_note_off", "note": 60, "frame": 130_000}),
    );
    let mut msgs = msgs;
    msgs.sort_by_key(|m| format!("{m:?}"));
    assert_eq!(
        msgs,
        [
            ControlMsg::NoteOff {
                track: TrackSlot(0),
                pitch: 60,
                at: Some(130_000)
            },
            ControlMsg::NoteOff {
                track: TrackSlot(1),
                pitch: 60,
                at: Some(130_000)
            },
        ]
    );
    // A velocity 0 note on is a note off, with its frame.
    run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 62, "velocity": 90}),
    );
    let (_, _, msgs) = run(
        &mut s,
        json!({"type": "live.input_note_on", "note": 62, "velocity": 0, "frame": 140_000}),
    );
    assert!(
        msgs.iter().all(|m| matches!(
            m,
            ControlMsg::NoteOff {
                at: Some(140_000),
                ..
            }
        )),
        "{msgs:?}"
    );
    assert_eq!(msgs.len(), 2);
    // An unstamped command serializes without the field.
    let plain = cmd(json!({"type": "live.input_note_on", "note": 60, "velocity": 90}));
    assert_eq!(
        serde_json::to_value(&plain).unwrap(),
        json!({"type": "live.input_note_on", "note": 60, "velocity": 90})
    );
    // A frame that is not a whole frame is refused by the schema.
    assert!(
        serde_json::from_value::<Command>(
            json!({"type": "live.input_note_off", "note": 60, "frame": -1})
        )
        .is_err()
    );
}
