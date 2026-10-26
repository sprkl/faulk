//! The warm-up of the apply path (#355) through the real `.wasm` build, run by an interpreter:
//! `fk_warm` runs every message on a private channel and reports success, and the channel the
//! host plays is left as it was (nothing applied, nothing rendered but silence).

use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Module, Store};

const SAMPLE_RATE: f32 = 48_000.0;

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

#[test]
fn the_warm_up_succeeds_and_leaves_the_real_channel_alone() {
    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes[..]).expect("valid module");
    let mut store = Store::new(&engine, ());
    let instance = Linker::<()>::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .expect("the module needs no imports");
    let new = instance
        .get_typed_func::<f32, i32>(&store, "fk_channel_new")
        .unwrap();
    let warm = instance
        .get_typed_func::<f32, i32>(&store, "fk_warm")
        .expect("export fk_warm");
    let counter = instance
        .get_typed_func::<(i32, i32), i32>(&store, "fk_host_counter")
        .unwrap();
    let render = instance
        .get_typed_func::<(i32, i32), i32>(&store, "fk_render")
        .unwrap();
    let left = instance
        .get_typed_func::<i32, i32>(&store, "fk_left")
        .unwrap();
    let memory = instance.get_memory(&store, "memory").unwrap();

    let ch = new.call(&mut store, SAMPLE_RATE).unwrap();
    assert_ne!(ch, 0);
    let counters = |store: &mut Store<()>| {
        (0..5)
            .map(|which| counter.call(&mut *store, (ch, which)).unwrap())
            .collect::<Vec<_>>()
    };
    let before = counters(&mut store);
    assert_eq!(warm.call(&mut store, SAMPLE_RATE).unwrap(), 0);
    assert_eq!(counters(&mut store), before);
    assert_eq!(before[0], 0, "nothing applied on the real channel");
    // The real channel still renders silence: the warm-up's sound stayed in its own host.
    assert_eq!(render.call(&mut store, (ch, 128)).unwrap(), 128);
    let at = left.call(&mut store, ch).unwrap() as usize;
    let mut block = vec![0u8; 128 * 4];
    memory.read(&store, at, &mut block).unwrap();
    assert!(block.iter().all(|b| *b == 0), "silence");
    // A second warm-up behaves as the first.
    assert_eq!(warm.call(&mut store, SAMPLE_RATE).unwrap(), 0);
}
