//! The prepare/apply cycle (MO-1) in the real `.wasm` build, run by an interpreter:
//!
//! - **parity**: the scenario of `engine::engine::cycle` (a held note while every runtime
//!   table grows) gives the same signal and the same counters as the native build;
//! - **linear memory growth during playback**: the scenario's large upload makes `prepare`
//!   grow the module's memory between two rendered blocks; the render goes on without a
//!   silent block and matches the native render;
//! - **refused growth**: under a memory cap, an upload whose staging does not fit returns
//!   the typed `OutOfMemory` code; the instance keeps rendering and accepts what follows.
//!
//! The web module's channel (two instances on a shared memory, MO-2) is not this one: here
//! one instance runs both sides, one call after another, on `wasm32-unknown-unknown`.

use engine::engine::cycle::{
    Cycle, SCENARIO_BLOCK, SCENARIO_RATE, Step, scenario, scenario_held_blocks,
};
use engine::msg::{ClipSlot, ControlMsg, MsgError, Note, TrackKind, TrackSlot, encode_stream};
use engine::time::Tick;
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

/// libm differs slightly between the native targets and wasm32 (cos), as in `parity.rs`.
const TOLERANCE: f32 = 1e-5;

/// Counters compared between the builds (`fk_cycle_counter`).
const COUNTERS: u32 = 10;

/// Builds the module with the cycle's exports in its own target directory (the feature
/// must not leak into the module `parity.rs` checks, and the build must not wait on the
/// lock of the `cargo test` running this test).
fn build_wasm() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target/parity-cycle");
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

/// One instance of the module and its exports.
struct Wasm {
    store: Store<StoreLimits>,
    memory: Memory,
    cycle: i32,
    inbox: TypedFunc<(i32, i32), i32>,
    submit: TypedFunc<(i32, i32), i32>,
    render: TypedFunc<(i32, i32), i32>,
    left: TypedFunc<i32, i32>,
    right: TypedFunc<i32, i32>,
    counter: TypedFunc<(i32, i32), i32>,
}

impl Wasm {
    /// An instance whose memory may not grow beyond `max_bytes`.
    fn new(bytes: &[u8], max_bytes: usize) -> Self {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes).expect("valid module");
        let limits = StoreLimitsBuilder::new().memory_size(max_bytes).build();
        let mut store = Store::new(&engine, limits);
        store.limiter(|limits| limits);
        let instance = Linker::<StoreLimits>::new(&engine)
            .instantiate_and_start(&mut store, &module)
            .expect("the module needs no imports");
        fn func<P: wasmi::WasmParams, R: wasmi::WasmResults>(
            i: &wasmi::Instance,
            s: &Store<StoreLimits>,
            name: &str,
        ) -> TypedFunc<P, R> {
            i.get_typed_func::<P, R>(s, name)
                .unwrap_or_else(|e| panic!("export {name}: {e}"))
        }
        let new = func::<f32, i32>(&instance, &store, "fk_cycle_new");
        let memory = instance.get_memory(&store, "memory").expect("memory");
        let cycle = new.call(&mut store, SCENARIO_RATE).unwrap();
        assert_ne!(cycle, 0, "the cycle was built");
        Wasm {
            inbox: func(&instance, &store, "fk_cycle_inbox"),
            submit: func(&instance, &store, "fk_cycle_submit"),
            render: func(&instance, &store, "fk_cycle_render"),
            left: func(&instance, &store, "fk_cycle_left"),
            right: func(&instance, &store, "fk_cycle_right"),
            counter: func(&instance, &store, "fk_cycle_counter"),
            store,
            memory,
            cycle,
        }
    }

    fn memory_bytes(&self) -> usize {
        self.memory.data_size(&self.store)
    }

    /// Writes `bytes` into the inbox and submits them: the status code.
    fn submit(&mut self, bytes: &[u8]) -> i32 {
        let at = self
            .inbox
            .call(&mut self.store, (self.cycle, bytes.len() as i32))
            .unwrap();
        // The inbox must fit: an `OutOfMemory` code must come from `prepare`, not from
        // the host's own buffer.
        assert_ne!(at, 0, "the inbox of {} bytes was refused", bytes.len());
        self.memory
            .write(&mut self.store, at as usize, bytes)
            .unwrap();
        self.submit
            .call(&mut self.store, (self.cycle, bytes.len() as i32))
            .unwrap()
    }

    /// Renders one block and appends both channels to `out`.
    fn render(&mut self, out: &mut Vec<f32>) {
        let frames = self
            .render
            .call(&mut self.store, (self.cycle, SCENARIO_BLOCK as i32))
            .unwrap() as usize;
        // The views are taken after the call: a growth during it leaves them valid.
        for side in [self.left, self.right] {
            let at = side.call(&mut self.store, self.cycle).unwrap() as usize;
            let mut raw = vec![0u8; frames * 4];
            self.memory.read(&self.store, at, &mut raw).unwrap();
            out.extend(
                raw.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b)),
            );
        }
    }

    fn counters(&mut self) -> Vec<u32> {
        (0..COUNTERS)
            .map(|w| {
                self.counter
                    .call(&mut self.store, (self.cycle, w as i32))
                    .unwrap() as u32
            })
            .collect()
    }
}

/// Peak of each block of `out` (blocks of both channels, `SCENARIO_BLOCK` frames each).
fn block_peaks(out: &[f32]) -> Vec<f32> {
    out.chunks(2 * SCENARIO_BLOCK)
        .map(|b| b.iter().fold(0f32, |m, s| m.max(s.abs())))
        .collect()
}

fn render_native(steps: &[Step]) -> (Vec<f32>, Vec<u32>) {
    let mut cycle = Cycle::new(SCENARIO_RATE).unwrap();
    let mut out = Vec::new();
    for step in steps {
        match step {
            Step::Submit(bytes) => cycle.submit(bytes).unwrap(),
            Step::Render(n) => {
                for _ in 0..*n {
                    let frames = cycle.render(SCENARIO_BLOCK);
                    out.extend_from_slice(&cycle.left()[..frames]);
                    out.extend_from_slice(&cycle.right()[..frames]);
                }
            }
        }
    }
    (out, (0..COUNTERS).map(|w| cycle.counter(w)).collect())
}

#[test]
fn the_cycle_renders_the_same_in_native_and_wasm_while_the_memory_grows() {
    let steps = scenario().unwrap();
    let (native, native_counters) = render_native(&steps);
    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let mut wasm = Wasm::new(&bytes, usize::MAX);

    let mut out = Vec::new();
    let mut grown_by_prepare = 0usize;
    for step in &steps {
        match step {
            Step::Submit(stream) => {
                // The inbox may grow the memory itself; what grows during `submit` is
                // `prepare`'s (staging, tables, devices).
                let at = wasm
                    .inbox
                    .call(&mut wasm.store, (wasm.cycle, stream.len() as i32))
                    .unwrap();
                assert_ne!(at, 0);
                wasm.memory
                    .write(&mut wasm.store, at as usize, stream)
                    .unwrap();
                let before = wasm.memory_bytes();
                let status = wasm
                    .submit
                    .call(&mut wasm.store, (wasm.cycle, stream.len() as i32))
                    .unwrap();
                assert_eq!(status, 0, "the scenario is accepted");
                grown_by_prepare += wasm.memory_bytes() - before;
            }
            Step::Render(n) => {
                for _ in 0..*n {
                    wasm.render(&mut out);
                }
            }
        }
    }
    eprintln!(
        "memory grown by prepare during playback: {} MiB (now {} MiB)",
        grown_by_prepare >> 20,
        wasm.memory_bytes() >> 20
    );
    assert!(
        grown_by_prepare >= 4 << 20,
        "prepare grew the linear memory during playback ({grown_by_prepare} bytes)"
    );

    // No gap: every block before the release sounds, in the WASM build too.
    let held = scenario_held_blocks(&steps);
    let peaks = block_peaks(&out);
    assert_eq!(peaks.len(), held + 8);
    for (i, peak) in peaks[..held].iter().enumerate() {
        assert!(*peak > 0.01, "block {i} is silent in the WASM build");
    }

    // Parity: the same samples and the same counters.
    assert_eq!(native.len(), out.len());
    let (index, diff) = native
        .iter()
        .zip(&out)
        .map(|(a, b)| (a - b).abs())
        .enumerate()
        .fold((0, 0.0f32), |m, (i, d)| if d > m.1 { (i, d) } else { m });
    assert!(
        diff <= TOLERANCE,
        "largest difference {diff} at sample {index}"
    );
    for (i, (a, b)) in native.iter().zip(&out).enumerate() {
        assert_eq!(*a == 0.0, *b == 0.0, "silence differs at sample {i}");
    }
    assert_eq!(native_counters, wasm.counters());
    assert_eq!(
        native_counters[0..4],
        [0, 0, 0, 0],
        "nothing rejected, skipped, leaked or pending"
    );
}

#[test]
fn a_refused_memory_growth_is_a_typed_error_and_the_instance_plays_on() {
    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    // Room for the module and the cycle, not for a large staging.
    let mut wasm = Wasm::new(&bytes, 8 << 20);
    let start = encode_stream(
        1,
        0,
        &[
            ControlMsg::Reset { epoch: 1 },
            ControlMsg::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
            ControlMsg::DeviceInsert {
                track: TrackSlot(0),
                device: engine::msg::DeviceSlot(0),
                kind: engine::device::testing::TONE.kind_id,
                state: vec![],
            },
            ControlMsg::NoteOn {
                track: TrackSlot(0),
                pitch: 69,
                velocity: 100,
                at: None,
            },
        ],
    )
    .unwrap();
    assert_eq!(wasm.submit(&start), 0);
    let mut out = Vec::new();
    for _ in 0..4 {
        wasm.render(&mut out);
    }
    // The first chunk of a 1 M-note upload: its staging (24 MiB) does not fit.
    let size_before = wasm.memory_bytes();
    let chunk = encode_stream(
        1,
        4,
        &[ControlMsg::ClipNotesUpload {
            clip: ClipSlot(0),
            length: Tick(960),
            looping: None,
            first: 0,
            total: 1 << 20,
            notes: vec![Note::default(); 1000],
        }],
    )
    .unwrap();
    assert_eq!(
        wasm.submit(&chunk),
        MsgError::OutOfMemory { bytes: 0 }.code(),
        "a refused memory.grow is the typed OutOfMemory"
    );
    assert!(wasm.memory_bytes() <= 8 << 20);
    eprintln!(
        "memory before the refused upload {} KiB, after {} KiB, cap 8 MiB",
        size_before >> 10,
        wasm.memory_bytes() >> 10
    );
    // The instance lives on: it renders, the note still sounds, and a small message lands.
    for _ in 0..4 {
        wasm.render(&mut out);
    }
    let peaks = block_peaks(&out);
    assert!(
        peaks.iter().all(|p| *p > 0.01),
        "no silent block: {peaks:?}"
    );
    let more = encode_stream(
        1,
        5,
        &[ControlMsg::TrackCreate {
            track: TrackSlot(1),
            kind: TrackKind::Midi,
        }],
    )
    .unwrap();
    assert_eq!(wasm.submit(&more), 0);
    wasm.render(&mut out);
    assert_eq!(wasm.counters()[5], 2, "two tracks held");
    assert_eq!(wasm.counters()[0..3], [0, 0, 0]);
}
