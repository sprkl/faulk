//! Native/WASM parity of the drums (DV-6), on the pattern of `synth_parity.rs`: the drums
//! inserted from the registry with an initial state, every sound hit (a sound hit again, a
//! closed hat after an open one, keys beyond the map), every parameter moved, level ramps, a
//! removal, rendered through the test host of the prepare/apply cycle natively and in the
//! real `.wasm` build run by an interpreter, in blocks of varied sizes, give the same signal.
//!
//! The graph hands live notes and parameter sets to the device at offset 0; the in-block
//! offsets of the device are covered natively (`crates/engine/tests/osc_synth.rs`), and the
//! code that splits a block at an event is the same in both builds.

use engine::devices::drums;
use engine::engine::cycle::Cycle;
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state, encode_stream,
};
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
    let d = DeviceSlot(0);
    let set = |param: u32, value: f32| M::ParamSet {
        track: t,
        device: d,
        param: ParamId(param),
        value,
    };
    let note = |pitch: u8, velocity: u8| M::NoteOn {
        track: t,
        pitch,
        velocity,
    };
    const MIXED: &[usize] = &[128, 37, 2048, 1, 500];

    submit(
        &[
            M::Reset { epoch },
            M::SetMaster {
                gain: 0.9,
                mute: false,
            },
            M::TrackCreate {
                track: t,
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: t,
                device: d,
                kind: drums::DESCRIPTOR.kind_id,
                state: encode_state(&[
                    (ParamId(drums::KICK_TUNE), 60.0),
                    (ParamId(drums::GAIN), 0.6),
                    (ParamId(drums::HAT_TONE), 9_000.0),
                ]),
            },
            note(36, 110),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 20]));
    submit(&[note(38, 100), note(39, 90), note(42, 80)], &mut steps);
    steps.push(Step::Render(MIXED));
    // A hit again on a ringing sound, an open hat then a closed one, keys beyond the map.
    submit(
        &[note(36, 127), note(46, 90), note(10, 100), note(100, 100)],
        &mut steps,
    );
    steps.push(Step::Render(MIXED));
    submit(&[note(42, 100)], &mut steps);
    steps.push(Step::Render(MIXED));
    // Every parameter moved once, the levels ramp.
    let moves: Vec<M> = drums::DESCRIPTOR
        .params
        .iter()
        .map(|p| set(p.id.0, (p.min + (p.max - p.min) * 0.3) as f32))
        .collect();
    submit(&moves, &mut steps);
    submit(
        &[note(36, 100), note(38, 100), note(39, 100), note(46, 100)],
        &mut steps,
    );
    steps.push(Step::Render(MIXED));
    submit(
        &[set(drums::GAIN, 0.2), set(drums::KICK_LEVEL, 0.1)],
        &mut steps,
    );
    steps.push(Step::Render(MIXED));
    submit(&[note(36, 127), note(38, 127)], &mut steps);
    steps.push(Step::Render(&[4_800; 30])); // the sounds end
    submit(
        &[M::DeviceRemove {
            track: t,
            device: d,
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
fn the_drums_render_the_same_in_native_and_wasm() {
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
