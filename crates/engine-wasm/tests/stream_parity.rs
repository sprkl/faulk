//! Native/WASM parity of the disk streaming (AU-3): the scenario of
//! `engine::audio_source::scenario` (a disk in memory, the host's test voice through a clip
//! start, a jump, a short loop from memory and a short loop through the ring, then the frames
//! each waited and the underruns) run natively and through the real `.wasm` build, run by an
//! interpreter, gives the same samples, bit for bit (the planner converts integers exactly).

use engine::audio_source::scenario::scenario;
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Module, Store};

/// Builds the module with the scenario's export in its own target directory, like
/// `device_parity.rs`.
fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity-stream");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "-p", "engine-wasm", "--release"])
        .args(["--features", "stream-scenario"])
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
        .get_typed_func::<(), i32>(&store, "fk_stream_scenario")
        .expect("export fk_stream_scenario");
    let len = instance
        .get_typed_func::<(), i32>(&store, "fk_stream_len")
        .expect("export fk_stream_len");
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

/// The scenario's control stream, recorded for the web test of the shipped module
/// (`web/src/lib/audio/stream/stream.test.ts`): `FAULK_RECORD=1` rewrites it.
#[test]
fn the_stream_fixture_is_the_scenario_s_control_stream() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stream.stream");
    let stream = engine::audio_source::scenario::rig_stream();
    if std::env::var_os("FAULK_RECORD").is_some() {
        std::fs::write(&path, &stream).unwrap();
    }
    let recorded = std::fs::read(&path).expect("tests/fixtures/stream.stream (FAULK_RECORD=1)");
    assert_eq!(recorded, stream);
}

#[test]
fn disk_streaming_gives_the_same_samples_in_native_and_wasm() {
    let native = scenario();
    let wasm = render_wasm();
    assert_eq!(native.len(), wasm.len());
    assert!(native.iter().any(|s| *s != 0.0), "the scenario must play");
    for (i, (a, b)) in native.iter().zip(&wasm).enumerate() {
        assert_eq!(a.to_bits(), b.to_bits(), "sample {i}: {a} native, {b} wasm");
    }
}
