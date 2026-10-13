//! Native/WASM parity: the same scripted performance rendered natively and through
//! the real `.wasm` build, run by an interpreter, must give the same signal.

use engine::Param;
use engine_wasm::Instance;
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Module, Store, TypedFunc};

const SAMPLE_RATE: f32 = 48_000.0;
const QUANTUM: u32 = 128;
/// libm differs slightly between the native targets and wasm32 (sin, exp2, tanh).
const TOLERANCE: f32 = 1e-5;

/// One step of the performance: an event, then `blocks` render quanta.
enum Step {
    NoteOn(u32, u32),
    NoteOff(u32),
    SetParam(Param, f32),
    Render(u32),
}

fn script() -> Vec<Step> {
    use Step::*;
    vec![
        SetParam(Param::Waveform, 1.0),
        NoteOn(60, 100),
        Render(40),
        NoteOn(64, 90),
        NoteOn(67, 80),
        Render(40),
        SetParam(Param::Waveform, 2.0),
        SetParam(Param::Gain, 0.4),
        Render(40),
        NoteOff(60),
        SetParam(Param::Waveform, 3.0),
        Render(40),
        SetParam(Param::Waveform, 0.0),
        SetParam(Param::Release, 0.05),
        NoteOff(64),
        NoteOff(67),
        Render(60),
    ]
}

fn render_native() -> Vec<f32> {
    let mut inst = Instance::new(SAMPLE_RATE);
    let mut out = Vec::new();
    for step in script() {
        match step {
            Step::NoteOn(n, v) => inst.synth_mut().note_on(n as u8, v as u8),
            Step::NoteOff(n) => inst.synth_mut().note_off(n as u8),
            Step::SetParam(p, v) => inst.synth_mut().set_param(p, v),
            Step::Render(blocks) => {
                for _ in 0..blocks {
                    let frames = inst.render(QUANTUM as usize);
                    out.extend_from_slice(&inst.left()[..frames]);
                    out.extend_from_slice(&inst.right()[..frames]);
                }
            }
        }
    }
    out
}

/// Builds the wasm module in its own target directory, so the build does not wait on
/// the lock of the `cargo test` that runs this test.
fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "-p", "engine-wasm", "--release"])
        .args(["--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(&target_dir)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the wasm build failed");
    target_dir.join("wasm32-unknown-unknown/release/engine_wasm.wasm")
}

fn render_wasm() -> Vec<f32> {
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
    let new = func::<f32, i32>(&instance, &store, "fk_new");
    let render = func::<(i32, i32), i32>(&instance, &store, "fk_render");
    let left_ptr = func::<i32, i32>(&instance, &store, "fk_left_ptr");
    let right_ptr = func::<i32, i32>(&instance, &store, "fk_right_ptr");
    let note_on = func::<(i32, i32, i32), ()>(&instance, &store, "fk_note_on");
    let note_off = func::<(i32, i32), ()>(&instance, &store, "fk_note_off");
    let set_param = func::<(i32, i32, f32), ()>(&instance, &store, "fk_set_param");
    let memory = instance
        .get_memory(&store, "memory")
        .expect("exported memory");

    let inst = new.call(&mut store, SAMPLE_RATE).unwrap();
    let left = left_ptr.call(&mut store, inst).unwrap() as usize;
    let right = right_ptr.call(&mut store, inst).unwrap() as usize;
    let read = |store: &Store<()>, at: usize, frames: usize, out: &mut Vec<f32>| {
        let mut bytes = vec![0u8; frames * 4];
        memory.read(store, at, &mut bytes).unwrap();
        out.extend(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        );
    };

    let mut out = Vec::new();
    for step in script() {
        match step {
            Step::NoteOn(n, v) => note_on
                .call(&mut store, (inst, n as i32, v as i32))
                .unwrap(),
            Step::NoteOff(n) => note_off.call(&mut store, (inst, n as i32)).unwrap(),
            Step::SetParam(p, v) => set_param
                .call(&mut store, (inst, p.id() as i32, v))
                .unwrap(),
            Step::Render(blocks) => {
                for _ in 0..blocks {
                    let frames = render.call(&mut store, (inst, QUANTUM as i32)).unwrap() as usize;
                    read(&store, left, frames, &mut out);
                    read(&store, right, frames, &mut out);
                }
            }
        }
    }
    out
}

#[test]
fn native_and_wasm_render_the_same_signal() {
    let native = render_native();
    let wasm = render_wasm();
    assert_eq!(native.len(), wasm.len());
    assert!(
        native.iter().any(|s| s.abs() > 0.1),
        "the script must make sound"
    );
    let (index, diff) = native
        .iter()
        .zip(&wasm)
        .map(|(a, b)| (a - b).abs())
        .enumerate()
        .fold((0, 0.0f32), |m, (i, d)| if d > m.1 { (i, d) } else { m });
    assert!(
        diff <= TOLERANCE,
        "largest difference {diff} at sample {index}"
    );
}

#[test]
fn null_instance_is_ignored() {
    // SAFETY: a null pointer is part of every export's contract.
    unsafe {
        let null = core::ptr::null_mut();
        engine_wasm::fk_note_on(null, 60, 100);
        engine_wasm::fk_set_param(null, 0, 1.0);
        assert_eq!(engine_wasm::fk_render(null, 128), 0);
        assert!(engine_wasm::fk_left_ptr(null).is_null());
        engine_wasm::fk_free(null);
    }
}

#[test]
fn render_size_is_clamped() {
    let mut inst = Instance::new(SAMPLE_RATE);
    assert_eq!(inst.render(100_000), engine_wasm::MAX_FRAMES);
}
