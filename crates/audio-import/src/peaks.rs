//! The waveform peaks of a media (AU-2, #111): what the page draws a clip's waveform from,
//! at any zoom, without reading its audio. `.faulk/cache/peaks/<sha256>.peaks`, next to the
//! PCM cache: named by the blob (every clip of a media shares it), regenerable from the blob
//! at any time, never versioned, so a missing or doubtful file is simply made again.
//!
//! **Levels.** Level 0 holds one peak per block of [`BLOCK`] (256) frames; levels 1, 2 and 3
//! one per 1 024, 4 096 and 16 384 frames ([`FACTOR`] 4: ×4, ×16, ×64). A peak is, for each
//! channel, the lowest and the highest sample of its block as two `i8` in [-127, 127]
//! ([`SCALE`]), rounded **outward** (the low one down, the high one up), so the envelope drawn
//! always holds every sample; a value outside [-1, 1] is clamped, a NaN is left out. A peak of
//! a coarser level is the lowest low and the highest high of the four peaks below it: since
//! rounding down and up keep the order, that is exactly the peak its whole block would have
//! (tested). The last peak of a level covers what is left of the media.
//!
//! **Size.** Two bytes per channel per block of 256 frames at level 0: in stereo, 45 000 bytes
//! a minute at 48 kHz (41 344 at 44.1 kHz); the four levels together are 1 + 1/4 + 1/16 +
//! 1/64 ≈ 1.33 times level 0 (an hour of 48 kHz stereo: 2.7 MB at level 0, 3.6 MB in all).
//!
//! Layout, little-endian: a 32-byte header, then levels 0 to 3, each [`count`] peaks of
//! `channels × [low, high]` bytes.
//!
//! | Bytes | Field |
//! |---|---|
//! | 0..8 | `FKPEAK01` |
//! | 8..12 | sample rate (`u32`) |
//! | 12..14 | channels (`u16`) |
//! | 14..16 | levels (`u16`, [`LEVELS`]) |
//! | 16..24 | frames (`u64`) |
//! | 24..28 | the decoder's version ([`crate::cache::DECODER`], `u32`) |
//! | 28..30 | frames of a level-0 block (`u16`, [`BLOCK`]) |
//! | 30..32 | the factor between levels (`u16`, [`FACTOR`]) |
//!
//! A file is valid only when its header is exactly the one of its media and of this decoder,
//! and its size the one its header gives ([`is_valid`]); anything else is made again.
//!
//! The same code runs natively and in WebAssembly, on the same decoded samples: the same
//! file of peaks, byte for byte, for the same decoded audio.

use crate::MediaInfo;
use crate::cache::DECODER;

/// The first bytes of a peaks file.
pub const MAGIC: &[u8; 8] = b"FKPEAK01";

/// Size of the header.
pub const HEADER_BYTES: usize = 32;

/// Frames of a level-0 block.
pub const BLOCK: u64 = 256;

/// How many blocks of a level make one of the next.
pub const FACTOR: u64 = 4;

/// The levels a file holds: blocks of 256, 1 024, 4 096 and 16 384 frames.
pub const LEVELS: usize = 4;

/// A sample of 1.0 is a peak of 127.
pub const SCALE: f32 = 127.0;

/// The directory of the peaks files in a project folder.
pub const DIR: &str = ".faulk/cache/peaks";

/// The path of the peaks of the blob `hash`, relative to the project's root.
pub fn path(hash: &project::Hash) -> String {
    format!("{DIR}/{hash}.peaks")
}

/// Frames of a block of `level`.
pub fn block_frames(level: usize) -> u64 {
    BLOCK * FACTOR.pow(level as u32)
}

/// Peaks of `level` for a media of `frames` frames.
pub fn count(frames: u64, level: usize) -> u64 {
    frames.div_ceil(block_frames(level))
}

/// The header of the peaks of a file with this metadata.
pub fn header(info: &MediaInfo) -> [u8; HEADER_BYTES] {
    let mut h = [0u8; HEADER_BYTES];
    h[..8].copy_from_slice(MAGIC);
    h[8..12].copy_from_slice(&info.sample_rate.to_le_bytes());
    h[12..14].copy_from_slice(&info.channels.to_le_bytes());
    h[14..16].copy_from_slice(&(LEVELS as u16).to_le_bytes());
    h[16..24].copy_from_slice(&info.frames.to_le_bytes());
    h[24..28].copy_from_slice(&DECODER.to_le_bytes());
    h[28..30].copy_from_slice(&(BLOCK as u16).to_le_bytes());
    h[30..32].copy_from_slice(&(FACTOR as u16).to_le_bytes());
    h
}

/// Bytes of one peak (every channel's low and high).
fn stride(channels: u16) -> u64 {
    2 * u64::from(channels)
}

/// The size of the peaks of this metadata, or `None` when it would overflow.
pub fn size(info: &MediaInfo) -> Option<u64> {
    let mut total = HEADER_BYTES as u64;
    for level in 0..LEVELS {
        total = total.checked_add(count(info.frames, level).checked_mul(stride(info.channels))?)?;
    }
    Some(total)
}

/// Where the bytes of `level` start in the file and how many there are.
pub fn level_span(info: &MediaInfo, level: usize) -> (usize, usize) {
    let mut at = HEADER_BYTES;
    for below in 0..level {
        at += (count(info.frames, below) * stride(info.channels)) as usize;
    }
    (
        at,
        (count(info.frames, level) * stride(info.channels)) as usize,
    )
}

/// Whether a peaks file is whole and of this metadata: its header (at least
/// [`HEADER_BYTES`] bytes of it) and its size.
pub fn is_valid(head: &[u8], file_len: u64, info: &MediaInfo) -> bool {
    head.len() >= HEADER_BYTES
        && head[..HEADER_BYTES] == header(info)
        && size(info) == Some(file_len)
}

/// The low of a block whose lowest sample is `v`: rounded down, clamped.
pub fn low(v: f32) -> i8 {
    (v * SCALE).floor().clamp(-SCALE, SCALE) as i8
}

/// The high of a block whose highest sample is `v`: rounded up, clamped.
pub fn high(v: f32) -> i8 {
    (v * SCALE).ceil().clamp(-SCALE, SCALE) as i8
}

/// Builds the peaks of a media as its decoded blocks come (interleaved `f32`).
pub struct PeakBuilder {
    channels: usize,
    low: Vec<f32>,
    high: Vec<f32>,
    /// Frames in the block being filled.
    filled: u64,
    /// Frames pushed in all.
    frames: u64,
    /// Level 0 so far.
    level0: Vec<i8>,
    /// Memory was refused: no peaks for this media.
    failed: bool,
}

impl PeakBuilder {
    pub fn new(channels: u16) -> Self {
        let channels = usize::from(channels);
        Self {
            channels,
            low: vec![f32::INFINITY; channels],
            high: vec![f32::NEG_INFINITY; channels],
            filled: 0,
            frames: 0,
            level0: Vec::new(),
            failed: channels == 0,
        }
    }

    /// Takes a block of interleaved samples (whole frames; a trailing partial frame is left
    /// out).
    pub fn push(&mut self, samples: &[f32]) {
        if self.failed {
            return;
        }
        for frame in samples.chunks_exact(self.channels) {
            for ((s, lo), hi) in frame.iter().zip(&mut self.low).zip(&mut self.high) {
                // `min`/`max` of f32 leave a NaN out.
                *lo = lo.min(*s);
                *hi = hi.max(*s);
            }
            self.filled += 1;
            self.frames += 1;
            if self.filled == BLOCK {
                self.flush();
            }
        }
    }

    /// Ends the block being filled.
    fn flush(&mut self) {
        if self.level0.try_reserve(2 * self.channels).is_err() {
            self.failed = true;
            self.level0 = Vec::new();
            return;
        }
        for (lo, hi) in self.low.iter_mut().zip(&mut self.high) {
            // A block of NaN only: nothing to show.
            let (l, h) = if lo > hi {
                (0, 0)
            } else {
                (low(*lo), high(*hi))
            };
            self.level0.push(l);
            self.level0.push(h);
            *lo = f32::INFINITY;
            *hi = f32::NEG_INFINITY;
        }
        self.filled = 0;
    }

    /// The frames pushed so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The whole peaks file of the media, or `None` when what was pushed is not this media
    /// (other channels, other frames) or memory was refused.
    pub fn finish(mut self, info: &MediaInfo) -> Option<Vec<u8>> {
        if self.failed || usize::from(info.channels) != self.channels || info.frames != self.frames
        {
            return None;
        }
        if self.filled > 0 {
            self.flush();
            if self.failed {
                return None;
            }
        }
        let total = usize::try_from(size(info)?).ok()?;
        let mut out = Vec::new();
        out.try_reserve_exact(total).ok()?;
        out.extend_from_slice(&header(info));
        out.extend(self.level0.iter().map(|b| *b as u8));
        let stride = 2 * self.channels;
        let mut below = self.level0;
        for level in 1..LEVELS {
            let n = count(info.frames, level) as usize;
            let mut this = Vec::new();
            this.try_reserve_exact(n * stride).ok()?;
            for i in 0..n {
                let group = &below[i * FACTOR as usize * stride
                    ..((i + 1) * FACTOR as usize * stride).min(below.len())];
                for c in 0..self.channels {
                    let peaks = group.chunks_exact(stride);
                    let l = peaks.clone().map(|p| p[2 * c]).min().unwrap_or(0);
                    let h = peaks.map(|p| p[2 * c + 1]).max().unwrap_or(0);
                    this.push(l);
                    this.push(h);
                }
            }
            out.extend(this.iter().map(|b| *b as u8));
            below = this;
        }
        debug_assert_eq!(out.len(), total);
        Some(out)
    }
}

/// The peaks of interleaved samples held whole (tests, a short file).
pub fn of(samples: &[f32], info: &MediaInfo) -> Option<Vec<u8>> {
    let mut builder = PeakBuilder::new(info.channels);
    builder.push(samples);
    builder.finish(info)
}
