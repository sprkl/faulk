//! The WASM render: the C ABI of the `engine-wasm` build, run in an interpreter.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use wasmi::{Engine, Linker, Module, Store, TypedFunc};

use crate::render::{QUANTUM, SAMPLE_RATE};
use crate::scenario::{Scenario, Step};
use crate::signal::Render;

/// Builds the wasm module in its own target directory, so the build does not wait on
/// the lock of the `cargo test` that runs the test.
pub(crate) fn wasm_module() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(build_wasm_once).clone()
}

fn build_wasm_once() -> PathBuf {
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

/// Renders the scenario through the `wasm32-unknown-unknown` build of `engine-wasm`
/// (`fk_push`, `fk_flush`, `fk_render`) in an interpreter. It is a sibling of the shipped
/// `wasm32-wasip1-threads` module, which the web tests run (`web/src/lib/engine/`): the
/// engine code is the same, the libm may differ. Panics when the module rejected,
/// skipped or still holds a message at the end.
pub fn render_wasm(scenario: &Scenario) -> Render {
    let bytes = std::fs::read(wasm_module()).expect("the wasm module exists");
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
    let new = func::<f32, i32>(&instance, &store, "fk_channel_new");
    let inbox = func::<(i32, i32), i32>(&instance, &store, "fk_inbox");
    let push = func::<(i32, i32, i32), i32>(&instance, &store, "fk_push");
    let render = func::<(i32, i32), i32>(&instance, &store, "fk_render");
    let left_ptr = func::<i32, i32>(&instance, &store, "fk_left");
    let right_ptr = func::<i32, i32>(&instance, &store, "fk_right");
    let flush = func::<i32, ()>(&instance, &store, "fk_flush");
    let prepare_counter = func::<(i32, i32), i32>(&instance, &store, "fk_prepare_counter");
    let host_counter = func::<(i32, i32), i32>(&instance, &store, "fk_host_counter");
    let memory = instance
        .get_memory(&store, "memory")
        .expect("exported memory");

    let ch = new.call(&mut store, SAMPLE_RATE).unwrap();
    assert_ne!(ch, 0);
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

    let (mut left, mut right) = (Vec::new(), Vec::new());
    for step in &scenario.steps {
        match step {
            Step::Push(bytes) => {
                let at = inbox.call(&mut store, (ch, bytes.len() as i32)).unwrap();
                memory.write(&mut store, at as usize, bytes).unwrap();
                let code = push.call(&mut store, (ch, at, bytes.len() as i32)).unwrap();
                assert_eq!(code, 0, "the module refused a push");
            }
            Step::Render(blocks) => {
                for _ in 0..*blocks {
                    flush.call(&mut store, ch).unwrap();
                    let frames = render.call(&mut store, (ch, QUANTUM as i32)).unwrap() as usize;
                    let l = left_ptr.call(&mut store, ch).unwrap() as usize;
                    let r = right_ptr.call(&mut store, ch).unwrap() as usize;
                    read(&store, l, frames, &mut left);
                    read(&store, r, frames, &mut right);
                }
            }
        }
    }
    flush.call(&mut store, ch).unwrap();
    let counter = |f: &TypedFunc<(i32, i32), i32>, store: &mut Store<()>, which| {
        f.call(store, (ch, which)).unwrap()
    };
    assert_eq!(
        counter(&prepare_counter, &mut store, 0),
        0,
        "messages were rejected"
    );
    assert_eq!(
        counter(&prepare_counter, &mut store, 1),
        0,
        "messages are still pending"
    );
    assert_eq!(
        counter(&host_counter, &mut store, 1),
        0,
        "messages were skipped"
    );
    Render::new(SAMPLE_RATE, left, right)
}
