//! The state the device commands run from, and one example of every branch of each of
//! them (and of `track.set_instrument` and its inverse). `tests/it/contract.rs` adds
//! [`examples`] to the catalog of examples (a command without one fails there);
//! `tests/it/device.rs` checks the behaviour in depth.
//!
//! The registry holds one effect (Echo, DV-3; DV-4 and DV-5 follow): under the `testing` feature, which the
//! tests of this crate enable, the commands also build the engine's test effect
//! `faulk.test_gain`.

#![allow(dead_code)]

use crate::clip_examples::{after, fx};
use ::project::{AutomationTarget, DeviceId, IdGenerator, Shape, Tick, TrackId};
use commands::{Author, Command, Context, State, automation, device, track};

/// The test effect of the engine, as the commands name it.
pub const EFFECT: &str = "faulk.test_gain";

pub fn d(command: device::Command) -> Command {
    Command::Device(command)
}

pub fn t(command: track::Command) -> Command {
    Command::Track(command)
}

/// A context whose ids never meet the ones of the fixture or of the examples' `context()`.
pub fn own_context() -> Context {
    Context::new(
        Author::User,
        IdGenerator::new(1_670_000_000_000, 0x00CC_0000_0000_0000_0000),
    )
}

/// Inserts an effect in the master's chain through the command, with the ids of
/// [`own_context`].
pub fn insert_master(state: &mut State, ctx: &mut Context, index: u32) -> DeviceId {
    let out = commands::run(
        &d(device::Command::Insert {
            track: None,
            master: true,
            kind: EFFECT.into(),
            index,
        }),
        state,
        ctx,
    )
    .unwrap();
    let commands::Output::Device(device::Output::Insert { device }) = out.output else {
        panic!("{:?}", out.output)
    };
    device
}

/// The state and the ids the device tests use.
pub struct Chain {
    /// A MIDI track playing the synth, then effect `a`, then effect `b`.
    pub state: State,
    pub track: TrackId,
    pub synth: DeviceId,
    pub a: DeviceId,
    pub b: DeviceId,
    /// An audio track holding the clip player.
    pub audio_track: TrackId,
    pub player: DeviceId,
    /// The master's chain: effects `ma` then `mb`.
    pub ma: DeviceId,
    pub mb: DeviceId,
}

/// Inserts an effect through the command, with the ids of [`own_context`].
pub fn insert(state: &mut State, ctx: &mut Context, track: &TrackId, index: u32) -> DeviceId {
    let out = commands::run(
        &d(device::Command::Insert {
            track: Some(track.clone()),
            master: false,
            kind: EFFECT.into(),
            index,
        }),
        state,
        ctx,
    )
    .unwrap();
    let commands::Output::Device(device::Output::Insert { device }) = out.output else {
        panic!("{:?}", out.output)
    };
    device
}

pub fn chain() -> Chain {
    let f = fx();
    let mut state = f.state.clone();
    let mut ctx = own_context();
    let a = insert(&mut state, &mut ctx, &f.track, 0);
    let b = insert(&mut state, &mut ctx, &f.track, 1);
    let ma = insert_master(&mut state, &mut ctx, 0);
    let mb = insert_master(&mut state, &mut ctx, 1);
    let synth = state.project.tracks[&f.track]
        .devices_in_order()
        .first()
        .map(|(id, _)| (*id).clone())
        .unwrap();
    let player = state.project.tracks[&f.audio_track]
        .devices
        .keys()
        .next()
        .cloned()
        .unwrap();
    state
        .project
        .validate()
        .expect("the device fixture is valid");
    Chain {
        state,
        track: f.track,
        synth,
        a,
        b,
        audio_track: f.audio_track,
        player,
        ma,
        mb,
    }
}

pub fn examples() -> Vec<(Command, State)> {
    let c = chain();
    let plain = fx().state;
    let on = |command: Command| (command, c.state.clone());
    let removed = after(
        c.state.clone(),
        &d(device::Command::Remove {
            device: c.a.clone(),
        }),
    );
    let moved = after(
        c.state.clone(),
        &d(device::Command::Move {
            device: c.b.clone(),
            index: 0,
        }),
    );
    let bypassed = after(
        c.state.clone(),
        &d(device::Command::SetBypass {
            device: c.a.clone(),
            bypass: true,
        }),
    );
    let replaced = after(
        c.state.clone(),
        &t(track::Command::SetInstrument {
            track: c.track.clone(),
            instrument: "faulk.drums".into(),
        }),
    );
    let laned = with_instrument_lane(&c);
    let swapped = after(
        laned.clone(),
        &t(track::Command::SetInstrument {
            track: c.track.clone(),
            instrument: "faulk.drums".into(),
        }),
    );
    let master_removed = after(
        c.state.clone(),
        &d(device::Command::Remove {
            device: c.ma.clone(),
        }),
    );
    vec![
        // The master's chain: insert first and last, remove, the inverse of a remove
        // (a restore into the master), move, bypass, set a parameter.
        (
            d(device::Command::Insert {
                track: None,
                master: true,
                kind: EFFECT.into(),
                index: 0,
            }),
            plain.clone(),
        ),
        on(d(device::Command::Insert {
            track: None,
            master: true,
            kind: EFFECT.into(),
            index: u32::MAX,
        })),
        on(d(device::Command::Remove {
            device: c.ma.clone(),
        })),
        (master_removed.1, master_removed.0),
        on(d(device::Command::Move {
            device: c.mb.clone(),
            index: 0,
        })),
        on(d(device::Command::SetBypass {
            device: c.mb.clone(),
            bypass: true,
        })),
        on(d(device::Command::SetParam {
            device: c.ma.clone(),
            param: "gain".into(),
            value: 0.25,
        })),
        // Insert: first, in the middle, past the end; on an audio track (after the clip
        // player).
        (
            d(device::Command::Insert {
                track: Some(c.track.clone()),
                master: false,
                kind: EFFECT.into(),
                index: 0,
            }),
            plain,
        ),
        on(d(device::Command::Insert {
            track: Some(c.track.clone()),
            master: false,
            kind: EFFECT.into(),
            index: 1,
        })),
        on(d(device::Command::Insert {
            track: Some(c.track.clone()),
            master: false,
            kind: EFFECT.into(),
            index: u32::MAX,
        })),
        on(d(device::Command::Insert {
            track: Some(c.audio_track.clone()),
            master: false,
            kind: EFFECT.into(),
            index: 0,
        })),
        on(d(device::Command::Remove {
            device: c.a.clone(),
        })),
        on(d(device::Command::Remove {
            device: c.b.clone(),
        })),
        // The inverse of a remove, from the state after it.
        (removed.1, removed.0),
        on(d(device::Command::Move {
            device: c.b.clone(),
            index: 0,
        })),
        on(d(device::Command::Move {
            device: c.a.clone(),
            index: 99,
        })),
        // The inverse of a move: the exact previous key.
        (moved.1, moved.0),
        on(d(device::Command::SetBypass {
            device: c.a.clone(),
            bypass: true,
        })),
        on(d(device::Command::SetBypass {
            device: c.synth.clone(),
            bypass: true,
        })),
        (
            d(device::Command::SetBypass {
                device: c.a.clone(),
                bypass: false,
            }),
            bypassed.0,
        ),
        // `device.set_param`: a continuous parameter, one rounded to its step, one clamped
        // to its range, on an effect.
        on(d(device::Command::SetParam {
            device: c.synth.clone(),
            param: "waveform".into(),
            value: 2.6,
        })),
        on(d(device::Command::SetParam {
            device: c.synth.clone(),
            param: "sustain".into(),
            value: 5.0,
        })),
        on(d(device::Command::SetParam {
            device: c.a.clone(),
            param: "gain".into(),
            value: 0.5,
        })),
        on(t(track::Command::SetInstrument {
            track: c.track.clone(),
            instrument: "faulk.drums".into(),
        })),
        // The inverse of `track.set_instrument`, from the state after it.
        (replaced.1, replaced.0),
        // The same two from a state where a lane targets the instrument (AM-4): the swap takes the lane,
        // its inverse carries it in `lanes` and gives it back.
        (
            t(track::Command::SetInstrument {
                track: c.track.clone(),
                instrument: "faulk.drums".into(),
            }),
            laned.clone(),
        ),
        (swapped.1, swapped.0),
    ]
}

/// A state where one lane, with two points, targets the track's instrument.
fn with_instrument_lane(c: &Chain) -> State {
    let create = Command::Automation(automation::Command::LaneCreate {
        track: Some(c.track.clone()),
        master: false,
        target: AutomationTarget::Device {
            device: c.synth.clone(),
            param: "gain".into(),
        },
    });
    // The lane's identifier comes from a generator of its own: it must meet neither the fixture's (`own_context`) nor the
    // one the contract harness runs the swap with.
    let mut state = c.state.clone();
    let mut ctx = Context::new(
        Author::User,
        IdGenerator::new(1_671_000_000_000, 0x00DD_0000_0000_0000_0000),
    );
    commands::run(&create, &mut state, &mut ctx).unwrap();
    let lane = state.project.tracks[&c.track]
        .automation
        .keys()
        .next()
        .cloned()
        .expect("the lane was created");
    let points = Command::Automation(automation::Command::PointsAdd {
        lane,
        points: vec![
            automation::Point {
                tick: Tick(0),
                value: 0.2,
                shape: Shape::Linear,
            },
            automation::Point {
                tick: Tick(960),
                value: 0.8,
                shape: Shape::Step,
            },
        ],
    });
    state = after(state, &points).0;
    state
}
