//! The encoder delay of an MP4 or M4A file (AAC), from its edit list.
//!
//! An AAC encoder starts every file with a delay (1 024 or 2 112 frames of priming) and pads
//! the end; the file says which frames to play in the edit list of its audio track
//! (`moov/trak/edts/elst`). symphonia 0.6 reads the list but does not apply it, so this
//! module finds it and [`crate::Decoding`] cuts the frames itself: an imported AAC file
//! starts on its first real sample, like a WAV, an MP3 with its LAME tag or a Vorbis file.
//! A list Faulk does not understand (several edits, a rate change, a dwell) is left alone.

use std::io::{Read, Seek, SeekFrom};

/// The edit of an audio track as the file writes it: skip `time` ticks of the media's
/// timescale, then play `duration` ticks of the movie's (0: to the end).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    time: u64,
    media_scale: u32,
    duration: u64,
    movie_scale: u32,
}

impl Edit {
    /// Frames to skip at the start, at `sample_rate`.
    pub fn start(&self, sample_rate: u32) -> Option<u64> {
        scale(self.time, self.media_scale, sample_rate)
    }

    /// Frames to keep after the start, at `sample_rate`, when the edit says (to the nearest
    /// frame: a movie's timescale is often 1 000 per second).
    pub fn length(&self, sample_rate: u32) -> Option<u64> {
        (self.duration > 0)
            .then(|| scale(self.duration, self.movie_scale, sample_rate))
            .flatten()
    }
}

struct Header {
    kind: [u8; 4],
    /// Where the body starts and ends.
    body: u64,
    end: u64,
}

fn read_exact(r: &mut impl Read, n: usize) -> Option<Vec<u8>> {
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).ok()?;
    Some(b)
}

fn u32_be(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_be(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// The boxes directly inside `[start, end)`.
fn children(r: &mut (impl Read + Seek), start: u64, end: u64) -> Option<Vec<Header>> {
    let mut out = Vec::new();
    let mut at = start;
    // A file's box tree is shallow and short: a bound keeps a hostile one from looping long.
    while at + 8 <= end && out.len() < 4096 {
        r.seek(SeekFrom::Start(at)).ok()?;
        let h = read_exact(r, 8)?;
        let size32 = u32_be(&h, 0)?;
        let kind: [u8; 4] = h[4..8].try_into().ok()?;
        let (size, header) = match size32 {
            0 => (end - at, 8),
            1 => (u64_be(&read_exact(r, 8)?, 0)?, 16),
            n => (u64::from(n), 8),
        };
        if size < header || at.checked_add(size)? > end {
            return None;
        }
        out.push(Header {
            kind,
            body: at + header,
            end: at + size,
        });
        at += size;
    }
    Some(out)
}

fn find<'a>(boxes: &'a [Header], kind: &[u8; 4]) -> Option<&'a Header> {
    boxes.iter().find(|b| &b.kind == kind)
}

fn body(r: &mut (impl Read + Seek), b: &Header, max: u64) -> Option<Vec<u8>> {
    let len = (b.end - b.body).min(max);
    r.seek(SeekFrom::Start(b.body)).ok()?;
    read_exact(r, len as usize)
}

/// The timescale of a `mvhd` or `mdhd` body (version 0 or 1).
fn timescale(full: &[u8]) -> Option<u32> {
    match full.first()? {
        0 => u32_be(full, 12),
        1 => u32_be(full, 20),
        _ => None,
    }
}

/// The edit of the first audio track of an MP4 file; `None` when the file is not an MP4 or
/// its edit list is not a plain trim.
pub fn edit(r: &mut (impl Read + Seek), len: u64) -> Option<Edit> {
    let top = children(r, 0, len)?;
    if top.first()?.kind != *b"ftyp" {
        return None;
    }
    let moov = find(&top, b"moov")?;
    let inside = children(r, moov.body, moov.end)?;
    let movie_scale = timescale(&body(r, find(&inside, b"mvhd")?, 32)?)?;
    for trak in inside.iter().filter(|b| &b.kind == b"trak") {
        let parts = children(r, trak.body, trak.end)?;
        let Some(mdia) = find(&parts, b"mdia") else {
            continue;
        };
        let media = children(r, mdia.body, mdia.end)?;
        let hdlr = body(r, find(&media, b"hdlr")?, 16)?;
        if hdlr.get(8..12)? != b"soun" {
            continue;
        }
        let media_scale = timescale(&body(r, find(&media, b"mdhd")?, 32)?)?;
        let edts = find(&parts, b"edts")?;
        let edit_boxes = children(r, edts.body, edts.end)?;
        let elst = find(&edit_boxes, b"elst")?;
        let list = body(r, elst, 4096)?;
        let version = *list.first()?;
        let count = u32_be(&list, 4)?;
        let entry = if version == 1 { 20 } else { 12 };
        let mut edits = Vec::new();
        for i in 0..count.min(64) as usize {
            let at = 8 + i * entry;
            let (duration, time) = if version == 1 {
                (u64_be(&list, at)?, u64_be(&list, at + 8)? as i64)
            } else {
                (
                    u64::from(u32_be(&list, at)?),
                    i64::from(u32_be(&list, at + 4)? as i32),
                )
            };
            let rate = u32_be(&list, at + entry - 4)?;
            // An empty edit (time -1) at the start is a pause before the media: not a trim.
            if time < 0 {
                return None;
            }
            if rate != 0x0001_0000 {
                return None;
            }
            edits.push((duration, time as u64));
        }
        let [(duration, time)] = edits[..] else {
            return None;
        };
        return Some(Edit {
            time,
            media_scale,
            duration,
            movie_scale,
        });
    }
    None
}

/// `value` ticks of `from` per second as frames at `to` per second, rounded.
fn scale(value: u64, from: u32, to: u32) -> Option<u64> {
    if from == 0 {
        return None;
    }
    let v = u128::from(value) * u128::from(to);
    u64::try_from((v + u128::from(from) / 2) / u128::from(from)).ok()
}
