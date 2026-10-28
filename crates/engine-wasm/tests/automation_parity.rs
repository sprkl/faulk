//! Automation playback (AM-2) renders the same natively and in WASM: a project built
//! through the commands (`automation.lane.create`, `automation.points.add`) with a gain, a
//! pan and a synth parameter lane, played, looped, sought, stopped and played again, under
//! a fader gesture, rendered by both builds.

use commands::{Command, automation, mixer, transport};
use project::{AutomationTarget, MixerParam, Shape, Tick, TickLoop};
use test_support::{Session, parity};

fn point(tick: i64, value: f64, shape: Shape) -> automation::Point {
    automation::Point {
        tick: Tick(tick),
        value,
        shape,
    }
}

fn lane(s: &mut Session, target: AutomationTarget, points: Vec<automation::Point>) {
    let track = s.state().project.tracks.keys().next().unwrap().clone();
    let out = s.exec(Command::Automation(automation::Command::LaneCreate {
        track,
        target,
    }));
    let commands::Output::Automation(automation::Output::LaneCreate { lane }) = out else {
        panic!("{out:?}");
    };
    s.run(Command::Automation(automation::Command::PointsAdd {
        lane,
        points,
    }));
}

#[test]
fn automation_renders_the_same_natively_and_in_wasm() {
    let mut s = Session::single_synth();
    let track = s.state().project.tracks.keys().next().unwrap().clone();
    let synth = s.state().project.tracks[&track]
        .devices
        .keys()
        .next()
        .unwrap()
        .clone();
    // A bar from −96 dB to 0 dB, a pan sweep, and the synth's gain in steps and ramps.
    lane(
        &mut s,
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        },
        vec![
            point(0, -96.0, Shape::Linear),
            point(3840, 0.0, Shape::Linear),
        ],
    );
    lane(
        &mut s,
        AutomationTarget::Mixer {
            param: MixerParam::Pan,
        },
        vec![
            point(0, -1.0, Shape::Linear),
            point(1920, 1.0, Shape::Linear),
            point(3840, 0.0, Shape::Step),
        ],
    );
    lane(
        &mut s,
        AutomationTarget::Device {
            device: synth,
            param: "gain".into(),
        },
        (0..16)
            .map(|i| {
                let shape = if i % 2 == 0 {
                    Shape::Step
                } else {
                    Shape::Linear
                };
                point(i * 240, 0.2 + (i % 5) as f64 * 0.15, shape)
            })
            .collect(),
    );
    s.note_on(57, 110).note_on(64, 100).render_secs(0.05);
    s.run(Command::Transport(transport::Command::SetLoop {
        loop_region: Some(TickLoop {
            start: Tick(960),
            end: Tick(3000),
        }),
    }));
    s.run(Command::Transport(transport::Command::Play {}))
        .render_secs(1.6);
    s.run(Command::Transport(transport::Command::Seek {
        tick: Tick(1500),
    }))
    .render_secs(0.4);
    // A fader gesture holds the gain lane until its next point.
    s.run(Command::Mixer(mixer::Command::SetVolume {
        track: track.clone(),
        gain_db: Some(-3.0),
    }))
    .render_secs(0.4);
    s.run(Command::Transport(transport::Command::Stop {}))
        .render_secs(0.2);
    s.run(Command::Transport(transport::Command::Play {}))
        .render_secs(0.6);
    let r = parity(s.scenario());
    r.assert_clean();
    // The gain lane plays: the play starts it at −96 dB, from the fader's 0 dB.
    let before = r.rms_in(0.0..0.05);
    let after = r.rms_in(0.06..0.08);
    assert!(
        before > 0.01 && after < before * 0.1,
        "{before} then {after}"
    );
    r.assert_audible(0.2..3.0, 0.01);
}
