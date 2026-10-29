//! DV-2 through the command layer and the engine: `device.insert` puts the filter after the
//! synth of a track, `device.set_param` (by name) moves it, `device.set_bypass` makes it
//! transparent, and an automation lane on its `cutoff` sweeps it. The render, a saw note
//! through the low-pass with the cutoff swept from 200 Hz to 8 kHz at a resonance of 0.7, is
//! the same natively and in WASM and is written to `.qa/renders/filter-sweep.wav` for the
//! ear. (The device itself is tested in `crates/engine/tests/filter.rs`.)

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
        kind: "faulk.filter".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    fx
}

/// How bright a window is: the rms of its first difference over its rms (a pure low note
/// is near 0, a buzzy one is high).
fn brightness(x: &[f32]) -> f32 {
    let rms = |v: &mut dyn Iterator<Item = f32>| {
        let (sum, n) = v.fold((0.0f32, 0usize), |(s, n), a| (s + a * a, n + 1));
        (sum / n as f32).sqrt()
    };
    let diff = rms(&mut x.windows(2).map(|w| w[1] - w[0]));
    diff / rms(&mut x.iter().copied()).max(1e-9)
}

#[test]
fn a_cutoff_lane_sweeps_the_low_pass_over_a_held_saw() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let fx = insert(&mut s, &f.track);
    set(&mut s, &fx, "mode", 0.0);
    set(&mut s, &fx, "resonance", 0.7);
    set(&mut s, &fx, "cutoff", 200.0);
    // Eight steps of an exponential sweep, 200 Hz to 8 kHz in 4 s (even to the ear).
    let Output::Automation(automation::Output::LaneCreate { lane }) =
        s.exec(Command::Automation(automation::Command::LaneCreate {
            track: f.track.clone(),
            target: AutomationTarget::Device {
                device: fx.clone(),
                param: "cutoff".into(),
            },
        }))
    else {
        panic!("a lane")
    };
    let points: Vec<automation::Point> = (0..=8)
        .map(|i| automation::Point {
            tick: Tick(i * TICKS_PER_SECOND / 2),
            value: 200.0 * 40f64.powf(i as f64 / 8.0),
            shape: Shape::Linear,
        })
        .collect();
    s.run(Command::Automation(automation::Command::PointsAdd {
        lane,
        points,
    }));
    s.run(Command::Transport(transport::Command::Play {}));
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 45,
        velocity: 110,
    }));
    s.render_secs(4.0);

    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.5..4.0, 0.01);
    let sr = r.sample_rate;
    let window = |from: f32| {
        let a = (from * sr) as usize;
        &r.left[a..a + (0.25 * sr) as usize]
    };
    let (early, late) = (brightness(window(0.25)), brightness(window(3.5)));
    println!("brightness: {early:.3} at 0.25 s, {late:.3} at 3.5 s");
    assert!(
        late > 3.0 * early,
        "the sweep opens the sound: {early} -> {late}"
    );
    r.write_wav("filter-sweep");
}

#[test]
fn bypassed_the_filter_changes_nothing_and_its_mix_blends() {
    let f = fixture();
    let render = |filter: Option<(&str, f64)>| {
        let mut s = Session::from_state(f.state.clone(), context());
        if let Some((param, value)) = filter {
            let fx = insert(&mut s, &f.track);
            set(&mut s, &fx, "cutoff", 300.0);
            if param == "bypass" {
                s.run(Command::Device(device::Command::SetBypass {
                    device: fx,
                    bypass: value != 0.0,
                }));
            } else {
                set(&mut s, &fx, param, value);
            }
        }
        s.run(Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note: 57,
            velocity: 100,
        }));
        s.render_secs(0.5);
        parity(s.scenario())
    };
    let plain = render(None);
    let bypassed = render(Some(("bypass", 1.0)));
    let dry = render(Some(("mix", 0.0)));
    let wet = render(Some(("mix", 1.0)));
    plain.assert_close(&bypassed, 1e-6);
    // The mix ramps for 20 ms from its default (1) to 0: the dry sound is exact after it.
    let from = (0.1 * plain.sample_rate) as usize;
    let tail = |r: &test_support::Render| {
        test_support::Render::new(r.sample_rate, r.left[from..].to_vec(), r.right[from..].to_vec())
    };
    tail(&plain).assert_close(&tail(&dry), 1e-6);
    let (diff, _) = plain.max_difference(&wet);
    assert!(
        diff > 0.05,
        "the filtered note differs from the dry one: {diff}"
    );
}
