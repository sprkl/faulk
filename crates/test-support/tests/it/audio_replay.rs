//! AU-4 (#113, taken over from AU-5's #114): an audio clip in a grid slot and one in the
//! arrangement, saved to a folder and opened again, play the same sound, sample for sample,
//! as the project they were saved from. The engine reads their file through the disk planner,
//! served here from the folder's blobs ([`test_support::Disk`]).

use commands::contract::Folder;
use commands::{Command, Output, clip, scene, track};
use project::file::{MemFs, SaveOptions, SaveState, save, save_session};
use project::{AudioMedia, ClipId, Frame, Hash, SampleRate, SceneId};
use serde_json::{Value, json};
use test_support::{Disk, QUANTUM, Render, SAMPLE_RATE, Session, render_native_with_disk};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

const MEDIA: [u8; 32] = [0x42; 32];
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

/// An audio track with the tone in a grid slot and placed at the start of the arrangement.
fn song() -> (Session, ClipId) {
    let mut s = Session::single_synth();
    let track = match s.exec(cmd(json!({"type": "track.create", "kind": "audio"}))) {
        Output::Track(track::Output::Create { track, .. }) => track,
        other => panic!("{other:?}"),
    };
    let scene: SceneId = match s.exec(cmd(json!({"type": "scene.create", "name": "A"}))) {
        Output::Scene(scene::Output::Create { scene }) => scene,
        other => panic!("{other:?}"),
    };
    s.stage_media(AudioMedia {
        hash: Hash(MEDIA),
        sample_rate: SampleRate::HZ_48000,
        channels: 2,
        frames: Frame::new(FRAMES),
        name: "tone.wav".into(),
    });
    let hash = Hash(MEDIA).to_string();
    let launched = match s.exec(cmd(json!({
        "type": "clip.import_audio", "track": track, "media": hash,
        "at": {"at": "slot", "scene": scene}
    }))) {
        Output::Clip(clip::Output::ImportAudio { clip, .. }) => clip,
        other => panic!("{other:?}"),
    };
    s.run(cmd(json!({
        "type": "clip.import_audio", "track": track, "media": hash,
        "at": {"at": "arrangement", "start": 0}
    })));
    (s, launched)
}

/// Lets the planner load the clips' starts, plays the arrangement for 1 s, then launches the
/// grid's clip for 1 s more.
fn play(s: &mut Session, launched: &ClipId) {
    s.render_secs(0.5);
    s.run(cmd(json!({"type": "transport.play"})));
    s.render_secs(1.0);
    s.run(cmd(json!({"type": "launch.clip", "clip": launched})));
    s.render_secs(1.0);
}

fn blocks_of(secs: f32) -> usize {
    (secs * SAMPLE_RATE / QUANTUM as f32).ceil() as usize * QUANTUM as usize
}

fn from_frame(render: &Render, frame: usize) -> Render {
    Render::new(
        render.sample_rate,
        render.left[frame..].to_vec(),
        render.right[frame..].to_vec(),
    )
}

#[test]
fn audio_clips_saved_and_opened_again_play_as_the_original() {
    let (mut original, launched) = song();
    play(&mut original, &launched);
    let rendered = render_native_with_disk(original.scenario(), &disk());
    rendered.assert_clean();
    // The placement from the play, then the grid's clip.
    rendered.assert_audible(0.6..1.4, 0.05);
    rendered.assert_audible(1.6..2.4, 0.05);

    let state = original.state().clone();
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &state.project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .expect("the project saves");
    save_session(&mut fs, &state.session).expect("the session saves");

    // Opened over a fresh session (its own Reset), then the same gestures.
    let mut opened = Session::single_synth();
    opened.render_secs(0.1);
    let before = blocks_of(0.1);
    opened.stage(Folder::new("song", fs));
    opened.open("song");
    assert_eq!(opened.state().project, state.project);
    play(&mut opened, &launched);
    let after = render_native_with_disk(opened.scenario(), &disk());
    let (diff, at) = from_frame(&after, before).max_difference(&rendered);
    assert_eq!(diff, 0.0, "differs at frame {at}");
}
