//! Native/WASM parity of Bloom (#696), on the pattern of `compressor_parity.rs`: Bloom inserted
//! after the synth with an initial state, notes through it, the transport playing (the synced
//! LFO on the position) then stopped, a tempo change, every parameter moved (to its bounds and
//! between), each mode, Bloom switched off and on (a reset), a removal, rendered through the test
//! host of the prepare/apply cycle natively and in the real `.wasm` build run by an
//! interpreter, in blocks of varied sizes, give the same signal.

use engine::OSC_SYNTH;
use engine::devices::bloom;
use engine::engine::cycle::Cycle;
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state, encode_stream,
};
use engine::time::{Quantize, Tempo, Tick, TimeSignature};
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
    let note = |pitch: u8, velocity: u8| M::NoteOn {
        track: t,
        pitch,
        velocity,
        at: None,
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
                pan: 0.0,
                mute: false,
            },
            M::SetTransport {
                tempo: Tempo::new(120.0).unwrap(),
                signature: TimeSignature::default(),
                launch_quantize: Quantize::default(),
            },
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
                kind: bloom::DESCRIPTOR.kind_id,
                state: encode_state(&[
                    (ParamId(bloom::MODE), bloom::ENSEMBLE),
                    (ParamId(bloom::DEPTH), 70.0),
                    (ParamId(bloom::FEEDBACK), 40.0),
                    (ParamId(bloom::STEREO_PHASE), 180.0),
                ]),
            },
            note(60, 120),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 40]));
    // The transport plays: synced on the position, then moves while the notes ring.
    submit(
        &[
            M::Play { from: Tick(480) },
            set(bloom::SYNC, 1.0),
            set(bloom::DIVISION, 3.0),
            note(67, 127),
        ],
        &mut steps,
    );
    steps.push(Step::Render(MIXED));
    submit(
        &[
            set(bloom::MODE, bloom::VIBRATO),
            set(bloom::SHAPE, bloom::TRIANGLE),
            set(bloom::DELAY, 0.002),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[777, 128, 2048, 2048, 2048]));
    submit(
        &[
            M::SetTransport {
                tempo: Tempo::new(173.0).unwrap(),
                signature: TimeSignature::default(),
                launch_quantize: Quantize::default(),
            },
            M::Seek { tick: Tick(9_000) },
        ],
        &mut steps,
    );
    steps.push(Step::Render(MIXED));
    // Every parameter at each of its bounds.
    for corner in 0..4u32 {
        let moves: Vec<M> = bloom::DESCRIPTOR
            .params
            .iter()
            .map(|p| {
                let high = (corner >> (p.id.0 % 2)) & 1 == 1;
                set(p.id.0, (if high { p.max } else { p.min }) as f32)
            })
            .collect();
        submit(&moves, &mut steps);
        steps.push(Step::Render(MIXED));
    }
    // Stopped, free, back to musical values, a new note.
    submit(
        &[
            M::Stop,
            set(bloom::MODE, bloom::CHORUS),
            set(bloom::SYNC, 0.0),
            set(bloom::RATE, 0.9),
            set(bloom::DEPTH, 40.0),
            set(bloom::DELAY, 0.012),
            set(bloom::FEEDBACK, -30.0),
            set(bloom::TONE, 120.0),
            set(bloom::MIX, 0.5),
            note(55, 100),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[2_048; 14]));
    // Switched off and on: the lines start empty again.
    submit(&[enable(false)], &mut steps);
    steps.push(Step::Render(MIXED));
    submit(&[enable(true), note(64, 110)], &mut steps);
    steps.push(Step::Render(&[2_048; 10]));
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
fn bloom_renders_the_same_in_native_and_wasm() {
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
