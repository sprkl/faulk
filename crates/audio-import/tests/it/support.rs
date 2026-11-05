//! Reading the fixtures and decoding them whole.

use audio_import::{ImportError, Imported};
use std::io::Cursor;
use std::path::PathBuf;

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

pub fn read(name: &str) -> Vec<u8> {
    std::fs::read(fixture_dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

pub fn extension(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, e)| e)
}

/// Imports bytes in memory: what the pipeline found, and every decoded sample.
pub fn import(bytes: Vec<u8>, hint: Option<&str>) -> Result<(Imported, Vec<f32>), ImportError> {
    let mut pcm = Vec::new();
    let imported = audio_import::import(Cursor::new(bytes), hint, |block| {
        pcm.extend_from_slice(block)
    })?;
    Ok((imported, pcm))
}

/// The 16-bit source every fixture was made from (`make_fixtures.py`), as `f32` the way the
/// decoder converts it: `i16 / 32 768`.
pub fn source(rate: u32, frames: usize, channels: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * channels);
    for i in 0..frames {
        let t = i as f64 / f64::from(rate);
        for (c, hz) in [440.0, 660.0].iter().take(channels).enumerate() {
            let _ = c;
            let v = (0.5 * (2.0 * std::f64::consts::PI * hz * t).sin() * 32767.0).round() as i16;
            out.push(f32::from(v) / 32_768.0);
        }
    }
    out
}

/// The SHA-256 of samples as the PCM cache stores them (little-endian `f32`), in hex.
pub fn pcm_digest(pcm: &[f32]) -> String {
    let mut h = audio_import::Sha256::new();
    let mut bytes = Vec::new();
    audio_import::cache::encode(pcm, &mut bytes);
    h.update(&bytes);
    h.finish().to_string()
}

/// `sine.webm` with its `OpusHead` (the track's `CodecPrivate`) saying another channel mapping
/// family: a layout of channels Faulk does not model.
pub fn webm_of_family(family: u8) -> Vec<u8> {
    let mut bytes = read("sine.webm");
    let at = bytes
        .windows(8)
        .position(|w| w == b"OpusHead")
        .expect("sine.webm holds its OpusHead");
    bytes[at + 18] = family;
    bytes
}
