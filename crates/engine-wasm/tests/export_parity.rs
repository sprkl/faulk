//! The export (`project.export_audio`, #297) rendered by the native target and by the
//! `fk_export_*` exports of the WASM module (in an interpreter): the same project must give
//! the same file, sample for sample within `PARITY_TOLERANCE`, with the same length.

use commands::export::{ExportBitDepth, ExportRegion, ExportSettings};
use project::{LoopRegion, Tick};
use test_support::PARITY_TOLERANCE;
use test_support::export::{Exported, demo_song, export_native, export_wasm};

fn same(native: &Exported, wasm: &Exported) {
    // The header and the figures are exact: the lengths come from the same rule.
    assert_eq!(native.body_frames, wasm.body_frames);
    assert_eq!(native.written, wasm.written, "the files differ in length");
    assert_eq!(native.rendered, wasm.rendered);
    assert_eq!(native.tail_cut, wasm.tail_cut);
    assert_eq!(native.silenced, wasm.silenced, "the silenced counts differ");
    assert_eq!(native.wav.len(), wasm.wav.len());
    let header = native.settings.bit_depth.format().header_bytes();
    assert_eq!(
        native.wav[..header],
        wasm.wav[..header],
        "the headers differ"
    );
    let (n, w) = (native.render(), wasm.render());
    n.assert_close(&w, PARITY_TOLERANCE);
    assert!((native.peak - wasm.peak).abs() < PARITY_TOLERANCE);
    // The output protection's reported gain reduction (`fk_export_stat(…, 7)`, #402) is the
    // same in both targets.
    assert!((native.reduction - wasm.reduction).abs() < PARITY_TOLERANCE);
}

#[test]
fn a_song_with_an_echo_exports_alike_natively_and_in_wasm() {
    let state = demo_song(1, Some(0.3));
    let settings = ExportSettings {
        region: ExportRegion::Song,
        bit_depth: ExportBitDepth::Float32,
        sample_rate: 48_000,
        ..ExportSettings::default()
    };
    let (native, wasm) = (
        export_native(&state, settings),
        export_wasm(&state, settings),
    );
    same(&native, &wasm);
    // The tail is there in both: the file is longer than the content.
    assert!(native.written > native.body_frames + 48_000);
    native.render().assert_clean();
}

#[test]
fn a_mix_pushed_into_the_limiter_reports_the_same_reduction_natively_and_in_wasm() {
    // demo_song clamps the master to +6 dB, so push it past the command layer here to slam the
    // limiter: the reported gain reduction (fk_export_stat(…, 7)) must be real and identical in
    // both targets, not 0 == 0 as in the quiet cases.
    let mut state = demo_song(1, None);
    state.project.master.gain_db = 24.0;
    let settings = ExportSettings {
        region: ExportRegion::Song,
        bit_depth: ExportBitDepth::Float32,
        sample_rate: 48_000,
        ..ExportSettings::default()
    };
    let (native, wasm) = (
        export_native(&state, settings),
        export_wasm(&state, settings),
    );
    same(&native, &wasm);
    assert!(
        native.reduction > 0.1,
        "the mix did not engage the limiter: reduction {}",
        native.reduction
    );
}

#[test]
fn a_loop_region_in_integers_exports_alike_natively_and_in_wasm() {
    let mut state = demo_song(2, None);
    state.project.transport.loop_region = LoopRegion::new(Tick(0), Tick(3840));
    state.project.transport.loop_enabled = true;
    let settings = ExportSettings {
        region: ExportRegion::Loop,
        bit_depth: ExportBitDepth::Pcm24,
        sample_rate: 44_100,
        ..ExportSettings::default()
    };
    let (native, wasm) = (
        export_native(&state, settings),
        export_wasm(&state, settings),
    );
    same(&native, &wasm);
    assert_eq!(native.body_frames, 2 * 44_100);
}
