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

/// Lossy files and the lowest signal-to-noise ratio (dB) of their decode against the source,
/// over the whole file: the tolerance of the reference comparison. Measured on 2026-11-03:
/// MP3 (LAME, 128 kb/s) 26.0 dB, AAC (FFmpeg's encoder, 128 kb/s) 24.6 dB, Vorbis
/// (libsndfile's default quality) 40.6 dB; the floors keep about 4 dB of margin. The length
/// is the source's exactly, and the decode is aligned on it to the sample: the encoder's
/// delay and padding are cut (LAME tag, Ogg granule positions, the MP4 edit list).
const LOSSY: &[(&str, f64)] = &[("sine.mp3", 22.0), ("sine.m4a", 21.0), ("sine.ogg", 36.0)];

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
    for (name, floor) in LOSSY {
        let (imported, pcm) = import(read(name), extension(name)).unwrap();
        assert_eq!(
            imported.info.frames, 22_050,
            "{name}: the encoder's delay and padding are cut"
        );
        assert_eq!(
            (imported.info.sample_rate, imported.info.channels),
            (44_100, 2),
            "{name}"
        );
        let source = source(44_100, 22_050, 2);
        let snr = snr_db(&source, &pcm);
        println!("{name}: SNR {snr:.1} dB (floor {floor} dB)");
        assert!(snr >= *floor, "{name}: SNR {snr:.1} dB under {floor} dB");
        // Aligned to the sample: shifting the decode by up to 40 frames either way (less than
        // half a period of the sines, so no other alignment matches) only makes it worse.
        let middle = |lag: i64| {
            let (mut s, mut n) = (0f64, 0f64);
            for f in 4_000..18_000i64 {
                let x = f64::from(source[(f * 2) as usize]);
                let y = f64::from(pcm[((f + lag) * 2) as usize]);
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
fn opus_and_text_are_refused_by_name() {
    assert_eq!(
        import(read("sine.opus"), Some("opus")).unwrap_err(),
        ImportError::UnsupportedCodec("Opus".into())
    );
    assert_eq!(
        import(read("not-audio.txt"), Some("txt")).unwrap_err(),
        ImportError::UnsupportedFormat
    );
    // Without the hint, the same answers.
    assert_eq!(
        import(read("sine.opus"), None).unwrap_err().code(),
        "unsupported_codec"
    );
    assert_eq!(
        import(read("not-audio.txt"), None).unwrap_err().code(),
        "unsupported_format"
    );
}
