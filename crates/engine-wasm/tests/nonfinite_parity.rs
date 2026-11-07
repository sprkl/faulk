//! Native/WASM parity of the live report of non-finite samples (#667): the scenario of
//! `engine::host::testing::nonfinite_scenario` (NaN frames made before the output protection,
//! then a report after each block) run natively and through the real `.wasm` build, run by an
//! interpreter, gives the same report bytes and the same output, bit for bit; the reports
//! carry 0, 3 then 131 silenced frames.

use engine::host::{decode_feedback, testing::nonfinite_scenario};
use engine::msg::Feedback;
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Module, Store};

/// Builds the module with the scenario's export in its own target directory, like
/// `stream_parity.rs`.
fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity-nonfinite");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "-p", "engine-wasm", "--release"])
        .args(["--features", "nonfinite-scenario"])
        .args(["--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(&target_dir)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the wasm build failed");
    target_dir.join("wasm32-unknown-unknown/release/engine_wasm.wasm")
}

fn render_wasm() -> Vec<u8> {
    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes[..]).expect("valid module");
    let mut store = Store::new(&engine, ());
    let instance = Linker::<()>::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .expect("the module needs no imports");
    let run = instance
        .get_typed_func::<(), i32>(&store, "fk_nonfinite_scenario")
        .expect("the scenario's export");
    let len = instance
        .get_typed_func::<(), i32>(&store, "fk_nonfinite_len")
        .expect("the scenario's length export");
    let memory = instance
        .get_memory(&store, "memory")
        .expect("exported memory");
    let at = run.call(&mut store, ()).unwrap() as usize;
    assert_ne!(at, 0, "the scenario rendered");
    let len = len.call(&mut store, ()).unwrap() as usize;
    let mut raw = vec![0u8; len];
    memory.read(&store, at, &mut raw).unwrap();
    raw
}

/// The three reports of the scenario's bytes, decoded, and the last block's output.
fn split(bytes: &[u8]) -> (Vec<Feedback>, Vec<f32>) {
    let report = Feedback::size_for(0, 0, 0, 0);
    assert_eq!(
        bytes.len(),
        3 * report + 2 * 128 * 4,
        "three reports, then the output"
    );
    let reports = (0..3)
        .map(|i| decode_feedback(&bytes[i * report..(i + 1) * report]).expect("a report"))
        .collect();
    let out = bytes[3 * report..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    (reports, out)
}

#[test]
fn the_native_scenario_reports_each_silenced_frame_and_stays_finite() {
    let (reports, out) = split(&nonfinite_scenario());
    let silenced: Vec<u32> = reports.iter().map(|r| r.silenced_frames).collect();
    assert_eq!(silenced, [0, 3, 131]);
    assert!(out.iter().all(|s| s.is_finite()), "the output stays finite");
}

#[test]
fn the_wasm_build_reports_the_same_bytes_as_the_native_one() {
    let native = nonfinite_scenario();
    let wasm = render_wasm();
    assert_eq!(native.len(), wasm.len());
    assert!(native == wasm, "the report bytes and the output differ");
    let (reports, _) = split(&wasm);
    assert_eq!(reports[2].silenced_frames, 131);
}

/// The scenario's bytes, recorded for the page's decoder test (`web/src/lib/engine/feedback.test.ts`): the
/// engine's own reports, with 0, 3 then 131 silenced frames. `FAULK_RECORD=1` rewrites it.
#[test]
fn the_nonfinite_fixture_is_the_scenario_s_bytes() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nonfinite.feedback");
    let bytes = nonfinite_scenario();
    if std::env::var_os("FAULK_RECORD").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    }
    let recorded =
        std::fs::read(&path).expect("tests/fixtures/nonfinite.feedback (FAULK_RECORD=1)");
    assert!(recorded == bytes, "the fixture is stale: FAULK_RECORD=1");
}
