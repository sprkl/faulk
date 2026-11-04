//! AU-6 (#115): what the arrangement commands do to an audio placement is what plays. A
//! tone placed at the start of the arrangement, then split, moved or trimmed: the split
//! plays on without a gap or a jump at the cut (the player's crossfade between two reads of
//! the same frames), the move plays the same frames later, and a start trimmed by half a
//! second plays the second half from where it fell. The engine reads the file through the
//! disk planner, served from the blobs ([`test_support::Disk`]).

use commands::{Command, Output, arrangement, clip, track};
use project::{AudioMedia, Frame, Hash, PlacementId, SampleRate};
use serde_json::{Value, json};
use test_support::{Disk, Render, SAMPLE_RATE, Session, render_native_with_disk};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

const MEDIA: [u8; 32] = [0x43; 32];
const FRAMES: u64 = 96_000;

/// A 2 s stereo WAV at 48 kHz: a 220 Hz tone on the left, 330 Hz on the right.
fn wav() -> Vec<u8> {
    let data = FRAMES as usize * 4;
    let mut w = Vec::with_capacity(44 + data);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&48_000u32.to_le_bytes());
    w.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data as u32).to_le_bytes());
    for f in 0..FRAMES {
        for hz in [220.0, 330.0] {
            let v = (f as f64 * hz * core::f64::consts::TAU / 48_000.0).sin() * 12_000.0;
            w.extend_from_slice(&(v as i16).to_le_bytes());
        }
    }
    w
}

fn disk() -> Disk {
    let mut d = Disk::default();
    d.blob(MEDIA, wav());
    d
}

/// An audio track with the tone placed whole at the start of the arrangement (120 bpm: a
/// second is 1920 ticks).
fn song() -> (Session, PlacementId) {
    let mut s = Session::single_synth();
    let track = match s.exec(cmd(json!({"type": "track.create", "kind": "audio"}))) {
        Output::Track(track::Output::Create { track, .. }) => track,
        other => panic!("{other:?}"),
    };
    s.stage_media(AudioMedia {
        hash: Hash(MEDIA),
        sample_rate: SampleRate::HZ_48000,
        channels: 2,
        frames: Frame::new(FRAMES),
        name: "tone.wav".into(),
    });
    let placement = match s.exec(cmd(json!({
        "type": "clip.import_audio", "track": track, "media": Hash(MEDIA).to_string(),
        "at": {"at": "arrangement", "start": 0}
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
    s.render_secs(0.5);
    s.run(cmd(json!({"type": "transport.play"})));
    s.render_secs(1.5);
    let rendered = render_native_with_disk(s.scenario(), &disk());
    rendered.assert_clean();
    rendered
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
    match cut.exec(Command::Arrangement(arrangement::Command::Split {
        placement,
        at: project::Tick(960),
    })) {
        Output::Arrangement(arrangement::Output::Split { .. }) => {}
        other => panic!("{other:?}"),
    }
    let split = play(&mut cut);
    reference.assert_audible(0.6..1.9, 0.05);
    // The cut falls half a second into the play, one second into the render. Before it the
    // two renders are the same; from the cut the player crossfades from the left part's read
    // to the right part's (AU-4's change of what plays), and once that fade is over the
    // right part plays the very frames the whole placement plays: the split loses no frame
    // and repeats none.
    let cut_at = frame(1.0);
    let fade = frame(0.005);
    let (before, _) = window(&split, 0, cut_at).max_difference(&window(&reference, 0, cut_at));
    assert_eq!(before, 0.0);
    let (after, at) = window(&split, cut_at + fade, frame(1.9)).max_difference(&window(
        &reference,
        cut_at + fade,
        frame(1.9),
    ));
    assert!(after < 1e-6, "after the cut, differs by {after} at {at}");
    // Within the fade the level dips (measured 0.09 at most on this 0.37 tone, a known
    // limit of the player's crossfade between two reads: #704); it never goes
    // silent.
    let (during, at) = window(&split, cut_at, cut_at + fade).max_difference(&window(
        &reference,
        cut_at,
        cut_at + fade,
    ));
    assert!(
        during < 0.15,
        "within the fade, differs by {during} at {at}"
    );
    window(&split, cut_at, cut_at + fade).assert_audible(0.0..0.004, 0.05);
}

#[test]
fn a_moved_audio_placement_plays_the_same_frames_later() {
    let (mut whole, _) = song();
    let reference = play(&mut whole);
    let (mut later, placement) = song();
    later.exec(Command::Arrangement(arrangement::Command::Move {
        placements: vec![placement],
        by: project::Tick(960),
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
