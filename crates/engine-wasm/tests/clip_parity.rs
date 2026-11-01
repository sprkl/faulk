//! The clip player (CL-2) renders the same natively and in WASM: the two-track clip
//! project of `test_support::clips` (launch at a bar line, an intro then a loop region, a
//! note cut at the loop end, fine tuning, a transport stop).

use test_support::clips::{PAD_EDIT_SECS, PAD_END_SECS, STOP_SECS, pad_edit, two_tracks};
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
    // note ranks before it in the clip; the same signal natively and in WASM.
    let r = parity(&pad_edit());
    r.assert_clean();
    r.assert_audible(PAD_EDIT_SECS + 0.3..PAD_END_SECS - 0.1, 0.02);
}
