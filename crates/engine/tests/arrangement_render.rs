//! The arrangement (AR-2) through the QA-1 harness: a two-track song built by commands
//! only (clips created in the arrangement, notes added, the transport played, sought and
//! looped, a grid clip launched and the tracks given back with
//! `launch.back_to_arrangement`), rendered with the built-in synth. The exact samples of
//! the events are `arrangement.rs`'s; the native/WASM comparison is
//! `crates/engine-wasm/tests/arrangement_parity.rs`.

use test_support::arrangement::{
    BACK_SECS, END_SECS, LAUNCH_SECS, Play, SEEK_SECS, STOP_SECS, song, song_with,
};
use test_support::{SAMPLE_RATE, render_native};

fn first_difference(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

#[test]
fn the_arrangement_sounds_on_play_and_is_silent_after_its_end_and_after_stop() {
    let r = render_native(&song(Play::Straight));
    r.assert_clean();
    // The bass's first note (0 to 1.5 s), the lead from bar 2 (2 s), the bass's second note.
    r.assert_audible(0.05..1.4, 0.05);
    r.assert_audible(2.05..3.4, 0.05);
    // The clips end at 4 s (the last note of the lead ends before it); nothing after.
    r.assert_silent(5.0..END_SECS);
    const { assert!(STOP_SECS > 5.0) };
    let path = r.write_wav("arrangement-song");
    assert!(std::fs::metadata(path).unwrap().len() > 44);
}

#[test]
fn a_seek_into_a_note_neither_starts_it_late_nor_leaves_it_sounding() {
    let r = render_native(&song(Play::SeekIntoNote));
    r.assert_clean();
    r.assert_audible(0.05..SEEK_SECS - 0.05, 0.05);
    // The bass note that was sounding ended at the seek (its release is over by 1.15 s);
    // the next notes of the timeline are 1.72 s away. A late start would sound here.
    r.assert_silent(1.15..1.70);
    r.assert_audible(1.75..2.2, 0.05);
}

#[test]
fn the_region_loop_plays_the_first_bar_again_each_pass() {
    let r = render_native(&song(Play::RegionLoop));
    r.assert_clean();
    for pass in 0..4 {
        let at = 2.0 * pass as f32;
        r.assert_audible(at + 0.05..at + 0.4, 0.05);
    }
}

#[test]
fn a_launched_clip_takes_the_track_at_the_bar_line_and_the_tracks_go_back() {
    let straight = render_native(&song(Play::Straight));
    let taken = render_native(&song(Play::GridTakeover));
    taken.assert_clean();
    // The renders are identical up to the bar line after the launch (2 s): the arrangement
    // played until the boundary, not from the launch at 1.3 s.
    let bar = (2.0 * SAMPLE_RATE) as usize;
    const { assert!(LAUNCH_SECS < 2.0 && BACK_SECS > 2.0) };
    let at = first_difference(&straight.left, &taken.left).expect("the grid clip is heard");
    assert!(
        (bar..bar + 480).contains(&at),
        "diverges at {at}, the boundary is at {bar}"
    );
    taken.assert_audible(2.05..3.1, 0.05);
    // Back at 3.2 s inside the lead's second note: it is not restarted, the next note
    // of the lead (3.56 s) plays on its tick, and then everything ends like the straight
    // render: identical once the tails of the two renders are over.
    taken.assert_audible(3.6..3.8, 0.05);
    let tail = (5.0 * SAMPLE_RATE) as usize;
    assert_eq!(
        first_difference(&straight.left[tail..], &taken.left[tail..]),
        None
    );
    taken.assert_silent(5.0..END_SECS);
}

#[test]
fn a_note_in_the_second_bar_of_a_lengthened_clip_sounds_on_play() {
    let r = render_native(&test_support::arrangement::lengthened());
    r.assert_clean();
    // The clip was created with one bar and lengthened to two before the note was drawn:
    // the first bar is silent, the note of the second bar (2 s) sounds.
    r.assert_silent(0.0..1.9);
    r.assert_audible(2.05..3.4, 0.05);
}

/// AM-2: lanes that leave the left channel alone (a gain at 0 dB, a pan between the centre
/// and the left) cut every track's blocks into pieces at their points and control frames;
/// the clip player reads the pieces like smaller blocks, so the left channel is the same
/// sample for sample, through the seek, the loop, the launch and the return.
#[test]
fn lanes_that_leave_the_left_channel_alone_do_not_change_it() {
    let plain = render_native(&song(Play::Everything));
    let laned = render_native(&song_with(Play::Everything, true));
    laned.assert_clean();
    assert_eq!(first_difference(&plain.left, &laned.left), None);
    // The pan lane plays: the right channel moves.
    assert!(first_difference(&plain.right, &laned.right).is_some());
}
