//! Lattice (#692) through the command layer and the engine: `project.set_scale` and
//! `device.insert` put Lattice before the synth of a track, a chromatic run played on the live
//! input comes out on the notes of the scale, the same natively and in WASM, and is written to
//! `.qa/renders/lattice-chromatic.wav` for the ear. A clip's notes go through it too. A note held
//! while the project's scale changes, or while Lattice is bypassed, removed or set otherwise, and
//! a clip's note through a stop or a jump of the transport, leave nothing sounding. (The device
//! itself is tested in `crates/engine/tests/it/lattice.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, clip, device, live, project as p, transport};
use project::{DeviceId, Note, ScaleKind, Tick, TrackId};
use test_support::{Render, Session, parity, render_native};

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

fn set_scale(s: &mut Session, root: u8, mode: ScaleKind) {
    s.run(Command::Project(p::Command::SetScale { root, mode }));
}

/// A session on the fixture's synth track, in C major, with Lattice before the synth at its
/// defaults (the project's scale, the nearest note), playing.
fn session() -> (Session, DeviceId, TrackId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    set_scale(&mut s, 0, ScaleKind::Major);
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
    s.run(Command::Transport(transport::Command::Play {}));
    (s, fx, f.track)
}

fn note_on(s: &mut Session, track: &TrackId, note: u8) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note,
        velocity: 100,
    }));
}

fn note_off(s: &mut Session, track: &TrackId, note: u8) {
    s.run(Command::Live(live::Command::NoteOff {
        track: track.clone(),
        note,
    }));
}

fn hz(key: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(key) - 69.0) / 12.0)
}

/// The strongest bin within a quarter tone of `key`, over `window`.
fn level_at(r: &Render, window: std::ops::Range<f32>, key: u8) -> f32 {
    let f = hz(key);
    r.spectrum(window).peak_in(f * 0.97..f * 1.03)
}

#[test]
fn a_chromatic_run_comes_out_on_the_scale_on_both_targets() {
    let (mut s, _, track) = session();
    // A4 to C5 chromatically, 300 ms each: A, A# (→ A, a tie), B, C. Then G#4 (→ G).
    for key in [69u8, 70, 71, 72, 68] {
        note_on(&mut s, &track, key);
        s.render_secs(0.25);
        note_off(&mut s, &track, key);
        s.render_secs(0.05);
    }
    s.render_secs(1.0);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_no_denormals();
    r.assert_audible(0.05..1.45, 0.01);
    // A# played A: the second note has A, no A#.
    let window = 0.35..0.55;
    let a = level_at(&r, window.clone(), 69);
    let a_sharp = level_at(&r, window, 70);
    assert!(a > 10.0 * a_sharp, "a {a}, a# {a_sharp}");
    // G# played G.
    let window = 1.25..1.45;
    let g = level_at(&r, window.clone(), 67);
    let g_sharp = level_at(&r, window, 68);
    assert!(g > 10.0 * g_sharp, "g {g}, g# {g_sharp}");
    // B stays B.
    let window = 0.65..0.85;
    assert!(level_at(&r, window.clone(), 71) > 10.0 * level_at(&r, window, 70));
    assert!(r.rms_in(2.0..2.5) < 1e-4, "tail {}", r.rms_in(2.0..2.5));
    r.write_wav("lattice-chromatic");
}

#[test]
fn a_note_held_while_the_project_scale_changes_ends_and_the_next_follows_the_new_scale() {
    let (mut s, _, track) = session();
    // C#5 in C major plays C5; the project goes to D major (C# in it) while the key is held.
    note_on(&mut s, &track, 73);
    s.render_secs(0.2);
    set_scale(&mut s, 2, ScaleKind::Major);
    s.render_secs(0.2);
    note_off(&mut s, &track, 73);
    s.render_secs(0.6);
    // The same key again: now C#5.
    note_on(&mut s, &track, 73);
    s.render_secs(0.3);
    note_off(&mut s, &track, 73);
    s.render_secs(1.0);
    let r = parity(s.scenario());
    r.assert_clean();
    let first = 0.05..0.35;
    assert!(level_at(&r, first.clone(), 72) > 10.0 * level_at(&r, first, 73));
    let second = 1.05..1.25;
    assert!(level_at(&r, second.clone(), 73) > 10.0 * level_at(&r, second, 72));
    // The first note ended at its release: silence between the two, and after.
    assert!(r.rms_in(0.75..0.95) < 1e-4, "{}", r.rms_in(0.75..0.95));
    assert!(r.rms_in(2.0..2.3) < 1e-4, "{}", r.rms_in(2.0..2.3));
}

#[test]
fn a_note_held_while_lattice_is_bypassed_removed_or_changed_leaves_nothing_sounding() {
    for case in ["bypass", "unbypass", "remove", "settings"] {
        let (mut s, fx, track) = session();
        if case == "unbypass" {
            s.run(Command::Device(device::Command::SetBypass {
                device: fx.clone(),
                bypass: true,
            }));
        }
        // Two held notes on one key (C and C# both play C), and one off the scale.
        note_on(&mut s, &track, 60);
        note_on(&mut s, &track, 61);
        note_on(&mut s, &track, 66);
        s.render_secs(0.2);
        match case {
            "bypass" | "unbypass" => {
                s.run(Command::Device(device::Command::SetBypass {
                    device: fx.clone(),
                    bypass: case == "bypass",
                }));
            }
            "remove" => {
                s.run(Command::Device(device::Command::Remove {
                    device: fx.clone(),
                }));
            }
            _ => {
                for (param, value) in [
                    ("source", 2.0),
                    ("map_c", 7.0),
                    ("rule", 2.0),
                    ("transpose", 12.0),
                ] {
                    set(&mut s, &fx, param, value);
                }
            }
        }
        s.render_secs(0.3);
        for key in [60u8, 61, 66] {
            note_off(&mut s, &track, key);
        }
        s.render_secs(1.0);
        let r = render_native(s.scenario());
        r.assert_clean();
        assert!(
            r.rms_in(1.0..1.5) < 1e-4,
            "{case}: still sounding {}",
            r.rms_in(1.0..1.5)
        );
    }
}

/// A clip's note through Lattice (the schedule, not the live input): C#4 plays C4 on both
/// targets; a stop and a jump of the transport while it sounds end it.
#[test]
fn a_clip_note_is_quantised_alike_on_both_targets_and_a_stop_or_a_jump_ends_it() {
    for end in ["none", "stop", "seek"] {
        let (mut s, _, track) = session();
        s.run(Command::Transport(transport::Command::Stop {}));
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
        s.run(Command::Clip(clip::Command::NotesAdd {
            clip: c,
            notes: vec![Note {
                start: Tick(0),
                pitch: 61,
                length: Tick(7680),
                velocity: 110,
                cents: 0,
                chance: Note::DEFAULT_CHANCE,
                repeat: Note::DEFAULT_REPEAT,
            }],
        }));
        s.run(Command::Transport(transport::Command::Play {}));
        s.render_secs(0.5);
        match end {
            "stop" => {
                s.run(Command::Transport(transport::Command::Stop {}));
            }
            // Mid-note, past its start: the clip does not start it again.
            "seek" => {
                s.run(Command::Transport(transport::Command::Seek {
                    tick: Tick(3840),
                }));
            }
            _ => {}
        }
        s.render_secs(1.0);
        if end == "none" {
            let r = parity(s.scenario());
            r.assert_clean();
            let w = 0.1..0.45;
            assert!(level_at(&r, w.clone(), 60) > 10.0 * level_at(&r, w, 61));
        } else {
            let r = render_native(s.scenario());
            r.assert_clean();
            r.assert_audible(0.05..0.45, 0.01);
            assert!(
                r.rms_in(1.0..1.5) < 1e-4,
                "{end}: still sounding {}",
                r.rms_in(1.0..1.5)
            );
        }
    }
}

/// A project that already has a scale when the engine is built (opening it, a resync, a switch
/// of mode: a `Reset` then the snapshot): Lattice follows it from the first note, on both
/// targets.
#[test]
fn a_project_opened_with_its_scale_is_quantised_from_the_first_note() {
    let f = fixture();
    let mut state = f.state.clone();
    let mut ctx = context();
    commands::run(
        &Command::Project(p::Command::SetScale {
            root: 0,
            mode: ScaleKind::Major,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    commands::run(
        &Command::Device(device::Command::Insert {
            track: Some(f.track.clone()),
            master: false,
            kind: "faulk.lattice".into(),
            index: 0,
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    // The session starts from that state: its engine gets the snapshot, nothing else.
    let mut s = Session::from_state(state, context());
    set(&mut s, &f.device, "release", 0.2);
    s.run(Command::Transport(transport::Command::Play {}));
    note_on(&mut s, &f.track, 73);
    s.render_secs(0.4);
    note_off(&mut s, &f.track, 73);
    s.render_secs(0.6);
    let r = parity(s.scenario());
    r.assert_clean();
    let w = 0.05..0.35;
    let (c, c_sharp) = (level_at(&r, w.clone(), 72), level_at(&r, w, 73));
    assert!(c > 10.0 * c_sharp, "c {c}, c# {c_sharp}");
}

/// A key released while Lattice is bypassed, then played again once it is back on, sounds: the
/// count of the key it played does not survive the switch (the engine resets a MIDI effect on
/// every switch, #219).
#[test]
fn a_key_released_during_a_bypass_plays_again_after_it() {
    let (mut s, fx, track) = session();
    let bypass = |bypass| {
        Command::Device(device::Command::SetBypass {
            device: fx.clone(),
            bypass,
        })
    };
    note_on(&mut s, &track, 73);
    s.render_secs(0.2);
    s.run(bypass(true));
    s.render_secs(0.1);
    note_off(&mut s, &track, 73);
    s.render_secs(0.5);
    s.run(bypass(false));
    s.render_secs(0.1);
    note_on(&mut s, &track, 73);
    s.render_secs(0.4);
    note_off(&mut s, &track, 73);
    s.render_secs(0.6);
    let r = render_native(s.scenario());
    r.assert_clean();
    // Silent between, and the second press plays C again.
    assert!(r.rms_in(0.7..0.85) < 1e-4, "{}", r.rms_in(0.7..0.85));
    let w = 0.95..1.25;
    assert!(r.rms_in(w.clone()) > 1e-3, "the key plays again");
    assert!(level_at(&r, w.clone(), 72) > 10.0 * level_at(&r, w, 73));
}
