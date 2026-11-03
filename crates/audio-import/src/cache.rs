//! The decoded PCM of a file that cannot be read directly (a compressed file, or an encoding
//! [`crate::PcmLayout`] does not take): `.faulk/cache/pcm/<sha256>.pcm`, next to nothing
//! versioned. Named by the blob, so every clip of a media shares it; regenerable from the
//! blob at any time (by the target that holds the disk), so a missing or doubtful cache is
//! simply made again.
//!
//! Layout, little-endian: a 32-byte header, then the frames, interleaved `f32`.
//!
//! | Bytes | Field |
//! |---|---|
//! | 0..8 | `FKPCM001` |
//! | 8..12 | sample rate (`u32`) |
//! | 12..14 | channels (`u16`) |
//! | 14..16 | reserved, 0 |
//! | 16..24 | frames (`u64`) |
//! | 24..28 | the decoder's version ([`DECODER`], `u32`) |
//! | 28..32 | reserved, 0 |
//!
//! A cache made by another version of the decoder is made again: a decoder whose output
//! changed (a dependency's fix) never leaves two machines playing two versions of a media.

use crate::MediaInfo;

/// The first bytes of a PCM cache file.
pub const MAGIC: &[u8; 8] = b"FKPCM001";

/// The version of what decoding produces. Raise it whenever a change (of this crate, or of
/// symphonia's version or features) may change one decoded sample: every cache is then made
/// again. The fixtures' PCM digests (`tests/fixtures/expected.json`) catch such a change.
pub const DECODER: u32 = 1;

/// Size of the header.
pub const HEADER_BYTES: usize = 32;

/// The directory of the PCM caches in a project folder.
pub const DIR: &str = ".faulk/cache/pcm";

/// The path of the cache of the blob `hash`, relative to the project's root.
pub fn path(hash: &project::Hash) -> String {
    format!("{DIR}/{hash}.pcm")
}

/// The header of the cache of a file with this metadata.
pub fn header(info: &MediaInfo) -> [u8; HEADER_BYTES] {
    let mut h = [0u8; HEADER_BYTES];
    h[..8].copy_from_slice(MAGIC);
    h[8..12].copy_from_slice(&info.sample_rate.to_le_bytes());
    h[12..14].copy_from_slice(&info.channels.to_le_bytes());
    h[16..24].copy_from_slice(&info.frames.to_le_bytes());
    h[24..28].copy_from_slice(&DECODER.to_le_bytes());
    h
}

/// The size a complete cache of this metadata has, or `None` when it would overflow.
pub fn size(info: &MediaInfo) -> Option<u64> {
    info.frames
        .checked_mul(u64::from(info.channels))?
        .checked_mul(4)?
        .checked_add(HEADER_BYTES as u64)
}

/// Whether a cache file is whole and of this metadata: its header (at least
/// [`HEADER_BYTES`] bytes of it) and its size. Anything else is made again.
pub fn is_valid(head: &[u8], file_len: u64, info: &MediaInfo) -> bool {
    head.len() >= HEADER_BYTES
        && head[..HEADER_BYTES] == header(info)
        && size(info) == Some(file_len)
}

/// Appends interleaved samples as cache bytes.
pub fn encode(samples: &[f32], out: &mut Vec<u8>) {
    out.reserve(samples.len() * 4);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
}
