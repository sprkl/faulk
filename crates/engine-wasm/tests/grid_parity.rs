//! The launch grid (GR-1) renders the same natively and in WASM: scenes launched, switched
//! at a bar line and stopped by `StopAll` (`test_support::clips::scene_grid`).

use test_support::clips::{GRID_END_SECS, STOP_ALL_SECS, scene_grid};
use test_support::parity;

#[test]
fn the_scene_grid_project_renders_the_same_natively_and_in_wasm() {
    let r = parity(&scene_grid(true, true));
    r.assert_clean();
    r.assert_audible(0.0..STOP_ALL_SECS, 0.05);
    r.assert_silent(STOP_ALL_SECS + 0.6..GRID_END_SECS);
}
