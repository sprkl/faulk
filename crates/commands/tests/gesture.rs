//! #417: a manual gesture that leaves a control as the project holds it. The command drops it
//! as unchanged (no state change, no history entry) but, when an automation lane with points
//! drives the control, reports `GestureUnchanged` so that the lane gives way to the gesture,
//! and the engine messages say so: `HoldMix` for a fader or a pan (a `TrackSetMix` that moves
//! nothing looks like a mute and holds nothing), the `ParamSet` of the stored value for a
//! device parameter. A mute never holds a lane.

use ::project::{AutomationTarget, DeviceId, LaneId, MixerParam, Shape, TrackId};
use commands::engine_msgs::Synced;
use commands::testing::context;
use commands::{Command, ControlMsg, ProjectEvent, Undo, automation, device, mixer};
use engine::msg::TrackSlot;

mod automation_examples;
mod clip_examples;
mod device_examples;

use automation_examples::{
    a, add, create, device_target, fx, gain_target, more_context, pan_target, pt,
};
use device_examples::d;

struct World {
    s: Synced,
    track: TrackId,
    effect: DeviceId,
    master_effect: DeviceId,
}

/// The fixture's track: a gain lane and a lane on effect `a`'s `gain` (both with points), a pan
/// lane without a point. The master: a gain lane, a pan lane and a lane on its effect `mb`'s
/// `gain`, all with points. Synced with an engine by a snapshot.
fn world() -> World {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = more_context();
    create(&mut state, &mut ctx, &f.track, pan_target());
    for target in [
        gain_target(),
        pan_target(),
        device_target(&f.chain.mb, "gain"),
    ] {
        let out = commands::run(
            &a(automation::Command::LaneCreate {
                track: None,
                master: true,
                target,
            }),
            &mut state,
            &mut ctx,
        )
        .unwrap();
        let commands::Output::Automation(automation::Output::LaneCreate { lane }) = out.output
        else {
            panic!("{:?}", out.output)
        };
        add(&mut state, &mut ctx, &lane, vec![pt(0, 0.0, Shape::Step)]);
    }
    let mut s = Synced::new(state, context(), 1);
    s.setup().unwrap();
    World {
        s,
        track: f.track.clone(),
        effect: f.chain.a.clone(),
        master_effect: f.chain.mb.clone(),
    }
}

impl World {
    fn run(&mut self, command: Command) -> commands::Outcome {
        self.s.run(&command).unwrap().0
    }

    fn volume(&self) -> Option<f64> {
        let db = self.s.state.project.tracks[&self.track].mixer.gain_db;
        (db != f64::NEG_INFINITY).then_some(db)
    }

    fn track_lane(&self, target: &AutomationTarget) -> LaneId {
        self.s.state.project.tracks[&self.track]
            .automation
            .iter()
            .find(|(_, l)| l.target == *target)
            .map(|(id, _)| id.clone())
            .unwrap()
    }
}

fn mx(command: mixer::Command) -> Command {
    Command::Mixer(command)
}

fn set_param(device: &DeviceId, param: &str, value: f64) -> Command {
    d(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    })
}

fn mixer_target(param: MixerParam) -> AutomationTarget {
    AutomationTarget::Mixer { param }
}

fn gain_of(db: Option<f64>) -> f32 {
    commands::engine_msgs::db_to_gain(db.unwrap_or(f64::NEG_INFINITY))
}

#[test]
fn a_volume_set_to_its_stored_value_under_a_lane_reports_the_gesture_and_holds_the_lane() {
    let mut w = world();
    let stored = w.volume();
    let before = w.s.state.clone();
    let out = w.run(mx(mixer::Command::SetVolume {
        track: w.track.clone(),
        gain_db: stored,
    }));
    assert_eq!(
        out.events,
        [ProjectEvent::GestureUnchanged {
            track: Some(w.track.clone()),
            target: mixer_target(MixerParam::Gain),
        }]
    );
    assert_eq!(out.undo, Undo::None, "no history entry");
    assert_eq!(
        out.control,
        [ControlMsg::HoldMix {
            track: TrackSlot(0),
            gain: Some(gain_of(stored)),
            pan: None,
        }]
    );
    assert_eq!(
        w.s.state.project, before.project,
        "the project is as it was"
    );
    assert_eq!(
        w.s.state.history, before.history,
        "the history is as it was"
    );
    assert!(
        out.events.iter().all(|e| !e.changes_project()),
        "the project is not edited"
    );
}

#[test]
fn a_changed_volume_reports_the_change_only() {
    let mut w = world();
    let out = w.run(mx(mixer::Command::SetVolume {
        track: w.track.clone(),
        gain_db: Some(-3.0),
    }));
    assert!(matches!(
        out.events.as_slice(),
        [
            ProjectEvent::MixerChanged { .. },
            ProjectEvent::HistoryChanged { .. }
        ]
    ));
    assert!(matches!(
        out.control.as_slice(),
        [ControlMsg::TrackSetMix { .. }]
    ));
}

#[test]
fn a_pan_under_a_lane_that_has_no_point_is_silent_as_before() {
    let mut w = world();
    let out = w.run(mx(mixer::Command::SetPan {
        track: w.track.clone(),
        pan: 0.0,
    }));
    assert!(out.events.is_empty() && out.control.is_empty(), "{out:?}");
    assert_eq!(out.undo, Undo::None);
}

#[test]
fn a_pan_set_to_its_stored_value_under_a_pan_lane_with_points_holds_the_pan_only() {
    let mut w = world();
    let lane = w.track_lane(&pan_target());
    // The first point makes the lane drive the pan.
    let out = w.run(a(automation::Command::PointsAdd {
        lane,
        points: vec![pt(0, 0.0, Shape::Step)],
    }));
    assert!(!out.control.is_empty());
    let out = w.run(mx(mixer::Command::SetPan {
        track: w.track.clone(),
        pan: 0.0,
    }));
    assert_eq!(
        out.events,
        [ProjectEvent::GestureUnchanged {
            track: Some(w.track.clone()),
            target: mixer_target(MixerParam::Pan),
        }]
    );
    assert_eq!(
        out.control,
        [ControlMsg::HoldMix {
            track: TrackSlot(0),
            gain: None,
            pan: Some(0.0),
        }]
    );
}

#[test]
fn a_master_volume_and_pan_set_to_their_stored_values_hold_the_master_lanes() {
    let mut w = world();
    let out = w.run(mx(mixer::Command::SetMasterVolume { gain_db: Some(0.0) }));
    assert_eq!(
        out.events,
        [ProjectEvent::GestureUnchanged {
            track: None,
            target: mixer_target(MixerParam::Gain),
        }]
    );
    assert_eq!(
        out.control,
        [ControlMsg::HoldMix {
            track: TrackSlot::MASTER,
            gain: Some(1.0),
            pan: None,
        }]
    );
    let out = w.run(mx(mixer::Command::SetMasterPan { pan: 0.0 }));
    assert_eq!(
        out.control,
        [ControlMsg::HoldMix {
            track: TrackSlot::MASTER,
            gain: None,
            pan: Some(0.0),
        }]
    );
    assert_eq!(out.undo, Undo::None);
}

#[test]
fn a_mute_never_holds_a_lane_on_a_track() {
    let mut w = world();
    // A mute that repeats the stored one, and ones that change it.
    for mute in [false, true, true, false] {
        let out = w.run(mx(mixer::Command::SetMute {
            track: w.track.clone(),
            mute,
        }));
        assert!(
            out.events
                .iter()
                .all(|e| !matches!(e, ProjectEvent::GestureUnchanged { .. })),
            "{out:?}"
        );
        assert!(
            out.control
                .iter()
                .all(|m| !matches!(m, ControlMsg::HoldMix { .. })),
            "{out:?}"
        );
    }
}

#[test]
fn a_changed_master_volume_sends_the_whole_master_and_no_hold() {
    // The master has no mute command: its mute travels in `SetMaster` only, which holds
    // nothing in the engine; a change of the volume sends no `HoldMix`.
    let mut w = world();
    let out = w.run(mx(mixer::Command::SetMasterVolume {
        gain_db: Some(-1.0),
    }));
    assert!(matches!(
        out.control.as_slice(),
        [ControlMsg::SetMaster { mute: false, .. }]
    ));
}

#[test]
fn a_device_parameter_set_to_its_stored_value_under_a_lane_sends_it_again() {
    let mut w = world();
    let stored = w.s.state.project.tracks[&w.track].devices[&w.effect].params["gain"];
    let out = w.run(set_param(&w.effect.clone(), "gain", stored));
    assert_eq!(
        out.events,
        [ProjectEvent::GestureUnchanged {
            track: Some(w.track.clone()),
            target: device_target(&w.effect, "gain"),
        }]
    );
    assert_eq!(out.undo, Undo::None);
    let [ControlMsg::ParamSet { track, value, .. }] = out.control.as_slice() else {
        panic!("{:?}", out.control)
    };
    assert_eq!(*track, TrackSlot(0));
    assert_eq!(*value, stored as f32);
}

#[test]
fn a_device_value_clamped_to_the_stored_one_is_an_unchanged_gesture_too() {
    let mut w = world();
    // Far above the range: the first clamps to the maximum (a change), the second finds it there.
    w.run(set_param(&w.effect.clone(), "gain", 1e6));
    let out = w.run(set_param(&w.effect.clone(), "gain", 2e6));
    assert!(matches!(
        out.events.as_slice(),
        [ProjectEvent::GestureUnchanged { .. }]
    ));
    assert!(matches!(
        out.control.as_slice(),
        [ControlMsg::ParamSet { .. }]
    ));
}

#[test]
fn a_master_effect_parameter_set_to_its_stored_value_holds_through_the_master() {
    let mut w = world();
    let stored = w.s.state.project.master_devices[&w.master_effect].params["gain"];
    let out = w.run(set_param(&w.master_effect.clone(), "gain", stored));
    assert_eq!(
        out.events,
        [ProjectEvent::GestureUnchanged {
            track: None,
            target: device_target(&w.master_effect, "gain"),
        }]
    );
    let [ControlMsg::ParamSet { track, .. }] = out.control.as_slice() else {
        panic!("{:?}", out.control)
    };
    assert!(track.is_master());
}

#[test]
fn a_parameter_no_lane_drives_stays_silent_as_before() {
    let mut w = world();
    let stored = w.s.state.project.tracks[&w.track].devices[&w.effect].params["mute"];
    let out = w.run(set_param(&w.effect.clone(), "mute", stored));
    assert!(out.events.is_empty() && out.control.is_empty(), "{out:?}");
}

#[test]
fn a_lane_cleared_of_its_points_no_longer_reports_the_gesture() {
    let mut w = world();
    let lane = w.track_lane(&gain_target());
    w.run(a(automation::Command::LaneClear { lane }));
    let out = w.run(mx(mixer::Command::SetVolume {
        track: w.track.clone(),
        gain_db: w.volume(),
    }));
    assert!(out.events.is_empty() && out.control.is_empty(), "{out:?}");
}
