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
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state, encode_stream,
};
use engine::time::{Quantize, Tempo, TimeSignature};
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

#[test]
fn the_compensation_renders_the_same_in_native_and_wasm() {
    let steps = scenario();
    let native = render_native(&steps);
    let peak = native
        .iter()
        .flat_map(|(l, _, _)| l)
        .fold(0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "the scenario sounds: peak {peak}");
    assert_eq!(misalignment(&native), 0.0, "native: aligned to the sample");

    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let mut wasm = Wasm::new(&bytes);
    let mut out: Blocks = Vec::new();
    for step in &steps {
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
}
