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
        track: Some(track),
        master: false,
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
        enabled: None,
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
    // The gain lane applies at the stopped playhead (#388): it starts at −96 dB, so the held
    // notes are silent before the play, and they sound later in the scenario.
    assert!(r.rms_in(0.0..0.05) < 1e-4);
    r.assert_audible(0.2..3.0, 0.01);
}

/// A stop and a seek while stopped apply the lanes at the playhead (#388), the same natively
/// and in WASM: a volume lane steps from -24 dB to 0 dB at the second bar, and the held
/// notes follow the playhead, then a fader gesture while stopped holds until the next seek.
#[test]
fn a_stop_and_a_stopped_seek_apply_the_lanes_in_both_targets() {
    let mut s = Session::single_synth();
    let track = s.state().project.tracks.keys().next().unwrap().clone();
    lane(
        &mut s,
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        },
        vec![point(0, -24.0, Shape::Step), point(3840, 0.0, Shape::Step)],
    );
    s.note_on(57, 110).render_secs(0.5);
    // Stopped at the start: -24 dB.
    s.run(Command::Transport(transport::Command::Seek {
        tick: Tick(3840),
    }))
    .render_secs(0.5);
    // Stopped at the second bar: 0 dB.
    s.run(Command::Transport(transport::Command::Play {}))
        .render_secs(0.5);
    s.run(Command::Transport(transport::Command::Stop {}))
        .render_secs(0.5);
    // The stop applies the lane where the clock stands (still the second bar), a seek back
    // applies the first value, a fader gesture holds, and a seek to the same tick lets go.
    s.run(Command::Transport(transport::Command::Seek {
        tick: Tick(0),
    }))
    .render_secs(0.5);
    s.run(Command::Mixer(mixer::Command::SetVolume {
        track,
        gain_db: Some(-6.0),
    }))
    .render_secs(0.5);
    s.run(Command::Transport(transport::Command::Seek {
        tick: Tick(0),
    }))
    .render_secs(0.5);
    let r = parity(s.scenario());
    r.assert_clean();
    let quiet = r.rms_in(0.2..0.5);
    let loud = r.rms_in(0.7..1.0);
    let db = 20.0 * (loud / quiet).log10();
    assert!((db - 24.0).abs() < 2.0, "{db} dB between the two bars");
    // The stop kept the second bar's value, the seek back went to -24 dB.
    let stopped = r.rms_in(1.7..2.0);
    assert!(
        (20.0 * (stopped / loud).log10()).abs() < 2.0,
        "{stopped} after the stop"
    );
    let back = r.rms_in(2.2..2.5);
    assert!(
        (20.0 * (back / quiet).log10()).abs() < 2.0,
        "{back} after the seek back"
    );
    // The gesture (-6 dB) holds, then the seek to the same tick gives the lane back.
    let held = r.rms_in(2.7..3.0);
    assert!(
        (20.0 * (held / quiet).log10() - 18.0).abs() < 2.0,
        "{held} under the gesture"
    );
    let lane_again = r.rms_in(3.2..3.5);
    assert!(
        (20.0 * (lane_again / quiet).log10()).abs() < 2.0,
        "{lane_again} after the second seek"
    );
}

/// A fader gesture that sets the value the project already holds (#417) is dropped by the
/// command as unchanged, yet it holds the lane: through the compiler (`HoldMix`) and the
/// engine, the strip goes back to its fader and stays there until the next seek, the same
/// natively and in WASM. A mute that repeats the fader holds nothing.
#[test]
fn an_unchanged_fader_gesture_holds_the_lane_in_both_targets() {
    let mut s = Session::single_synth();
    let track = s.state().project.tracks.keys().next().unwrap().clone();
    let stored = s.state().project.tracks[&track].mixer.gain_db;
    lane(
        &mut s,
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        },
        vec![point(0, -24.0, Shape::Step)],
    );
    s.note_on(57, 110).render_secs(0.5);
    // Stopped: the lane's -24 dB sounds. A mute on and off keeps the lane: nothing is held.
    s.run(Command::Mixer(mixer::Command::SetMute {
        track: track.clone(),
        mute: true,
    }))
    .render_secs(0.2);
    s.run(Command::Mixer(mixer::Command::SetMute {
        track: track.clone(),
        mute: false,
    }))
    .render_secs(0.5);
    // The same volume again: the project does not change, the lane gives way.
    s.run(Command::Mixer(mixer::Command::SetVolume {
        track,
        gain_db: Some(stored),
    }))
    .render_secs(0.5);
    // A seek gives the lane the hand back.
    s.run(Command::Transport(transport::Command::Seek {
        tick: Tick(0),
    }))
    .render_secs(0.5);
    let r = parity(s.scenario());
    r.assert_clean();
    let lane_level = r.rms_in(0.2..0.5);
    let after_mute = r.rms_in(1.0..1.2);
    assert!(
        (20.0 * (after_mute / lane_level).log10()).abs() < 2.0,
        "{after_mute} after the mute: the lane still plays"
    );
    let held = r.rms_in(1.4..1.7);
    assert!(
        (20.0 * (held / lane_level).log10() - 24.0).abs() < 2.0,
        "{held} under the unchanged gesture: the fader at 0 dB"
    );
    let back = r.rms_in(2.0..2.2);
    assert!(
        (20.0 * (back / lane_level).log10()).abs() < 2.0,
        "{back} after the seek"
    );
}

/// A two-bar fade from 0 dB to -96 dB (#389) is the same natively and in WASM, and it goes
/// down along the fader's travel: at the bar line the position is half way, 0.39, which the
/// taper makes -21 dB (the amplitude law would give -6 dB there, the dB law -48 dB).
#[test]
fn a_volume_fade_follows_the_fader_law_in_both_targets() {
    let mut s = Session::single_synth();
    lane(
        &mut s,
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        },
        vec![
            point(0, 0.0, Shape::Linear),
            point(7680, -96.0, Shape::Linear),
        ],
    );
    s.note_on(57, 110).render_secs(0.3);
    s.run(Command::Transport(transport::Command::Play {}))
        .render_secs(4.2);
    let r = parity(s.scenario());
    r.assert_clean();
    let start = r.rms_in(0.3..0.32);
    let middle = r.rms_in(2.29..2.31);
    let db = 20.0 * (middle / start).log10();
    // 0.39 of the travel is -21 dB; the start window sits 0.2 dB below unity.
    assert!((db + 20.8).abs() < 1.0, "{db} dB at the bar line");
    // The end of the fade is silence.
    assert!(r.rms_in(4.4..4.5) < 1e-6);
}

/// The curved shapes (#881) render the same natively and in WASM: a volume lane rises from
/// -60 dB to 0 dB over a bar four times, slow then fast, fast then slow, as an S and in a
/// straight line; then a pan lane and a synth parameter lane curve over bars 5 to 7, after
/// the bars measured. Half way through a bar, the slow start is quieter than the straight line
/// and the fast start louder, the S as loud as the line; at a quarter, the S is quieter.
#[test]
fn curved_lanes_render_the_same_natively_and_in_wasm() {
    let mut s = Session::single_synth();
    let track = s.state().project.tracks.keys().next().unwrap().clone();
    let synth = s.state().project.tracks[&track]
        .devices
        .keys()
        .next()
        .unwrap()
        .clone();
    let bar = 3840;
    let shapes = [Shape::EaseIn, Shape::EaseOut, Shape::Smooth, Shape::Linear];
    let mut gain = Vec::new();
    for (i, shape) in shapes.into_iter().enumerate() {
        let at = i as i64 * bar;
        gain.push(point(at, -60.0, shape));
        gain.push(point(at + bar - 1, 0.0, Shape::Step));
    }
    lane(
        &mut s,
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        },
        gain,
    );
    lane(
        &mut s,
        AutomationTarget::Mixer {
            param: MixerParam::Pan,
        },
        vec![
            point(4 * bar, -1.0, Shape::Smooth),
            point(5 * bar, 1.0, Shape::EaseOut),
            point(6 * bar, 0.0, Shape::Step),
        ],
    );
    lane(
        &mut s,
        AutomationTarget::Device {
            device: synth,
            param: "gain".into(),
        },
        vec![
            point(4 * bar, 0.2, Shape::EaseIn),
            point(6 * bar, 0.9, Shape::Step),
        ],
    );
    s.note_on(57, 110).note_on(64, 100);
    // 120 BPM: a bar is 2 s.
    s.run(Command::Transport(transport::Command::Play {}))
        .render_secs(12.5);
    let r = parity(s.scenario());
    r.assert_clean();
    // The level around a fraction of a bar (its own window of 0.1 s).
    let at = |bar: f32, part: f32| {
        r.rms_in(bar * 2.0 + part * 2.0 - 0.05..bar * 2.0 + part * 2.0 + 0.05)
    };
    let (ease_in, ease_out, smooth, linear) =
        (at(0.0, 0.5), at(1.0, 0.5), at(2.0, 0.5), at(3.0, 0.5));
    assert!(
        ease_in < 0.8 * linear && 1.2 * linear < ease_out,
        "{ease_in} {linear} {ease_out}"
    );
    assert!((smooth / linear - 1.0).abs() < 0.2, "{smooth} {linear}");
    assert!(
        at(2.0, 0.25) < 0.8 * at(3.0, 0.25),
        "the S starts slower than the line"
    );
    r.assert_audible(8.0..12.0, 0.01);
}
