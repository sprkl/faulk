//! AU-6 (#115): what the arrangement commands do to an audio placement is what plays. A
//! tone placed at the start of the arrangement, then split, moved or trimmed: the split
//! plays like the whole placement, sample for sample through the cut (#704: the right part
//! takes over the left part's read, at any cut, resampled or not), two placements that follow
//! in time but not in the file crossfade, the move plays the same frames later, and a start
//! trimmed by half a second plays the second half from where it fell. The engine reads the
//! file through the disk planner, served from the blobs ([`test_support::Disk`]).

use commands::{Command, Output, arrangement, clip, track};
use project::{AudioMedia, Frame, Hash, PlacementId, SampleRate};
use serde_json::{Value, json};
use test_support::{Disk, Render, SAMPLE_RATE, Session, render_native_with_disk};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

const MEDIA: [u8; 32] = [0x43; 32];

/// A 2 s stereo WAV at `rate`: a 220 Hz tone on the left, 330 Hz on the right.
fn wav(rate: u32) -> Vec<u8> {
    let frames = 2 * u64::from(rate);
    let data = frames as usize * 4;
    let mut w = Vec::with_capacity(44 + data);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data as u32).to_le_bytes());
    for f in 0..frames {
        for hz in [220.0, 330.0] {
            let v = (f as f64 * hz * core::f64::consts::TAU / f64::from(rate)).sin() * 12_000.0;
            w.extend_from_slice(&(v as i16).to_le_bytes());
        }
    }
    w
}

fn disk(rate: u32) -> Disk {
    let mut d = Disk::default();
    d.blob(MEDIA, wav(rate));
    d
}

/// An audio track with the tone placed whole at the start of the arrangement (120 bpm: a
/// second is 1920 ticks).
fn song() -> (Session, PlacementId) {
    song_at(48_000, None, 0)
}

/// [`song`] with the tone's file at `rate`, the tempo at `bpm` (set before the import) and
/// the placement at tick `start`.
fn song_at(rate: u32, bpm: Option<f64>, start: i64) -> (Session, PlacementId) {
    let mut s = Session::single_synth();
    if let Some(bpm) = bpm {
        s.run(cmd(json!({"type": "transport.set_tempo", "bpm": bpm})));
    }
    let track = match s.exec(cmd(json!({"type": "track.create", "kind": "audio"}))) {
        Output::Track(track::Output::Create { track, .. }) => track,
        other => panic!("{other:?}"),
    };
    s.stage_media(AudioMedia {
        hash: Hash(MEDIA),
        sample_rate: SampleRate::new(rate).expect("a rate"),
        channels: 2,
        frames: Frame::new(2 * u64::from(rate)),
        name: "tone.wav".into(),
    });
    let placement = match s.exec(cmd(json!({
        "type": "clip.import_audio", "track": track, "media": Hash(MEDIA).to_string(),
        "at": {"at": "arrangement", "start": start}
    }))) {
        Output::Clip(clip::Output::ImportAudio {
            placement: Some(p), ..
        }) => p,
        other => panic!("{other:?}"),
    };
    (s, placement)
}

/// Lets the planner load the clip's start, then plays the arrangement for 1.5 s.
fn play(s: &mut Session) -> Render {
    play_at(s, 48_000)
}

/// [`play`] with the tone's file at `rate`.
fn play_at(s: &mut Session, rate: u32) -> Render {
    s.render_secs(0.5);
    s.run(cmd(json!({"type": "transport.play"})));
    s.render_secs(1.5);
    let rendered = render_native_with_disk(s.scenario(), &disk(rate));
    rendered.assert_clean();
    rendered
}

/// Splits `placement` at tick `at`; returns the right part.
fn split(s: &mut Session, placement: PlacementId, at: i64) -> PlacementId {
    match s.exec(Command::Arrangement(arrangement::Command::Split {
        placement,
        at: project::Tick(at),
    })) {
        Output::Arrangement(arrangement::Output::Split { placement }) => placement,
        other => panic!("{other:?}"),
    }
}

/// The frame of the render at `secs` seconds.
fn frame(secs: f32) -> usize {
    (secs * SAMPLE_RATE) as usize
}

fn window(render: &Render, from: usize, to: usize) -> Render {
    Render::new(
        render.sample_rate,
        render.left[from..to].to_vec(),
        render.right[from..to].to_vec(),
    )
}

#[test]
fn a_split_audio_placement_plays_on_through_the_cut() {
    let (mut whole, _) = song();
    let reference = play(&mut whole);
    let (mut cut, placement) = song();
    split(&mut cut, placement, 960);
    let split = play(&mut cut);
    reference.assert_audible(0.6..1.9, 0.05);
    // The cut falls half a second into the play, about one second into the render. The right
    // part takes over the left part's read (#704): the split plays the very samples of the
    // whole placement, through the cut too (before #704, a crossfade between the two reads
    // dipped by 0.09 within the 5 ms after the cut on this 0.37 tone).
    let (diff, at) = split.max_difference(&reference);
    assert_eq!(diff, 0.0, "the split differs by {diff} at {at}");
}

#[test]
fn a_split_audio_placement_plays_like_the_whole_one_at_any_cut_resampled_or_not() {
    // 130 BPM: a tick is not a whole number of samples; the placement starts at tick 77, on
    // no sample either; a file at 44.1 kHz is resampled to the engine's 48 kHz.
    for rate in [48_000, 44_100] {
        let (mut whole, _) = song_at(rate, Some(130.0), 77);
        let reference = play_at(&mut whole, rate);
        reference.assert_audible(0.6..1.9, 0.05);
        for at in [1_001, 1_003, 1_111, 1_234, 1_357, 1_500, 1_618, 1_777] {
            let (mut cut, placement) = song_at(rate, Some(130.0), 77);
            split(&mut cut, placement, at);
            let split = play_at(&mut cut, rate);
            let (diff, i) = split.max_difference(&reference);
            assert_eq!(
                diff, 0.0,
                "{rate} Hz, cut at {at}: differs by {diff} at {i}"
            );
        }
    }
}

#[test]
fn audio_placements_that_follow_in_time_but_not_in_the_file_crossfade() {
    let (mut whole, _) = song();
    let reference = play(&mut whole);
    // Split at half a second, the right part's first quarter second trimmed away, then the
    // right part moved back to the cut: it follows the left part in time, a quarter second
    // later in the file.
    let (mut s, placement) = song();
    let right = split(&mut s, placement, 960);
    s.run(Command::Arrangement(arrangement::Command::Resize {
        placement: right.clone(),
        edge: arrangement::Edge::Start,
        to: project::Tick(1_440),
    }));
    s.run(Command::Arrangement(arrangement::Command::Move {
        placements: vec![right],
        by: project::Tick(-480),
        if_occupied: Default::default(),
    }));
    let jumped = play(&mut s);
    // The play starts on a quantum of the render, the cut half a second later: where the
    // two renders part.
    let cut_at = (0..jumped.frames())
        .find(|&i| jumped.left[i] != reference.left[i])
        .expect("the renders part at the cut");
    assert!(cut_at.abs_diff(frame(1.0)) < 128, "the cut at {cut_at}");
    // The player's 2 ms at 48 kHz.
    let fade = 96;
    let skip = frame(0.25);
    // After the crossfade, the frames a quarter second later.
    let to = frame(1.7);
    let (after, at) = window(&jumped, cut_at + fade, to).max_difference(&window(
        &reference,
        cut_at + fade + skip,
        to + skip,
    ));
    assert_eq!(
        after, 0.0,
        "after the crossfade, differs by {after} at {at}"
    );
    // Within it, both reads: neither the old one nor the new one alone.
    let (a, b) = (cut_at + fade / 4, cut_at + 3 * fade / 4);
    let (old, _) = window(&jumped, a, b).max_difference(&window(&reference, a, b));
    let (new, _) = window(&jumped, a, b).max_difference(&window(&reference, a + skip, b + skip));
    println!("within the crossfade: {old} from the old read, {new} from the new one");
    assert!(old > 0.01 && new > 0.01, "a crossfade ({old}, {new})");
}

#[test]
fn a_moved_audio_placement_plays_the_same_frames_later() {
    let (mut whole, _) = song();
    let reference = play(&mut whole);
    let (mut later, placement) = song();
    later.exec(Command::Arrangement(arrangement::Command::Move {
        placements: vec![placement],
        by: project::Tick(960),
        if_occupied: Default::default(),
    }));
    let moved = play(&mut later);
    // Silent for the first half second of the play, then the tone from its first frame.
    moved.assert_silent(0.52..0.98);
    let half = frame(0.5);
    let (diff, at) = window(&moved, frame(1.1), frame(1.9)).max_difference(&window(
        &reference,
        frame(1.1) - half,
        frame(1.9) - half,
    ));
    assert!(diff < 1e-6, "the moved placement differs by {diff} at {at}");
}

#[test]
fn an_audio_placement_trimmed_at_the_start_plays_from_where_the_cut_fell() {
    let (mut whole, _) = song();
    let reference = play(&mut whole);
    let (mut trimmed, placement) = song();
    trimmed.exec(Command::Arrangement(arrangement::Command::Resize {
        placement,
        edge: arrangement::Edge::Start,
        to: project::Tick(960),
    }));
    let resized = play(&mut trimmed);
    resized.assert_silent(0.52..0.98);
    // From the cut on, the same sound as the whole placement at the same time.
    let (diff, at) = window(&resized, frame(1.1), frame(1.9)).max_difference(&window(
        &reference,
        frame(1.1),
        frame(1.9),
    ));
    assert!(
        diff < 1e-6,
        "the trimmed placement differs by {diff} at {at}"
    );
}
