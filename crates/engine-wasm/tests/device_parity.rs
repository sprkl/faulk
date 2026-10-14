//! Native/WASM parity of the device contract (CT-5): the scenario of
//! `engine::device::testing` (a MIDI effect, an instrument and an effect, with notes, fine
//! tuning and parameter events at odd offsets in varied block sizes) rendered natively and
//! through the real `.wasm` build, run by an interpreter, gives the same signal.

use engine::device::testing::{SCENARIO_FRAMES, scenario};
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Module, Store};

/// libm differs slightly between the native targets and wasm32 (cos, exp2), as in
/// `parity.rs`.
const TOLERANCE: f32 = 1e-5;

/// Builds the module with the contract's export in its own target directory: the feature
/// must not leak into the module `parity.rs` checks, and the build must not wait on the
/// lock of the `cargo test` running this test.
fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity-device");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "-p", "engine-wasm", "--release"])
        .args(["--features", "device-contract"])
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
        .get_typed_func::<(), i32>(&store, "fk_contract_scenario")
        .expect("export fk_contract_scenario");
    let len = instance
        .get_typed_func::<(), i32>(&store, "fk_contract_len")
        .expect("export fk_contract_len");
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
fn the_device_contract_renders_the_same_in_native_and_wasm() {
    let mut native = vec![0.0f32; 2 * SCENARIO_FRAMES];
    assert_eq!(scenario(&mut native), native.len());
    let wasm = render_wasm();
    assert_eq!(native.len(), wasm.len());
    assert!(
        native.iter().any(|s| s.abs() > 0.1),
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
    // Silence lands on the same samples in both builds (exact offsets survive the target).
    for (i, (a, b)) in native.iter().zip(&wasm).enumerate() {
        assert_eq!(*a == 0.0, *b == 0.0, "silence differs at sample {i}");
    }
}
