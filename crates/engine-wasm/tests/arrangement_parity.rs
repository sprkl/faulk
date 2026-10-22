//! The arrangement (AR-2) renders the same natively and in WASM: the song looped over a
//! region, sought into the middle of a note, then taken over by a grid clip and given back
//! (`test_support::arrangement::song`, `Play::Everything`).

use test_support::arrangement::{Play, song};
use test_support::parity;

#[test]
fn the_song_renders_the_same_natively_and_in_wasm_in_every_way_of_playing_it() {
    let r = parity(&song(Play::Everything));
    r.assert_clean();
    r.assert_audible(0.05..0.4, 0.05);
    // After the wrap at 3.72 s the loop plays the first bar again.
    r.assert_audible(3.8..4.1, 0.05);
    // The grid clip holds the lead from the bar line at 5.72 s to 6.5 s.
    r.assert_audible(5.8..6.4, 0.05);
}
