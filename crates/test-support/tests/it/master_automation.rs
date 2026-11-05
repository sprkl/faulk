//! #378 through the command layer and the engine: a lane on the master (`automation.lane.create`
//! with `master: true`) drives the master's volume and the parameters of its chain of effects;
//! the render is the same natively and in WASM, and a volume fade follows the fader's law bar
//! by bar.

use commands::{Command, Output, automation, device, transport};
use project::{AutomationTarget, DeviceId, MixerParam, Shape, Tick};
use test_support::{Session, parity, render_native};

fn point(tick: i64, value: f64, shape: Shape) -> automation::Point {
    automation::Point {
        tick: Tick(tick),
        value,
        shape,
    }
}

fn master_lane(s: &mut Session, target: AutomationTarget, points: Vec<automation::Point>) {
    let out = s.exec(Command::Automation(automation::Command::LaneCreate {
        track: None,
        master: true,
        target,
    }));
    let Output::Automation(automation::Output::LaneCreate { lane }) = out else {
        panic!("{out:?}");
    };
    s.run(Command::Automation(automation::Command::PointsAdd {
        lane,
        points,
    }));
}

fn insert_master(s: &mut Session, kind: &str) -> DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: None,
        master: true,
        kind: kind.into(),
        index: u32::MAX,
    }));
    let Output::Device(device::Output::Insert { device }) = out else {
        panic!("{out:?}")
    };
    device
}

/// At 240 BPM a bar is one second and a tick 1/960 s. A held note, 0.3 s of silence-free
/// lead-in, then the transport plays for `secs`; `lane` prepares the session.
fn song(secs: f32, lane: impl Fn(&mut Session)) -> Session {
    let mut s = Session::single_synth();
    s.run(Command::Transport(transport::Command::SetTempo {
        bpm: 240.0,
        placements: None,
    }));
    lane(&mut s);
    s.note_on(57, 110).render_secs(0.3);
    s.run(Command::Transport(transport::Command::Play {}))
        .render_secs(secs);
    s
}

/// The fader's law, from the engine: the dB at a position of the travel.
fn law_db(position: f64) -> f64 {
    20.0 * f64::from(engine::fader::gain_at(position)).log10()
}

#[test]
fn a_master_volume_fade_made_with_commands_goes_down_bar_by_bar_like_the_fader_law() {
    const BARS: i64 = 4;
    let faded = song(5.0, |s| {
        master_lane(
            s,
            AutomationTarget::Mixer {
                param: MixerParam::Gain,
            },
            vec![
                point(0, 0.0, Shape::Linear),
                point(BARS * 3840, -96.0, Shape::Linear),
            ],
        );
    });
    let reference = render_native(song(5.0, |_| {}).scenario());
    let faded = parity(faded.scenario());
    faded.assert_clean();
    let window = |bar: f32| (0.3 + bar - 0.01)..(0.3 + bar + 0.01);
    for bar in 1..BARS {
        let want = law_db(0.78 * (1.0 - bar as f64 / BARS as f64));
        let got = 20.0
            * f64::from(faded.rms_in(window(bar as f32)) / reference.rms_in(window(bar as f32)))
                .log10();
        println!("bar {bar}: {got:.2} dB (law {want:.2} dB)");
        assert!(
            (got - want).abs() < 0.5,
            "bar {bar}: {got} dB instead of {want} dB"
        );
    }
    // Silence from the end of the fade, exactly.
    faded.assert_silent(4.4..5.2);
    // The first bar is at full level.
    let start = faded.rms_in(0.31..0.33) / reference.rms_in(0.31..0.33);
    assert!((20.0 * f64::from(start).log10()).abs() < 0.2, "{start}");
}

#[test]
fn a_master_effect_lane_changes_the_sound_in_both_targets() {
    // The compressor squeezes the held note at a -40 dB threshold; a lane opens the threshold
    // to 0 dB at the third bar, where the squeeze stops.
    let treated = |lane: bool| {
        song(4.0, |s| {
            let fx = insert_master(s, "faulk.compressor");
            for (param, value) in [("threshold", -40.0), ("ratio", 20.0), ("attack", 0.0005)] {
                s.run(Command::Device(device::Command::SetParam {
                    device: fx.clone(),
                    param: param.into(),
                    value,
                }));
            }
            if lane {
                master_lane(
                    s,
                    AutomationTarget::Device {
                        device: fx,
                        param: "threshold".into(),
                    },
                    vec![
                        point(0, -40.0, Shape::Step),
                        point(2 * 3840, 0.0, Shape::Step),
                    ],
                );
            }
        })
    };
    let still = render_native(treated(false).scenario());
    let moving = parity(treated(true).scenario());
    moving.assert_clean();
    // Before the lane's step the two renders are the same; after it the lane lets go of the
    // squeeze and the note is louder than the squeezed one.
    let early = moving.rms_in(0.6..1.0) / still.rms_in(0.6..1.0);
    assert!((early - 1.0).abs() < 0.01, "before the step: {early}");
    let late = moving.rms_in(2.6..3.2) / still.rms_in(2.6..3.2);
    println!("after the step: x{late:.2}");
    assert!(late > 1.5, "the threshold lane did not act: x{late}");
}

#[test]
fn a_master_volume_gesture_holds_the_master_lane_until_its_next_point_and_native_equals_wasm() {
    // 0 dB, then -30 dB at the second bar (2 s of playing). A `mixer.set_master_volume` at 0.8 s
    // of playing holds the lane: the fader's -12 dB sounds until the lane's next point.
    let mut s = song(0.8, |s| {
        master_lane(
            s,
            AutomationTarget::Mixer {
                param: MixerParam::Gain,
            },
            vec![
                point(0, 0.0, Shape::Step),
                point(2 * 3840, -30.0, Shape::Step),
            ],
        );
    });
    s.run(Command::Mixer(commands::mixer::Command::SetMasterVolume {
        gain_db: Some(-12.0),
    }))
    .render_secs(2.4);
    let held = parity(s.scenario());
    held.assert_clean();
    let reference = render_native(song(3.2, |_| {}).scenario());
    let db = |window: std::ops::Range<f32>| {
        20.0 * f64::from(held.rms_in(window.clone()) / reference.rms_in(window)).log10()
    };
    // Before the gesture the lane's 0 dB; then the fader's -12 dB, held through the lane's
    // next point at 2.3 s of the render (0.3 s lead-in, 2 s of the second bar)... the lane
    // takes over at that point: -30 dB.
    let before = db(0.5..0.9);
    let under = db(1.3..2.2);
    let after = db(2.5..3.1);
    println!("before {before:.2} dB, held {under:.2} dB, after the point {after:.2} dB");
    assert!(before.abs() < 0.3, "{before}");
    assert!((under + 12.0).abs() < 0.5, "{under}");
    assert!((after + 30.0).abs() < 0.8, "{after}");
}
