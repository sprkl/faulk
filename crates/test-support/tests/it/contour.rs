//! #156 through the command layer and the engine: `device.insert` puts the equaliser Contour
//! after the synth of a track, `device.set_param` (by key) moves every band, `device.set_bypass`
//! makes it transparent, and an automation lane on `bell2_freq` sweeps a narrow +12 dB bell fast
//! over a held saw. The renders are the same natively and in WASM (`parity`), clean, and the
//! sweep is written to `.qa/renders/contour-sweep.wav` for the ear. (The device itself is
//! tested in `crates/engine/tests/it/contour.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, automation, device, live, transport};
use project::{AutomationTarget, DeviceId, Shape, Tick};
use test_support::{Session, parity};

/// Ticks per second at 120 BPM (960 per quarter note, two quarter notes per second).
const TICKS_PER_SECOND: i64 = 1920;

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

fn insert(s: &mut Session, track: &project::TrackId) -> DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.contour".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    fx
}

fn note(s: &mut Session, track: &project::TrackId, key: u8) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note: key,
        velocity: 110,
    }));
}

#[test]
fn a_fast_bell_sweep_by_automation_is_clean_and_the_same_in_native_and_wasm() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let fx = insert(&mut s, &f.track);
    set(&mut s, &fx, "bell2_gain", 12.0);
    set(&mut s, &fx, "bell2_q", 8.0);
    set(&mut s, &fx, "hp_on", 1.0);
    set(&mut s, &fx, "hp_freq", 60.0);
    set(&mut s, &fx, "lp_on", 1.0);
    set(&mut s, &fx, "lp_freq", 12_000.0);
    set(&mut s, &fx, "output", -9.0);
    let Output::Automation(automation::Output::LaneCreate { lane }) =
        s.exec(Command::Automation(automation::Command::LaneCreate {
            track: Some(f.track.clone()),
            master: false,
            target: AutomationTarget::Device {
                device: fx.clone(),
                param: "bell2_freq".into(),
            },
        }))
    else {
        panic!("a lane")
    };
    // Up and down between 100 Hz and 10 kHz every quarter of a second, for 4 s.
    let points: Vec<automation::Point> = (0..=16)
        .map(|i| automation::Point {
            tick: Tick(i * TICKS_PER_SECOND / 4),
            value: if i % 2 == 0 { 100.0 } else { 10_000.0 },
            shape: Shape::Linear,
        })
        .collect();
    s.run(Command::Automation(automation::Command::PointsAdd {
        lane,
        points,
    }));
    s.run(Command::Transport(transport::Command::Play {}));
    note(&mut s, &f.track, 45);
    s.render_secs(4.0);

    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.5..4.0, 0.01);
    r.write_wav("contour-sweep");
}

#[test]
fn a_bell_lifts_its_band_and_bypass_or_flat_settings_change_nothing() {
    let f = fixture();
    let render = |setup: &dyn Fn(&mut Session, &DeviceId)| {
        let mut s = Session::from_state(f.state.clone(), context());
        let fx = insert(&mut s, &f.track);
        setup(&mut s, &fx);
        note(&mut s, &f.track, 45);
        s.render_secs(1.0);
        parity(s.scenario())
    };
    let plain = {
        let mut s = Session::from_state(f.state.clone(), context());
        note(&mut s, &f.track, 45);
        s.render_secs(1.0);
        parity(s.scenario())
    };
    // At its defaults, Contour gives out its input: the same render as without it.
    let flat = render(&|_, _| {});
    plain.assert_close(&flat, 1e-6);
    // A bell of +12 dB at 2 kHz, bypassed: the same again.
    let bypassed = render(&|s, fx| {
        set(s, fx, "bell2_freq", 2000.0);
        set(s, fx, "bell2_gain", 12.0);
        s.run(Command::Device(device::Command::SetBypass {
            device: fx.clone(),
            bypass: true,
        }));
    });
    plain.assert_close(&bypassed, 1e-6);
    // Working: the harmonics around 2 kHz gain about 12 dB, those far below do not.
    let boosted = render(&|s, fx| {
        set(s, fx, "bell2_freq", 2000.0);
        set(s, fx, "bell2_gain", 12.0);
        set(s, fx, "bell2_q", 2.0);
    });
    boosted.assert_clean();
    let band = |r: &test_support::Render, lo: f32, hi: f32| r.spectrum(0.3..1.0).peak_in(lo..hi);
    let lift_db = 20.0 * (band(&boosted, 1900.0, 2100.0) / band(&plain, 1900.0, 2100.0)).log10();
    let low_db = 20.0 * (band(&boosted, 100.0, 300.0) / band(&plain, 100.0, 300.0)).log10();
    println!("bell at 2 kHz: +{lift_db:.2} dB around it, {low_db:+.2} dB at 100-300 Hz");
    assert!((10.5..13.0).contains(&lift_db), "lift {lift_db}");
    assert!(low_db.abs() < 0.5, "low band {low_db}");
}
