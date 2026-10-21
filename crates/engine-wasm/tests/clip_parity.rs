//! The clip player (CL-2) renders the same natively and in WASM: the two-track clip
//! project of `test_support::clips` (launch at a bar line, an intro then a loop region, a
//! note cut at the loop end, fine tuning, a transport stop).

use test_support::clips::{STOP_SECS, two_tracks};
use test_support::parity;

#[test]
fn the_two_track_clip_project_renders_the_same_natively_and_in_wasm() {
    let r = parity(&two_tracks(true));
    r.assert_clean();
    r.assert_audible(0.0..STOP_SECS, 0.05);
}
