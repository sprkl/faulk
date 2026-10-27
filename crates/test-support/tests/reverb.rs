//! DV-4 through the command layer and the engine: `device.insert` puts the reverb after the
//! synth of a track, `device.set_param` moves its size and `device.set_bypass` cuts a ringing
//! tail. The render is written to `.qa/renders/reverb.wav` for the ear: the same hit dry,
//! then with a small, a medium and a large size, then a large one cut by the bypass.

use commands::testing::{context, fixture};
use commands::{Command, Output, State, device, live};
use test_support::{Session, render_native};

/// A short chord (0.08 s) then silence until `rest` seconds after its start.
fn hit(s: &mut Session, track: &project::TrackId, rest: f32) {
    for pitch in [57u8, 64, 69] {
        s.run(Command::Live(live::Command::NoteOn {
            track: track.clone(),
            note: pitch,
            velocity: 110,
        }));
    }
    s.render_secs(0.08);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    s.render_secs(rest - 0.08);
}

#[test]
fn the_reverb_has_a_tail_that_follows_its_size_and_the_bypass() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    for (param, value) in [("attack", 0.002), ("release", 0.01), ("decay", 0.15)] {
        s.run(Command::Device(device::Command::SetParam {
            device: f.device.clone(),
            param: param.into(),
            value,
        }));
    }
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.reverb".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: reverb }) = out else {
        panic!("{out:?}")
    };
    let set = |s: &mut Session, param: &str, value: f64| {
        s.run(Command::Device(device::Command::SetParam {
            device: reverb.clone(),
            param: param.into(),
            value,
        }));
    };
    let bypass = |s: &mut Session, on: bool| {
        s.run(Command::Device(device::Command::SetBypass {
            device: reverb.clone(),
            bypass: on,
        }));
    };
    set(&mut s, "mix", 0.35);
    set(&mut s, "damping", 0.4);

    // 1. Dry (bypassed), starts at 0 s.
    bypass(&mut s, true);
    hit(&mut s, &f.track, 4.0);
    // 2. Small, starts at 4.5 s.
    bypass(&mut s, false);
    set(&mut s, "size", 0.15);
    s.render_secs(0.5);
    hit(&mut s, &f.track, 4.0);
    // 3. Medium, starts at 9 s.
    set(&mut s, "size", 0.5);
    s.render_secs(0.5);
    hit(&mut s, &f.track, 4.0);
    // 4. Large, starts at 13.5 s.
    set(&mut s, "size", 0.95);
    s.render_secs(0.5);
    hit(&mut s, &f.track, 8.0);

    // 5. A large hit at 22 s, bypassed at 22.5 s while its tail rings: the tail is cut.
    set(&mut s, "size", 0.95);
    s.render_secs(0.5);
    hit(&mut s, &f.track, 0.5);
    bypass(&mut s, true);
    s.render_secs(1.0);

    // Native only: the interpreter takes minutes on 30 s; the parity is `reverb_parity.rs`.
    let r = render_native(s.scenario());
    r.assert_clean();
    // The dry hit leaves nothing after its release.
    r.assert_audible(0.0..0.08, 0.05);
    r.assert_silent(0.3..4.0);
    // Each reverb tail rings after the note, longer with the size.
    let small = r.rms_in(4.8..5.1);
    let medium = r.rms_in(9.3..9.6);
    let large = r.rms_in(13.8..14.1);
    println!("rms 0.3 s after the hit: small {small}, medium {medium}, large {large}");
    assert!(small > 1e-4 && medium > 1e-3 && large > 1e-3);
    // Later on, the small room is gone and the large one still rings.
    let late = |from: f32| r.rms_in(from + 2.0..from + 2.3);
    println!(
        "rms 2 s after the hit: small {}, large {}",
        late(4.5),
        late(13.5)
    );
    assert!(late(4.5) < 0.1 * late(13.5));
    assert!(late(13.5) > 1e-4);
    r.assert_audible(22.3..22.5, 0.002);
    r.assert_silent(22.6..23.4);
    r.write_wav("reverb");
}

/// Issue #353: a held note whose pre-delay moves while it sounds, in each of the three
/// settings of `time_change`, through the commands, natively and in WASM; the render goes to
/// `.qa/renders/reverb-predelay-modes.wav` for the ear.
#[test]
fn the_reverb_changes_its_pre_delay_as_the_setting_says_through_commands() {
    use test_support::parity;
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    for (param, value) in [
        ("waveform", 0.0),
        ("attack", 0.005),
        ("release", 0.05),
        ("sustain", 1.0),
    ] {
        s.run(Command::Device(device::Command::SetParam {
            device: f.device.clone(),
            param: param.into(),
            value,
        }));
    }
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.reverb".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: reverb }) = out else {
        panic!("{out:?}")
    };
    let set = |s: &mut Session, param: &str, value: f64| {
        s.run(Command::Device(device::Command::SetParam {
            device: reverb.clone(),
            param: param.into(),
            value,
        }));
    };
    // A short, bright room, fully wet: only the reverb is heard, so its pitch is the pitch
    // the pre-delay gives it.
    set(&mut s, "size", 0.0);
    set(&mut s, "damping", 0.0);
    set(&mut s, "mix", 1.0);
    set(&mut s, "predelay", 0.0);
    // Per setting (2.7 s): a sine A4 held from 0 s to 2.1 s; at 0.3 s the pre-delay moves to
    // 0.1 s, at 1.2 s back to 0.02 s; the tail rings out, then the pre-delay is back to 0.
    for setting in [0.0, 1.0, 2.0] {
        set(&mut s, "time_change", setting);
        s.run(Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note: 69,
            velocity: 110,
        }));
        s.render_secs(0.3);
        set(&mut s, "predelay", 0.1);
        s.render_secs(0.9);
        set(&mut s, "predelay", 0.02);
        s.render_secs(0.9);
        s.run(Command::Live(live::Command::AllNotesOff {}));
        s.render_secs(0.5);
        set(&mut s, "predelay", 0.0);
        s.render_secs(0.1);
    }
    let r = parity(s.scenario());
    r.assert_clean();
    let start = |i: usize| i as f32 * 2.7;
    let hz = |i: usize, from: f32| {
        r.spectrum(start(i) + from..start(i) + from + 0.2)
            .dominant_hz()
    };
    // 0.1 s after the first move the delay is still moving under Glide (0 to 0.1 s is 4 800
    // frames at 0.25 frame per frame: 0.4 s; 0.1 s back to 0.02 s takes 0.32 s), the pitch is
    // bent down by a quarter, then up; Fade and Jump keep the note's own 440 Hz.
    let down = [hz(0, 0.4), hz(1, 0.4), hz(2, 0.4)];
    let up = [hz(0, 1.3), hz(1, 1.3), hz(2, 1.3)];
    println!("pitch after the first move (glide, fade, jump): {down:?}");
    println!("pitch after the second move (glide, fade, jump): {up:?}");
    assert!(down[0] < 0.85 * down[1], "{down:?}");
    assert!(up[0] > 1.1 * up[1], "{up:?}");
    for pitch in [down[1], down[2], up[1], up[2]] {
        assert!((pitch / 440.0 - 1.0).abs() < 0.03, "{pitch} Hz");
    }
    r.write_wav("reverb-predelay-modes");
}

/// Issue #353: a project saved before `time_change` existed (a reverb whose parameters lack
/// the key) plays Glide: the same sound as one that says Glide, and not Jump's.
#[test]
fn a_project_without_the_time_change_key_plays_glide() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.reverb".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: reverb }) = out else {
        panic!("{out:?}")
    };
    // The same project with the setting at `value`, or without the key (as saved before it
    // existed).
    let saved = s.state().clone();
    let with_setting = |value: Option<f64>| {
        let mut state = saved.clone();
        let mut found = 0;
        for track in state.project.tracks.values_mut() {
            if let Some(d) = track.devices.get_mut(&reverb) {
                found += 1;
                match value {
                    Some(v) => d.params.insert("time_change".into(), v),
                    None => d.params.remove("time_change"),
                };
            }
        }
        assert_eq!(found, 1);
        state
    };
    let phrase = |state: State| {
        let mut s = Session::from_state(state, context());
        let set = |s: &mut Session, param: &str, value: f64| {
            s.run(Command::Device(device::Command::SetParam {
                device: reverb.clone(),
                param: param.into(),
                value,
            }));
        };
        set(&mut s, "mix", 1.0);
        set(&mut s, "size", 0.1);
        set(&mut s, "predelay", 0.0);
        s.run(Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note: 69,
            velocity: 110,
        }));
        s.render_secs(0.3);
        set(&mut s, "predelay", 0.15);
        s.render_secs(0.7);
        render_native(s.scenario())
    };
    let without = phrase(with_setting(None));
    let glide = phrase(with_setting(Some(0.0)));
    let jump = phrase(with_setting(Some(2.0)));
    without.assert_clean();
    without.assert_close(&glide, 0.0);
    let (difference, _) = without.max_difference(&jump);
    assert!(difference > 0.01, "jump differs from glide: {difference}");
}
