//! Native/WASM parity of the equaliser Contour (#156), on the pattern of `filter_parity.rs`:
//! Contour inserted after the synth with an initial state, notes through it, every band and
//! slope moved (every parameter to its bounds and back to its defaults), the device switched
//! off and on (a reset), a removal, rendered through the test host of the prepare/apply cycle
//! natively and in the real `.wasm` build run by an interpreter, in blocks of varied sizes,
//! give the same signal.

use engine::OSC_SYNTH;
use engine::devices::contour::{self as comp, BandParam};
use engine::engine::cycle::Cycle;
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state, encode_stream,
};
use engine::time::{Quantize, Tempo, TimeSignature};
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Memory, Module, Store, TypedFunc};

/// libm differs slightly between the native targets and wasm32 (sin, exp2, tanh, tan, pow), as
/// in `parity.rs`: the tolerance of the musical settings, relative to `max(1, |x|)`.
const TOLERANCE: f32 = 1e-5;
/// The tolerance while every parameter sits at a bound: five bands at +24 dB on one frequency
/// and +24 dB of output are +144 dB, which lifts the last-bit differences of `tan` and `pow`
/// between the two libms by as much, and the output protection then works on them.
const EXTREME_TOLERANCE: f32 = 2e-3;
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
    /// The tolerance of the samples that follow.
    Tolerance(f32),
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
    let p = |band: usize, what: BandParam| comp::band_param(band, what).unwrap();
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
                kind: comp::DESCRIPTOR.kind_id,
                state: encode_state(&[
                    (ParamId(p(comp::BELL_2, BandParam::Gain)), 9.0),
                    (ParamId(p(comp::BELL_2, BandParam::Q)), 4.0),
                    (ParamId(p(comp::HIGH_PASS, BandParam::On)), 1.0),
                    (ParamId(p(comp::HIGH_PASS, BandParam::Freq)), 120.0),
                ]),
            },
            note(60, 120),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[128; 40]));
    // A sweep of a bell, a shelf, the slopes and the low-pass, then a second note.
    submit(
        &[
            set(p(comp::BELL_2, BandParam::Freq), 6000.0),
            set(p(comp::LOW_SHELF, BandParam::Gain), -9.0),
            set(p(comp::HIGH_PASS, BandParam::Slope), comp::DB48 as f32),
            set(p(comp::LOW_PASS, BandParam::On), 1.0),
            set(p(comp::LOW_PASS, BandParam::Freq), 3000.0),
            note(67, 127),
        ],
        &mut steps,
    );
    steps.push(Step::Render(MIXED));
    submit(
        &[
            set(p(comp::BELL_1, BandParam::Gain), 12.0),
            set(p(comp::BELL_3, BandParam::On), 0.0),
            set(p(comp::HIGH_SHELF, BandParam::Gain), 6.0),
            set(p(comp::LOW_PASS, BandParam::Slope), comp::DB12 as f32),
            set(comp::OUTPUT, -6.0),
        ],
        &mut steps,
    );
    steps.push(Step::Render(&[777, 128, 2048, 2048, 2048]));
    // Every parameter at each of its bounds.
    steps.push(Step::Tolerance(EXTREME_TOLERANCE));
    for corner in 0..4u32 {
        let moves: Vec<M> = comp::DESCRIPTOR
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
    // Back to the defaults, a new note.
    let defaults: Vec<M> = comp::DESCRIPTOR
        .params
        .iter()
        .map(|p| set(p.id.0, p.default as f32))
        .chain([note(55, 100)])
        .collect();
    submit(&defaults, &mut steps);
    steps.push(Step::Render(&[2_048; 4]));
    // The tails of the extreme settings are gone: the musical tolerance again.
    steps.push(Step::Tolerance(TOLERANCE));
    steps.push(Step::Render(&[2_048; 10]));
    // Switched off and on: the state starts empty again.
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

/// The native render and the tolerance of each of its samples.
fn render_native(steps: &[Step]) -> (Vec<f32>, Vec<f32>) {
    let mut cycle = Cycle::new(RATE).unwrap();
    let (mut out, mut tolerance) = (Vec::new(), Vec::new());
    let mut tol = TOLERANCE;
    for step in steps {
        match step {
            Step::Tolerance(t) => tol = *t,
            Step::Submit(bytes) => cycle.submit(bytes).unwrap(),
            Step::Render(sizes) => {
                for size in *sizes {
                    let frames = cycle.render(*size);
                    out.extend_from_slice(&cycle.left()[..frames]);
                    out.extend_from_slice(&cycle.right()[..frames]);
                    tolerance.resize(out.len(), tol);
                }
            }
        }
    }
    (out, tolerance)
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
fn contour_renders_the_same_in_native_and_wasm() {
    let steps = scenario();
    let (native, tolerance) = render_native(&steps);
    let peak = native.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.1, "the scenario sounds: peak {peak}");
    assert!(native.iter().all(|s| s.is_finite()));

    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let mut wasm = Wasm::new(&bytes);
    let mut out = Vec::new();
    for step in &steps {
        match step {
            Step::Tolerance(_) => {}
            Step::Submit(stream) => assert_eq!(wasm.submit(stream), 0, "accepted"),
            Step::Render(sizes) => {
                for size in *sizes {
                    wasm.render(*size, &mut out);
                }
            }
        }
    }
    assert_eq!(out.len(), native.len());
    // The largest difference relative to max(1, |x|), at the musical and at the extreme
    // settings.
    let mut worst = [0f32; 2];
    for (i, (a, b)) in native.iter().zip(&out).enumerate() {
        let d = (a - b).abs() / a.abs().max(1.0);
        let extreme = tolerance[i] > TOLERANCE;
        assert!(d <= tolerance[i], "sample {i}: native {a}, wasm {b}");
        worst[usize::from(extreme)] = worst[usize::from(extreme)].max(d);
    }
    println!(
        "{} samples, largest difference {:e} at the musical settings (tolerance {TOLERANCE:e}), {:e} at the bounds (tolerance {EXTREME_TOLERANCE:e})",
        out.len(),
        worst[0],
        worst[1]
    );
}
