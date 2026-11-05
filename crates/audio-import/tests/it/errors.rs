//! A damaged or foreign file is refused with a typed error, never a panic out of the pipeline:
//! truncations, flipped bytes, random bytes, headers out of the project's limits, and a
//! time-bounded fuzz (`FAULK_FUZZ_SEED=<n>` replays a run, `FAULK_FUZZ_SECS=<s>` lengthens it).

use crate::support::*;
use audio_import::ImportError;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

/// Every fixture that imports.
const AUDIO: &[&str] = &[
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
];

/// A small deterministic generator (xorshift64*), so a failing run replays from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Imports `bytes`: `Ok` or a typed error; a panic escaping the pipeline fails the test.
fn outcome(bytes: Vec<u8>, hint: Option<&str>, what: &str) -> Result<(), ImportError> {
    match catch_unwind(AssertUnwindSafe(|| import(bytes, hint))) {
        Ok(result) => result.map(|_| ()),
        Err(_) => panic!("{what}: a panic escaped the pipeline"),
    }
}

/// A WAV header for 16-bit PCM, then `frames` silent frames.
fn wav(channels: u16, rate: u32, frames: u32) -> Vec<u8> {
    let align = 2 * channels;
    let data = frames * u32::from(align);
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    b.extend_from_slice(&align.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    b.resize(b.len() + data as usize, 0);
    b
}

#[test]
fn truncated_files_give_a_typed_error_or_their_whole_packets() {
    for name in AUDIO {
        let bytes = read(name);
        for k in 0..64 {
            let cut = bytes.len() * k / 64;
            let what = format!("{name} cut at {cut}");
            match outcome(bytes[..cut].to_vec(), extension(name), &what) {
                // A cut on a packet's edge, or in the padding, leaves a shorter but whole file.
                Ok(()) => {}
                Err(e) => assert!(
                    matches!(
                        e.code(),
                        "corrupt" | "empty" | "unsupported_format" | "unsupported_codec"
                    ),
                    "{what}: {e:?}"
                ),
            }
        }
        // Nothing at all.
        assert_eq!(
            outcome(Vec::new(), extension(name), name),
            Err(ImportError::Empty)
        );
    }
}

#[test]
fn flipped_bytes_give_a_typed_error_or_a_decode() {
    let mut rng = Rng(0x00c0_ffee);
    for name in AUDIO {
        let bytes = read(name);
        for round in 0..48 {
            let mut damaged = bytes.clone();
            for _ in 0..1 + round % 8 {
                let at = rng.below(damaged.len());
                damaged[at] ^= 1 << rng.below(8);
            }
            let _ = outcome(
                damaged,
                extension(name),
                &format!("{name} flip round {round}"),
            );
        }
    }
}

#[test]
fn headers_out_of_the_projects_limits_are_refused_by_name() {
    assert_eq!(outcome(wav(2, 44_100, 100), Some("wav"), "plain"), Ok(()));
    assert_eq!(
        outcome(wav(2, 4_000, 100), Some("wav"), "4 kHz"),
        Err(ImportError::BadSampleRate(4_000))
    );
    // 65 channels: symphonia's WAV reader refuses the header first (it takes 32 at most
    // without a channel mask), else the pipeline would (`BadChannels`).
    let many = outcome(wav(65, 44_100, 10), Some("wav"), "65 channels").unwrap_err();
    assert!(
        matches!(many, ImportError::Corrupt(_) | ImportError::BadChannels(65)),
        "{many:?}"
    );
    assert_eq!(
        outcome(wav(2, 44_100, 0), Some("wav"), "no frame"),
        Err(ImportError::Empty)
    );
    // Zero channels: the reader refuses the header.
    let zero = outcome(wav(0, 44_100, 0), Some("wav"), "0 channels").unwrap_err();
    assert!(
        matches!(zero.code(), "corrupt" | "bad_channels" | "empty"),
        "{zero:?}"
    );
    // A WAV header with no data chunk.
    let mut header = wav(2, 44_100, 0);
    header.truncate(36);
    assert!(outcome(header, Some("wav"), "no data chunk").is_err());
}

#[test]
fn random_bytes_and_wrong_hints_are_refused() {
    let mut rng = Rng(42);
    for len in [1, 7, 64, 4096, 100_000] {
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        for hint in [
            None,
            Some("wav"),
            Some("mp3"),
            Some("flac"),
            Some("m4a"),
            Some("ogg"),
            Some("opus"),
            Some("webm"),
        ] {
            assert!(
                outcome(
                    bytes.clone(),
                    hint,
                    &format!("{len} random bytes as {hint:?}")
                )
                .is_err()
            );
        }
    }
    // A file named for another format still imports by its content.
    assert_eq!(
        outcome(read("sine.flac"), Some("mp3"), "flac named mp3"),
        Ok(())
    );
    assert_eq!(
        outcome(read("sine.mp3"), Some("wav"), "mp3 named wav"),
        Ok(())
    );
}

#[test]
fn fuzz_of_every_fixture_never_lets_a_panic_out() {
    let seed: u64 = std::env::var("FAULK_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
                | 1
        });
    let secs: f64 = std::env::var("FAULK_FUZZ_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.5);
    println!("audio import fuzz: FAULK_FUZZ_SEED={seed} for {secs} s");
    let mut rng = Rng(seed);
    let files: Vec<Vec<u8>> = AUDIO.iter().map(|n| read(n)).collect();
    let deadline = Instant::now() + Duration::from_secs_f64(secs);
    let mut runs = 0;
    while Instant::now() < deadline {
        let pick = rng.below(files.len());
        let mut bytes = files[pick].clone();
        match rng.below(4) {
            0 => bytes.truncate(rng.below(bytes.len())),
            1 => {
                for _ in 0..1 + rng.below(32) {
                    let at = rng.below(bytes.len());
                    bytes[at] = rng.next() as u8;
                }
            }
            2 => {
                // A run of bytes overwritten in the headers' region.
                let at = rng.below(bytes.len().min(512));
                for b in bytes.iter_mut().skip(at).take(1 + rng.below(16)) {
                    *b = rng.next() as u8;
                }
            }
            _ => {
                let at = rng.below(bytes.len());
                let len = rng.below(bytes.len() - at);
                bytes.drain(at..at + len);
            }
        }
        let _ = outcome(
            bytes,
            extension(AUDIO[pick]),
            &format!("fuzz seed {seed} run {runs} on {}", AUDIO[pick]),
        );
        runs += 1;
    }
    println!("audio import fuzz: {runs} damaged files, no panic out of the pipeline");
    assert!(runs > 0);
}
