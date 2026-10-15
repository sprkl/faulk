//! Native/WASM parity of the timed events (MO-4): the scenario of `engine::events::testing`
//! (the test instrument and effect on one track, a schedule of notes with fine tuning and
//! parameter values for both devices, a loop, a seek, a stop and a play, odd block sizes)
//! rendered natively and through the real `.wasm` build, run by an interpreter, gives the
//! same signal, and its silences and onsets fall on the same samples.

use engine::events::testing::scenario;
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Module, Store};

/// libm differs slightly between the native targets and wasm32 (cos), as in
/// `device_parity.rs`.
const TOLERANCE: f32 = 1e-5;

/// Builds the module with the scenario's export in its own target directory, like
/// `device_parity.rs`.
fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity-events");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "-p", "engine-wasm", "--release"])
        .args(["--features", "events-scenario"])
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
    let run = instance
        .get_typed_func::<(), i32>(&store, "fk_events_scenario")
        .expect("export fk_events_scenario");
    let len = instance
        .get_typed_func::<(), i32>(&store, "fk_events_len")
        .expect("export fk_events_len");
    let memory = instance
        .get_memory(&store, "memory")
        .expect("exported memory");
    let at = run.call(&mut store, ()).unwrap() as usize;
    assert_ne!(at, 0, "the scenario rendered");
    let len = len.call(&mut store, ()).unwrap() as usize;
    let mut raw = vec![0u8; len * 4];
    memory.read(&store, at, &mut raw).unwrap();
    raw.as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

#[test]
fn timed_events_render_the_same_in_native_and_wasm() {
    let native = scenario().expect("the scenario renders");
    let wasm = render_wasm();
    assert_eq!(native.len(), wasm.len());
    assert!(
        native.iter().any(|s| s.abs() > 0.05),
        "the scenario must make sound"
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
    // Silence and onsets land on the same samples in both builds.
    for (i, (a, b)) in native.iter().zip(&wasm).enumerate() {
        assert_eq!(*a == 0.0, *b == 0.0, "silence differs at sample {i}");
    }
}
