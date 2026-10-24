//! The export (`project.export_audio`, #297), rendered offline by `engine::offline`: the
//! file's header and length, the level, the tail, the loop region, the formats, the
//! metronome, and the cost of a three-minute song.

use commands::State;
use commands::export::{ExportBitDepth, ExportRegion, ExportSettings, arrangement_end};
use project::{LoopRegion, Tick};
use test_support::export::{Exported, demo_song, export_native};

const RATE: f32 = 48_000.0;

fn settings(region: ExportRegion, bit_depth: ExportBitDepth, sample_rate: u32) -> ExportSettings {
    ExportSettings {
        region,
        bit_depth,
        sample_rate,
        ..ExportSettings::default()
    }
}

fn song(state: &State) -> Exported {
    export_native(state, ExportSettings::default())
}

#[test]
fn the_song_is_rendered_from_bar_one_to_the_end_of_its_last_placement() {
    // Four bars at 120 BPM in 4/4: eight seconds.
    let state = demo_song(4, None);
    assert_eq!(arrangement_end(&state.project), Tick(4 * 3840));
    let exported = song(&state);
    assert_eq!(exported.body_frames, 8 * 48_000);
    let r = exported.render();
    r.assert_clean();
    assert!(
        r.frames() as u64 >= exported.body_frames,
        "the content is whole"
    );
    // The first note is at the very start of the file, and the song is not silent.
    r.assert_onset_at(0.0, 0.005, 0.001, 0.0);
    assert!(r.rms_in(0.0..8.0) > 0.01, "silent: {}", r.rms());
    // The last bar is played, not just the first.
    r.assert_audible(7.0..8.0, 0.05);
    assert_eq!(exported.clipped, 0);
    assert!(
        exported.peak > 0.05 && exported.peak <= 1.0,
        "{}",
        exported.peak
    );
}

#[test]
fn the_file_ends_where_the_sound_falls_under_minus_90_dbfs() {
    let state = demo_song(2, Some(0.5));
    let exported = song(&state);
    let r = exported.render();
    r.assert_clean();
    let body = exported.body_frames as usize;
    // The echo rings on after the last note: the file is longer than the content...
    assert!(r.frames() > body + 48_000, "no tail: {} frames", r.frames());
    // ... but not by the cap: the repeats fall silent, and the file stops there.
    assert!(!exported.tail_cut);
    assert!(r.frames() < body + 20 * 48_000, "{} frames", r.frames());
    let tail_start = body as f32 / RATE;
    r.assert_audible(tail_start..tail_start + 0.5, 0.001);
    // The last samples are quiet (24-bit quantisation included), and the tail's last loud
    // sample is the file's last: nothing quiet is kept after it.
    let last = r.frames() - 1;
    assert!(
        r.left[last].abs() < 1e-3 && r.right[last].abs() < 1e-3,
        "the file ends loud: {} {}",
        r.left[last],
        r.right[last]
    );
    let end = r.frames() as f32 / RATE;
    assert!(
        r.peak_in(end - 0.005..end) < 1e-3,
        "the last 5 ms are not quiet"
    );
}

#[test]
fn a_tail_that_never_dies_is_cut_at_the_cap() {
    // A feedback of 0.95 needs about a minute to fall under -90 dBFS.
    let state = demo_song(1, Some(0.95));
    let exported = song(&state);
    assert!(exported.tail_cut);
    let tail = exported.written - exported.body_frames;
    assert_eq!(tail, 30 * 48_000, "the tail is cut at thirty seconds");
    exported.render().assert_clean();
}

#[test]
fn the_loop_region_is_rendered_once_and_nothing_after_it() {
    let mut state = demo_song(4, None);
    // The second bar only.
    state.project.transport.loop_region = LoopRegion::new(Tick(3840), Tick(2 * 3840));
    for loop_enabled in [false, true] {
        state.project.transport.loop_enabled = loop_enabled;
        let exported = export_native(
            &state,
            settings(ExportRegion::Loop, ExportBitDepth::Pcm24, 48_000),
        );
        assert_eq!(exported.body_frames, 2 * 48_000, "one bar is two seconds");
        let r = exported.render();
        r.assert_clean();
        r.assert_onset_at(0.0, 0.005, 0.001, 0.0);
        r.assert_audible(1.0..2.0, 0.05);
        // The third and fourth bars would play on if the transport did: the file ends with
        // the release of the notes cut at the end of the region.
        assert!(
            r.frames() < 3 * 48_000,
            "the render goes on past the region: {} s",
            r.seconds()
        );
        // The loop being on does not loop the render.
        assert!(exported.written >= exported.body_frames);
    }
    // The song is not the loop: four bars.
    let song = export_native(
        &state,
        settings(ExportRegion::Song, ExportBitDepth::Pcm24, 48_000),
    );
    assert_eq!(song.body_frames, 8 * 48_000);
}

#[test]
fn a_range_stops_on_its_last_frame_whatever_the_rate() {
    // A bar of 88 200 frames at 44.1 kHz or 192 000 at 96 kHz is not a multiple of the
    // engine's 128-frame blocks: the notes that start with the next bar must not be heard in
    // the file. The same bar rendered out of a longer song and out of a song that stops there
    // gives the same file, sample for sample.
    for rate in [44_100, 96_000] {
        let render = |bars: i64| {
            let mut state = demo_song(bars, None);
            state.project.transport.loop_region = LoopRegion::new(Tick(0), Tick(3840));
            let loop_ = export_native(
                &state,
                settings(ExportRegion::Loop, ExportBitDepth::Float32, rate),
            );
            let bars_ = export_native(
                &state,
                ExportSettings {
                    region: ExportRegion::Bars,
                    from_bar: Some(1),
                    to_bar: Some(1),
                    ..settings(ExportRegion::Loop, ExportBitDepth::Float32, rate)
                },
            );
            assert_eq!(
                loop_.wav, bars_.wav,
                "the loop region and bar 1 are the same range"
            );
            loop_
        };
        let (short, long) = (render(1), render(2));
        assert_eq!(short.body_frames, 2 * u64::from(rate));
        assert_eq!(
            short.written, long.written,
            "{rate} Hz: another note was heard"
        );
        assert_eq!(short.wav, long.wav, "{rate} Hz");
    }
}

#[test]
fn bars_are_rendered_from_their_first_to_their_last() {
    let state = demo_song(4, None);
    let bars = |from, to| {
        export_native(
            &state,
            ExportSettings {
                region: ExportRegion::Bars,
                from_bar: Some(from),
                to_bar: Some(to),
                ..ExportSettings::default()
            },
        )
    };
    // Bars 2 and 3: four seconds.
    let middle = bars(2, 3);
    assert_eq!(middle.body_frames, 4 * 48_000);
    middle.render().assert_clean();
    // The first note of bar 2 is at the start of the file.
    middle.render().assert_onset_at(0.0, 0.005, 0.001, 0.0);
    // Past the end of the arrangement the range is cut to what is there? No: it renders
    // what was asked, silence included, so the last bar of the song and the next one is two
    // bars, the second one empty.
    let past = bars(4, 5);
    assert_eq!(past.body_frames, 4 * 48_000);
    let r = past.render();
    r.assert_audible(0.0..1.0, 0.05);
    assert!(r.peak_in(2.5..3.9) < 1e-4, "{}", r.peak_in(2.5..3.9));
    // The whole song as bars is the song.
    let all = bars(1, 4);
    assert_eq!(all.wav, song(&state).wav);
}

#[test]
fn the_loop_region_does_not_start_where_the_song_does() {
    // Bar 2 of the song is a different chord than bar 1: the loop region's render starts
    // with the notes of bar 2, not with those of bar 1.
    let mut state = demo_song(4, None);
    let whole = song(&state).render();
    state.project.transport.loop_region = LoopRegion::new(Tick(3840), Tick(2 * 3840));
    let part = export_native(
        &state,
        settings(ExportRegion::Loop, ExportBitDepth::Pcm24, 48_000),
    )
    .render();
    // The same notes, a bar later: the pitch and the level of each beat agree (the
    // oscillators' phases are free, so the samples themselves do not).
    for beat in 0..4 {
        let (from, to) = (beat as f32 * 0.5 + 0.05, beat as f32 * 0.5 + 0.45);
        let (a, b) = (
            part.spectrum(from..to),
            whole.spectrum(from + 2.0..to + 2.0),
        );
        assert!(
            (a.dominant_hz() - b.dominant_hz()).abs() < 5.0,
            "beat {beat}: {} Hz against {} Hz",
            a.dominant_hz(),
            b.dominant_hz()
        );
        let (ra, rb) = (part.rms_in(from..to), whole.rms_in(from + 2.0..to + 2.0));
        assert!(
            (ra - rb).abs() < 0.1 * rb,
            "beat {beat}: rms {ra} against {rb}"
        );
    }
    // And it is not the first bar: its chord is another one.
    let first = whole.spectrum(0.05..0.45).dominant_hz();
    assert!((part.spectrum(0.05..0.45).dominant_hz() - first).abs() > 5.0);
}

#[test]
fn the_metronome_is_never_in_the_file() {
    let mut state = demo_song(2, None);
    let silent = song(&state).render();
    state.project.transport.metronome.enabled = true;
    state.project.transport.metronome.gain_db = 0.0;
    let clicking = song(&state).render();
    assert_eq!(silent.left, clicking.left);
    assert_eq!(silent.right, clicking.right);
}

#[test]
fn an_export_is_deterministic() {
    let state = demo_song(2, Some(0.4));
    assert_eq!(song(&state).wav, song(&state).wav);
}

#[test]
fn every_format_and_rate_writes_a_valid_file_of_the_same_sound() {
    let state = demo_song(2, Some(0.4));
    let reference = export_native(
        &state,
        settings(ExportRegion::Song, ExportBitDepth::Float32, 48_000),
    );
    let float = reference.render();
    float.assert_finite();
    for (depth, tolerance) in [(ExportBitDepth::Pcm24, 2e-7), (ExportBitDepth::Pcm16, 4e-5)] {
        let r = export_native(&state, settings(ExportRegion::Song, depth, 48_000)).render();
        assert_eq!(r.frames(), float.frames(), "{depth:?}");
        for i in 0..r.frames() {
            assert!(
                (r.left[i] - float.left[i]).abs() <= tolerance,
                "{depth:?} sample {i}"
            );
            assert!(
                (r.right[i] - float.right[i]).abs() <= tolerance,
                "{depth:?} sample {i}"
            );
        }
    }
    for rate in [44_100u32, 96_000] {
        let exported = export_native(
            &state,
            settings(ExportRegion::Song, ExportBitDepth::Pcm24, rate),
        );
        assert_eq!(exported.body_frames, 4 * u64::from(rate));
        let r = exported.render();
        r.assert_clean();
        r.assert_audible(0.0..1.0, 0.05);
    }
}

#[test]
fn a_signal_over_full_scale_is_clipped_in_integers_and_kept_in_floats() {
    let mut state = demo_song(2, None);
    // Gain staging that overloads the output: the master 24 dB up.
    state.project.master.gain_db = 24.0;
    let int = export_native(
        &state,
        settings(ExportRegion::Song, ExportBitDepth::Pcm24, 48_000),
    );
    assert!(int.clipped > 0, "peak {}", int.peak);
    assert!(int.peak > 1.0);
    let r = int.render();
    assert!(r.peak() <= 1.0);
    r.assert_finite();
    let float = export_native(
        &state,
        settings(ExportRegion::Song, ExportBitDepth::Float32, 48_000),
    );
    assert_eq!(float.clipped, 0);
    assert!(float.render().peak() > 1.0, "the floats are kept");
}

#[test]
fn a_project_with_nothing_in_the_arrangement_is_refused() {
    let state = commands::testing::fixture().state;
    let refused = commands::export::job(&state.project, &state.session, ExportSettings::default());
    assert!(matches!(
        refused,
        Err(commands::export::ExportError::Refused(
            commands::CommandError::Invalid { .. }
        ))
    ));
}

/// An example file for listening: `cargo test -p test-support --test export -- --ignored
/// example` writes `.qa/renders/export-wav-example.wav` (eight bars with an echo on the lead,
/// 24 bits, 48 kHz). Only the user judges by ear.
#[test]
#[ignore = "writes a file for listening"]
fn an_example_file_is_written_for_listening() {
    let exported = song(&demo_song(8, Some(0.5)));
    let path = exported.write("export-wav-example");
    println!(
        "{}: {:.1} s, peak {:.3}",
        path.display(),
        exported.written as f64 / 48_000.0,
        exported.peak
    );
}

/// The cost of a three-minute song with an echo: `cargo test -p test-support --release
/// --test export -- --ignored --nocapture`.
#[test]
#[ignore = "a measure, not a check: run it in release"]
fn a_three_minute_song_renders_much_faster_than_it_plays() {
    let state = demo_song(90, Some(0.5));
    let started = std::time::Instant::now();
    let exported = song(&state);
    let took = started.elapsed().as_secs_f64();
    let seconds = exported.written as f64 / 48_000.0;
    println!(
        "export: {seconds:.1} s of audio ({} MB) rendered in {took:.2} s, {:.0}x real time",
        exported.wav.len() / 1_000_000,
        seconds / took
    );
    exported.render().assert_clean();
    assert!(seconds > 180.0);
}
