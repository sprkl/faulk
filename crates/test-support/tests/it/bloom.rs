//! Bloom (#696) through the command layer and the engine: `device.insert` puts `faulk.bloom`
//! after the synth of a track; `device.set_param` sets each of its parameters by name (schema,
//! description and undo checked by `commands::testing::check`, the undo of a setting brings the
//! previous value back); a pad chord dry, through a soft chorus and through a wide ensemble
//! renders the same natively and in WASM, the chorus and the ensemble widen it (left/right
//! correlation) and leave its level about where it was. The render is written to
//! `.qa/renders/bloom.wav`, and each of its three parts to `.qa/696/` for the founder's ear.
//! (The device itself is tested in `crates/engine/tests/it/bloom.rs`.)

use commands::testing::{check, context, fixture};
use commands::{Command, Output, device, history, live, transport};
use engine::devices::bloom::DESCRIPTOR;
use project::{DeviceId, DeviceRole};
use test_support::{Render, Session, parity};

fn set(device: &DeviceId, param: &str, value: f64) -> Command {
    Command::Device(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    })
}

fn insert(s: &mut Session, track: &project::TrackId) -> DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.bloom".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    fx
}

#[test]
fn bloom_goes_after_the_synth_and_each_parameter_is_set_by_name_and_undone() {
    let f = fixture();
    let insert_cmd = Command::Device(device::Command::Insert {
        track: Some(f.track.clone()),
        master: false,
        kind: "faulk.bloom".into(),
        index: 0,
    });
    check(&insert_cmd, &f.state, &context()).unwrap();
    let mut s = Session::from_state(f.state.clone(), context());
    let fx = insert(&mut s, &f.track);
    let chain = s.state().project.tracks[&f.track].devices_in_order();
    assert_eq!(chain[0].0, &f.device, "the synth comes first");
    assert_eq!(chain[1].0, &fx);
    assert_eq!(chain[1].1.role, DeviceRole::Effect);
    let value_of = |s: &Session, key: &str, default: f64| {
        s.state().project.tracks[&f.track].devices[&fx]
            .params
            .get(key)
            .copied()
            .unwrap_or(default)
    };
    for p in DESCRIPTOR.params {
        for value in [p.min, p.max, p.min + (p.max - p.min) * 0.37] {
            let cmd = set(&fx, p.key, value);
            // An edit must change something: when the value is already there (a default), the
            // parameter is moved to its other bound first, so every value is checked.
            if p.sanitize_f64(value) == Some(value_of(&s, p.key, p.default)) {
                let other = if value == p.max { p.min } else { p.max };
                s.run(set(&fx, p.key, other));
            }
            let before = value_of(&s, p.key, p.default);
            check(&cmd, s.state(), s.ctx()).unwrap_or_else(|e| panic!("{} = {value}: {e}", p.key));
            let out = s.exec(cmd);
            let Output::Device(device::Output::SetParam { value: stored, .. }) = out else {
                panic!("{out:?}")
            };
            assert_eq!(Some(stored), p.sanitize_f64(value), "{} = {value}", p.key);
            assert_eq!(value_of(&s, p.key, p.default), stored);
            // The undo brings the previous value back, the redo the new one.
            s.run(Command::History(history::Command::Undo {}));
            assert_eq!(value_of(&s, p.key, p.default), before, "undo of {}", p.key);
            s.run(Command::History(history::Command::Redo {}));
            assert_eq!(value_of(&s, p.key, p.default), stored, "redo of {}", p.key);
        }
    }
    // An unknown parameter is refused.
    assert!(commands::testing::check_refusal(&set(&fx, "speed", 1.0), s.state(), s.ctx()).is_ok());
}

/// One part of the render: an A minor pad chord held 2.6 s, then 0.6 s of release.
const PART: f32 = 3.2;

fn chord(s: &mut Session, track: &project::TrackId) {
    for note in [57u8, 60, 64, 69] {
        s.run(Command::Live(live::Command::NoteOn {
            track: track.clone(),
            note,
            velocity: 96,
        }));
    }
    s.render_secs(2.6);
    s.run(Command::Live(live::Command::AllNotesOff {}));
    s.render_secs(PART - 2.6);
}

fn part(r: &Render, k: usize) -> Render {
    let frames = (PART * r.sample_rate) as usize;
    let (from, to) = (k * frames, ((k + 1) * frames).min(r.left.len()));
    Render::new(
        r.sample_rate,
        r.left[from..to].to_vec(),
        r.right[from..to].to_vec(),
    )
}

fn correlation(r: &Render, from: f32, to: f32) -> f64 {
    let (a, b) = (
        (from * r.sample_rate) as usize,
        (to * r.sample_rate) as usize,
    );
    let (l, rr) = (&r.left[a..b], &r.right[a..b]);
    let n = l.len() as f64;
    let ml = l.iter().map(|x| f64::from(*x)).sum::<f64>() / n;
    let mr = rr.iter().map(|x| f64::from(*x)).sum::<f64>() / n;
    let (mut lr, mut ll, mut r2) = (0.0, 0.0, 0.0);
    for (x, y) in l.iter().zip(rr) {
        let (x, y) = (f64::from(*x) - ml, f64::from(*y) - mr);
        lr += x * y;
        ll += x * x;
        r2 += y * y;
    }
    lr / (ll * r2).sqrt()
}

#[test]
fn a_pad_dry_through_a_soft_chorus_and_a_wide_ensemble_natively_and_in_wasm() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    // A soft saw pad (the factory's Soft Pad, a little quieter: four voices).
    for (param, value) in [
        ("waveform", 1.0),
        ("attack", 0.4),
        ("decay", 1.5),
        ("sustain", 0.8),
        ("release", 0.5),
        ("gain", 0.35),
    ] {
        s.run(set(&f.device, param, value));
    }
    let fx = insert(&mut s, &f.track);
    s.run(Command::Transport(transport::Command::Play {}));
    // 1. Dry: Bloom bypassed.
    s.run(Command::Device(device::Command::SetBypass {
        device: fx.clone(),
        bypass: true,
    }));
    chord(&mut s, &f.track);
    // 2. A soft chorus (the Soft Chorus preset's settings).
    s.run(Command::Device(device::Command::SetBypass {
        device: fx.clone(),
        bypass: false,
    }));
    for (param, value) in [
        ("mode", 0.0),
        ("rate", 0.6),
        ("depth", 35.0),
        ("delay", 0.014),
        ("stereo_phase", 90.0),
        ("tone", 150.0),
        ("mix", 0.45),
    ] {
        s.run(set(&fx, param, value));
    }
    chord(&mut s, &f.track);
    // 3. A wide ensemble, synced to a whole note (the Wide Ensemble preset, on the tempo).
    for (param, value) in [
        ("mode", 1.0),
        ("sync", 1.0),
        ("division", 9.0),
        ("depth", 45.0),
        ("delay", 0.01),
        ("stereo_phase", 180.0),
        ("tone", 200.0),
        ("mix", 0.6),
    ] {
        s.run(set(&fx, param, value));
    }
    chord(&mut s, &f.track);

    let r = parity(s.scenario());
    r.assert_clean();
    let (dry, chorus, ensemble) = (part(&r, 0), part(&r, 1), part(&r, 2));
    for p in [&dry, &chorus, &ensemble] {
        p.assert_clean();
        p.assert_audible(0.5..2.5, 0.02);
    }
    let level = |p: &Render| p.rms_in(0.8..2.5);
    let width = |p: &Render| correlation(p, 0.8, 2.5);
    let db = |a: f32, b: f32| 20.0 * (a / b).log10();
    println!(
        "dry: rms {:.4}, correlation {:.3}\nchorus: rms {:.4} ({:+.1} dB), correlation {:.3}\nensemble: rms {:.4} ({:+.1} dB), correlation {:.3}",
        level(&dry),
        width(&dry),
        level(&chorus),
        db(level(&chorus), level(&dry)),
        width(&chorus),
        level(&ensemble),
        db(level(&ensemble), level(&dry)),
        width(&ensemble),
    );
    // The fixture's synth is mono: the dry pad has both channels alike.
    assert!(width(&dry) > 0.999, "{}", width(&dry));
    assert!(width(&chorus) < 0.8, "{}", width(&chorus));
    assert!(width(&ensemble) < 0.8, "{}", width(&ensemble));
    // The chorus's equal-power mix holds the level; the ensemble's wet signal is the mean of
    // three unrelated voices (a third of the power), so at mix 0.6 it sits a few dB lower.
    let chorus_db = db(level(&chorus), level(&dry));
    let ensemble_db = db(level(&ensemble), level(&dry));
    assert!(
        chorus_db.abs() < 1.5,
        "chorus level moves by {chorus_db} dB"
    );
    assert!(
        (-5.0..0.0).contains(&ensemble_db),
        "ensemble level moves by {ensemble_db} dB"
    );
    r.write_wav("bloom");
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.qa/696");
    std::fs::create_dir_all(&dir).expect("the folder is created");
    for (name, p) in [
        ("bloom-pad-dry", &dry),
        ("bloom-pad-soft-chorus", &chorus),
        ("bloom-pad-wide-ensemble", &ensemble),
    ] {
        std::fs::write(dir.join(format!("{name}.wav")), p.wav()).expect("the WAV is written");
    }
}
