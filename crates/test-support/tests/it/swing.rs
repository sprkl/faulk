//! Swing (#168) through the command layer and the engine: `device.insert` puts Swing before the
//! synth of a track; a clip of sixteenth notes (a hat) played at 50 % and at 66 % comes out with
//! its off-beats 40 ms later at 66 % (76.8 ticks at 120 BPM), the beats where they were, the same
//! natively and in WASM and the same on a second render at equal seed; both renders are written to
//! `.qa/renders/` for the ear. A sweep of every parameter at its bounds renders finite audio. (The
//! device itself is tested in `crates/engine/tests/it/swing.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, clip, device, transport};
use project::{DeviceId, Note, Tick, TrackId};
use test_support::{Render, Session, parity, render_native};

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

/// A session on the fixture's synth track with Swing before the synth, the synth short and
/// quick (a hat-like tick).
fn session() -> (Session, DeviceId, TrackId, DeviceId) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.swing".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    set(&mut s, &f.device, "release", 0.02);
    set(&mut s, &f.device, "gain", 0.5);
    (s, fx, f.track, f.device)
}

/// Thirty-two sixteenth notes (240 ticks of 960 per beat at 120 BPM), 100 ticks long, over two
/// bars, as a clip: a hat in sixteenths.
fn hats(s: &mut Session, track: &TrackId) {
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
    let notes = (0..32u32)
        .map(|i| Note {
            start: Tick(i64::from(i) * 240),
            pitch: 84,
            length: Tick(100),
            velocity: 100,
            cents: 0,
            chance: Note::DEFAULT_CHANCE,
            repeat: Note::DEFAULT_REPEAT,
        })
        .collect();
    s.run(Command::Clip(clip::Command::NotesAdd { clip: c, notes }));
}

fn swung(amount: f64) -> Session {
    let (mut s, fx, track, _) = session();
    set(&mut s, &fx, "amount", amount);
    hats(&mut s, &track);
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(4.0);
    s.run(Command::Transport(transport::Command::Stop {}));
    s.render_secs(1.0);
    s
}

/// The onset of the note `i` of the clip, searched from 5 ms before its straight start.
fn onset(r: &Render, i: u32) -> f32 {
    let from = i as f32 * 0.125 - 0.005;
    r.onset(0.002, from).expect("a note")
}

#[test]
fn a_hat_clip_through_swing_delays_its_off_beats_by_40_ms_at_66_percent_alike_on_both_targets() {
    let straight = parity(swung(50.0).scenario());
    straight.assert_clean();
    straight.assert_audible(0.0..3.9, 0.001);
    straight.write_wav("swing-hats-50");
    let triplet = parity(swung(66.0).scenario());
    triplet.assert_clean();
    triplet.assert_no_denormals();
    triplet.write_wav("swing-hats-66");
    for i in 0..32 {
        let (a, b) = (onset(&straight, i), onset(&triplet, i));
        let expected = if i % 2 == 1 { 0.04 } else { 0.0 };
        assert!(
            (b - a - expected).abs() < 0.0015,
            "note {i}: {a} -> {b}, expected a shift of {expected}"
        );
    }
    // A second render at the same settings is the same, sample for sample.
    render_native(swung(66.0).scenario()).assert_close(&triplet, 0.0);
    // Nothing sounds after the stop and the release.
    assert!(triplet.rms_in(4.4..5.0) < 1e-4);
    // With humanize on, a second render at the same seed is the same, another seed is not.
    let humanized = |seed: f64| {
        let (mut s, fx, track, _) = session();
        set(&mut s, &fx, "amount", 66.0);
        set(&mut s, &fx, "humanize_time", 0.01);
        set(&mut s, &fx, "humanize_velocity", 10.0);
        set(&mut s, &fx, "seed", seed);
        hats(&mut s, &track);
        s.run(Command::Transport(transport::Command::Play {}));
        s.render_secs(4.0);
        s.run(Command::Transport(transport::Command::Stop {}));
        s.render_secs(1.0);
        s
    };
    let a = parity(humanized(5.0).scenario());
    render_native(humanized(5.0).scenario()).assert_close(&a, 0.0);
    assert!(
        render_native(humanized(6.0).scenario())
            .max_difference(&a)
            .0
            > 1e-3
    );
}

#[test]
fn every_parameter_at_its_bounds_renders_finite_bounded_audio_alike_on_both_targets() {
    let params = [
        ("amount", [50.0, 58.0, 75.0]),
        ("grid", [0.0, 1.0, 2.0]),
        ("window", [0.0, 25.0, 50.0]),
        ("humanize_time", [0.0, 0.01, 0.02]),
        ("humanize_velocity", [0.0, 10.0, 20.0]),
        ("seed", [0.0, 500.0, 1000.0]),
    ];
    for (key, values) in params {
        for value in values {
            let (mut s, fx, track, _) = session();
            set(&mut s, &fx, "amount", 75.0);
            set(&mut s, &fx, "window", 50.0);
            set(&mut s, &fx, key, value);
            sweep_one(s, fx, track, key, value);
        }
    }
    // The device at its own defaults, whole.
    let (s, fx, track, _) = session();
    sweep_one(s, fx, track, "defaults", 0.0);
}

fn sweep_one(mut s: Session, _fx: DeviceId, track: TrackId, key: &str, value: f64) {
    hats(&mut s, &track);
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(2.0);
    s.run(Command::Transport(transport::Command::Stop {}));
    s.render_secs(0.5);
    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_no_denormals();
    assert!(r.peak() <= 1.0, "{key} at {value}: peak {}", r.peak());
}
