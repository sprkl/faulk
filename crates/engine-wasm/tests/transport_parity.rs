//! Native/WASM parity of the transport clock (TR-1): the scenario of
//! `engine::transport::testing` (odd block sizes, a loop, non-dyadic tempos, a seek, a stop
//! and a play) run natively and through the real `.wasm` build, run by an interpreter, gives
//! the same segments, ticks, offsets and positions, bit for bit.

use engine::transport::testing::scenario;
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Module, Store};

/// Builds the module with the scenario's export in its own target directory, like
/// `device_parity.rs`.
fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity-transport");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "-p", "engine-wasm", "--release"])
        .args(["--features", "transport-scenario"])
        .args(["--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(&target_dir)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the wasm build failed");
    target_dir.join("wasm32-unknown-unknown/release/engine_wasm.wasm")
}

fn run_wasm() -> Vec<i64> {
    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes[..]).expect("valid module");
    let mut store = Store::new(&engine, ());
    let instance = Linker::<()>::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .expect("the module needs no imports");
    let run = instance
        .get_typed_func::<(), i32>(&store, "fk_transport_scenario")
        .expect("export fk_transport_scenario");
    let len = instance
        .get_typed_func::<(), i32>(&store, "fk_transport_len")
        .expect("export fk_transport_len");
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
fn the_transport_clock_is_identical_in_native_and_wasm() {
    let native = scenario();
    assert!(native.len() > 4000, "the scenario produces segments");
    let wasm = run_wasm();
    assert_eq!(native.len(), wasm.len());
    if let Some(i) = native.iter().zip(&wasm).position(|(a, b)| a != b) {
        panic!(
            "first difference at value {i}: native {} wasm {}",
            native[i], wasm[i]
        );
    }
}
