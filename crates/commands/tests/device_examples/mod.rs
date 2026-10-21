//! The state the device commands run from, and one example of every branch of each of
//! them (and of `track.set_instrument` and its inverse). `tests/contract.rs` adds
//! [`examples`] to the catalog of examples (a command without one fails there);
//! `tests/device.rs` checks the behaviour in depth.
//!
//! The registry holds no effect yet (DV-3 to DV-5): under the `testing` feature, which the
//! tests of this crate enable, the commands also build the engine's test effect
//! `faulk.test_gain`.

#![allow(dead_code)]

use crate::clip_examples::{after, fx};
use ::project::{DeviceId, IdGenerator, TrackId};
use commands::{Author, Command, Context, State, device, track};

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
}

/// Inserts an effect through the command, with the ids of [`own_context`].
pub fn insert(state: &mut State, ctx: &mut Context, track: &TrackId, index: u32) -> DeviceId {
    let out = commands::run(
        &d(device::Command::Insert {
            track: track.clone(),
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
    vec![
        // Insert: first, in the middle, past the end; on an audio track (after the clip
        // player).
        (
            d(device::Command::Insert {
                track: c.track.clone(),
                kind: EFFECT.into(),
                index: 0,
            }),
            plain,
        ),
        on(d(device::Command::Insert {
            track: c.track.clone(),
            kind: EFFECT.into(),
            index: 1,
        })),
        on(d(device::Command::Insert {
            track: c.track.clone(),
            kind: EFFECT.into(),
            index: u32::MAX,
        })),
        on(d(device::Command::Insert {
            track: c.audio_track.clone(),
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
    ]
}
