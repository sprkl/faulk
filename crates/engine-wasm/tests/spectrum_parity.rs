//! Native/WASM parity of the spectrum analysis (#156): the scenario of
//! `engine::host::testing::spectrum_scenario` (a synth through two Contours, the windows copied by
//! the audio side, analysed by the prepare side, carried by the report) run natively and through
//! the real `.wasm` build, run by an interpreter, gives the same spectra: the same slots and
//! serials, and bands within `TOLERANCE_DB` (the last bits of `sin`, `cos`, `log10`, `tan` and `pow`
//! differ between the two libms, and a band is the log of a sum).

use engine::analysis::band_edge;
use engine::host::{decode_feedback, testing::spectrum_scenario};
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Module, Store};

/// The largest gap between the two builds' bands, in dB.
const TOLERANCE_DB: f32 = 1e-3;

/// Builds the module with the scenario's export in its own target directory, like
/// `nonfinite_parity.rs`.
fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity-spectrum");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "-p", "engine-wasm", "--release"])
        .args(["--features", "spectrum-scenario"])
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
        .get_typed_func::<(), i32>(&store, "fk_spectrum_scenario")
        .expect("the scenario's export");
    let len = instance
        .get_typed_func::<(), i32>(&store, "fk_spectrum_len")
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

#[test]
fn the_spectra_of_the_report_are_the_same_in_native_and_wasm() {
    let native = decode_feedback(&spectrum_scenario()).expect("a native report");
    let wasm = decode_feedback(&render_wasm()).expect("a wasm report");
    assert_eq!(native.spectra.len(), 2, "both Contours analysed");
    assert_eq!(wasm.spectra.len(), native.spectra.len());
    let mut worst = 0f32;
    for (n, w) in native.spectra.iter().zip(&wasm.spectra) {
        assert_eq!((n.track, n.device, n.serial), (w.track, w.device, w.serial));
        assert_eq!(n.sample_rate, w.sample_rate);
        for (i, (a, b)) in n.bands.iter().zip(&w.bands).enumerate() {
            let d = (a - b).abs();
            worst = worst.max(d);
            assert!(
                d <= TOLERANCE_DB,
                "band {i} ({} Hz): native {a}, wasm {b}",
                band_edge(i)
            );
        }
    }
    // The scenario sounds: bands well above the floor.
    assert!(native.spectra[0].bands.iter().any(|b| *b > -60.0));
    println!(
        "{} spectra of {} bands, largest gap {worst:e} dB (tolerance {TOLERANCE_DB:e})",
        native.spectra.len(),
        native.spectra[0].bands.len()
    );
}
