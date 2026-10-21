//! Native/WASM parity of the echo (DV-3), on the pattern of `drums_parity.rs`: the echo
//! inserted after the synth with an initial state, notes through it, every parameter moved
//! (free then synced, every note value), tempo changes while the delay glides, the echo
//! switched off and on, a removal, rendered through the test host of the prepare/apply cycle
//! natively and in the real `.wasm` build run by an interpreter, in blocks of varied sizes,
//! give the same signal.

use engine::OSC_SYNTH;
use engine::devices::echo;
use engine::engine::cycle::Cycle;
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state, encode_stream,
};
use engine::time::{Quantize, Tempo, TimeSignature};
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Memory, Module, Store, TypedFunc};

/// libm differs slightly between the native targets and wasm32 (sin, exp2, tanh), as in
/// `parity.rs`.
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
    /// Blocks of these sizes, in turn.
    Render(&'static [usize]),
}

fn scenario() -> Vec<Step> {
    let epoch = 1;
    let mut seq = 0u32;
    let mut steps = Vec::new();
    let mut submit = |msgs: &[M], steps: &mut Vec<Step>| {
        steps.push(Step::Submit(encode_stream(epoch, seq, msgs).unwrap()));
        seq = seq.wrapping_add(msgs.len() as u32);
    };
    let t = TrackSlot(0);
    let (synth, fx) = (DeviceSlot(0), DeviceSlot(1));
    let set = |param: u32, value: f32| M::ParamSet {
        track: t,
        device: fx,
        param: ParamId(param),
        value,
    };
    let tempo = |bpm: f64| M::SetTransport {
        tempo: Tempo::new(bpm).unwrap(),
        signature: TimeSignature::default(),
        launch_quantize: Quantize::default(),
    };
    let note = |pitch: u8, velocity: u8| M::NoteOn {
        track: t,
        pitch,
        velocity,
    };
    let enable = |enabled: bool| M::DeviceSetEnabled {
        track: t,
        device: fx,
        enabled,
    };
    const MIXED: &[usize] = &[128, 37, 2048, 1, 500];

    submit(
        &[
            M::Reset { epoch },
            M::SetMaster {
                gain: 0.9,
                mute: false,
            },
            tempo(120.0),
            M::TrackCreate {
                track: t,
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: t,
                device: synth,
                kind: OSC_SYNTH.kind_id,
                state: encode_state(&[]),
            },
            M::DeviceInsert {
                track: t,
                device: fx,
                kind: echo::DESCRIPTOR.kind_id,
                state: encode_state(&[(ParamId(echo::FEEDBACK), 0.6), (ParamId(echo::MIX), 0.5)]),
            },
            note(60, 110),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 20]));
    // The tempo moves while the delay glides, then back.
    submit(&[tempo(90.0)], &mut steps);
    steps.push(Step::Render(MIXED));
    submit(&[tempo(173.0), note(64, 100)], &mut steps);
    steps.push(Step::Render(&[4_800; 4]));
    // Every note value, in turn, at a tempo that is not a round number.
    for division in 0..echo::DIVISION_BEATS.len() {
        submit(
            &[tempo(131.0), set(echo::DIVISION, division as f32)],
            &mut steps,
        );
        steps.push(Step::Render(&[777, 128, 2048]));
    }
    // Free time, fully wet, a short and a long time, then back to synced.
    submit(
        &[
            set(echo::MODE, 0.0),
            set(echo::TIME, 0.02),
            set(echo::MIX, 1.0),
            note(67, 120),
        ],
        &mut steps,
    );
    steps.push(Step::Render(MIXED));
    submit(
        &[set(echo::TIME, 1.7), set(echo::FEEDBACK, 0.95)],
        &mut steps,
    );
    steps.push(Step::Render(&[4_800; 10]));
    submit(
        &[set(echo::MODE, 1.0), tempo(20.0), set(echo::DIVISION, 9.0)],
        &mut steps,
    );
    steps.push(Step::Render(&[4_800; 10]));
    // Switched off and on: no tail comes back.
    submit(&[enable(false)], &mut steps);
    steps.push(Step::Render(MIXED));
    submit(&[enable(true), note(55, 100)], &mut steps);
    steps.push(Step::Render(&[4_800; 6]));
    // Every parameter moved once.
    let moves: Vec<M> = echo::DESCRIPTOR
        .params
        .iter()
        .map(|p| set(p.id.0, (p.min + (p.max - p.min) * 0.3) as f32))
        .collect();
    submit(&moves, &mut steps);
    steps.push(Step::Render(MIXED));
    submit(
        &[M::DeviceRemove {
            track: t,
            device: fx,
        }],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 10]));
    steps
}

fn render_native(steps: &[Step]) -> Vec<f32> {
    let mut cycle = Cycle::new(RATE).unwrap();
    let mut out = Vec::new();
    for step in steps {
        match step {
            Step::Submit(bytes) => cycle.submit(bytes).unwrap(),
            Step::Render(sizes) => {
                for size in *sizes {
                    let frames = cycle.render(*size);
                    out.extend_from_slice(&cycle.left()[..frames]);
                    out.extend_from_slice(&cycle.right()[..frames]);
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

    fn render(&mut self, size: usize, out: &mut Vec<f32>) {
        let frames = self
            .render
            .call(&mut self.store, (self.cycle, size as i32))
            .unwrap() as usize;
        for side in [self.left, self.right] {
            let at = side.call(&mut self.store, self.cycle).unwrap() as usize;
            let mut raw = vec![0u8; frames * 4];
            self.memory.read(&self.store, at, &mut raw).unwrap();
            out.extend(
                raw.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b)),
            );
        }
    }
}

#[test]
fn the_echo_renders_the_same_in_native_and_wasm() {
    let steps = scenario();
    let native = render_native(&steps);
    let peak = native.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.1, "the scenario sounds: peak {peak}");
    assert!(native.iter().all(|s| s.is_finite()));

    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let mut wasm = Wasm::new(&bytes);
    let mut out = Vec::new();
    for step in &steps {
        match step {
            Step::Submit(stream) => assert_eq!(wasm.submit(stream), 0, "accepted"),
            Step::Render(sizes) => {
                for size in *sizes {
                    wasm.render(*size, &mut out);
                }
            }
        }
    }
    assert_eq!(out.len(), native.len());
    let mut worst = 0f32;
    for (i, (a, b)) in native.iter().zip(&out).enumerate() {
        let d = (a - b).abs();
        assert!(d <= TOLERANCE, "sample {i}: native {a}, wasm {b}");
        worst = worst.max(d);
    }
    println!("{} samples, largest difference {worst:e}", out.len());
}
