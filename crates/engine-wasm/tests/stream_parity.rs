//! Native/WASM parity of the disk streaming (AU-3): the scenario of
//! `engine::audio_source::scenario` (a disk in memory, the host's test voice through a clip
//! start, a jump, a short loop from memory and a short loop through the ring, then the frames
//! each waited and the underruns) run natively and through the real `.wasm` build, run by an
//! interpreter, gives the same samples, bit for bit (the planner converts integers exactly).
//! The same for the audio clip player's scenario (AU-4, `player_scenario`: grid launches, a
//! placement entered by a seek, loops from memory and through the ring, a stop, then the
//! same clips resampled in an engine at 11 025 Hz): its
//! resampler and fades use integer phases and `f32` products only, so the builds agree to
//! the bit.

use engine::audio_source::scenario::{player_scenario, scenario};
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

/// The output of the module's scenario export `name` (its length from `len`).
fn render_wasm_of(name: &str, len: &str) -> Vec<f32> {
    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes[..]).expect("valid module");
    let mut store = Store::new(&engine, ());
    let instance = Linker::<()>::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .expect("the module needs no imports");
    let run = instance
        .get_typed_func::<(), i32>(&store, name)
        .expect("the scenario's export");
    let len = instance
        .get_typed_func::<(), i32>(&store, len)
        .expect("the scenario's length export");
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

/// The audio clip player's control streams (AU-4), recorded for the web test of the shipped
/// module: the setup (`player.stream`), the play and launch of clip 0 (`player-launch.stream`),
/// the launch of the looped clip 1 (`player-loop.stream`). `FAULK_RECORD=1` rewrites them.
#[test]
fn the_player_fixtures_are_the_scenario_s_control_streams() {
    use engine::audio_source::scenario::{launch, player_setup};
    use engine::msg::{ControlMsg, encode_stream};
    let mut setup = vec![ControlMsg::Reset { epoch: 1 }];
    setup.extend(player_setup());
    let streams = [
        ("player.stream", setup),
        (
            "player-launch.stream",
            vec![
                ControlMsg::Play {
                    from: engine::time::Tick(0),
                },
                launch(0),
            ],
        ),
        ("player-loop.stream", vec![launch(1)]),
    ];
    for (name, msgs) in streams {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let stream = encode_stream(1, 0, &msgs).unwrap();
        if std::env::var_os("FAULK_RECORD").is_some() {
            std::fs::write(&path, &stream).unwrap();
        }
        let recorded = std::fs::read(&path).expect("the fixture (FAULK_RECORD=1)");
        assert_eq!(recorded, stream, "{name}");
    }
}

fn render_wasm() -> Vec<f32> {
    render_wasm_of("fk_stream_scenario", "fk_stream_len")
}

#[test]
fn the_audio_clip_player_gives_the_same_samples_in_native_and_wasm() {
    let native = player_scenario();
    let wasm = render_wasm_of("fk_player_scenario", "fk_player_len");
    assert_eq!(native.len(), wasm.len());
    assert!(native.iter().any(|s| *s != 0.0), "the scenario must play");
    for (i, (a, b)) in native.iter().zip(&wasm).enumerate() {
        assert_eq!(a.to_bits(), b.to_bits(), "sample {i}: {a} native, {b} wasm");
    }
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
