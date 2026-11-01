//! The master's chain of effects (#364) at the engine's own level, from raw control messages:
//! the slot `TrackSlot::MASTER` addresses it in the four device messages and nowhere else, it
//! holds effects only, it processes the sum before the master's fader, a bypassed or removed
//! chain leaves the signal bit for bit, a `Reset` empties it, and the whole run is under the
//! allocation detector. The project-level render (commands, export, native/WASM parity) is in
//! `crates/test-support/tests/master_chain.rs`.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::OSC_SYNTH;
use engine::devices::{compressor, echo};
use engine::host::channel;
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state, encode_stream,
};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: u32 = 48_000;
const BLOCK: usize = 128;

struct Rig {
    prepare: engine::host::Prepare,
    host: engine::host::Host,
    seq: u32,
    left: Vec<f32>,
}

impl Rig {
    fn new() -> Self {
        let (prepare, host) = channel(RATE as f32).unwrap();
        let mut rig = Rig {
            prepare,
            host,
            seq: 0,
            left: vec![],
        };
        rig.send(&[
            M::Reset { epoch: 1 },
            M::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: vec![],
            },
        ]);
        rig
    }

    fn send(&mut self, msgs: &[M]) {
        let bytes = encode_stream(1, self.seq, msgs).unwrap();
        self.seq = self.seq.wrapping_add(msgs.len() as u32);
        self.prepare.submit(&bytes).unwrap();
        self.prepare.flush();
    }

    fn secs(&mut self, s: f32) {
        let mut left = (s * RATE as f32) as usize;
        while left > 0 {
            self.prepare.flush();
            let n = assert_no_alloc(|| self.host.render(BLOCK.min(left)));
            self.left.extend_from_slice(&self.host.left()[..n]);
            left -= n;
        }
    }

    fn rejected(&self) -> u32 {
        self.prepare.preparer().rejected()
    }

    fn skipped(&self) -> u32 {
        self.host.engine().skipped()
    }

    /// Non-finite samples the output protection (#402) silenced. The `is_finite` checks on the
    /// render output are vacuous (the limiter is the last stage); this is the real no-NaN guard.
    fn silenced(&self) -> u64 {
        self.host.engine().limiter_silenced()
    }
}

fn squeeze() -> Vec<u8> {
    encode_state(&[
        (ParamId(compressor::THRESHOLD), -40.0),
        (ParamId(compressor::RATIO), 20.0),
        (ParamId(compressor::ATTACK), 0.0005),
    ])
}

fn compressor_on_master(slot: u16) -> M {
    M::DeviceInsert {
        track: TrackSlot::MASTER,
        device: DeviceSlot(slot),
        kind: compressor::DESCRIPTOR.kind_id,
        state: squeeze(),
    }
}

fn play(rig: &mut Rig) {
    rig.send(&[M::NoteOn {
        track: TrackSlot(0),
        pitch: 60,
        velocity: 120,
    }]);
    rig.secs(0.6);
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn a_master_effect_changes_the_sum_and_a_bypassed_or_removed_one_leaves_it_bit_for_bit() {
    let mut dry = Rig::new();
    play(&mut dry);

    let mut treated = Rig::new();
    treated.send(&[compressor_on_master(0)]);
    play(&mut treated);
    assert_eq!((treated.rejected(), treated.skipped()), (0, 0));
    assert!(
        rms(&treated.left[9600..]) < 0.7 * rms(&dry.left[9600..]),
        "the chain did not act"
    );
    assert!(treated.left.iter().all(|s| s.is_finite()));
    assert_eq!(
        treated.silenced(),
        0,
        "the master chain produced a non-finite sample"
    );

    let mut bypassed = Rig::new();
    bypassed.send(&[
        compressor_on_master(0),
        M::DeviceSetEnabled {
            track: TrackSlot::MASTER,
            device: DeviceSlot(0),
            enabled: false,
        },
    ]);
    play(&mut bypassed);
    assert_eq!(bypassed.left, dry.left, "a bypassed chain is bit-exact");

    let mut removed = Rig::new();
    removed.send(&[
        compressor_on_master(0),
        M::DeviceRemove {
            track: TrackSlot::MASTER,
            device: DeviceSlot(0),
        },
    ]);
    play(&mut removed);
    assert_eq!(removed.left, dry.left, "a removed chain is bit-exact");
    assert_eq!((removed.rejected(), removed.skipped()), (0, 0));
}

#[test]
fn the_chain_runs_before_the_master_fader() {
    let fader = |db: f32| M::SetMaster {
        gain: 10f32.powf(db / 20.0),
        pan: 0.0,
        mute: false,
    };
    // The threshold (-20 dB) sits above what the synth gives after a -20 dB fader (below
    // -30 dB rms) and below what it gives before it (about -10 dB rms): a chain after the
    // fader would leave the sound untouched, a chain before it squeezes it.
    let at = |threshold: f32| M::DeviceInsert {
        track: TrackSlot::MASTER,
        device: DeviceSlot(0),
        kind: compressor::DESCRIPTOR.kind_id,
        state: encode_state(&[
            (ParamId(compressor::THRESHOLD), threshold),
            (ParamId(compressor::RATIO), 20.0),
            (ParamId(compressor::ATTACK), 0.0005),
        ]),
    };
    let mut plain = Rig::new();
    plain.send(&[fader(-20.0)]);
    play(&mut plain);
    let mut treated = Rig::new();
    treated.send(&[fader(-20.0), at(-20.0)]);
    play(&mut treated);
    let ratio = rms(&treated.left[9600..]) / rms(&plain.left[9600..]);
    assert!(
        ratio < 0.8,
        "the chain did not act before the fader: {ratio}"
    );
    // Control: a threshold under the level the fader leaves would act after it too, so the
    // test above separates the two orders.
    let mut low = Rig::new();
    low.send(&[fader(-20.0), at(-60.0)]);
    play(&mut low);
    assert!(rms(&low.left[9600..]) < rms(&treated.left[9600..]) * 1.01);
}

/// Whether the engine refuses `msg`: the stream stops on it, or `prepare` rejects it.
fn refuses(rig: &mut Rig, msg: M) -> bool {
    let before = rig.rejected();
    let bytes = encode_stream(1, rig.seq, &[msg]).unwrap();
    rig.seq = rig.seq.wrapping_add(1);
    rig.prepare.submit(&bytes).is_err() || rig.rejected() > before
}

#[test]
fn the_master_slot_is_the_chain_in_the_device_messages_and_nothing_else() {
    let mut rig = Rig::new();
    // Not a track: creating it, mixing it or playing on it is refused.
    assert!(refuses(
        &mut rig,
        M::TrackCreate {
            track: TrackSlot::MASTER,
            kind: TrackKind::Midi,
        }
    ));
    assert!(refuses(
        &mut rig,
        M::TrackSetMix {
            track: TrackSlot::MASTER,
            gain: 0.5,
            pan: 0.0,
            mute: false,
        }
    ));
    assert!(refuses(
        &mut rig,
        M::NoteOn {
            track: TrackSlot::MASTER,
            pitch: 60,
            velocity: 100,
        }
    ));
    // Only effects: an instrument on the master is refused by `prepare`.
    assert!(refuses(
        &mut rig,
        M::DeviceInsert {
            track: TrackSlot::MASTER,
            device: DeviceSlot(0),
            kind: OSC_SYNTH.kind_id,
            state: vec![],
        }
    ));
    // An empty master slot cannot be removed or set, as on a track.
    assert!(refuses(
        &mut rig,
        M::DeviceRemove {
            track: TrackSlot::MASTER,
            device: DeviceSlot(3),
        }
    ));
    // The refused messages left nothing behind: the engine renders the track.
    play(&mut rig);
    assert!(rms(&rig.left[9600..]) > 0.01);
    assert_eq!(rig.skipped(), 0);
    assert!(
        rig.host
            .engine()
            .state()
            .master_devices
            .as_slice()
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn the_chain_grows_past_its_first_row_and_runs_in_slot_order() {
    let echo_at = |slot: u16| M::DeviceInsert {
        track: TrackSlot::MASTER,
        device: DeviceSlot(slot),
        kind: echo::DESCRIPTOR.kind_id,
        state: encode_state(&[(ParamId(echo::MIX), 0.0)]),
    };
    // Six slots: the row starts with four and doubles.
    let mut rig = Rig::new();
    rig.send(&(0..6).map(echo_at).collect::<Vec<_>>());
    play(&mut rig);
    assert_eq!((rig.rejected(), rig.skipped()), (0, 0));
    assert!(rig.left.iter().all(|s| s.is_finite()));
    assert_eq!(
        rig.silenced(),
        0,
        "the master chain produced a non-finite sample"
    );
    assert!(rms(&rig.left[9600..]) > 0.01);
    // Slot order is run order: a compressor before a loud echo and the same pair swapped do
    // not give the same sound.
    let loud_echo = |slot: u16| M::DeviceInsert {
        track: TrackSlot::MASTER,
        device: DeviceSlot(slot),
        kind: echo::DESCRIPTOR.kind_id,
        state: encode_state(&[
            (ParamId(echo::MODE), 0.0),
            (ParamId(echo::TIME), 0.05),
            (ParamId(echo::FEEDBACK), 0.8),
            (ParamId(echo::MIX), 1.0),
        ]),
    };
    let mut ab = Rig::new();
    ab.send(&[compressor_on_master(0), loud_echo(1)]);
    play(&mut ab);
    let mut ba = Rig::new();
    ba.send(&[loud_echo(0), compressor_on_master(1)]);
    play(&mut ba);
    assert!(ab.left.iter().chain(&ba.left).all(|s| s.is_finite()));
    assert_eq!(
        (ab.silenced(), ba.silenced()),
        (0, 0),
        "a chain order produced a non-finite sample"
    );
    assert_ne!(
        ab.left, ba.left,
        "the order of the chain is the order of the run"
    );
}

#[test]
fn a_reset_empties_the_master_chain() {
    let mut rig = Rig::new();
    rig.send(&[compressor_on_master(0)]);
    rig.secs(0.01);
    assert!(rig.host.engine().state().master_devices.as_slice()[0].is_some());
    rig.left.clear();
    let bytes = encode_stream(
        2,
        rig.seq,
        &[
            M::Reset { epoch: 2 },
            M::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: vec![],
            },
            M::NoteOn {
                track: TrackSlot(0),
                pitch: 60,
                velocity: 120,
            },
        ],
    )
    .unwrap();
    rig.prepare.submit(&bytes).unwrap();
    rig.secs(0.6);
    assert!(
        rig.host
            .engine()
            .state()
            .master_devices
            .as_slice()
            .iter()
            .all(Option::is_none)
    );
    let mut dry = Rig::new();
    play(&mut dry);
    assert_eq!(rig.left, dry.left, "the new state has no chain");
}
