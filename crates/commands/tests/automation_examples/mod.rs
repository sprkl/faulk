//! The state the automation commands run from, and one example of every command (and of
//! `device.remove` / `device.restore` with lanes). `tests/contract.rs` adds [`examples`] to
//! the catalog of examples (a command without one fails there); `tests/automation.rs` checks
//! the behaviour in depth.

#![allow(dead_code)]

use crate::clip_examples::after;
use crate::device_examples::{Chain, chain, d};
use ::project::{
    AutomationTarget, DeviceId, IdGenerator, LaneId, MixerParam, Shape, Tick, TrackId,
};
use commands::{Author, Command, Context, State, automation, device};

pub fn a(command: automation::Command) -> Command {
    Command::Automation(command)
}

/// A point.
pub fn pt(tick: i64, value: f64, shape: Shape) -> automation::Point {
    automation::Point {
        tick: Tick(tick),
        value,
        shape,
    }
}

pub fn gain_target() -> AutomationTarget {
    AutomationTarget::Mixer {
        param: MixerParam::Gain,
    }
}

pub fn pan_target() -> AutomationTarget {
    AutomationTarget::Mixer {
        param: MixerParam::Pan,
    }
}

pub fn device_target(device: &DeviceId, param: &str) -> AutomationTarget {
    AutomationTarget::Device {
        device: device.clone(),
        param: param.into(),
    }
}

/// A context whose ids never meet the ones of the chain or of the examples' `context()`.
pub fn lane_context() -> Context {
    Context::new(
        Author::User,
        IdGenerator::new(1_661_000_000_000, 0x00D1_0000_0000_0000_0000),
    )
}

/// A context for what a test creates after [`fx`]: ids apart from the fixture's.
pub fn more_context() -> Context {
    Context::new(
        Author::User,
        IdGenerator::new(1_662_000_000_000, 0x00D2_0000_0000_0000_0000),
    )
}

/// Creates a lane through the command and returns its id.
pub fn create(
    state: &mut State,
    ctx: &mut Context,
    track: &TrackId,
    target: AutomationTarget,
) -> LaneId {
    let out = commands::run(
        &a(automation::Command::LaneCreate {
            track: Some(track.clone()),
            master: false,
            target,
        }),
        state,
        ctx,
    )
    .unwrap();
    let commands::Output::Automation(automation::Output::LaneCreate { lane }) = out.output else {
        panic!("{:?}", out.output)
    };
    lane
}

/// Adds points through the command.
pub fn add(state: &mut State, ctx: &mut Context, lane: &LaneId, points: Vec<automation::Point>) {
    commands::run(
        &a(automation::Command::PointsAdd {
            lane: lane.clone(),
            points,
        }),
        state,
        ctx,
    )
    .unwrap();
}

/// The device fixture plus lanes: on the MIDI track, a gain lane with three points and a lane
/// on effect `a`'s `gain` with two; on effect `b`, none.
pub struct Lanes {
    pub chain: Chain,
    pub state: State,
    pub track: TrackId,
    /// The mixer gain lane: 0 at -6 dB linear, 960 at 0 dB step, 1920 at -12 dB linear.
    pub gain_lane: LaneId,
    /// The lane on effect `a`'s `gain`: 0 at 0.5 linear, 960 at 1.5 step.
    pub effect_lane: LaneId,
}

pub fn fx() -> Lanes {
    let c = chain();
    let mut state = c.state.clone();
    let mut ctx = lane_context();
    let gain_lane = create(&mut state, &mut ctx, &c.track, gain_target());
    add(
        &mut state,
        &mut ctx,
        &gain_lane,
        vec![
            pt(0, -6.0, Shape::Linear),
            pt(960, 0.0, Shape::Step),
            pt(1920, -12.0, Shape::Linear),
        ],
    );
    let effect_lane = create(&mut state, &mut ctx, &c.track, device_target(&c.a, "gain"));
    add(
        &mut state,
        &mut ctx,
        &effect_lane,
        vec![pt(0, 0.5, Shape::Linear), pt(960, 1.5, Shape::Step)],
    );
    state.project.validate().expect("the lane fixture is valid");
    // The lanes were made by commands: the fixture's history is not part of the examples.
    state.history = chain().state.history;
    Lanes {
        track: c.track.clone(),
        chain: c,
        state,
        gain_lane,
        effect_lane,
    }
}

pub fn examples() -> Vec<(Command, State)> {
    let f = fx();
    let on = |command: Command| (command, f.state.clone());
    let (deleted, restore_lane) = after(
        f.state.clone(),
        &a(automation::Command::LaneDelete {
            lane: f.gain_lane.clone(),
        }),
    );
    let (removed, restore_points) = after(
        f.state.clone(),
        &a(automation::Command::PointsRemove {
            lane: f.gain_lane.clone(),
            ticks: vec![Tick(960)],
        }),
    );
    let (device_removed, restore_device) = after(
        f.state.clone(),
        &d(device::Command::Remove {
            device: f.chain.a.clone(),
        }),
    );
    vec![
        on(a(automation::Command::LaneCreate {
            track: Some(f.track.clone()),
            master: false,
            target: pan_target(),
        })),
        on(a(automation::Command::LaneCreate {
            track: Some(f.track.clone()),
            master: false,
            target: device_target(&f.chain.b, "mute"),
        })),
        on(a(automation::Command::LaneDelete {
            lane: f.gain_lane.clone(),
        })),
        (restore_lane, deleted),
        on(a(automation::Command::PointsAdd {
            lane: f.gain_lane.clone(),
            points: vec![pt(3840, -3.004, Shape::Linear), pt(4800, -1.0, Shape::Step)],
        })),
        on(a(automation::Command::PointsRemove {
            lane: f.gain_lane.clone(),
            ticks: vec![Tick(0), Tick(960), Tick(5)],
        })),
        on(a(automation::Command::PointsUpdate {
            lane: f.gain_lane.clone(),
            points: vec![pt(960, -2.0, Shape::Linear)],
        })),
        on(a(automation::Command::LaneClear {
            lane: f.gain_lane.clone(),
        })),
        (restore_points, removed),
        // The restore that also takes a tick out.
        on(a(automation::Command::PointsRestore {
            lane: f.gain_lane.clone(),
            points: vec![pt(960, -1.0, Shape::Linear)],
            remove: vec![Tick(1920)],
        })),
        // A device with a lane leaves with it, and comes back with it.
        on(d(device::Command::Remove {
            device: f.chain.a.clone(),
        })),
        (restore_device, device_removed),
    ]
}
