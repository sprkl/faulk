//! Faulk's audio import pipeline (AU-1, #110), written once and run where the bytes are: in
//! the page's project store Worker (compiled to WebAssembly, `crates/audio-import-wasm`) and,
//! later, in the helper (native, AU-7). Pure Rust: it reads through a [`Source`] the host
//! gives and hands the decoded PCM to a sink; it never opens a file itself.
//!
//! What an import computes, for a file the user chose:
//!
//! - its SHA-256 in a stream ([`Sha256`]): the name of the blob `blobs/<sha256>` that keeps
//!   the file's original bytes (project model § 3), the same `oid` as Git LFS and as
//!   `sha256sum`;
//! - its decoding ([`Decoding`], `symphonia`): every packet is decoded, so a damaged file is
//!   refused at import rather than at playback; the result is the metadata the clip writes
//!   in clear ([`MediaInfo`]: sample rate, channels, frames) and the PCM as `f32`;
//! - whether the file can be read directly by ranges ([`PcmLayout`]: WAV and AIFF holding
//!   16, 24 or 32-bit integers or 32-bit floats). Such a file needs no decoded copy: the
//!   disk Worker of AU-3 reads its samples from the blob. Any other file gets the **PCM
//!   cache** ([`cache`]), `.faulk/cache/pcm/<sha256>.pcm`: regenerable, never versioned.
//!
//! The rate is the file's own: resampling happens at playback (AU-4), never here.
//!
//! **Formats** (decided by AU-1): WAV (PCM 8 to 32-bit, float 32 and 64, A-law, µ-law,
//! IMA and Microsoft ADPCM), AIFF and AIFF-C, CAF, FLAC, MP3 (and MP1, MP2), AAC-LC in MP4,
//! M4A or ADTS, ALAC, Ogg Vorbis. Not Opus: symphonia 0.6 has no Opus decoder, and an Opus
//! file is refused with [`ImportError::UnsupportedCodec`]. No SIMD feature of symphonia is
//! enabled, so the native and the WebAssembly builds run the same scalar code.
//!
//! Every failure is an [`ImportError`]; a panic inside a decoder (a bug of a dependency on a
//! hostile file) is caught where unwinding exists and reported as [`ImportError::Corrupt`].
//! In WebAssembly a panic aborts the instance: the page catches the trap, reports the same
//! error and starts a new instance.

pub mod cache;
mod decode;
mod error;
mod hash;
mod layout;
mod mp4;

pub use decode::{BLOCK_FRAMES, Decoding, MediaInfo, Source};
pub use error::ImportError;
pub use hash::Sha256;
pub use layout::{PcmLayout, SampleEncoding};

use std::io::{Read, Seek, SeekFrom};

/// What [`import`] found: the file's metadata, its hash, and how it can be read later.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub hash: project::Hash,
    pub info: MediaInfo,
    /// `Some` when the blob can be read directly by ranges (no PCM cache needed).
    pub layout: Option<PcmLayout>,
}

/// The whole pipeline on a source that can be read twice (a file on disk, natively): hash,
/// decode (every block goes to `sink`, interleaved `f32`), layout. The page runs the same
/// steps one by one through `crates/audio-import-wasm`, to report progress and write as it
/// goes. `hint` is the file's extension, if any (it only helps the probe).
pub fn import<S: Read + Seek + Send + Sync + 'static>(
    mut source: S,
    hint: Option<&str>,
    mut sink: impl FnMut(&[f32]),
) -> Result<Imported, ImportError> {
    source.seek(SeekFrom::Start(0)).map_err(ImportError::io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = source.read(&mut buffer).map_err(ImportError::io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let hash = hasher.finish();
    source.seek(SeekFrom::Start(0)).map_err(ImportError::io)?;
    let mut head = vec![0u8; layout::HEAD_BYTES];
    let read = read_up_to(&mut source, &mut head)?;
    head.truncate(read);
    let len = source.seek(SeekFrom::End(0)).map_err(ImportError::io)?;
    source.seek(SeekFrom::Start(0)).map_err(ImportError::io)?;
    let mut decoding = Decoding::open(Box::new(Plain(source, len)), hint)?;
    let mut block = Vec::new();
    while decoding.next(&mut block)? {
        sink(&block);
    }
    let info = decoding.finish()?;
    let layout = PcmLayout::parse(&head, len).filter(|l| l.matches(&info));
    Ok(Imported { hash, info, layout })
}

fn read_up_to(source: &mut impl Read, buffer: &mut [u8]) -> Result<usize, ImportError> {
    let mut at = 0;
    while at < buffer.len() {
        let n = source.read(&mut buffer[at..]).map_err(ImportError::io)?;
        if n == 0 {
            break;
        }
        at += n;
    }
    Ok(at)
}

/// A seekable reader of known length as a [`Source`].
struct Plain<S>(S, u64);

impl<S: Read> Read for Plain<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<S: Seek> Seek for Plain<S> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl<S: Read + Seek + Send + Sync> Source for Plain<S> {
    fn len(&self) -> u64 {
        self.1
    }
}
