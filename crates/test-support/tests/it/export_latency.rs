//! #755 through the command layer: the export of a song with a Ceiling below its ceiling on a
//! track and one on the master (`device.insert`, `device.set_param`) starts on the song's first
//! frame and holds its whole body and tail. The engine compensates the Ceilings' latency
//! (#749), and the offline render drops it from the start of the file: the file is the one of
//! the same project with Ceilings that have no latency (no look-ahead, no true-peak
//! detection), to the byte, natively and in WASM, and so are the levels it measured. The
//! engine's own renders are in `crates/engine/tests/it/offline.rs`.

use commands::export::{ExportBitDepth, ExportSettings};
use commands::{Command, Output, State, device};
use project::TrackId;
use test_support::export::{Exported, demo_song, export_native, export_wasm};
use test_support::{PARITY_TOLERANCE, Session};

/// Two bars of the demo song, its lead through an echo (a tail that rings on), with a Ceiling
/// at 0 dB at the end of the lead's chain and one on the master; `latent`: at their default
/// look-ahead, the master's with the true-peak detection; otherwise without a look-ahead nor
/// the true-peak detection (no latency).
fn song(latent: bool) -> State {
    let state = demo_song(2, Some(0.5));
    let lead: TrackId = state
        .project
        .tracks
        .iter()
        .find(|(_, t)| t.name == "Lead")
        .map(|(id, _)| id.clone())
        .expect("the lead");
    let mut s = Session::from_state(state, commands::testing::context());
    for (track, true_peak) in [(Some(lead), false), (None, true)] {
        let out = s.exec(Command::Device(device::Command::Insert {
            master: track.is_none(),
            track,
            kind: "faulk.ceiling".into(),
            index: u32::MAX,
        }));
        let Output::Device(device::Output::Insert { device }) = out else {
            panic!("{out:?}")
        };
        let mut params = vec![("ceiling", 0.0)];
        if latent {
            params.push(("true_peak", if true_peak { 1.0 } else { 0.0 }));
        } else {
            params.extend([("lookahead", 0.0), ("true_peak", 0.0)]);
        }
        for (param, value) in params {
            s.run(Command::Device(device::Command::SetParam {
                device: device.clone(),
                param: param.into(),
                value,
            }));
        }
    }
    s.state().clone()
}

fn float32() -> ExportSettings {
    ExportSettings {
        bit_depth: ExportBitDepth::Float32,
        ..ExportSettings::default()
    }
}

/// Two exports of one target: the same file to the byte, and the same levels.
fn same(what: &str, a: &Exported, b: &Exported) {
    assert_eq!(a.written, b.written, "{what}: the frames written");
    assert_eq!(a.wav.len(), b.wav.len(), "{what}: the file's length");
    let differs = a.wav.iter().zip(&b.wav).position(|(x, y)| x != y);
    assert_eq!(differs, None, "{what}: the first byte that differs");
    assert_eq!(a.levels, b.levels, "{what}: the levels");
    assert_eq!(a.measured_frames, b.measured_frames, "{what}");
}

/// The native and the WASM export: the same length, the samples within the parity tolerance.
fn parity(what: &str, native: &Exported, wasm: &Exported) {
    assert_eq!(native.written, wasm.written, "{what}: the frames written");
    assert_eq!(native.measured_frames, wasm.measured_frames, "{what}");
    native
        .render()
        .assert_close(&wasm.render(), PARITY_TOLERANCE);
    let (d, at) = native.render().max_difference(&wasm.render());
    println!("{what}: native and WASM differ by at most {d:e} (frame {at})");
}

#[test]
fn a_song_through_ceilings_exports_from_its_first_frame_natively_and_in_wasm() {
    let latent = song(true);
    let flat = song(false);
    let native = export_native(&latent, float32());
    let wasm = export_wasm(&latent, float32());
    let reference = export_native(&flat, float32());
    let reference_wasm = export_wasm(&flat, float32());
    println!(
        "body {} frames, written {} natively and {} in WASM, {} without latency; peak {}",
        native.body_frames, native.written, wasm.written, reference.written, native.peak
    );
    let r = native.render();
    r.assert_clean();
    assert_eq!(native.silenced, 0);
    assert_eq!(native.reduction, 0.0, "below the output protection");
    // The first note on the file's first frames, the last bar played, and the echo's tail.
    r.assert_onset_at(0.0, 0.005, 0.001, 0.0);
    r.assert_audible(3.5..4.0, 0.05);
    assert!(native.written > native.body_frames, "the tail is kept");
    assert_eq!(native.measured_frames, native.body_frames);
    same(
        "native, against Ceilings without latency",
        &native,
        &reference,
    );
    same(
        "WASM, against Ceilings without latency",
        &wasm,
        &reference_wasm,
    );
    parity("with latency", &native, &wasm);
    parity("without latency", &reference, &reference_wasm);
    // The Ceilings do have their latency, in both targets: each render ran it more (the 72
    // frames of the lead's Ceiling and the 84 of the master's, true peak, at 48 kHz).
    assert_eq!(native.rendered - reference.rendered, 72 + 84, "natively");
    assert_eq!(wasm.rendered - reference_wasm.rendered, 72 + 84, "in WASM");
    println!(
        "for listening: {}",
        native.write("export-latency-755").display()
    );
}
