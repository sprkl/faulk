//! The clip player (CL-2) renders the same natively and in WASM: the two-track clip
//! project of `test_support::clips` (launch at a bar line, an intro then a loop region, a
//! note cut at the loop end, fine tuning, a transport stop).

use test_support::clips::{
    PAD_END_SECS, PAD_SOUNDING_FROM_SECS, STOP_SECS, pad_cut, pad_edit, two_tracks,
};
use test_support::parity;

#[test]
fn the_two_track_clip_project_renders_the_same_natively_and_in_wasm() {
    let r = parity(&two_tracks(true));
    r.assert_clean();
    r.assert_audible(0.0..STOP_SECS, 0.05);
}

#[test]
fn a_note_added_before_a_sounding_pad_does_not_cut_it_natively_or_in_wasm() {
    // #434: the pad (3000 ticks, until 1.56 s) sounds on across the edit at 0.5 s, whose new
    // note ranks before it in the clip; the same signal natively and in WASM. The window
    // starts 0.6 s after the edit (twice the synth's default release, #443).
    let r = parity(&pad_edit());
    r.assert_clean();
    r.assert_audible(PAD_SOUNDING_FROM_SECS..PAD_END_SECS - 0.1, 0.02);
}

#[test]
fn the_window_of_the_pad_tells_a_cut_note_from_a_continued_one_natively_and_in_wasm() {
    // The control of the test above (#443): a pad that ends at the edit, as if it were cut,
    // leaves the window silent, so a peak there can only come from a pad that went on.
    let cut = parity(&pad_cut());
    cut.assert_clean();
    let kept = parity(&pad_edit());
    let window = PAD_SOUNDING_FROM_SECS..PAD_END_SECS - 0.1;
    let (cut_peak, kept_peak) = (cut.peak_in(window.clone()), kept.peak_in(window));
    assert!(
        cut_peak < 0.001,
        "a cut pad is silent in the window: {cut_peak}"
    );
    assert!(kept_peak > 0.02, "a continued pad sounds: {kept_peak}");
}
