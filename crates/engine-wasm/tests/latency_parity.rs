//! Native/WASM parity of the device latency compensation (#749), on the pattern of
//! `ceiling_parity.rs`: two identical synth tracks, hard left and hard right, the right one
//! through a Ceiling below its ceiling, a Ceiling on the master; the look-ahead and the
//! true-peak detection changed while it sounds, the track's Ceiling bypassed and back, then
//! removed; rendered through the test host of the prepare/apply cycle natively and in the
//! real `.wasm` build run by an interpreter, in blocks of varied sizes. Both builds give the
//! same signal, and in each the two channels are equal to the sample once every move settled.

use engine::OSC_SYNTH;
use engine::devices::ceiling as lim;
use engine::engine::cycle::Cycle;
use engine::msg::{
    AutomationPoint, AutomationTarget, ControlMsg as M, Curve, DeviceSlot, LaneSlot, ParamId,
    TrackKind, TrackSlot, encode_state, encode_stream,
};
use engine::time::{Quantize, Tempo, Tick, TimeSignature};
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Memory, Module, Store, TypedFunc};

/// libm differs slightly between the native targets and wasm32, as in `parity.rs`.
const TOLERANCE: f32 = 1e-5;
const RATE: f32 = 48_000.0;

/// Builds the module with the cycle's exports, in the target directory `cycle_parity.rs`
/// uses (same features, so one build serves both).
fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity-cycle");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "-p", "engine-wasm", "--release"])
        .args(["--features", "engine-cycle"])
        .args(["--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(&target_dir)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the wasm build failed");
    target_dir.join("wasm32-unknown-unknown/release/engine_wasm.wasm")
}

enum Step {
    Submit(Vec<u8>),
    /// Blocks of these sizes, in turn; `true`: the two channels must be equal over them.
    Render(&'static [usize], bool),
}

fn scenario() -> Vec<Step> {
    let epoch = 1;
    let mut seq = 0u32;
    let mut steps = Vec::new();
    let mut submit = |msgs: &[M], steps: &mut Vec<Step>| {
        steps.push(Step::Submit(encode_stream(epoch, seq, msgs).unwrap()));
        seq = seq.wrapping_add(msgs.len() as u32);
    };
    let (a, b) = (TrackSlot(0), TrackSlot(1));
    let fx = DeviceSlot(1);
    let set = |param: u32, value: f32| M::ParamSet {
        track: b,
        device: fx,
        param: ParamId(param),
        value,
    };
    let note = |track: TrackSlot, pitch: u8| M::NoteOn {
        track,
        pitch,
        velocity: 90,
        at: None,
    };
    let enable = |enabled: bool| M::DeviceSetEnabled {
        track: b,
        device: fx,
        enabled,
    };
    const MIXED: &[usize] = &[128, 37, 2048, 1, 500];
    const SETTLED: &[usize] = &[128; 40];

    let mut setup = vec![
        M::Reset { epoch },
        M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::default(),
            launch_quantize: Quantize::default(),
        },
        M::SetMaster {
            gain: 0.5,
            pan: 0.0,
            mute: false,
        },
    ];
    for (track, pan) in [(a, -1.0), (b, 1.0)] {
        setup.push(M::TrackCreate {
            track,
            kind: TrackKind::Midi,
        });
        setup.push(M::TrackSetMix {
            track,
            gain: 1.0,
            pan,
            mute: false,
        });
        setup.push(M::DeviceInsert {
            track,
            device: DeviceSlot(0),
            kind: OSC_SYNTH.kind_id,
            state: encode_state(&[]),
        });
    }
    setup.push(M::DeviceInsert {
        track: b,
        device: fx,
        kind: lim::DESCRIPTOR.kind_id,
        state: encode_state(&[(ParamId(lim::CEILING), 0.0)]),
    });
    setup.push(M::DeviceInsert {
        track: TrackSlot::MASTER,
        device: DeviceSlot(0),
        kind: lim::DESCRIPTOR.kind_id,
        state: encode_state(&[(ParamId(lim::CEILING), 0.0), (ParamId(lim::TRUE_PEAK), 1.0)]),
    });
    submit(&setup, &mut steps);
    steps.push(Step::Render(&[128; 8], false));
    submit(&[note(a, 60), note(b, 60)], &mut steps);
    steps.push(Step::Render(SETTLED, true));
    steps.push(Step::Render(MIXED, true));
    // The look-ahead, then the true peak, changed while it sounds.
    submit(&[set(lim::LOOKAHEAD, 0.005)], &mut steps);
    steps.push(Step::Render(&[128; 40], false));
    steps.push(Step::Render(SETTLED, true));
    submit(
        &[set(lim::TRUE_PEAK, 1.0), note(a, 67), note(b, 67)],
        &mut steps,
    );
    steps.push(Step::Render(&[777, 128, 2048], false));
    // The settle (50 ms), the fades and the silence of the new latency.
    steps.push(Step::Render(&[128; 20], false));
    steps.push(Step::Render(SETTLED, true));
    // Bypassed and back: aligned all along.
    submit(&[enable(false)], &mut steps);
    steps.push(Step::Render(MIXED, true));
    submit(&[enable(true)], &mut steps);
    steps.push(Step::Render(MIXED, true));
    steps.push(Step::Render(SETTLED, true));
    // Removed: the left track moves back, then they are aligned again.
    submit(
        &[M::DeviceRemove {
            track: b,
            device: fx,
        }],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 20], false));
    steps.push(Step::Render(SETTLED, true));
    steps
}

/// Both channels of each block, and whether the step asks them equal.
type Blocks = Vec<(Vec<f32>, Vec<f32>, bool)>;

fn render_native(steps: &[Step]) -> Blocks {
    let mut cycle = Cycle::new(RATE).unwrap();
    let mut out = Vec::new();
    for step in steps {
        match step {
            Step::Submit(bytes) => cycle.submit(bytes).unwrap(),
            Step::Render(sizes, aligned) => {
                for size in *sizes {
                    let frames = cycle.render(*size);
                    out.push((
                        cycle.left()[..frames].to_vec(),
                        cycle.right()[..frames].to_vec(),
                        *aligned,
                    ));
                }
            }
        }
    }
    out
}

struct Wasm {
    store: Store<()>,
    memory: Memory,
    cycle: i32,
    inbox: TypedFunc<(i32, i32), i32>,
    submit: TypedFunc<(i32, i32), i32>,
    render: TypedFunc<(i32, i32), i32>,
    left: TypedFunc<i32, i32>,
    right: TypedFunc<i32, i32>,
}

impl Wasm {
    fn new(bytes: &[u8]) -> Self {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes).expect("valid module");
        let mut store = Store::new(&engine, ());
        let instance = Linker::<()>::new(&engine)
            .instantiate_and_start(&mut store, &module)
            .expect("the module needs no imports");
        fn func<P: wasmi::WasmParams, R: wasmi::WasmResults>(
            i: &wasmi::Instance,
            s: &Store<()>,
            name: &str,
        ) -> TypedFunc<P, R> {
            i.get_typed_func::<P, R>(s, name)
                .unwrap_or_else(|e| panic!("export {name}: {e}"))
        }
        let new = func::<f32, i32>(&instance, &store, "fk_cycle_new");
        let memory = instance.get_memory(&store, "memory").expect("memory");
        let cycle = new.call(&mut store, RATE).unwrap();
        assert_ne!(cycle, 0, "the cycle was built");
        Wasm {
            inbox: func(&instance, &store, "fk_cycle_inbox"),
            submit: func(&instance, &store, "fk_cycle_submit"),
            render: func(&instance, &store, "fk_cycle_render"),
            left: func(&instance, &store, "fk_cycle_left"),
            right: func(&instance, &store, "fk_cycle_right"),
            store,
            memory,
            cycle,
        }
    }

    fn submit(&mut self, bytes: &[u8]) -> i32 {
        let at = self
            .inbox
            .call(&mut self.store, (self.cycle, bytes.len() as i32))
            .unwrap();
        assert_ne!(at, 0);
        self.memory
            .write(&mut self.store, at as usize, bytes)
            .unwrap();
        self.submit
            .call(&mut self.store, (self.cycle, bytes.len() as i32))
            .unwrap()
    }

    fn render(&mut self, size: usize) -> (Vec<f32>, Vec<f32>) {
        let frames = self
            .render
            .call(&mut self.store, (self.cycle, size as i32))
            .unwrap() as usize;
        let mut sides = Vec::new();
        for side in [self.left, self.right] {
            let at = side.call(&mut self.store, self.cycle).unwrap() as usize;
            let mut raw = vec![0u8; frames * 4];
            self.memory.read(&self.store, at, &mut raw).unwrap();
            sides.push(
                raw.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect::<Vec<f32>>(),
            );
        }
        let right = sides.pop().unwrap();
        (sides.pop().unwrap(), right)
    }
}

/// The largest difference between the two channels over the blocks that ask them equal.
fn misalignment(blocks: &Blocks) -> f32 {
    blocks
        .iter()
        .filter(|(_, _, aligned)| *aligned)
        .flat_map(|(l, r, _)| l.iter().zip(r).map(|(a, b)| (a - b).abs()))
        .fold(0f32, f32::max)
}

/// The strip behind a latent chain (#758): the pair, the right track through a Ceiling and a
/// Ceiling on the master, the same gain lane (steps to silence and back, a ramp) on both
/// tracks and a gain lane on the master, played from 0: the two channels are equal to the
/// sample all along (the right track's lane lands on its delayed sound).
fn strip_scenario() -> Vec<Step> {
    let epoch = 1;
    let mut seq = 0u32;
    let mut steps = Vec::new();
    let mut submit = |msgs: &[M], steps: &mut Vec<Step>| {
        steps.push(Step::Submit(encode_stream(epoch, seq, msgs).unwrap()));
        seq = seq.wrapping_add(msgs.len() as u32);
    };
    let (a, b) = (TrackSlot(0), TrackSlot(1));
    let pt = |tick: i64, value: f32, curve: Curve| AutomationPoint {
        tick: Tick(tick),
        value,
        curve,
    };
    let gain = [
        pt(0, 0.0, Curve::Step),
        pt(40, -96.0, Curve::Step),
        pt(60, -6.0, Curve::Linear),
        pt(100, -30.0, Curve::Step),
        pt(120, 0.0, Curve::Step),
        pt(170, -12.0, Curve::Step),
    ];
    let master = [
        pt(0, 0.0, Curve::Step),
        pt(30, -6.0, Curve::Linear),
        pt(90, -20.0, Curve::Step),
        pt(150, -3.0, Curve::Step),
    ];
    let lane = |lane: u32, track: TrackSlot, points: &[AutomationPoint]| M::AutomationLaneUpload {
        lane: LaneSlot(lane),
        track,
        target: AutomationTarget::MixerGain,
        first: 0,
        total: points.len() as u32,
        points: points.to_vec(),
    };
    let mut setup = vec![
        M::Reset { epoch },
        M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::default(),
            launch_quantize: Quantize::default(),
        },
    ];
    for (track, pan) in [(a, -1.0), (b, 1.0)] {
        setup.push(M::TrackCreate {
            track,
            kind: TrackKind::Midi,
        });
        setup.push(M::TrackSetMix {
            track,
            gain: 1.0,
            pan,
            mute: false,
        });
        setup.push(M::DeviceInsert {
            track,
            device: DeviceSlot(0),
            kind: OSC_SYNTH.kind_id,
            state: encode_state(&[]),
        });
    }
    setup.push(M::DeviceInsert {
        track: b,
        device: DeviceSlot(1),
        kind: lim::DESCRIPTOR.kind_id,
        state: encode_state(&[(ParamId(lim::CEILING), 0.0)]),
    });
    setup.push(M::DeviceInsert {
        track: TrackSlot::MASTER,
        device: DeviceSlot(0),
        kind: lim::DESCRIPTOR.kind_id,
        state: encode_state(&[(ParamId(lim::CEILING), 0.0)]),
    });
    submit(&setup, &mut steps);
    steps.push(Step::Render(&[128; 8], false));
    let note = |track: TrackSlot| M::NoteOn {
        track,
        pitch: 60,
        velocity: 90,
        at: None,
    };
    submit(
        &[
            note(a),
            note(b),
            lane(0, a, &gain),
            lane(1, b, &gain),
            lane(2, TrackSlot::MASTER, &master),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 8], true));
    submit(&[M::Play { from: Tick(0) }], &mut steps);
    steps.push(Step::Render(&[128, 37, 2048, 1, 500, 97, 1024], true));
    steps.push(Step::Render(&[128; 20], true));
    steps
}

/// Criterion 3 of #758: one centred synth track through a Ceiling of `lookahead` seconds, a
/// Ceiling of the same look-ahead on the master, a gain lane (steps to silence and back, a
/// ramp) and a pan lane on the track and on the master, played from 0.
fn shift_scenario(lookahead: f32) -> Vec<Step> {
    let epoch = 1;
    let mut seq = 0u32;
    let mut steps = Vec::new();
    let mut submit = |msgs: &[M], steps: &mut Vec<Step>| {
        steps.push(Step::Submit(encode_stream(epoch, seq, msgs).unwrap()));
        seq = seq.wrapping_add(msgs.len() as u32);
    };
    let a = TrackSlot(0);
    let pt = |tick: i64, value: f32, curve: Curve| AutomationPoint {
        tick: Tick(tick),
        value,
        curve,
    };
    let gain = [
        pt(0, 0.0, Curve::Step),
        pt(40, -96.0, Curve::Step),
        pt(60, -6.0, Curve::Linear),
        pt(100, -30.0, Curve::Step),
        pt(120, 0.0, Curve::Step),
        pt(170, -12.0, Curve::Step),
    ];
    let pan = [
        pt(0, 0.0, Curve::Step),
        pt(50, -0.7, Curve::Step),
        pt(90, 0.5, Curve::Linear),
        pt(130, 0.0, Curve::Step),
    ];
    let master_gain = [
        pt(0, 0.0, Curve::Step),
        pt(30, -6.0, Curve::Linear),
        pt(90, -20.0, Curve::Step),
        pt(150, -3.0, Curve::Step),
    ];
    let master_pan = [pt(0, 0.0, Curve::Step), pt(80, 0.6, Curve::Step)];
    let lane =
        |lane: u32, track: TrackSlot, target: AutomationTarget, points: &[AutomationPoint]| {
            M::AutomationLaneUpload {
                lane: LaneSlot(lane),
                track,
                target,
                first: 0,
                total: points.len() as u32,
                points: points.to_vec(),
            }
        };
    let ceiling = |track: TrackSlot, device: u16| M::DeviceInsert {
        track,
        device: DeviceSlot(device),
        kind: lim::DESCRIPTOR.kind_id,
        state: encode_state(&[
            (ParamId(lim::CEILING), 0.0),
            (ParamId(lim::LOOKAHEAD), lookahead),
        ]),
    };
    submit(
        &[
            M::Reset { epoch },
            M::SetTransport {
                tempo: Tempo::new(120.0).unwrap(),
                signature: TimeSignature::default(),
                launch_quantize: Quantize::default(),
            },
            M::TrackCreate {
                track: a,
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: a,
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: encode_state(&[]),
            },
            ceiling(a, 1),
            ceiling(TrackSlot::MASTER, 0),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 8], false));
    submit(
        &[
            M::NoteOn {
                track: a,
                pitch: 60,
                velocity: 90,
                at: None,
            },
            lane(0, a, AutomationTarget::MixerGain, &gain),
            lane(1, a, AutomationTarget::MixerPan, &pan),
            lane(
                2,
                TrackSlot::MASTER,
                AutomationTarget::MixerGain,
                &master_gain,
            ),
            lane(
                3,
                TrackSlot::MASTER,
                AutomationTarget::MixerPan,
                &master_pan,
            ),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 4], false));
    submit(&[M::Play { from: Tick(0) }], &mut steps);
    steps.push(Step::Render(
        &[128, 37, 2048, 1, 500, 97, 1024, 128, 128],
        false,
    ));
    steps
}

#[test]
fn the_compensation_renders_the_same_in_native_and_wasm() {
    check_parity(&scenario());
}

#[test]
fn the_strip_behind_a_latent_chain_renders_the_same_in_native_and_wasm() {
    check_parity(&strip_scenario());
}

#[test]
fn the_strip_behind_a_ceiling_is_its_render_without_latency_one_latency_later_in_both_builds() {
    let latent = check_parity(&shift_scenario(0.0015));
    let plain = check_parity(&shift_scenario(0.0));
    // 72 frames on the track, 72 on the master.
    let n = 2 * lim::lookahead_frames(0.0015, RATE) as usize;
    assert_eq!(n, 144);
    for (build, latent, plain) in [
        ("native", &latent.0, &plain.0),
        ("wasm", &latent.1, &plain.1),
    ] {
        let (ll, lr) = flatten(latent);
        let (pl, pr) = flatten(plain);
        let len = pl.len() - n;
        let off =
            |a: &[f32], b: &[f32]| a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
        let (off_l, off_r) = (off(&ll[n..], &pl[..len]), off(&lr[n..], &pr[..len]));
        println!(
            "{build}: behind 144 frames, shifted back: left off by {off_l:e}, right by {off_r:e}"
        );
        assert_eq!(off_l, 0.0, "{build}");
        assert_eq!(off_r, 0.0, "{build}");
    }
}

/// Both channels of every block, end to end.
fn flatten(blocks: &Blocks) -> (Vec<f32>, Vec<f32>) {
    let left = blocks
        .iter()
        .flat_map(|(l, _, _)| l.iter().copied())
        .collect();
    let right = blocks
        .iter()
        .flat_map(|(_, r, _)| r.iter().copied())
        .collect();
    (left, right)
}

/// Renders `steps` natively and in the `.wasm`, checks both against each other (and the blocks
/// that ask it aligned), and returns both renders.
fn check_parity(steps: &[Step]) -> (Blocks, Blocks) {
    let native = render_native(steps);
    let peak = native
        .iter()
        .flat_map(|(l, _, _)| l)
        .fold(0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "the scenario sounds: peak {peak}");
    assert_eq!(misalignment(&native), 0.0, "native: aligned to the sample");

    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let mut wasm = Wasm::new(&bytes);
    let mut out: Blocks = Vec::new();
    for step in steps {
        match step {
            Step::Submit(stream) => assert_eq!(wasm.submit(stream), 0, "accepted"),
            Step::Render(sizes, aligned) => {
                for size in *sizes {
                    let (l, r) = wasm.render(*size);
                    out.push((l, r, *aligned));
                }
            }
        }
    }
    assert_eq!(misalignment(&out), 0.0, "wasm: aligned to the sample");
    assert_eq!(out.len(), native.len());
    let mut worst = 0f32;
    let mut samples = 0usize;
    for (i, ((nl, nr, _), (wl, wr, _))) in native.iter().zip(&out).enumerate() {
        assert_eq!(nl.len(), wl.len());
        for (a, b) in nl.iter().chain(nr).zip(wl.iter().chain(wr)) {
            let d = (a - b).abs();
            assert!(d <= TOLERANCE, "block {i}: native {a}, wasm {b}");
            worst = worst.max(d);
            samples += 1;
        }
    }
    println!("{samples} samples, largest difference {worst:e}");
    (native, out)
}
