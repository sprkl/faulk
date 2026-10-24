//! DV-3 through the command layer and the engine: `device.insert` puts the echo after the
//! synth of a track, the echo follows `transport.set_tempo`, `device.set_param` moves it to
//! a free time, `device.set_bypass` silences it, and the undo of each brings it back. The
//! render is the same natively and in WASM, and is written to `.qa/renders/` for the ear.

use commands::testing::{context, fixture};
use commands::{Command, Output, device, history, live, transport};
use test_support::{Session, parity};

fn note(s: &mut Session, track: &project::TrackId, pitch: u8) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note: pitch,
        velocity: 110,
    }));
}

/// A short note (0.1 s) then silence until `rest` seconds after its start.
fn pluck(s: &mut Session, track: &project::TrackId, pitch: u8, rest: f32) {
    note(s, track, pitch);
    s.render_secs(0.1);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    s.render_secs(rest - 0.1);
}

#[test]
fn the_echo_follows_the_tempo_the_free_time_the_bypass_and_the_undo() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    // A pluck: the synth releases in a few milliseconds.
    for (param, value) in [("attack", 0.002), ("release", 0.01)] {
        s.run(Command::Device(device::Command::SetParam {
            device: f.device.clone(),
            param: param.into(),
            value,
        }));
    }
    let out = s.exec(Command::Device(device::Command::Insert {
        track: f.track.clone(),
        kind: "faulk.echo".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: echo }) = out else {
        panic!("{out:?}")
    };
    let set = |s: &mut Session, param: &str, value: f64| {
        s.run(Command::Device(device::Command::SetParam {
            device: echo.clone(),
            param: param.into(),
            value,
        }));
    };

    // 1. The defaults: synced on a dotted eighth, 120 BPM: 0.375 s.
    pluck(&mut s, &f.track, 60, 2.0);
    // 2. 90 BPM: the dotted eighth is 0.5 s (the delay glides there).
    s.run(Command::Transport(transport::Command::SetTempo {
        bpm: 90.0,
    }));
    s.render_secs(1.0);
    pluck(&mut s, &f.track, 64, 2.0); // starts at 3.0 s
    // 3. Free, 200 ms, more feedback and mix.
    set(&mut s, "mode", 0.0);
    set(&mut s, "time", 0.2);
    set(&mut s, "feedback", 0.6);
    set(&mut s, "mix", 0.5);
    s.render_secs(1.0);
    pluck(&mut s, &f.track, 67, 2.0); // starts at 6.0 s
    // 4. Bypassed: the note has no echo. Then the undo: the echo is back.
    s.run(Command::Device(device::Command::SetBypass {
        device: echo.clone(),
        bypass: true,
    }));
    s.render_secs(0.5);
    pluck(&mut s, &f.track, 60, 1.5); // starts at 8.5 s
    s.run(Command::History(history::Command::Undo {}));
    s.render_secs(0.5);
    pluck(&mut s, &f.track, 60, 1.5); // starts at 10.5 s

    let r = parity(s.scenario());
    r.assert_clean();
    // The clock is in blocks of 128 frames (2.67 ms): each render rounds up, so the starts drift by a few milliseconds.
    // 1. The note, the silence, the first echo at 0.375 s, the second at 0.75 s, weaker.
    r.assert_audible(0.0..0.1, 0.05);
    r.assert_silent(0.15..0.36);
    r.assert_onset_at(0.375, 0.006, 0.01, 0.15);
    let first = r.peak_in(0.37..0.5);
    let second = r.peak_in(0.745..0.87);
    assert!(
        first > 0.03 && second > 0.3 * first && second < 0.6 * first,
        "{first} {second}"
    );
    // 2. At 90 BPM the echo is at 0.5 s.
    r.assert_onset_at(3.5, 0.006, 0.01, 3.15);
    // 3. Free at 200 ms.
    r.assert_onset_at(6.2, 0.02, 0.01, 6.15);
    // 4. Bypassed: the dry note, then nothing; after the undo: the echo again.
    r.assert_audible(8.5..8.6, 0.05);
    r.assert_silent(8.65..9.9);
    r.assert_onset_at(10.7, 0.03, 0.01, 10.65);
    r.write_wav("echo-through-commands");
}

/// A note held `held` seconds, then silence until `rest` seconds after its start.
fn hold(s: &mut Session, track: &project::TrackId, pitch: u8, held: f32, rest: f32) {
    note(s, track, pitch);
    s.render_secs(held);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    s.render_secs(rest - held);
}

/// Issue #342: the same phrase with the time moved while the echo rings, in each of the
/// three settings of `time_change`, through the commands, natively and in WASM; the render
/// goes to `.qa/renders/echo-modes.wav` for the ear.
#[test]
fn the_echo_changes_its_time_as_the_setting_says_through_commands() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    for (param, value) in [("attack", 0.005), ("release", 0.05)] {
        s.run(Command::Device(device::Command::SetParam {
            device: f.device.clone(),
            param: param.into(),
            value,
        }));
    }
    let out = s.exec(Command::Device(device::Command::Insert {
        track: f.track.clone(),
        kind: "faulk.echo".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: echo }) = out else {
        panic!("{out:?}")
    };
    let set = |s: &mut Session, param: &str, value: f64| {
        s.run(Command::Device(device::Command::SetParam {
            device: echo.clone(),
            param: param.into(),
            value,
        }));
    };
    set(&mut s, "mode", 0.0);
    set(&mut s, "time", 0.3);
    set(&mut s, "feedback", 0.6);
    set(&mut s, "mix", 0.5);
    // Per setting (7 s): a note and its echoes at 0.3 s; at 0.6 s the time moves to 0.12 s
    // while they ring; a second note at 2.6 s; at 3.2 s the time moves to 0.45 s; at 5.2 s it
    // is back to 0.3 s, in silence, ready for the next setting.
    for setting in [0.0, 1.0, 2.0] {
        set(&mut s, "time_change", setting);
        hold(&mut s, &f.track, 60, 0.25, 0.6);
        set(&mut s, "time", 0.12);
        s.render_secs(2.0);
        hold(&mut s, &f.track, 67, 0.25, 0.6);
        set(&mut s, "time", 0.45);
        s.render_secs(2.0);
        set(&mut s, "time", 0.3);
        s.render_secs(1.8);
    }
    let r = parity(s.scenario());
    r.assert_clean();
    for start in [0.0, 7.0, 14.0] {
        r.assert_audible(start..start + 0.25, 0.05);
        // The first echo, at 0.3 s, is there in every setting.
        r.assert_audible(start + 0.3..start + 0.55, 0.01);
    }
    r.write_wav("echo-modes");
}
