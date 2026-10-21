//! The clip player (CL-2) through the QA-1 harness: a two-track project built by commands
//! whose clips are launched, played in loops and stopped, rendered with the built-in synth.
//! The native/WASM comparison of the same project is
//! `crates/engine-wasm/tests/clip_parity.rs`.

use test_support::clips::{END_SECS, SECOND_START_SECS, STOP_SECS, two_tracks};
use test_support::{SAMPLE_RATE, render_native};

#[test]
fn a_two_track_clip_project_renders_its_clips_at_their_samples() {
    let both = render_native(&two_tracks(true));
    let one = render_native(&two_tracks(false));
    both.assert_clean();
    both.assert_audible(0.0..STOP_SECS, 0.05);
    // The second clip starts on the bar line after its launch: the two renders are
    // identical before it.
    let start = (SECOND_START_SECS * SAMPLE_RATE) as usize;
    let first_diff = both
        .left
        .iter()
        .zip(&one.left)
        .position(|(a, b)| a != b)
        .expect("the second track plays");
    // The synth's envelope starts from 0 at the note-on sample: the first sample that
    // differs is the next one. (`clip_player.rs` checks the events' samples exactly.)
    assert_eq!(first_diff, start + 1);
    // The bass loop plays from the first sample and again on every bar.
    for bar in 0..3 {
        let at = bar as f32 * 2.0;
        assert!(one.peak_in(at..at + 0.1) > 0.05, "bar {bar} of the bass");
    }
    // The stop releases every note: silence once the synth's release is over.
    both.assert_silent(STOP_SECS + 0.6..END_SECS);
    let path = both.write_wav("clip-player-two-tracks");
    assert!(std::fs::metadata(path).unwrap().len() > 44);
}
