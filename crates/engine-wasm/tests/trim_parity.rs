//! Native/WASM parity of Trim (#693). Two runs:
//!
//! - the event stream of `engine::devices::trim::testing::scenario` (seeded notes, every
//!   parameter moved to its bounds and between, tempos with non-integer frame lengths, key
//!   tracking, stops, jumps, an all-notes-off and an overflow of the note table, in blocks of
//!   varied sizes), native and in the real `.wasm` build run by an interpreter, bit for bit;
//! - Trim before the synth through the test host of the prepare/apply cycle: notes in both
//!   triggers, a play and a stop, every parameter moved, Trim switched off and on and removed,
//!   rendered natively and in WASM in blocks of varied sizes: the same signal (the synth
//!   starts and releases its voices at the frames of Trim's note-ons and note-offs).

use engine::OSC_SYNTH;
use engine::devices::trim::{self, testing::scenario as event_scenario};
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

/// Builds the module with `feature` in its own target directory (the cycle's is the one
/// `cycle_parity.rs` uses, so one build serves both).
fn build(feature: &str, dir: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join(dir);
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "-p", "engine-wasm", "--release"])
        .args(["--features", feature])
        .args(["--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(&target_dir)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the wasm build failed");
    target_dir.join("wasm32-unknown-unknown/release/engine_wasm.wasm")
}

fn events_wasm() -> Vec<i64> {
    let bytes = std::fs::read(build("trim-scenario", "target/parity-trim"))
        .expect("the wasm module exists");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes[..]).expect("valid module");
    let mut store = Store::new(&engine, ());
    let instance = Linker::<()>::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .expect("the module needs no imports");
    let run = instance
        .get_typed_func::<(), i32>(&store, "fk_trim_scenario")
        .expect("export fk_trim_scenario");
    let len = instance
        .get_typed_func::<(), i32>(&store, "fk_trim_len")
        .expect("export fk_trim_len");
    let memory = instance
        .get_memory(&store, "memory")
        .expect("exported memory");
    let at = run.call(&mut store, ()).unwrap() as usize;
    assert_ne!(at, 0, "the scenario ran");
    let len = len.call(&mut store, ()).unwrap() as usize;
    let mut raw = vec![0u8; len * 8];
    memory.read(&store, at, &mut raw).unwrap();
    raw.as_chunks::<8>()
        .0
        .iter()
        .map(|b| i64::from_le_bytes(*b))
        .collect()
}

#[test]
fn the_event_stream_of_trim_is_identical_in_native_and_wasm() {
    let native = event_scenario();
    let stride = trim::testing::STRIDE;
    let events = native.len() / stride;
    let ons = native.chunks(stride).filter(|e| e[1] == 0).count();
    let offs = native.chunks(stride).filter(|e| e[1] == 1).count();
    assert!(
        ons > 500 && offs > 500,
        "the scenario plays notes: {ons} on, {offs} off"
    );
    let wasm = events_wasm();
    assert_eq!(native.len(), wasm.len());
    if let Some(i) = native.iter().zip(&wasm).position(|(a, b)| a != b) {
        panic!(
            "first difference at event {}: native {:?} wasm {:?}",
            i / stride,
            &native[i / stride * stride..][..stride],
            &wasm[i / stride * stride..][..stride]
        );
    }
    println!("{events} events ({ons} note-ons, {offs} note-offs), identical");
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
    let (fx, synth) = (DeviceSlot(0), DeviceSlot(1));
    let set = |param: u32, value: f32| M::ParamSet {
        track: t,
        device: fx,
        param: ParamId(param),
        value,
    };
    let on = |pitch: u8, velocity: u8| M::NoteOn {
        track: t,
        pitch,
        velocity,
        at: None,
    };
    let off = |pitch: u8| M::NoteOff {
        track: t,
        pitch,
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
                tempo: Tempo::new(174.0).unwrap(),
                signature: TimeSignature::default(),
                launch_quantize: Quantize::default(),
            },
            M::TrackCreate {
                track: t,
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: t,
                device: fx,
                kind: trim::DESCRIPTOR.kind_id,
                state: encode_state(&[
                    (ParamId(trim::DIVISION), trim::DIV_1_8 as f32),
                    (ParamId(trim::KEY_TRACK), 37.0),
                ]),
            },
            M::DeviceInsert {
                track: t,
                device: synth,
                kind: OSC_SYNTH.kind_id,
                state: encode_state(&[]),
            },
            M::Play { from: Tick(0) },
            on(60, 120),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 40]));
    submit(&[off(60), on(48, 100), on(72, 90)], &mut steps);
    steps.push(Step::Render(MIXED));
    // Release trigger, free time, then a note released.
    submit(
        &[
            set(trim::TRIGGER, trim::RELEASE as f32),
            set(trim::MODE, trim::FREE as f32),
            set(trim::TIME, 0.05),
            off(48),
            off(72),
            on(64, 110),
        ],
        &mut steps,
    );
    steps.push(Step::Render(MIXED));
    submit(&[off(64)], &mut steps);
    steps.push(Step::Render(&[777, 128, 2048, 2048]));
    // Every parameter at each of its bounds, with a note each time.
    for corner in 0..4u8 {
        let mut moves: Vec<M> = trim::DESCRIPTOR
            .params
            .iter()
            .map(|p| {
                let high = (u32::from(corner) >> (p.id.0 % 2)) & 1 == 1;
                set(p.id.0, (if high { p.max } else { p.min }) as f32)
            })
            .collect();
        moves.push(on(55 + corner, 100));
        submit(&moves, &mut steps);
        steps.push(Step::Render(MIXED));
        submit(&[off(55 + corner)], &mut steps);
        steps.push(Step::Render(MIXED));
    }
    // Back to a long tempo note, then a stop while it sounds.
    submit(
        &[
            set(trim::MODE, trim::TEMPO as f32),
            set(trim::TRIGGER, trim::START as f32),
            set(trim::DIVISION, trim::DIV_1_BAR as f32),
            set(trim::GATE, 100.0),
            set(trim::KEY_TRACK, 0.0),
            on(67, 127),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[2_048; 6]));
    submit(&[M::Stop, off(67)], &mut steps);
    steps.push(Step::Render(&[2_048; 10]));
    // Switched off and on, then removed with a note sounding.
    submit(&[enable(false), on(62, 100)], &mut steps);
    steps.push(Step::Render(MIXED));
    submit(&[off(62), enable(true), on(65, 110)], &mut steps);
    steps.push(Step::Render(&[2_048; 4]));
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
fn trim_before_the_synth_renders_the_same_in_native_and_wasm() {
    let steps = scenario();
    let native = render_native(&steps);
    let peak = native.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.1, "the scenario sounds: peak {peak}");
    assert!(native.iter().all(|s| s.is_finite()));

    let bytes = std::fs::read(build("engine-cycle", "target/parity-cycle"))
        .expect("the wasm module exists");
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
