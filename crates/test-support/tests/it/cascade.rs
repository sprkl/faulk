//! Cascade (#166) through the command layer and the engine: `device.insert` puts Cascade before
//! the synth of a track; a C major chord held for a second by a clip comes out as eight sixteenth
//! notes 125 ms apart at 120 BPM, each attack where the event stream puts it, the same natively
//! and in WASM and the same on a second render; the chord is the notes of the clip, one at a time
//! (the clip itself never sounds as a chord). Nothing sounds after a stop and the release. Both
//! renders are written to `.qa/renders/` for the ear. A sweep of every parameter at its bounds
//! renders finite audio. (The device itself is tested in `crates/engine/tests/it/cascade.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, clip, device, live, transport};
use project::{DeviceId, Note, Tick, TrackId};
use test_support::{Render, Session, parity, render_native};

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

/// A session on the fixture's synth track with Cascade before the synth, the synth short and
/// quick (a plucked tick).
fn session() -> (Session, DeviceId, TrackId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.cascade".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    set(&mut s, &f.device, "release", 0.02);
    set(&mut s, &f.device, "gain", 0.5);
    (s, fx, f.track)
}

/// A C major chord of one second (1920 ticks at 120 BPM) at the start of the arrangement.
fn chord(s: &mut Session, track: &TrackId) {
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
    let notes = [60, 64, 67]
        .into_iter()
        .map(|pitch| Note {
            start: Tick(0),
            pitch,
            length: Tick(1920),
            velocity: 100,
            cents: 0,
            chance: Note::DEFAULT_CHANCE,
            repeat: Note::DEFAULT_REPEAT,
        })
        .collect();
    s.run(Command::Clip(clip::Command::NotesAdd { clip: c, notes }));
}

fn played(configure: impl Fn(&mut Session, &DeviceId)) -> Session {
    let (mut s, fx, track) = session();
    configure(&mut s, &fx);
    chord(&mut s, &track);
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(2.0);
    s.run(Command::Transport(transport::Command::Stop {}));
    s.render_secs(1.0);
    s
}

/// The onset of the step `i` (125 ms apart), searched from 5 ms before its start.
fn onset(r: &Render, i: u32) -> f32 {
    let from = i as f32 * 0.125 - 0.005;
    r.onset(0.002, from).expect("a note")
}

#[test]
fn a_held_chord_through_cascade_attacks_every_125_ms_alike_on_both_targets() {
    let up = parity(played(|_, _| {}).scenario());
    up.assert_clean();
    up.assert_no_denormals();
    up.assert_audible(0.0..0.9, 0.001);
    up.write_wav("cascade-up-1-16");
    // The attacks of the event stream: one every 6000 frames (125 ms), from the first.
    for i in 0..8 {
        let at = onset(&up, i);
        assert!(
            (at - i as f32 * 0.125).abs() < 0.0015,
            "step {i} attacks at {at}"
        );
    }
    // The chord is gone after its second (the notes end, the arpeggio stops with them), and
    // nothing sounds after the stop and the release.
    assert!(up.rms_in(1.25..1.45) < 1e-4);
    assert!(up.rms_in(2.2..3.0) < 1e-4);
    // A second render at the same settings is the same, sample for sample.
    render_native(played(|_, _| {}).scenario()).assert_close(&up, 0.0);
    // A triplet 1/8, down, two octaves, repeats: another pattern, a different render.
    let other = |s: &mut Session, fx: &DeviceId| {
        set(s, fx, "order", 1.0);
        set(s, fx, "rate", 3.0);
        set(s, fx, "feel", 2.0);
        set(s, fx, "octaves", 2.0);
        set(s, fx, "repeats", 2.0);
        set(s, fx, "gate", 50.0);
    };
    let r = parity(played(other).scenario());
    r.assert_clean();
    r.write_wav("cascade-down-triplet");
    assert!(r.max_difference(&up).0 > 1e-3);
    // The seeded random order: the same seed twice is the same render, another seed is not.
    let random = |seed: f64| {
        move |s: &mut Session, fx: &DeviceId| {
            set(s, fx, "order", 5.0);
            set(s, fx, "octaves", 3.0);
            set(s, fx, "seed", seed);
        }
    };
    let a = parity(played(random(5.0)).scenario());
    render_native(played(random(5.0)).scenario()).assert_close(&a, 0.0);
    assert!(
        render_native(played(random(6.0)).scenario())
            .max_difference(&a)
            .0
            > 1e-3
    );
}

#[test]
fn a_chord_held_while_cascade_is_bypassed_removed_changed_latched_or_stopped_leaves_nothing_sounding()
 {
    let cases = [
        "bypass",
        "unbypass",
        "remove",
        "settings",
        "latch_bypass",
        "latch_bypass_back",
        "latch_remove",
        "latch_stop",
    ];
    for case in cases {
        let (mut s, fx, track) = session();
        let latch = case.starts_with("latch");
        let bypass = |s: &mut Session, on: bool| {
            s.run(Command::Device(device::Command::SetBypass {
                device: fx.clone(),
                bypass: on,
            }));
        };
        if latch {
            set(&mut s, &fx, "latch", 1.0);
        }
        if case == "unbypass" {
            bypass(&mut s, true);
        }
        if case == "latch_stop" {
            s.run(Command::Transport(transport::Command::Play {}));
        }
        let keys = [60u8, 64, 67];
        let hold = |s: &mut Session, on: bool| {
            for note in keys {
                s.run(Command::Live(if on {
                    live::Command::NoteOn {
                        track: track.clone(),
                        note,
                        velocity: 100,
                    }
                } else {
                    live::Command::NoteOff {
                        track: track.clone(),
                        note,
                    }
                }));
            }
        };
        hold(&mut s, true);
        s.render_secs(0.3);
        if latch {
            // The keys are up and the pattern goes on, latched.
            hold(&mut s, false);
            s.render_secs(0.3);
        }
        match case {
            "bypass" | "latch_bypass" => bypass(&mut s, true),
            "unbypass" => bypass(&mut s, false),
            "latch_bypass_back" => {
                bypass(&mut s, true);
                s.render_secs(0.05);
                bypass(&mut s, false);
            }
            "remove" | "latch_remove" => {
                s.run(Command::Device(device::Command::Remove {
                    device: fx.clone(),
                }));
            }
            "latch_stop" => {
                s.run(Command::Transport(transport::Command::Stop {}));
            }
            _ => {
                for (param, value) in [
                    ("order", 6.0),
                    ("rate", 2.0),
                    ("octaves", 3.0),
                    ("gate", 100.0),
                ] {
                    set(&mut s, &fx, param, value);
                }
            }
        }
        s.render_secs(0.3);
        hold(&mut s, false);
        s.render_secs(2.0);
        let r = render_native(s.scenario());
        r.assert_clean();
        let tail = if latch { 1.5..2.5 } else { 1.2..2.2 };
        assert!(
            r.rms_in(tail.clone()) < 1e-4,
            "{case}: still sounding {}",
            r.rms_in(tail)
        );
    }
}

#[test]
fn every_parameter_at_its_bounds_renders_finite_bounded_audio_alike_on_both_targets() {
    let params: [(&str, [f64; 3]); 12] = [
        ("order", [0.0, 3.0, 6.0]),
        ("rate", [0.0, 4.0, 6.0]),
        ("feel", [0.0, 1.0, 2.0]),
        ("octaves", [1.0, 2.0, 4.0]),
        ("repeats", [1.0, 2.0, 4.0]),
        ("gate", [1.0, 80.0, 200.0]),
        ("velocity_source", [0.0, 1.0, 1.0]),
        ("velocity", [1.0, 100.0, 127.0]),
        ("accent", [0.0, 30.0, 60.0]),
        ("latch", [0.0, 1.0, 1.0]),
        ("restart", [0.0, 1.0, 1.0]),
        ("seed", [0.0, 500.0, 1000.0]),
    ];
    // The sweep holds every parameter of the descriptor, and no other.
    let keys: Vec<&str> = params.iter().map(|(k, _)| *k).collect();
    let described: Vec<&str> = engine::devices::cascade::DESCRIPTOR
        .params
        .iter()
        .map(|p| p.key)
        .collect();
    assert_eq!(keys, described);
    for (key, values) in params {
        for value in values {
            let (mut s, fx, track) = session();
            // The seed only matters to the random order.
            if key == "seed" {
                set(&mut s, &fx, "order", 5.0);
            }
            set(&mut s, &fx, key, value);
            chord(&mut s, &track);
            s.run(Command::Transport(transport::Command::Play {}));
            s.render_secs(1.5);
            s.run(Command::Transport(transport::Command::Stop {}));
            s.render_secs(0.5);
            let r = parity(s.scenario());
            r.assert_clean();
            r.assert_no_denormals();
            assert!(r.peak() <= 1.0, "{key} at {value}: peak {}", r.peak());
        }
    }
}
