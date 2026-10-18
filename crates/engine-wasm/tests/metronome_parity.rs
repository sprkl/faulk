//! Native/WASM parity of the metronome (TR-2) and of the clock wiring: the same messages
//! and block sizes (odd ones, a loop, a count-in, 6/8 at a fractional tempo) through the
//! native `Cycle` and through the real `.wasm` build (the `engine-cycle` exports, run by an
//! interpreter) give the same click onsets, sample for sample, and the same signal within
//! the libm tolerance of `cos` and `exp`.

use engine::engine::cycle::{Cycle, SCENARIO_RATE};
use engine::msg::{ControlMsg as M, encode_stream};
use engine::time::{LoopRegion, Quantize, Tempo, Tick, TimeSignature};
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Memory, Module, Store, TypedFunc};

/// libm differs slightly between native targets and wasm32 (cos, exp).
const TOLERANCE: f32 = 1e-5;

const SIZES: [usize; 6] = [1, 7, 127, 480, 64, 1000];

enum Step {
    Submit(Vec<u8>),
    Render(usize),
}

fn scenario() -> Vec<Step> {
    let msgs = |m: &[M], seq: u32| Step::Submit(encode_stream(1, seq, m).unwrap());
    let mut steps = vec![msgs(
        &[
            M::Reset { epoch: 1 },
            M::SetTransport {
                tempo: Tempo::new(97.3).unwrap(),
                signature: TimeSignature::new(6, 8).unwrap(),
                launch_quantize: Quantize::default(),
            },
            M::SetMetronome {
                enabled: true,
                volume: 0.8,
                count_in_bars: 1,
            },
            M::SetLoop {
                region: LoopRegion::new(Tick(0), Tick(2880 * 2)),
            },
            M::SetRecording { recording: true },
            M::Play { from: Tick(0) },
        ],
        0,
    )];
    for i in 0..400 {
        steps.push(Step::Render(SIZES[i % SIZES.len()]));
    }
    steps.push(msgs(
        &[
            M::SetTransport {
                tempo: Tempo::new(140.0).unwrap(),
                signature: TimeSignature::COMMON,
                launch_quantize: Quantize::default(),
            },
            M::SetMetronome {
                enabled: true,
                volume: 1.0,
                count_in_bars: 0,
            },
        ],
        6,
    ));
    for i in 0..400 {
        steps.push(Step::Render(SIZES[i % SIZES.len()]));
    }
    steps
}

fn run_native() -> Vec<f32> {
    let mut cycle = Cycle::new(SCENARIO_RATE).unwrap();
    let mut out = Vec::new();
    for step in scenario() {
        match step {
            Step::Submit(bytes) => cycle.submit(&bytes).unwrap(),
            Step::Render(n) => {
                let frames = cycle.render(n);
                assert_eq!(cycle.left()[..frames], cycle.right()[..frames]);
                out.extend_from_slice(&cycle.left()[..frames]);
            }
        }
    }
    out
}

fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity-metronome");
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

fn run_wasm() -> Vec<f32> {
    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes[..]).expect("valid module");
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
    let inbox = func::<(i32, i32), i32>(&instance, &store, "fk_cycle_inbox");
    let submit = func::<(i32, i32), i32>(&instance, &store, "fk_cycle_submit");
    let render = func::<(i32, i32), i32>(&instance, &store, "fk_cycle_render");
    let left = func::<i32, i32>(&instance, &store, "fk_cycle_left");
    let right = func::<i32, i32>(&instance, &store, "fk_cycle_right");
    let memory: Memory = instance.get_memory(&store, "memory").expect("memory");
    let cycle = new.call(&mut store, SCENARIO_RATE).unwrap();
    assert_ne!(cycle, 0);
    let mut out = Vec::new();
    for step in scenario() {
        match step {
            Step::Submit(bytes) => {
                let at = inbox.call(&mut store, (cycle, bytes.len() as i32)).unwrap();
                assert_ne!(at, 0);
                memory.write(&mut store, at as usize, &bytes).unwrap();
                let code = submit
                    .call(&mut store, (cycle, bytes.len() as i32))
                    .unwrap();
                assert_eq!(code, 0, "the stream was accepted");
            }
            Step::Render(n) => {
                let frames = render.call(&mut store, (cycle, n as i32)).unwrap() as usize;
                let at = left.call(&mut store, cycle).unwrap() as usize;
                let mut raw = vec![0u8; frames * 4];
                memory.read(&store, at, &mut raw).unwrap();
                let mut raw_right = vec![0u8; frames * 4];
                let at_right = right.call(&mut store, cycle).unwrap() as usize;
                memory.read(&store, at_right, &mut raw_right).unwrap();
                assert_eq!(raw, raw_right, "the click is centred");
                out.extend(
                    raw.as_chunks::<4>()
                        .0
                        .iter()
                        .map(|b| f32::from_le_bytes(*b)),
                );
            }
        }
    }
    out
}

/// Samples where a click starts: a non-zero sample after at least 1000 zeros.
fn onsets(out: &[f32]) -> Vec<usize> {
    let mut quiet = 1000;
    let mut found = Vec::new();
    for (i, s) in out.iter().enumerate() {
        if *s != 0.0 {
            if quiet >= 1000 {
                found.push(i);
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    found
}

#[test]
fn the_metronome_is_identical_in_native_and_wasm() {
    let native = run_native();
    let wasm = run_wasm();
    assert_eq!(native.len(), wasm.len());
    let clicks = onsets(&native);
    assert!(clicks.len() > 10, "the scenario clicks: {}", clicks.len());
    assert_eq!(clicks, onsets(&wasm), "the onsets are the same samples");
    if let Some(i) = native
        .iter()
        .zip(&wasm)
        .position(|(a, b)| (a - b).abs() > TOLERANCE)
    {
        panic!(
            "first difference at {i}: native {} wasm {}",
            native[i], wasm[i]
        );
    }
}
