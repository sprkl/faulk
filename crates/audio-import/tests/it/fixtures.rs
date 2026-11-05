//! The reference files (`tests/fixtures/`, written by `make_fixtures.py` from one known
//! signal): what each import finds, compared with `sha256sum`, with the source signal and with
//! the record `expected.json` that the WebAssembly build is checked against too
//! (`web/src/lib/audio/import/import.test.ts`): the page and the helper decode the same file
//! to the same PCM (bit for bit, but MP3 and AAC at a tolerance: `REFERENCE_PCM`).
//!
//! `FAULK_BLESS=1 cargo test -p audio-import --test it fixtures::` rewrites `expected.json`
//! (after a change of a fixture or of the decoder: raise `cache::DECODER` then).

use crate::support::*;
use audio_import::{ImportError, PcmLayout};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Every fixture, with what its import must give.
const FIXTURES: &[&str] = &[
    "sine-s16.wav",
    "sine-s24.wav",
    "sine-f32.wav",
    "sine-u8.wav",
    "sine-s16.aiff",
    "sine-s24.aiff",
    "sine.flac",
    "sine-48k-mono.flac",
    "sine-alac.m4a",
    "sine.mp3",
    "sine.m4a",
    "sine.ogg",
    "sine.opus",
    "sine-mono.opus",
    "sine.webm",
    "recorded.webm",
    "voip-mono.opus",
    "voip.webm",
    "not-audio.txt",
];

/// Lossless files: the decoded samples are the source's, exactly.
const LOSSLESS: &[&str] = &[
    "sine-s16.wav",
    "sine-s24.wav",
    "sine-f32.wav",
    "sine-s16.aiff",
    "sine-s24.aiff",
    "sine.flac",
    "sine-48k-mono.flac",
    "sine-alac.m4a",
];

/// A lossy file: the lowest signal-to-noise ratio (dB) of its decode against the source, over
/// the whole file (the tolerance of the reference comparison), and the source's rate, frames
/// and channels. Measured on 2026-11-03: MP3 (LAME, 128 kb/s) 26.0 dB, AAC (FFmpeg's encoder,
/// 128 kb/s) 24.6 dB, Vorbis (libsndfile's default quality) 40.6 dB; on 2026-11-05 (#609),
/// decoded by opuscule: Opus (libopus) in Ogg at 96 kb/s 41.3 dB, mono at 64 kb/s 41.3 dB, in
/// WebM at 96 kb/s 41.3 dB. The floors keep about 4 dB of margin. The length is the source's
/// exactly, and the decode is aligned on it to the sample: the encoder's delay and padding
/// are cut (LAME tag, Ogg granule positions, the MP4 edit list, Opus's pre-skip, a WebM's
/// `DiscardPadding`).
struct Lossy {
    name: &'static str,
    floor: f64,
    rate: u32,
    frames: u64,
    channels: u16,
}

const LOSSY: &[Lossy] = &[
    Lossy {
        name: "sine.mp3",
        floor: 22.0,
        rate: 44_100,
        frames: 22_050,
        channels: 2,
    },
    Lossy {
        name: "sine.m4a",
        floor: 21.0,
        rate: 44_100,
        frames: 22_050,
        channels: 2,
    },
    Lossy {
        name: "sine.ogg",
        floor: 36.0,
        rate: 44_100,
        frames: 22_050,
        channels: 2,
    },
    Lossy {
        name: "sine.opus",
        floor: 37.0,
        rate: 48_000,
        frames: 24_000,
        channels: 2,
    },
    Lossy {
        name: "sine-mono.opus",
        floor: 37.0,
        rate: 48_000,
        frames: 24_000,
        channels: 1,
    },
    Lossy {
        name: "sine.webm",
        floor: 37.0,
        rate: 48_000,
        frames: 24_000,
        channels: 2,
    },
];

#[test]
fn opus_speech_modes_keep_the_source_length() {
    // SILK (`voip-mono.opus`) and hybrid then SILK then CELT (`voip.webm`), at speech
    // bitrates: the length is the source's, the pre-skip and the padding cut. They are not
    // compared with the source: SILK codes a sine at 10 kb/s at about 9 dB and does not keep
    // its phase to the sample. Their decode is held to the native one by `expected.json`.
    for (name, channels) in [("voip-mono.opus", 1), ("voip.webm", 2)] {
        let (imported, _) = import(read(name), extension(name)).unwrap();
        assert_eq!(
            (
                imported.info.sample_rate,
                imported.info.channels,
                imported.info.frames
            ),
            (48_000, channels, 24_000),
            "{name}"
        );
    }
}

/// The lossy files whose native decode is kept whole beside them (`<name>.pcm`, little-endian
/// `f32`): MP3 and AAC decode with transcendental functions (`powf`, `cos`) whose last bit
/// differs between the native C library and WebAssembly's Rust one, so the page's decode of
/// them is compared with these samples at a tolerance (`import.test.ts`: 1e-6, measured 3.3e-7
/// at most, -130 dBFS), where every other fixture is compared bit for bit by its digest.
const REFERENCE_PCM: &[&str] = &["sine.mp3", "sine.m4a"];

fn record_path() -> std::path::PathBuf {
    fixture_dir().join("expected.json")
}

fn snr_db(signal: &[f32], decoded: &[f32]) -> f64 {
    let (mut s, mut n) = (0f64, 0f64);
    for (a, b) in signal.iter().zip(decoded) {
        s += f64::from(*a).powi(2);
        n += f64::from(a - b).powi(2);
    }
    10.0 * (s / n.max(1e-30)).log10()
}

/// How far a decode of `REFERENCE_PCM` may be from the samples kept (measured: 3.3e-7 at most
/// between Windows and WebAssembly).
const PCM_TOLERANCE: f32 = 1e-6;

/// The largest difference between `pcm` and the decode kept in `<name>.pcm`; infinite when their
/// lengths differ.
fn reference_gap(name: &str, pcm: &[f32]) -> f32 {
    let kept: Vec<f32> = read(&format!("{name}.pcm"))
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    if kept.len() != pcm.len() {
        return f32::INFINITY;
    }
    kept.iter()
        .zip(pcm)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = audio_import::Sha256::new();
    h.update(bytes);
    h.finish().to_string()
}

fn outcome(name: &str) -> Value {
    match import(read(name), extension(name)) {
        // MP3 and AAC: their samples are checked against `<name>.pcm` at a tolerance, so the
        // record names that file instead of a digest no other C library would give.
        Ok((imported, pcm)) => json!({
            "sha256": imported.hash.to_string(),
            "sample_rate": imported.info.sample_rate,
            "channels": imported.info.channels,
            "frames": imported.info.frames,
            "direct": imported.layout.is_some(),
            "pcm_sha256": if REFERENCE_PCM.contains(&name) { Value::Null } else { pcm_digest(&pcm).into() },
            "pcm_reference": if REFERENCE_PCM.contains(&name) { format!("{name}.pcm").into() } else { Value::Null },
            // The waveform peaks (AU-2): the page's build must give these bytes too.
            "peaks_sha256": imported.peaks.as_deref().map(sha256_hex),
        }),
        Err(e) => json!({ "error": e.code() }),
    }
}

#[test]
fn each_fixture_imports_as_recorded() {
    let found: BTreeMap<&str, Value> = FIXTURES.iter().map(|n| (*n, outcome(n))).collect();
    if std::env::var("FAULK_BLESS").as_deref() == Ok("1") {
        let text = serde_json::to_string_pretty(&found).unwrap() + "\n";
        std::fs::write(record_path(), text).unwrap();
        for name in REFERENCE_PCM {
            let (_, pcm) = import(read(name), extension(name)).unwrap();
            let mut bytes = Vec::new();
            audio_import::cache::encode(&pcm, &mut bytes);
            std::fs::write(fixture_dir().join(format!("{name}.pcm")), bytes).unwrap();
        }
        return;
    }
    // The decodes kept whole match this build's to a millionth: the last bit of `powf` and `cos`
    // differs between C libraries (Windows, Linux, macOS) as between native and WebAssembly.
    for name in REFERENCE_PCM {
        let (_, pcm) = import(read(name), extension(name)).unwrap();
        let worst = reference_gap(name, &pcm);
        println!("{name}: this build and {name}.pcm differ by {worst:e} at most");
        assert!(
            worst <= PCM_TOLERANCE,
            "{name}: decode differs from {name}.pcm by {worst}"
        );
    }
    let text =
        std::fs::read_to_string(record_path()).expect("expected.json (FAULK_BLESS=1 writes it)");
    let expected: BTreeMap<String, Value> = serde_json::from_str(&text).unwrap();
    assert_eq!(
        expected.len(),
        FIXTURES.len(),
        "expected.json names every fixture once"
    );
    for (name, value) in &found {
        assert_eq!(
            Some(value),
            expected.get(*name),
            "{name}: the import differs from expected.json"
        );
    }
}

#[test]
fn the_hash_is_the_one_sha256sum_prints() {
    let sums = std::fs::read_to_string(fixture_dir().join("SHA256SUMS")).unwrap();
    let listed: BTreeMap<&str, &str> = sums
        .lines()
        .filter_map(|l| l.split_once("  "))
        .map(|(hash, name)| (name, hash))
        .collect();
    for name in FIXTURES {
        let mut h = audio_import::Sha256::new();
        h.update(&read(name));
        assert_eq!(
            listed.get(name).copied(),
            Some(h.finish().to_string().as_str()),
            "{name}"
        );
        // The pipeline's hash is the same streamed one, whatever the import's outcome.
        if let Ok((imported, _)) = import(read(name), extension(name)) {
            assert_eq!(imported.hash.to_string(), listed[name], "{name}");
        }
    }
}

#[test]
fn lossless_files_decode_to_the_source_exactly() {
    for name in LOSSLESS {
        let (imported, pcm) = import(read(name), extension(name)).unwrap();
        let info = imported.info;
        let source = source(
            info.sample_rate,
            info.frames as usize,
            usize::from(info.channels),
        );
        assert_eq!(
            info.frames as usize * usize::from(info.channels),
            pcm.len(),
            "{name}"
        );
        assert!(pcm == source, "{name}: a sample differs from the source");
    }
    // 8-bit: the source rounded to 8 bits, one step of 1/128 at most.
    let (imported, pcm) = import(read("sine-u8.wav"), Some("wav")).unwrap();
    let source = source(44_100, imported.info.frames as usize, 2);
    let worst = pcm
        .iter()
        .zip(&source)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(worst <= 1.0 / 128.0, "8-bit WAV off by {worst}");
}

#[test]
fn lossy_files_keep_the_source_length_and_stay_close_to_it() {
    for Lossy {
        name,
        floor,
        rate,
        frames,
        channels,
    } in LOSSY
    {
        let (imported, pcm) = import(read(name), extension(name)).unwrap();
        assert_eq!(
            imported.info.frames, *frames,
            "{name}: the encoder's delay and padding are cut"
        );
        assert_eq!(
            (imported.info.sample_rate, imported.info.channels),
            (*rate, *channels),
            "{name}"
        );
        let ch = usize::from(*channels);
        let source = source(*rate, *frames as usize, ch);
        let snr = snr_db(&source, &pcm);
        println!("{name}: SNR {snr:.1} dB (floor {floor} dB)");
        assert!(snr >= *floor, "{name}: SNR {snr:.1} dB under {floor} dB");
        // Aligned to the sample: shifting the decode by up to 40 frames either way (less than
        // half a period of the sines, so no other alignment matches) only makes it worse.
        let middle = |lag: i64| {
            let (mut s, mut n) = (0f64, 0f64);
            for f in 4_000..18_000i64 {
                let x = f64::from(source[f as usize * ch]);
                let y = f64::from(pcm[(f + lag) as usize * ch]);
                s += x * x;
                n += (x - y) * (x - y);
            }
            s / n
        };
        let best = (-40i64..=40)
            .max_by(|a, b| middle(*a).total_cmp(&middle(*b)))
            .unwrap();
        assert_eq!(
            best, 0,
            "{name}: the decode is {best} frames off the source"
        );
    }
}

#[test]
fn a_direct_layout_reads_the_samples_the_decoder_gives() {
    let mut direct = 0;
    for name in FIXTURES {
        let Ok((imported, pcm)) = import(read(name), extension(name)) else {
            continue;
        };
        let Some(layout) = imported.layout else {
            continue;
        };
        direct += 1;
        let bytes = read(name);
        let start = layout.data_offset as usize;
        let end = start + layout.frames as usize * layout.frame_bytes();
        let mut converted = Vec::new();
        layout.convert(&bytes[start..end], &mut converted);
        assert!(
            converted == pcm,
            "{name}: the layout's samples differ from the decoder's"
        );
    }
    // WAV 16, 24 and float 32, AIFF 16 and 24.
    assert_eq!(direct, 5);
}

#[test]
fn only_wav_and_aiff_of_plain_samples_are_read_directly() {
    for name in FIXTURES {
        let Ok((imported, _)) = import(read(name), extension(name)) else {
            continue;
        };
        let plain = matches!(
            *name,
            "sine-s16.wav" | "sine-s24.wav" | "sine-f32.wav" | "sine-s16.aiff" | "sine-s24.aiff"
        );
        assert_eq!(imported.layout.is_some(), plain, "{name}");
    }
    // A layout that disagrees with the decoder is not used.
    let bytes = read("sine-s16.wav");
    let layout = PcmLayout::parse(&bytes, bytes.len() as u64).unwrap();
    let (imported, _) = import(bytes, Some("wav")).unwrap();
    let mut other = imported.info;
    other.frames -= 1;
    let m = |i: &audio_import::MediaInfo| layout.matches(i.sample_rate, i.channels, i.frames);
    assert!(m(&imported.info) && !m(&other));
}

#[test]
fn text_is_refused_by_name() {
    assert_eq!(
        import(read("not-audio.txt"), Some("txt")).unwrap_err(),
        ImportError::UnsupportedFormat
    );
    // Without the hint, the same answer.
    assert_eq!(
        import(read("not-audio.txt"), None).unwrap_err().code(),
        "unsupported_format"
    );
}

#[test]
fn opus_in_ogg_and_webm_imports_and_a_layout_faulk_does_not_model_is_refused_by_name() {
    for name in ["sine.opus", "sine-mono.opus", "sine.webm", "recorded.webm"] {
        let (imported, _) = import(read(name), None).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(imported.info.sample_rate, 48_000, "{name}");
    }
    for family in [2, 3, 255] {
        let e = import(webm_of_family(family), Some("webm")).unwrap_err();
        assert_eq!(e.code(), "unsupported_codec", "family {family}: {e}");
        assert!(e.to_string().contains(&format!("family {family}")), "{e}");
    }
}

#[test]
fn a_browser_recording_imports_whole_with_its_two_tones() {
    // Recorded by Chromium's MediaRecorder (`make_fixtures.py`): two oscillators, 440 Hz on
    // the left and 660 Hz on the right, one second; 16 packets of 60 ms, no pre-skip, no
    // padding named: every frame is kept.
    let (imported, pcm) = import(read("recorded.webm"), Some("webm")).unwrap();
    assert_eq!(
        (
            imported.info.sample_rate,
            imported.info.channels,
            imported.info.frames
        ),
        (48_000, 2, 16 * 2_880)
    );
    // The strongest of 400 to 700 Hz, by 10 Hz, in each channel, after the first 100 ms.
    for (channel, hz) in [(0, 440.0), (1, 660.0)] {
        let x: Vec<f64> = pcm
            .iter()
            .skip(channel)
            .step_by(2)
            .skip(4_800)
            .map(|s| f64::from(*s))
            .collect();
        let power = |f: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, s) in x.iter().enumerate() {
                let w = 2.0 * std::f64::consts::PI * f * i as f64 / 48_000.0;
                re += s * w.cos();
                im += s * w.sin();
            }
            re * re + im * im
        };
        let best = (40..=70)
            .map(|k| f64::from(k) * 10.0)
            .max_by(|a, b| power(*a).total_cmp(&power(*b)))
            .unwrap();
        assert_eq!(best, hz, "channel {channel}");
    }
}

#[test]
fn a_webm_padding_is_cut_from_the_last_block_only() {
    // `sine.webm` names 648 frames of padding (13.5 ms) in its last block group, after
    // 26 packets of 960 frames and a pre-skip of 312.
    let whole = read("sine.webm");
    let at = whole
        .windows(3)
        .position(|w| w == [0x75, 0xa2, 0x84])
        .expect("sine.webm names its DiscardPadding")
        + 3;
    let with = |ns: u32| {
        let mut bytes = whole.clone();
        bytes[at..at + 4].copy_from_slice(&ns.to_be_bytes());
        import(bytes, Some("webm")).unwrap().0.info.frames
    };
    assert_eq!(with(13_500_000), 24_000);
    // No padding: every frame after the pre-skip.
    assert_eq!(with(0), 26 * 960 - 312);
    // More padding than the last block holds: that block goes, no other.
    assert_eq!(with(u32::MAX >> 1), 25 * 960 - 312);
    // A negative padding (at the start, by the specification) is not an end padding.
    assert_eq!(with(u32::MAX), 26 * 960 - 312);
    // A pre-skip of more than two packets: their blocks hold no frame and are not held back,
    // and the padding is still cut from the last block.
    let mut long_skip = whole.clone();
    let head = long_skip.windows(8).position(|w| w == b"OpusHead").unwrap();
    long_skip[head + 10..head + 12].copy_from_slice(&2_000u16.to_le_bytes());
    let (imported, _) = import(long_skip, Some("webm")).unwrap();
    assert_eq!(imported.info.frames, 26 * 960 - 2_000 - 648);
}
