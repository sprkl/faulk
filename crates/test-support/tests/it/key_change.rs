//! Key changes (#953) through the command layer and the engine: Lattice puts a clip's notes on
//! the key in force where they play, from the very first note of the bar the change starts, the
//! same natively and in WASM; a project opened with key changes follows them from the first
//! block, played live at the playhead too.

use commands::testing::{context, fixture};
use commands::{Command, Output, arrangement, clip, device, live, project as p, transport};
use project::{DeviceId, Note, ScaleKind, Tick, TrackId};
use test_support::{Render, Session, parity};

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

/// A session on the fixture's synth track, in C major with a change to D major at bar 2, Lattice
/// before the synth at its defaults (the project's scale, the nearest note).
fn session() -> (Session, DeviceId, TrackId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    s.run(Command::Project(p::Command::SetScale {
        root: 0,
        mode: ScaleKind::Major,
    }));
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.lattice".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    set(&mut s, &f.device, "release", 0.2);
    set(&mut s, &f.device, "gain", 0.5);
    s.run(Command::Arrangement(arrangement::Command::SetKeyChange {
        tick: Tick(3840),
        root: 2,
        mode: ScaleKind::Major,
    }));
    (s, fx, f.track)
}

fn hz(key: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(key) - 69.0) / 12.0)
}

fn level_at(r: &Render, window: std::ops::Range<f32>, key: u8) -> f32 {
    let f = hz(key);
    r.spectrum(window).peak_in(f * 0.97..f * 1.03)
}

/// C#4 at bar 1 (C major) plays C4, the same C#4 on the downbeat of bar 2 (D major, where C# is a
/// note of the scale) plays C#4.
#[test]
fn a_key_change_on_a_downbeat_applies_to_the_first_note_of_the_bar_on_both_targets() {
    let (mut s, _, track) = session();
    let Output::Clip(clip::Output::Create { clip: c, .. }) =
        s.exec(Command::Clip(clip::Command::Create {
            track: track.clone(),
            at: clip::ClipTarget::Arrangement { start: Tick(0) },
            length: Some(Tick(7680)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    let note = |start: i64| Note {
        start: Tick(start),
        pitch: 61,
        length: Tick(1920),
        velocity: 110,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    };
    s.run(Command::Clip(clip::Command::NotesAdd {
        clip: c,
        notes: vec![note(0), note(3840)],
    }));
    s.run(Command::Transport(transport::Command::Seek {
        tick: Tick(0),
    }));
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(4.5);
    let r = parity(s.scenario());
    r.assert_clean();
    let (first, second) = (0.1..0.9, 2.1..2.9);
    assert!(level_at(&r, first.clone(), 60) > 10.0 * level_at(&r, first, 61));
    assert!(level_at(&r, second.clone(), 61) > 10.0 * level_at(&r, second, 60));
}

/// A project opened with key changes (a `Reset` then the snapshot), a note played live with the
/// playhead in the second bar: the key at the playhead.
#[test]
fn a_project_opened_with_key_changes_follows_them_for_a_live_note() {
    let (s0, _, track) = session();
    // Rebuild a session from the state the commands produced: the engine gets the snapshot.
    let mut s = Session::from_state(s0.state().clone(), context());
    s.run(Command::Transport(transport::Command::Seek {
        tick: Tick(3840),
    }));
    s.run(Command::Transport(transport::Command::Play {}));
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note: 61,
        velocity: 100,
    }));
    s.render_secs(0.4);
    s.run(Command::Live(live::Command::NoteOff { track, note: 61 }));
    s.render_secs(0.6);
    let r = parity(s.scenario());
    r.assert_clean();
    let w = 0.05..0.35;
    assert!(level_at(&r, w.clone(), 61) > 10.0 * level_at(&r, w, 60));
}
