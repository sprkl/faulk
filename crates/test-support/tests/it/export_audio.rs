//! #647: the export (`project.export_audio`) plays the audio clips. The offline render reads
//! their files through the engine's disk planner, served here from the project folder's blobs
//! ([`test_support::Disk`]), and waits for each read: the file is the playback of the same
//! project, sample for sample, whatever the speed of the disk; a file that is missing leaves
//! its clip silent, is reported, and does not stop the render.

use commands::export::{ExportBitDepth, ExportSettings};
use commands::{Command, Output, track};
use project::{AudioMedia, Frame, Hash, SampleRate};
use serde_json::{Value, json};
use test_support::export::{Exported, export_native_with_disk, export_wasm_with_disk};
use test_support::render_native_with_disk;
use test_support::{Disk, PARITY_TOLERANCE, QUANTUM, Render, SAMPLE_RATE, Session};

fn cmd(v: Value) -> Command {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

const MEDIA: [u8; 32] = [0x47; 32];
/// Longer than `engine::audio_source::SHORT_SOURCE_MS`: the clip is not kept whole in memory,
/// its voice hands over from the preloaded start to a ring the planner fills as it plays.
const SECONDS: u64 = 32;
const AMPLITUDE: f64 = 12_000.0 / 32_768.0;

/// A stereo 16-bit WAV of `SECONDS` at `rate`: a 220 Hz tone on the left, 330 Hz on the right.
fn wav(rate: u32) -> Vec<u8> {
    let frames = SECONDS * u64::from(rate);
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
            let v = (f as f64 * hz * core::f64::consts::TAU / f64::from(rate)).sin()
                * AMPLITUDE
                * 32_768.0;
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

/// An audio track with the tone placed at the start of the arrangement (`rate`: the file's).
fn song(rate: u32) -> Session {
    song_at(rate, 0)
}

/// [`song`] with the tone placed at `start` (ticks).
fn song_at(rate: u32, start: i64) -> Session {
    let mut s = Session::single_synth();
    let track = match s.exec(cmd(json!({"type": "track.create", "kind": "audio"}))) {
        Output::Track(track::Output::Create { track, .. }) => track,
        other => panic!("{other:?}"),
    };
    s.stage_media(AudioMedia {
        hash: Hash(MEDIA),
        sample_rate: SampleRate::new(rate).expect("a rate"),
        channels: 2,
        frames: Frame::new(SECONDS * u64::from(rate)),
        name: "tone.wav".into(),
    });
    s.run(cmd(json!({
        "type": "clip.import_audio", "track": track, "media": Hash(MEDIA).to_string(),
        "at": {"at": "arrangement", "start": start}
    })));
    s
}

fn float32(sample_rate: u32) -> ExportSettings {
    ExportSettings {
        bit_depth: ExportBitDepth::Float32,
        sample_rate,
        ..ExportSettings::default()
    }
}

fn body(exported: &Exported) -> Render {
    let r = exported.render();
    let n = exported.body_frames as usize;
    Render::new(r.sample_rate, r.left[..n].to_vec(), r.right[..n].to_vec())
}

#[test]
fn an_audio_clip_is_exported_sample_for_sample_as_it_plays() {
    let mut s = song(48_000);
    let exported = export_native_with_disk(s.state(), float32(48_000), &disk(48_000));
    assert_eq!(exported.body_frames, SECONDS * 48_000);
    assert!(exported.missing.is_empty(), "{:?}", exported.missing);
    // The render waited for the disk (the preloads, then the ring at each refill) and no
    // voice ever lacked a frame.
    assert!(exported.disk_waits > 0);
    assert_eq!(exported.underruns, 0);
    assert_eq!(exported.silenced, 0);
    let file = body(&exported);
    file.assert_clean();
    // Heard from the first second to the last, past the handover to the ring (1.5 s).
    for second in [0.5, 1.4, 1.6, 10.0, 20.0, 31.0] {
        file.assert_audible(second..second + 0.4, 0.2);
    }

    // The same project played: the planner loads the clip's start, then the transport plays
    // from bar 1 (a disk served after every block, as the page's executor does).
    let before = (0.5 * SAMPLE_RATE / QUANTUM as f32).ceil() as usize * QUANTUM as usize;
    s.render_secs(0.5);
    s.run(cmd(json!({"type": "transport.play"})));
    s.render_secs(SECONDS as f32 + 0.1);
    let played = render_native_with_disk(s.scenario(), &disk(48_000));
    let n = exported.body_frames as usize;
    let played = Render::new(
        played.sample_rate,
        played.left[before..before + n].to_vec(),
        played.right[before..before + n].to_vec(),
    );
    let (diff, at) = file.max_difference(&played);
    assert_eq!(
        diff, 0.0,
        "the file differs from the playback at frame {at}"
    );
}

#[test]
fn an_audio_clip_is_resampled_and_the_wasm_build_writes_the_same_file() {
    // A 44.1 kHz file in a 48 kHz export.
    let s = song(44_100);
    let native = export_native_with_disk(s.state(), float32(48_000), &disk(44_100));
    let wasm = export_wasm_with_disk(s.state(), float32(48_000), &disk(44_100));
    assert_eq!(native.underruns, 0);
    assert_eq!(wasm.underruns, 0);
    assert_eq!(native.written, wasm.written);
    let (n, w) = (native.render(), wasm.render());
    n.assert_clean();
    n.assert_close(&w, PARITY_TOLERANCE);
    // The tone keeps its pitch: 220 Hz on the left.
    let peak = body(&native).spectrum(2.0..3.0).dominant_hz();
    assert!((peak - 220.0).abs() < 2.0, "{peak} Hz");

    // At 96 kHz the clip is as loud as at 48 kHz.
    let high = export_native_with_disk(s.state(), float32(96_000), &disk(44_100));
    assert_eq!(high.underruns, 0);
    let (a, b) = (
        body(&native).rms_in(2.0..30.0),
        body(&high).rms_in(2.0..30.0),
    );
    assert!((a - b).abs() < 0.01 * a, "{a} at 48 kHz, {b} at 96 kHz");
    assert!((a - AMPLITUDE as f32 / 2f32.sqrt()).abs() < 0.02, "{a}");
}

#[test]
fn a_missing_file_leaves_its_clip_silent_is_reported_and_does_not_stop_the_export() {
    let s = song(48_000);
    let exported = export_native_with_disk(s.state(), float32(48_000), &Disk::default());
    assert_eq!(exported.missing, vec![MEDIA]);
    assert_eq!(exported.body_frames, SECONDS * 48_000);
    let file = body(&exported);
    file.assert_clean();
    file.assert_silent(0.0..SECONDS as f32);
    // Same in the WASM build.
    let wasm = export_wasm_with_disk(s.state(), float32(48_000), &Disk::default());
    assert_eq!(wasm.missing, vec![MEDIA]);
    assert_eq!(wasm.written, exported.written);
}

#[test]
fn a_missing_file_outside_the_exported_range_is_not_reported() {
    // The tone from bar 21 on, a synth clip in bar 1: bars 1 to 2 do not play the tone.
    let mut s = song_at(48_000, 20 * 3840);
    let synth = s
        .state()
        .project
        .tracks
        .keys()
        .next()
        .expect("the synth")
        .clone();
    s.run(cmd(json!({
        "type": "clip.create", "track": synth, "at": {"at": "arrangement", "start": 0},
        "length": 3840
    })));
    let bars = ExportSettings {
        region: commands::export::ExportRegion::Bars,
        from_bar: Some(1),
        to_bar: Some(2),
        ..float32(48_000)
    };
    let exported = export_native_with_disk(s.state(), bars, &Disk::default());
    assert!(exported.missing.is_empty(), "{:?}", exported.missing);
    // The song does play it: there it is reported.
    let song = export_native_with_disk(s.state(), float32(48_000), &Disk::default());
    assert_eq!(song.missing, vec![MEDIA]);
}

/// The cost of the disk waits: `cargo test -p test-support --release --test it
/// export_audio:: -- --ignored --nocapture`.
#[test]
#[ignore = "a measure, not a check: run it in release"]
fn an_audio_clip_export_renders_much_faster_than_it_plays() {
    let s = song(44_100);
    let disk = disk(44_100);
    let started = std::time::Instant::now();
    let exported = export_native_with_disk(s.state(), float32(48_000), &disk);
    let took = started.elapsed().as_secs_f64();
    let seconds = exported.written as f64 / 48_000.0;
    println!(
        "audio export: {seconds:.1} s of a 44.1 kHz clip rendered in {took:.3} s, {:.0}x real time, {} waits for the disk",
        seconds / took,
        exported.disk_waits
    );
    assert_eq!(exported.underruns, 0);
}
