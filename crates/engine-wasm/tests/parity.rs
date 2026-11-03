//! Native/WASM parity on a recorded message stream (MO-2): a session of the page (the setup,
//! notes, chords, every synth parameter, the panic button), encoded by the command layer
//! (`commands::engine_msgs`), plus a growth of the track table to 64 tracks while notes
//! sound, recorded in `tests/fixtures/session.stream`. It is replayed natively through
//! `engine::host` and through the module's `fk_push` / `fk_render` in an interpreter
//! (`wasm32-unknown-unknown`, one instance holding both halves); the outputs must agree.
//! The native render is saved as `tests/fixtures/session.native.f32`, the reference the
//! Node test of the shipped `wasm32-wasip1-threads` module compares against
//! (`web/src/lib/engine/channel.test.ts`).
//!
//! A second recording, `tests/fixtures/growth.stream`, fills the track table to its limit
//! (4 095 more tracks, each with a synth, pushed 256 at a time): the load of the shared-memory
//! test in Node. A third, `tests/fixtures/tracks.stream`, fills it with empty tracks (pushes
//! of 512): the load of the Chromium measurement, whose render must stay real-time. No
//! message of these tests is ever encoded outside Rust.
//!
//! `FAULK_RECORD=1 cargo test -p engine-wasm --test parity` rewrites the fixtures.

use std::path::PathBuf;
use std::process::Command;

use commands::engine_msgs::{EngineStream, Synced, single_synth};
use commands::{Command as Cmd, device, live};
use engine::OSC_SYNTH;
use engine::msg::{ControlMsg, DeviceSlot, TrackKind, TrackSlot};
use wasmi::{Engine, Linker, Module, Store, TypedFunc};

const SAMPLE_RATE: f32 = 48_000.0;
const QUANTUM: u32 = 128;
/// libm differs slightly between the native targets and wasm32 (sin, exp2, tanh).
const TOLERANCE: f32 = 1e-5;

/// One step of a recorded session.
#[derive(Debug, PartialEq)]
enum Step {
    Push(Vec<u8>),
    Render(u32),
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The session, as the command layer encodes it today.
fn record() -> Vec<Step> {
    use Step::*;
    let track = single_synth::track;
    let synth = single_synth::device;
    let on = |n: u8, v: u8| {
        Cmd::Live(live::Command::NoteOn {
            track: track(),
            note: n,
            velocity: v,
        })
    };
    let off = |n: u8| {
        Cmd::Live(live::Command::NoteOff {
            track: track(),
            note: n,
        })
    };
    let set = |p: &str, v: f64| {
        Cmd::Device(device::Command::SetParam {
            device: synth(),
            param: p.into(),
            value: v,
        })
    };
    let mut s = Synced::single_synth(1);
    let mut steps = vec![Push(s.setup().unwrap())];
    macro_rules! c {
        ($cmd:expr) => {
            Push(s.run(&$cmd).unwrap().1)
        };
    }
    steps.push(c!(set("waveform", 1.0)));
    steps.push(c!(on(60, 100)));
    steps.push(Render(40));
    steps.push(c!(on(64, 90)));
    steps.push(c!(on(67, 80)));
    steps.push(Render(40));
    steps.push(c!(set("waveform", 2.0)));
    steps.push(c!(set("gain", 0.4)));
    steps.push(c!(set("attack", 0.01)));
    steps.push(c!(set("decay", 0.1)));
    steps.push(c!(set("sustain", 0.5)));
    steps.push(Render(40));
    steps.push(c!(off(60)));
    steps.push(c!(set("waveform", 3.0)));
    steps.push(Render(40));
    // The track table grows from 16 to 64 slots while the chord sounds.
    let mut msgs = Vec::new();
    for slot in 1..48u32 {
        msgs.push(ControlMsg::TrackCreate {
            track: TrackSlot(slot),
            kind: TrackKind::Midi,
        });
        msgs.push(ControlMsg::DeviceInsert {
            track: TrackSlot(slot),
            device: DeviceSlot(0),
            kind: OSC_SYNTH.kind_id,
            state: Vec::new(),
        });
    }
    steps.push(Push(s.encode(&msgs).unwrap()));
    steps.push(Render(20));
    steps.push(c!(set("waveform", 0.0)));
    steps.push(c!(set("release", 0.05)));
    steps.push(c!(off(64)));
    steps.push(c!(Cmd::Live(live::Command::AllNotesOff {})));
    steps.push(Render(60));
    steps
}

/// Tracks of the growth recording, after the session's track 0: up to the limit.
const GROWTH_TRACKS: u32 = engine::msg::limits::MAX_TRACKS - 1;
/// Tracks per push of the growth recording.
const GROWTH_PER_PUSH: u32 = 256;

/// The growth: tracks 1 to 4 095 with a synth each, in pushes of 256 tracks with two blocks
/// rendered between them, in the session's epoch (it follows the session's setup).
fn record_growth() -> Vec<Step> {
    let mut s = EngineStream::new(1);
    let mut steps = Vec::new();
    let mut slot = 1;
    while slot <= GROWTH_TRACKS {
        let mut msgs = Vec::new();
        for t in slot..(slot + GROWTH_PER_PUSH).min(GROWTH_TRACKS + 1) {
            msgs.push(ControlMsg::TrackCreate {
                track: TrackSlot(t),
                kind: TrackKind::Midi,
            });
            msgs.push(ControlMsg::DeviceInsert {
                track: TrackSlot(t),
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: Vec::new(),
            });
        }
        steps.push(Step::Push(s.encode(&msgs).unwrap()));
        steps.push(Step::Render(2));
        slot += GROWTH_PER_PUSH;
    }
    steps
}

/// Tracks 1 to 4 095, empty, in pushes of 512 with two blocks rendered between them.
fn record_tracks() -> Vec<Step> {
    let mut s = EngineStream::new(1);
    let mut steps = Vec::new();
    let mut slot = 1;
    while slot <= GROWTH_TRACKS {
        let msgs: Vec<ControlMsg> = (slot..(slot + 512).min(GROWTH_TRACKS + 1))
            .map(|t| ControlMsg::TrackCreate {
                track: TrackSlot(t),
                kind: TrackKind::Midi,
            })
            .collect();
        steps.push(Step::Push(s.encode(&msgs).unwrap()));
        steps.push(Step::Render(2));
        slot += 512;
    }
    steps
}

/// The fixture format: per step a kind byte (0 push, 1 render), a little-endian u32 (the
/// byte count of a push, the blocks of a render), then a push's bytes.
fn encode(steps: &[Step]) -> Vec<u8> {
    let mut out = Vec::new();
    for step in steps {
        match step {
            Step::Push(b) => {
                out.push(0);
                out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                out.extend_from_slice(b);
            }
            Step::Render(n) => {
                out.push(1);
                out.extend_from_slice(&n.to_le_bytes());
            }
        }
    }
    out
}

fn decode(mut bytes: &[u8]) -> Vec<Step> {
    let mut steps = Vec::new();
    while let [kind, a, b, c, d, rest @ ..] = bytes {
        let n = u32::from_le_bytes([*a, *b, *c, *d]);
        if *kind == 0 {
            steps.push(Step::Push(rest[..n as usize].to_vec()));
            bytes = &rest[n as usize..];
        } else {
            steps.push(Step::Render(n));
            bytes = rest;
        }
    }
    steps
}

fn fixture(name: &str, record: fn() -> Vec<Step>) -> Vec<Step> {
    let path = fixtures().join(name);
    if std::env::var_os("FAULK_RECORD").is_some() {
        std::fs::create_dir_all(fixtures()).unwrap();
        std::fs::write(&path, encode(&record())).unwrap();
    }
    decode(
        &std::fs::read(&path).unwrap_or_else(|_| panic!("tests/fixtures/{name} (FAULK_RECORD=1)")),
    )
}

fn recorded() -> Vec<Step> {
    fixture("session.stream", record)
}

fn render_native(steps: &[Step]) -> Vec<f32> {
    let (out, _, capacity) = run_native(steps);
    assert_eq!(capacity, 64);
    out
}

/// The render and the report region (its bytes) after the last step.
fn run_native(steps: &[Step]) -> (Vec<f32>, Vec<u8>, usize) {
    run_native_at(steps, None)
}

/// [`run_native`] with a host clock (#499): the host frame of the engine's first sample, told
/// to the engine before each block (`Host::set_host_frame`), as the worklet does.
fn run_native_at(steps: &[Step], origin: Option<u64>) -> (Vec<f32>, Vec<u8>, usize) {
    run_native_with(steps, origin, None)
}

/// A caller's clock for the CPU load (#486): its step in seconds and the time each block
/// (by its index) took, the same for both builds.
#[derive(Clone, Copy)]
struct Timing {
    resolution: f64,
    block: fn(u64) -> f64,
}

/// [`run_native_at`] with each block timed by `timing` (`Host::add_render_time`).
fn run_native_with(
    steps: &[Step],
    origin: Option<u64>,
    timing: Option<Timing>,
) -> (Vec<f32>, Vec<u8>, usize) {
    let (mut prepare, mut host) = engine::host::channel(SAMPLE_RATE).unwrap();
    if let Some(timing) = timing {
        host.set_clock_resolution(timing.resolution as f32);
    }
    let mut block = 0u64;
    let mut out = Vec::new();
    let mut rendered = 0u64;
    for step in steps {
        match step {
            Step::Push(bytes) => prepare.submit(bytes).unwrap(),
            Step::Render(blocks) => {
                for _ in 0..*blocks {
                    if let Some(origin) = origin {
                        host.set_host_frame(origin + rendered);
                    }
                    let n = host.render(QUANTUM as usize);
                    if let Some(timing) = timing {
                        host.add_render_time((timing.block)(block) as f32, n as u32);
                    }
                    block += 1;
                    rendered += n as u64;
                    out.extend_from_slice(&host.left()[..n]);
                    out.extend_from_slice(&host.right()[..n]);
                }
            }
        }
    }
    assert_eq!(prepare.preparer().rejected(), 0);
    assert_eq!(host.engine().skipped(), 0);
    let region = host.feedback();
    let report = region
        .iter()
        .flat_map(|w| w.load(std::sync::atomic::Ordering::Relaxed).to_ne_bytes())
        .collect();
    (out, report, host.engine().state().tracks.capacity())
}

/// Builds the wasm module in its own target directory, so the build does not wait on
/// the lock of the `cargo test` that runs this test.
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

fn render_wasm(steps: &[Step]) -> Vec<f32> {
    run_wasm(steps).0
}

/// The render and the report region (its bytes) after the last step.
fn run_wasm(steps: &[Step]) -> (Vec<f32>, Vec<u8>) {
    run_wasm_at(steps, None)
}

/// [`run_wasm`] with a host clock, through `fk_render_at` (#499).
fn run_wasm_at(steps: &[Step], origin: Option<u64>) -> (Vec<f32>, Vec<u8>) {
    run_wasm_with(steps, origin, None)
}

/// [`run_wasm_at`] with each block timed by `timing`, through `fk_add_render_time` (#486).
fn run_wasm_with(
    steps: &[Step],
    origin: Option<u64>,
    timing: Option<Timing>,
) -> (Vec<f32>, Vec<u8>) {
    let bytes = std::fs::read(build_wasm()).expect("the wasm module exists");
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
    let render_at = func::<(i32, i32, f64), i32>(&instance, &store, "fk_render_at");
    let left_ptr = func::<i32, i32>(&instance, &store, "fk_left");
    let right_ptr = func::<i32, i32>(&instance, &store, "fk_right");
    let prepare_counter = func::<(i32, i32), i32>(&instance, &store, "fk_prepare_counter");
    let add_render_time = func::<(i32, f64, i32), ()>(&instance, &store, "fk_add_render_time");
    let set_resolution = func::<(i32, f64), ()>(&instance, &store, "fk_set_clock_resolution");
    let memory = instance
        .get_memory(&store, "memory")
        .expect("exported memory");

    let ch = new.call(&mut store, SAMPLE_RATE).unwrap();
    assert_ne!(ch, 0);
    if let Some(timing) = timing {
        set_resolution
            .call(&mut store, (ch, timing.resolution))
            .unwrap();
    }
    let mut block = 0u64;
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

    let mut out = Vec::new();
    let mut rendered = 0u64;
    for step in steps {
        match step {
            Step::Push(bytes) => {
                let at = inbox.call(&mut store, (ch, bytes.len() as i32)).unwrap();
                memory.write(&mut store, at as usize, bytes).unwrap();
                let code = push.call(&mut store, (ch, at, bytes.len() as i32)).unwrap();
                assert_eq!(code, 0);
            }
            Step::Render(blocks) => {
                for _ in 0..*blocks {
                    let frames = match origin {
                        Some(origin) => render_at
                            .call(&mut store, (ch, QUANTUM as i32, (origin + rendered) as f64))
                            .unwrap(),
                        None => render.call(&mut store, (ch, QUANTUM as i32)).unwrap(),
                    } as usize;
                    if let Some(timing) = timing {
                        add_render_time
                            .call(&mut store, (ch, (timing.block)(block), frames as i32))
                            .unwrap();
                    }
                    block += 1;
                    rendered += frames as u64;
                    let left = left_ptr.call(&mut store, ch).unwrap() as usize;
                    let right = right_ptr.call(&mut store, ch).unwrap() as usize;
                    read(&store, left, frames, &mut out);
                    read(&store, right, frames, &mut out);
                }
            }
        }
    }
    assert_eq!(
        prepare_counter.call(&mut store, (ch, 0)).unwrap(),
        0,
        "rejected"
    );
    let feedback = func::<i32, i32>(&instance, &store, "fk_feedback");
    let feedback_len = func::<i32, i32>(&instance, &store, "fk_feedback_len");
    let at = feedback.call(&mut store, ch).unwrap() as usize;
    let len = feedback_len.call(&mut store, ch).unwrap() as usize;
    let mut report = vec![0u8; len];
    memory.read(&store, at, &mut report).unwrap();
    (out, report)
}

#[test]
fn the_fixture_is_what_the_command_layer_records() {
    assert!(
        recorded() == record(),
        "the mapping changed: FAULK_RECORD=1 cargo test -p engine-wasm --test parity"
    );
}

#[test]
fn the_growth_fixture_is_recorded_and_fills_the_track_table_natively() {
    let growth = fixture("growth.stream", record_growth);
    assert!(
        growth == record_growth(),
        "FAULK_RECORD=1 cargo test -p engine-wasm --test parity"
    );
    let (mut prepare, mut host) = engine::host::channel(SAMPLE_RATE).unwrap();
    let session = recorded();
    for step in session.iter().take_while(|s| matches!(s, Step::Push(_))) {
        let Step::Push(bytes) = step else { break };
        prepare.submit(bytes).unwrap();
    }
    for step in &growth {
        match step {
            Step::Push(bytes) => prepare.submit(bytes).unwrap(),
            Step::Render(n) => {
                for _ in 0..*n {
                    host.render(QUANTUM as usize);
                }
            }
        }
    }
    // The control thread's regular flush pushes what waited behind the full ring.
    for _ in 0..64 {
        prepare.flush();
        host.render(QUANTUM as usize);
    }
    assert_eq!(prepare.preparer().rejected(), 0);
    let state = host.engine().state();
    assert_eq!(
        state.tracks.capacity(),
        engine::msg::limits::MAX_TRACKS as usize
    );
    assert_eq!(
        state.tracks.as_slice().iter().flatten().count(),
        engine::msg::limits::MAX_TRACKS as usize
    );
    assert_eq!(host.engine().skipped(), 0);
}

#[test]
fn the_tracks_fixture_is_recorded_and_fills_the_track_table_natively() {
    let tracks = fixture("tracks.stream", record_tracks);
    assert!(
        tracks == record_tracks(),
        "FAULK_RECORD=1 cargo test -p engine-wasm --test parity"
    );
    let (mut prepare, mut host) = engine::host::channel(SAMPLE_RATE).unwrap();
    for step in recorded().iter().take_while(|s| matches!(s, Step::Push(_))) {
        let Step::Push(bytes) = step else { break };
        prepare.submit(bytes).unwrap();
    }
    for step in &tracks {
        if let Step::Push(bytes) = step {
            prepare.submit(bytes).unwrap();
        }
        for _ in 0..40 {
            prepare.flush();
            host.render(QUANTUM as usize);
        }
    }
    assert_eq!(prepare.preparer().rejected(), 0);
    let state = host.engine().state();
    assert_eq!(
        state.tracks.capacity(),
        engine::msg::limits::MAX_TRACKS as usize
    );
    assert_eq!(
        state.tracks.as_slice().iter().flatten().count(),
        engine::msg::limits::MAX_TRACKS as usize
    );
}

#[test]
fn native_and_wasm_render_the_recorded_stream_alike() {
    let steps = recorded();
    let native = render_native(&steps);
    let reference = fixtures().join("session.native.f32");
    if std::env::var_os("FAULK_RECORD").is_some() {
        let bytes: Vec<u8> = native.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(&reference, bytes).unwrap();
    }
    let saved: Vec<f32> = std::fs::read(&reference)
        .expect("tests/fixtures/session.native.f32 (FAULK_RECORD=1)")
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let stale = saved.len() != native.len()
        || saved
            .iter()
            .zip(&native)
            .any(|(a, b)| (a - b).abs() > TOLERANCE);
    assert!(!stale, "the native reference is stale (FAULK_RECORD=1)");

    let wasm = render_wasm(&steps);
    assert_eq!(native.len(), wasm.len());
    assert!(
        native.iter().any(|s| s.abs() > 0.1),
        "the session must make sound"
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
}

#[test]
fn null_channel_is_ignored() {
    // SAFETY: a null pointer is part of every export's contract.
    unsafe {
        let null = core::ptr::null_mut();
        assert!(engine_wasm::fk_inbox(null, 16).is_null());
        assert_eq!(engine_wasm::fk_push(null, core::ptr::null(), 0), -12);
        engine_wasm::fk_flush(null);
        assert_eq!(engine_wasm::fk_render(null, 128), 0);
        assert_eq!(engine_wasm::fk_pump(null), 0);
        assert!(engine_wasm::fk_left(null).is_null());
        assert!(engine_wasm::fk_feedback(null).is_null());
        assert_eq!(engine_wasm::fk_host_counter(null, 0), u32::MAX);
        engine_wasm::fk_add_render_time(null, 0.001, 128);
        engine_wasm::fk_set_clock_resolution(null, 0.001);
    }
}

#[test]
fn a_push_outside_the_inbox_is_refused_and_sizes_are_clamped() {
    let ch = engine_wasm::fk_channel_new(SAMPLE_RATE);
    // SAFETY: a live channel, used from this one thread.
    unsafe {
        let inbox = engine_wasm::fk_inbox(ch, 32);
        assert!(!inbox.is_null());
        let other = [0u8; 32];
        assert_eq!(engine_wasm::fk_push(ch, other.as_ptr(), 32), -1);
        assert_eq!(engine_wasm::fk_push(ch, inbox, 33), -1);
        assert_eq!(engine_wasm::fk_push(ch, inbox, 0), 0);
        assert_eq!(
            engine_wasm::fk_render(ch, 100_000) as usize,
            engine_wasm::MAX_FRAMES
        );
    }
}

/// A session on four tracks (the page never builds one yet): faders, a master gain, a
/// muted track, notes on three tracks, the clock running, in pushes between renders.
fn report_session() -> Vec<Step> {
    use engine::msg::{ControlMsg as M, encode_stream};
    use engine::time::Tick;
    let mut seq = 0;
    let mut push = |msgs: &[M]| {
        let bytes = encode_stream(1, seq, msgs).unwrap();
        seq += msgs.len() as u32;
        Step::Push(bytes)
    };
    let mut setup = vec![M::Reset { epoch: 1 }];
    for t in 0..4u32 {
        setup.push(M::TrackCreate {
            track: TrackSlot(t),
            kind: TrackKind::Midi,
        });
        setup.push(M::DeviceInsert {
            track: TrackSlot(t),
            device: DeviceSlot(0),
            kind: OSC_SYNTH.kind_id,
            state: Vec::new(),
        });
    }
    let mix = |t: u32, gain: f32, pan: f32, mute: bool| M::TrackSetMix {
        track: TrackSlot(t),
        gain,
        pan,
        mute,
    };
    let note = |t: u32, pitch: u8| M::NoteOn {
        track: TrackSlot(t),
        pitch,
        velocity: 100,
        at: None,
    };
    vec![
        push(&setup),
        push(&[
            mix(1, 0.5, -0.5, false),
            mix(2, 1.0, 0.0, true),
            mix(3, 0.8, 0.9, false),
            M::SetMaster {
                gain: 0.7,
                pan: 0.3,
                mute: false,
            },
            note(0, 57),
            note(1, 64),
            note(2, 67),
            note(3, 72),
            M::Play { from: Tick(0) },
        ]),
        // 30 blocks of 128 frames: past two report periods (1 600 frames each); the last
        // report is the one compared.
        Step::Render(30),
    ]
}

#[test]
fn native_and_wasm_write_the_same_report() {
    use engine::host::decode_feedback;
    let steps = report_session();
    let (_, native, _) = run_native(&steps);
    let (_, wasm) = run_wasm(&steps);
    assert_eq!(native.len(), wasm.len());
    let n = decode_feedback(&native).expect("a native report");
    let w = decode_feedback(&wasm).expect("a wasm report");
    assert_eq!(n.seq, w.seq);
    assert!(n.seq >= 2, "a report was written");
    assert_eq!((n.transport, n.epoch), (w.transport, w.epoch));
    assert_eq!(n.transport, engine::msg::TransportState::Playing);
    assert_eq!(
        (n.playhead_tick, n.playhead_frame),
        (w.playhead_tick, w.playhead_frame)
    );
    assert!(n.playhead_frame.0 > 0);
    assert_eq!(
        (n.live_overflows, n.rejected, n.disk_underruns),
        (w.live_overflows, w.rejected, w.disk_underruns)
    );
    assert_eq!(n.cpu_load, engine::host::NOT_MEASURED);
    assert_eq!(w.cpu_load, engine::host::NOT_MEASURED);
    assert_eq!(n.voices, 4, "the muted track still has its voice");
    assert_eq!(n.voices, w.voices);
    assert_eq!(n.meters.len(), 4);
    assert_eq!(n.meters.len(), w.meters.len());
    let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
    for (slot, (a, b)) in n.meters.iter().zip(&w.meters).enumerate() {
        assert!(
            close(a.peak_l, b.peak_l)
                && close(a.peak_r, b.peak_r)
                && close(a.rms_l, b.rms_l)
                && close(a.rms_r, b.rms_r),
            "track {slot}: {a:?} against {b:?}"
        );
    }
    assert!(n.meters[0].peak_l > 0.05, "track 0 sounds");
    assert!(n.meters[1].peak_l > n.meters[1].peak_r, "panned left");
    assert!(n.meters[3].peak_r > n.meters[3].peak_l, "panned right");
    assert_eq!(n.meters[2], Default::default(), "muted");
    let (a, b) = (n.master, w.master);
    assert!(
        close(a.peak_l, b.peak_l) && close(a.peak_r, b.peak_r) && close(a.rms_l, b.rms_l),
        "master {a:?} against {b:?}"
    );
    assert!(a.peak_l > 0.05);
}

/// A take on two tracks, one armed (RE-2): notes held across report periods, one still held
/// when the recording stops (cut there), in pushes between renders.
fn record_session() -> Vec<Step> {
    use engine::msg::{ControlMsg as M, encode_stream};
    use engine::time::Tick;
    let mut seq = 0;
    let mut push = |msgs: &[M]| {
        let bytes = encode_stream(1, seq, msgs).unwrap();
        seq += msgs.len() as u32;
        Step::Push(bytes)
    };
    let mut setup = vec![
        M::Reset { epoch: 1 },
        M::SetMetronome {
            enabled: false,
            volume: 1.0,
            count_in_bars: 0,
        },
    ];
    for t in 0..2u32 {
        setup.push(M::TrackCreate {
            track: TrackSlot(t),
            kind: TrackKind::Midi,
        });
    }
    setup.push(M::TrackArm {
        track: TrackSlot(1),
        armed: true,
    });
    setup.push(M::SetRecording { recording: true });
    setup.push(M::Play { from: Tick(480) });
    let on = |t: u32, pitch: u8| M::NoteOn {
        track: TrackSlot(t),
        pitch,
        velocity: 80 + pitch % 20,
        at: None,
    };
    let off = |t: u32, pitch: u8| M::NoteOff {
        track: TrackSlot(t),
        pitch,
        at: None,
    };
    vec![
        push(&setup),
        Step::Render(3),
        push(&[on(0, 60), on(1, 60), on(1, 64)]),
        Step::Render(17),
        push(&[off(1, 60), on(1, 67)]),
        Step::Render(9),
        push(&[off(0, 60), off(1, 64)]),
        Step::Render(5),
        push(&[M::SetRecording { recording: false }]),
        Step::Render(30),
    ]
}

#[test]
fn native_and_wasm_record_the_same_take() {
    use engine::host::decode_feedback;
    let steps = record_session();
    let (_, native, _) = run_native(&steps);
    let (_, wasm) = run_wasm(&steps);
    let n = decode_feedback(&native).expect("a native report");
    let w = decode_feedback(&wasm).expect("a wasm report");
    assert_eq!(n.record_take, 1);
    assert!(!n.record_capturing);
    assert_eq!(
        n.record_total, 3,
        "the armed track's three notes, the last one cut"
    );
    assert!(n.recorded.iter().all(|r| r.track == TrackSlot(1)));
    assert_eq!(
        (
            n.record_take,
            n.record_total,
            n.record_dropped,
            n.record_capturing
        ),
        (
            w.record_take,
            w.record_total,
            w.record_dropped,
            w.record_capturing
        )
    );
    assert_eq!(n.recorded, w.recorded);
}

/// A take of notes stamped with the host frame they were played at (#499), each reaching the
/// engine some blocks later, on a host clock 1 000 000 frames ahead of the engine.
fn stamped_session() -> Vec<Step> {
    use engine::msg::{ControlMsg as M, encode_stream};
    use engine::time::Tick;
    const ORIGIN: u64 = 1_000_000;
    let mut seq = 0;
    let mut push = |msgs: &[M]| {
        let bytes = encode_stream(1, seq, msgs).unwrap();
        seq += msgs.len() as u32;
        Step::Push(bytes)
    };
    let setup = [
        M::Reset { epoch: 1 },
        M::SetMetronome {
            enabled: false,
            volume: 1.0,
            count_in_bars: 0,
        },
        M::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Midi,
        },
        M::TrackArm {
            track: TrackSlot(0),
            armed: true,
        },
        M::SetRecording { recording: true },
        M::Play { from: Tick(0) },
    ];
    let note = |on: bool, pitch: u8, sample: u64| {
        let at = Some(ORIGIN + sample);
        let track = TrackSlot(0);
        if on {
            M::NoteOn {
                track,
                pitch,
                velocity: 100,
                at,
            }
        } else {
            M::NoteOff { track, pitch, at }
        }
    };
    // The take plays from sample 0 (the first block applies the play); a beat is 24 000
    // samples at the default 120 BPM.
    vec![
        push(&setup),
        // Beat 1 played at 24 000, taken at block 200 (25 600).
        Step::Render(200),
        push(&[note(true, 60, 24_000)]),
        // Released at 30 000, taken at block 240 (30 720).
        Step::Render(40),
        push(&[note(false, 60, 30_000)]),
        // Beat 2 and a note between two ticks, both taken at block 430 (55 040).
        Step::Render(190),
        push(&[note(true, 64, 48_000), note(true, 67, 48_013)]),
        Step::Render(5),
        push(&[note(false, 64, 50_000), note(false, 67, 50_000)]),
        Step::Render(5),
        push(&[M::SetRecording { recording: false }]),
        // Long enough for a report after the take.
        Step::Render(30),
    ]
}

#[test]
fn native_and_wasm_record_stamped_notes_on_their_beats() {
    use engine::host::decode_feedback;
    use engine::time::Tick;
    let steps = stamped_session();
    let (native_out, native, _) = run_native_at(&steps, Some(1_000_000));
    let (wasm_out, wasm) = run_wasm_at(&steps, Some(1_000_000));
    let n = decode_feedback(&native).expect("a native report");
    let w = decode_feedback(&wasm).expect("a wasm report");
    assert_eq!(n.record_total, 3);
    assert_eq!(n.recorded, w.recorded);
    let mut starts: Vec<(u8, Tick, Tick)> = n
        .recorded
        .iter()
        .map(|r| (r.note.pitch, r.note.start, r.note.length))
        .collect();
    starts.sort();
    // On the beats exactly (25 samples per tick); 13 samples after beat 2 is still its tick,
    // and that note is 1 987 samples long (79.48 ticks).
    assert_eq!(
        starts,
        [
            (60, Tick(960), Tick(240)),
            (64, Tick(1920), Tick(80)),
            (67, Tick(1920), Tick(79)),
        ]
    );
    assert_eq!(native_out.len(), wasm_out.len());
}

/// [`report_session`] under load: eight more notes held on each of the four tracks, the clock
/// running, 40 blocks (three report periods).
fn loaded_session() -> Vec<Step> {
    use engine::msg::{ControlMsg as M, encode_stream};
    let mut steps = report_session();
    steps.pop();
    let notes: Vec<M> = (0..4u32)
        .flat_map(|t| {
            (0..8u8).map(move |i| M::NoteOn {
                track: TrackSlot(t),
                pitch: 40 + i * 5 + t as u8,
                velocity: 90,
                at: None,
            })
        })
        .collect();
    // The two pushes of `report_session` used the sequence numbers 0 to 17.
    steps.push(Step::Push(encode_stream(1, 18, &notes).unwrap()));
    steps.push(Step::Render(40));
    steps
}

/// The duration of one block, in seconds.
fn block_secs() -> f64 {
    f64::from(QUANTUM) / f64::from(SAMPLE_RATE)
}

/// Block times with a spike: the blocks at 10 to 40 % of their 2.67 ms, block 33 at 95 %.
fn spiky(block: u64) -> f64 {
    if block == 33 {
        0.95 * block_secs()
    } else {
        (0.1 + 0.1 * (block % 4) as f64) * block_secs()
    }
}

/// The same block times read by a clock of 1 ms steps (the worklet's `Date.now()`).
fn spiky_coarse(block: u64) -> f64 {
    let start = block as f64 * block_secs();
    ((start + spiky(block)) * 1000.0).floor() / 1000.0 - (start * 1000.0).floor() / 1000.0
}

/// Web/Boost parity of the CPU load (#486) on a loaded render: the same timed blocks give the
/// same load and peak through the native host (the helper's) and the WASM build (the
/// worklet's), with a fine clock and with the worklet's 1 ms one. The real clocks cannot be
/// compared here (the WASM build runs in an interpreter): `a_real_clock_measures_the_loaded_render`
/// and the helper's and the page's tests time the real renders.
#[test]
fn native_and_wasm_report_the_same_cpu_load_on_a_loaded_render() {
    use engine::host::decode_feedback;
    let steps = loaded_session();
    for (timing, fine) in [
        (
            Timing {
                resolution: 0.0,
                block: spiky,
            },
            true,
        ),
        (
            Timing {
                resolution: 0.001,
                block: spiky_coarse,
            },
            false,
        ),
    ] {
        let (native_out, native, _) = run_native_with(&steps, None, Some(timing));
        let (wasm_out, wasm) = run_wasm_with(&steps, None, Some(timing));
        let n = decode_feedback(&native).expect("a native report");
        let w = decode_feedback(&wasm).expect("a wasm report");
        assert!(n.voices >= 32, "a loaded render: {} voices", n.voices);
        assert_eq!(n.voices, w.voices);
        assert!(native_out.iter().any(|s| s.abs() > 0.01), "it sounds");
        assert_eq!(native_out.len(), wasm_out.len());
        assert_eq!(n.seq, w.seq);
        assert_eq!(n.cpu_load.to_bits(), w.cpu_load.to_bits(), "{n:?} {w:?}");
        assert_eq!(n.cpu_peak.to_bits(), w.cpu_peak.to_bits());
        assert!((0.1..0.5).contains(&n.cpu_load), "{}", n.cpu_load);
        if fine {
            // Block 33 is in the last report's period: a fine clock shows it whole.
            assert!((n.cpu_peak - 0.95).abs() < 1e-3, "{}", n.cpu_peak);
        } else {
            // A 1 ms clock takes its peak over a window of about 100 ms.
            assert!((0.1..0.5).contains(&n.cpu_peak), "{}", n.cpu_peak);
        }
    }
}

/// The helper's way with a real clock, on the same loaded render natively: each block timed
/// with `Instant`, the load is measured (more than 0, at most 1).
#[test]
fn a_real_clock_measures_the_loaded_render() {
    use engine::host::decode_feedback;
    let steps = loaded_session();
    let (mut prepare, mut host) = engine::host::channel(SAMPLE_RATE).unwrap();
    for step in &steps {
        match step {
            Step::Push(bytes) => prepare.submit(bytes).unwrap(),
            Step::Render(blocks) => {
                for _ in 0..*blocks {
                    let start = std::time::Instant::now();
                    let n = host.render(QUANTUM as usize);
                    host.add_render_time(start.elapsed().as_secs_f32(), n as u32);
                }
            }
        }
    }
    let report: Vec<u8> = host
        .feedback()
        .iter()
        .flat_map(|w| w.load(std::sync::atomic::Ordering::Relaxed).to_ne_bytes())
        .collect();
    let r = decode_feedback(&report).unwrap();
    assert!(r.voices >= 32);
    assert!(r.cpu_load > 0.0 && r.cpu_load <= 1.0, "{}", r.cpu_load);
    assert!(r.cpu_peak >= r.cpu_load && r.cpu_peak <= 1.0);
}
