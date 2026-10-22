//! The launch grid (GR-1) through the QA-1 harness: a two-scene, two-track project built by
//! commands, whose scenes are launched, switched and stopped at bar lines, rendered with the
//! built-in synth. The native/WASM comparison is `crates/engine-wasm/tests/grid_parity.rs`.

use test_support::clips::{
    GRID_END_SECS, SCENE_SWITCH_SECS, STOP_ALL_SECS, STOPPED_LAUNCH_END_SECS, StoppedLaunch,
    launch_from_stop, scene_grid,
};
use test_support::{SAMPLE_RATE, render_native};

fn first_difference(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

#[test]
fn a_scene_switches_and_stops_every_track_at_its_bar_line() {
    let held = render_native(&scene_grid(false, false));
    let switched = render_native(&scene_grid(true, false));
    let stopped = render_native(&scene_grid(true, true));
    for r in [&held, &switched, &stopped] {
        r.assert_clean();
    }
    held.assert_audible(0.0..GRID_END_SECS, 0.05);
    // Scene B takes over on the bar line after its launch: the renders are identical up to
    // it (the synth's envelope starts from 0 at the note-on sample, so the first sample
    // that differs is the next one; `launcher.rs` checks the events' samples exactly).
    let switch = (SCENE_SWITCH_SECS * SAMPLE_RATE) as usize;
    assert_eq!(
        first_difference(&held.left, &switched.left),
        Some(switch + 1)
    );
    assert_eq!(
        first_difference(&held.right, &switched.right),
        Some(switch + 1)
    );
    switched.assert_audible(SCENE_SWITCH_SECS..GRID_END_SECS, 0.05);
    // StopAll takes effect on the next bar line, and only then: identical before it, silence
    // once the synth's release is over, while the scene that goes on keeps sounding.
    let stop = (STOP_ALL_SECS * SAMPLE_RATE) as usize;
    let diverges = first_difference(&switched.left, &stopped.left).expect("the stop is heard");
    assert!(
        (stop..stop + 480).contains(&diverges),
        "diverges at {diverges}, the stop is at {stop}"
    );
    stopped.assert_silent(STOP_ALL_SECS + 0.6..GRID_END_SECS);
    switched.assert_audible(STOP_ALL_SECS + 0.6..GRID_END_SECS, 0.05);
    let path = stopped.write_wav("launch-grid-scenes");
    assert!(std::fs::metadata(path).unwrap().len() > 44);
}

/// GR-2: a clip launched by `launch.clip` while the transport is stopped starts it, and the
/// clip sounds from the first sample (the cue is on a bar line). The command's order, the
/// launch before the play, holds when the audio thread takes the two in different blocks;
/// the other order would leave the cue before the launch arrives, a bar late.
#[test]
fn a_launch_from_a_stopped_transport_sounds_at_the_first_boundary() {
    let command = render_native(&launch_from_stop(StoppedLaunch::Command));
    command.assert_clean();
    command.assert_onset_at(0.0, 0.01, 1e-3, 0.0);
    command.assert_audible(0.0..STOPPED_LAUNCH_END_SECS, 0.05);

    let split = render_native(&launch_from_stop(StoppedLaunch::LaunchThenPlay));
    split.assert_clean();
    // Nothing plays between the launch and the play; the clip then starts at the play.
    let at = split.onset(1e-3, 0.0).expect("the clip sounds");
    assert!(at <= 0.01 + 128.0 / 48_000.0, "the clip starts at {at} s");

    let late = render_native(&launch_from_stop(StoppedLaunch::PlayThenLaunch));
    late.assert_clean();
    late.assert_onset_at(2.0, 0.01, 1e-3, 0.0);
    let wav = command.write_wav("launch-from-stop");
    assert!(std::fs::metadata(wav).unwrap().len() > 44);
}
