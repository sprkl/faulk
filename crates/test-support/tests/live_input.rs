//! The live input (RE-1) against the engine: a note goes to the selected and the armed
//! tracks, and a note held while the selection or the arming changes still ends.

use commands::{Command, Output, live, track};
use project::TrackId;
use serde_json::{Value, json};
use test_support::{Session, parity};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

fn created(out: Output) -> TrackId {
    match out {
        Output::Track(track::Output::Create { track, .. }) => track,
        other => panic!("not a track creation: {other:?}"),
    }
}

/// Two MIDI tracks with a short release, `a` selected, nothing armed.
fn start() -> (Session, TrackId, TrackId) {
    let mut s = Session::single_synth();
    let a = s.state().project.tracks.keys().next().unwrap().clone();
    let b = created(s.exec(cmd(json!({"type": "track.create", "kind": "midi"}))));
    for t in [&a, &b] {
        let device = s.state().project.tracks[t]
            .devices
            .keys()
            .next()
            .unwrap()
            .clone();
        s.run(cmd(json!({
            "type": "device.set_param", "device": device, "param": "release", "value": 0.05
        })));
    }
    s.run(cmd(json!({"type": "view.select_track", "track": a})));
    (s, a, b)
}

fn input_on(note: u8) -> Command {
    Command::Live(live::Command::InputNoteOn {
        note,
        velocity: 100,
    })
}

fn input_off(note: u8) -> Command {
    Command::Live(live::Command::InputNoteOff { note })
}

#[test]
fn changing_track_during_a_held_note_does_not_stick_it() {
    let (mut s, _a, b) = start();
    s.run(input_on(69));
    s.render_secs(0.3);
    // The selection moves to another track, which is also armed, while the note is held.
    s.run(cmd(json!({"type": "view.select_track", "track": b})));
    s.run(cmd(json!({"type": "track.arm", "track": b, "armed": true})));
    s.render_secs(0.2);
    s.run(input_off(69));
    s.render_secs(0.5);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.1..0.5, 0.05);
    r.assert_silent(0.75..r.seconds());
    assert!(s.ctx().engine.held.is_empty());
}

#[test]
fn a_note_goes_to_the_selected_and_the_armed_tracks() {
    let (mut s, a, b) = start();
    s.run(cmd(json!({"type": "track.arm", "track": b, "armed": true})));
    let Output::Live(live::Output::InputNoteOn { tracks }) = s.exec(input_on(60)) else {
        panic!("not a live input note");
    };
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(tracks, expected);
    s.render_secs(0.3);
    let two = parity(s.scenario());
    s.run(input_off(60));
    s.render_secs(0.4);
    parity(s.scenario()).assert_silent(0.55..0.7);

    // One track alone is quieter than two playing the same note.
    let (mut one, _, _) = start();
    one.run(input_on(60));
    one.render_secs(0.3);
    let single = parity(one.scenario());
    assert!(two.rms_in(0.1..0.3) > single.rms_in(0.1..0.3) * 1.5);
}

#[test]
fn an_unreached_track_hears_nothing_and_a_deleted_one_holds_nothing() {
    let (mut s, _a, b) = start();
    s.run(cmd(json!({"type": "view.select_track", "track": null})));
    s.run(input_on(64));
    s.render_secs(0.2);
    assert!(s.ctx().engine.held.is_empty(), "no target: nothing held");
    s.run(cmd(json!({"type": "track.arm", "track": b, "armed": true})));
    s.run(input_on(64));
    assert_eq!(s.ctx().engine.held.len(), 1);
    s.run(cmd(json!({"type": "track.delete", "track": b})));
    assert!(
        s.ctx().engine.held.is_empty(),
        "the track's notes went with it"
    );
    s.run(input_off(64));
    s.render_secs(0.2);
    parity(s.scenario()).assert_silent(0.0..0.2);
}

#[test]
fn all_notes_off_and_velocity_zero_release_the_held_notes() {
    let (mut s, _a, _b) = start();
    s.run(input_on(60));
    s.run(input_on(64));
    assert_eq!(s.ctx().engine.held.len(), 2);
    s.run(Command::Live(live::Command::InputNoteOn {
        note: 60,
        velocity: 0,
    }));
    assert_eq!(s.ctx().engine.held.len(), 1);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    assert!(s.ctx().engine.held.is_empty());
    s.render_secs(0.4);
    parity(s.scenario()).assert_silent(0.2..0.4);
}
