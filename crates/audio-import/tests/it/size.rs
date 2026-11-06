//! The cap on what a file decodes to (#780): a file that compresses very well (a FLAC of
//! silence, ~35 000 to 1) is refused with a typed error, early when its header declares the
//! length, otherwise by the block that would pass the cap, which the host never receives; the
//! memory of the decode stays bounded whatever the file declares or holds; an hour imports.

use crate::support::*;
use audio_import::{Decoding, ImportError, MAX_DECODED_BYTES, MediaInfo, Source};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::Cursor;
use std::time::Instant;

// --- Memory: the heap a thread holds, and its peak ------------------------------------------

/// The system allocator, counting the bytes each thread holds and their peak, so that a test
/// measures the heap its own decode takes (the tests of the binary run on other threads).
struct Counting;

thread_local! {
    static HELD: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn count(delta: isize) {
    // `try_with`: nothing is counted while the thread's locals are torn down.
    let _ = HELD.try_with(|held| {
        let now = held.get() + delta;
        held.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
}

// SAFETY: every call is forwarded to `System` with the same arguments, so the allocator's
// contract is `System`'s; the counting touches only `Cell`s of const thread locals, which never
// allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract for `alloc` is `System.alloc`'s.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            count(layout.size() as isize);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was given by this allocator, so by `System`, with this layout.
        unsafe { System.dealloc(ptr, layout) };
        count(-(layout.size() as isize));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` was given by `System` with this layout; the caller's contract for
        // `realloc` is `System.realloc`'s.
        let moved = unsafe { System.realloc(ptr, layout, new_size) };
        if !moved.is_null() {
            count(new_size as isize - layout.size() as isize);
        }
        moved
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` and gives its result and the most heap it held above what the thread held before.
fn peak_of<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let start = HELD.with(Cell::get);
    PEAK.with(|p| p.set(start));
    let out = f();
    let peak = PEAK.with(Cell::get) - start;
    (out, peak.max(0) as usize)
}

// --- Crafted files ---------------------------------------------------------------------------

/// CRC-8 of FLAC frame headers (polynomial 0x07).
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// CRC-16 of FLAC frames (polynomial 0x8005).
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// A frame number as FLAC writes it (UTF-8's scheme, up to 36 bits).
fn utf8_number(n: u64, out: &mut Vec<u8>) {
    if n < 0x80 {
        out.push(n as u8);
        return;
    }
    let mut bytes = 2;
    while n >= 1 << (5 * bytes + 1) {
        bytes += 1;
    }
    out.push((0xFF00u16 >> bytes) as u8 | (n >> (6 * (bytes - 1))) as u8);
    for i in (0..bytes - 1).rev() {
        out.push(0x80 | ((n >> (6 * i)) & 0x3F) as u8);
    }
}

/// Frames per FLAC block: the format's largest.
const FLAC_BLOCK: u64 = 65_535;

/// A FLAC of silence at 48 kHz, 16 bits: `frames` frames of `channels` channels in blocks of
/// 65 535 frames whose subframes are CONSTANT (about 10 bytes per block and channel). Its
/// `STREAMINFO` declares `frames`, or no length (0) when `declare` is false.
pub fn silent_flac(channels: u8, frames: u64, declare: bool) -> Vec<u8> {
    let mut b = b"fLaC".to_vec();
    // The last metadata block, STREAMINFO, 34 bytes.
    b.extend_from_slice(&[0x80, 0, 0, 34]);
    let block = FLAC_BLOCK.min(frames).max(16) as u16;
    // Fixed blocking: the minimum and the maximum are the same (the last block may be shorter).
    b.extend_from_slice(&block.to_be_bytes());
    b.extend_from_slice(&block.to_be_bytes());
    b.extend_from_slice(&[0; 6]); // frame sizes unknown
    let total = if declare { frames } else { 0 };
    // 20 bits of rate, 3 of channels - 1, 5 of bits - 1, 36 of total samples.
    let packed: u64 = (48_000u64 << 44)
        | (u64::from(channels - 1) << 41)
        | (15u64 << 36)
        | (total & ((1 << 36) - 1));
    b.extend_from_slice(&packed.to_be_bytes());
    b.extend_from_slice(&[0; 16]); // MD5 unknown
    let mut at = 0u64;
    let mut number = 0u64;
    while at < frames {
        let n = (frames - at).min(FLAC_BLOCK);
        let start = b.len();
        // Sync, fixed blocking; block size from the end of the header (16 bits), 48 kHz;
        // independent channels, 16 bits.
        b.extend_from_slice(&[0xFF, 0xF8, 0x7A, ((channels - 1) << 4) | 0x08]);
        utf8_number(number, &mut b);
        b.extend_from_slice(&((n - 1) as u16).to_be_bytes());
        let crc = crc8(&b[start..]);
        b.push(crc);
        for _ in 0..channels {
            // A CONSTANT subframe of 0.
            b.extend_from_slice(&[0x00, 0x00, 0x00]);
        }
        let crc = crc16(&b[start..]);
        b.extend_from_slice(&crc.to_be_bytes());
        at += n;
        number += 1;
    }
    b
}

/// A WAV of IMA ADPCM, stereo 48 kHz, holding `blocks` blocks of silence (2 041 frames each).
/// With `lie`, its header says what a writer that streamed it leaves (`u32::MAX` as the RIFF and
/// `data` sizes, which symphonia reads as unknown) and its `fact` chunk claims `lie` frames;
/// otherwise every size is the true one.
fn adpcm_wav(blocks: u32, lie: Option<u32>) -> Vec<u8> {
    const ALIGN: u32 = 2048;
    const PER_BLOCK: u16 = 2041; // (2048 - 4 × 2) × 8 / (4 × 2) + 1
    let data = blocks * ALIGN;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    let riff = lie.map_or(4 + 28 + 12 + 8 + data, |_| u32::MAX);
    b.extend_from_slice(&riff.to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&20u32.to_le_bytes());
    b.extend_from_slice(&0x11u16.to_le_bytes()); // IMA ADPCM
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&48_000u32.to_le_bytes());
    b.extend_from_slice(&(48_000 * ALIGN / u32::from(PER_BLOCK)).to_le_bytes());
    b.extend_from_slice(&(ALIGN as u16).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&PER_BLOCK.to_le_bytes());
    b.extend_from_slice(b"fact");
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(&lie.unwrap_or(blocks * u32::from(PER_BLOCK)).to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&lie.map_or(data, |_| u32::MAX).to_le_bytes());
    b.resize(b.len() + data as usize, 0);
    b
}

/// An AIFF of 16-bit stereo at 44.1 kHz holding `frames` silent frames, whose `COMM` declares
/// `declared` frames (a writer that streamed it and never came back to the header).
fn aiff(frames: u32, declared: u32) -> Vec<u8> {
    let data = frames * 4;
    let mut b = Vec::new();
    b.extend_from_slice(b"FORM");
    b.extend_from_slice(&(4 + 26 + 16 + data).to_be_bytes());
    b.extend_from_slice(b"AIFFCOMM");
    b.extend_from_slice(&18u32.to_be_bytes());
    b.extend_from_slice(&2u16.to_be_bytes());
    b.extend_from_slice(&declared.to_be_bytes());
    b.extend_from_slice(&16u16.to_be_bytes());
    // 44 100 as an 80-bit extended float.
    b.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
    b.extend_from_slice(b"SSND");
    b.extend_from_slice(&(8 + data).to_be_bytes());
    b.extend_from_slice(&[0; 8]);
    b.resize(b.len() + data as usize, 0);
    b
}

/// `sine.webm` whose last `DiscardPadding` says 2.1 s (more than the whole file).
fn webm_with_huge_padding() -> Vec<u8> {
    let mut bytes = read("sine.webm");
    let at = bytes
        .windows(3)
        .rposition(|w| w == [0x75, 0xA2, 0x84])
        .expect("sine.webm ends with a DiscardPadding of 4 bytes");
    bytes[at + 3..at + 7].copy_from_slice(&i32::MAX.to_be_bytes());
    bytes
}

// --- Decoding with a cap -------------------------------------------------------------------

struct Memory(Cursor<Vec<u8>>, u64);

impl std::io::Read for Memory {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Seek for Memory {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl Source for Memory {
    fn byte_len(&self) -> u64 {
        self.1
    }
}

/// What a capped decode of `bytes` gave, and the bytes of samples handed out (what a host
/// would have written to the cache) before it ended.
struct Run {
    result: Result<MediaInfo, ImportError>,
    handed: u64,
    /// Whether the refusal came at the opening, before any block.
    at_open: bool,
}

fn decode_capped(bytes: Vec<u8>, hint: Option<&str>, cap: u64) -> Run {
    let len = bytes.len() as u64;
    let mut handed = 0u64;
    let mut decoding =
        match Decoding::open_capped(Box::new(Memory(Cursor::new(bytes), len)), hint, cap) {
            Ok(d) => d,
            Err(e) => {
                return Run {
                    result: Err(e),
                    handed,
                    at_open: true,
                };
            }
        };
    let mut block = Vec::new();
    let result = loop {
        match decoding.next(&mut block) {
            Ok(true) => handed += block.len() as u64 * 4,
            Ok(false) => break decoding.finish(),
            Err(e) => break Err(e),
        }
    };
    Run {
        result,
        handed,
        at_open: false,
    }
}

// --- The tests -----------------------------------------------------------------------------

#[test]
fn the_crafted_flac_decodes_to_its_silence() {
    for (channels, frames) in [(1u8, 100u64), (2, 65_535), (2, 200_000), (6, 70_001)] {
        for declare in [true, false] {
            let (imported, pcm) = import(silent_flac(channels, frames, declare), Some("flac"))
                .unwrap_or_else(|e| panic!("{channels} ch, {frames} frames: {e:?}"));
            assert_eq!(imported.info.frames, frames);
            assert_eq!(imported.info.channels, u16::from(channels));
            assert_eq!(pcm.len() as u64, frames * u64::from(channels));
            assert!(pcm.iter().all(|&s| s == 0.0));
        }
    }
    // About 35 000 to 1: a megabyte of it would be 35 GB of cache.
    let flac = silent_flac(2, 100 * FLAC_BLOCK, false);
    let ratio = (100 * FLAC_BLOCK * 2 * 4) as f64 / flac.len() as f64;
    assert!(ratio > 25_000.0, "{ratio}");
}

#[test]
fn the_cap_is_the_projects_and_an_hour_of_common_formats_fits() {
    assert_eq!(MAX_DECODED_BYTES, 16 << 30);
    let hour = |rate: u64, channels: u64| rate * 3600 * channels * 4;
    for (rate, channels) in [
        (48_000, 2),
        (96_000, 2),
        (192_000, 2),
        (96_000, 6),
        (96_000, 8),
    ] {
        assert!(
            hour(rate, channels) <= MAX_DECODED_BYTES,
            "{rate} Hz, {channels} ch"
        );
    }
    // An hour of 16 channels at 96 kHz does not.
    assert!(hour(96_000, 16) > MAX_DECODED_BYTES);
    // A WAV or AIFF read directly has 32-bit chunk sizes and samples of 16 bits or more: its
    // decoding stays under the cap, which never refuses a file that writes no cache.
    assert!((u64::from(u32::MAX) / 2) * 4 < MAX_DECODED_BYTES);
}

#[test]
fn a_header_that_declares_more_than_the_cap_is_refused_before_any_packet() {
    // FLAC's STREAMINFO declaring 2^36 - 1 frames of 8 channels (2 TiB), with 2 blocks in it.
    let mut flac = silent_flac(8, 2 * FLAC_BLOCK, true);
    flac[18..26].copy_from_slice(
        &((48_000u64 << 44) | (7u64 << 41) | (15u64 << 36) | ((1 << 36) - 1)).to_be_bytes(),
    );
    let run = decode_capped(flac, Some("flac"), MAX_DECODED_BYTES);
    assert_eq!(run.result, Err(ImportError::TooLarge(MAX_DECODED_BYTES)));
    assert!(run.at_open);
    // An honest FLAC of 16 GiB and one frame more: declared, refused at the opening (its 24 KB
    // are never decoded).
    let frames = MAX_DECODED_BYTES / 8 + 1;
    let flac = silent_flac(2, frames, true);
    assert!(flac.len() < 1 << 20, "{}", flac.len());
    let run = decode_capped(flac, Some("flac"), MAX_DECODED_BYTES);
    assert_eq!(run.result, Err(ImportError::TooLarge(MAX_DECODED_BYTES)));
    assert!(run.at_open);
    assert_eq!(run.handed, 0);
    // A WAV whose header lies about its length: a `fact` chunk of u32::MAX frames of stereo
    // ADPCM (34 GB of `f32`) in a file of 3 blocks.
    let run = decode_capped(adpcm_wav(3, Some(u32::MAX)), Some("wav"), MAX_DECODED_BYTES);
    assert_eq!(run.result, Err(ImportError::TooLarge(MAX_DECODED_BYTES)));
    assert!(run.at_open);
    // The same WAV, honest, imports.
    let (imported, _) = import(adpcm_wav(3, None), Some("wav")).unwrap();
    assert_eq!(imported.info.frames, 3 * 2041);
    // Through the whole pipeline too, with the error's text.
    let error = import(adpcm_wav(3, Some(u32::MAX)), Some("wav")).unwrap_err();
    assert_eq!(error.code(), "too_large");
    assert_eq!(
        error.to_string(),
        "the file decodes to more than 16 GiB of audio, the most a project keeps of one file"
    );
}

#[test]
fn plain_pcm_that_overstates_its_length_still_imports_and_is_read_directly() {
    // An AIFF whose COMM says u32::MAX frames of stereo (more than 16 GiB of `f32`) but holds
    // 1 000: decoded for what it holds, read directly, never refused by its header.
    let (imported, pcm) = import(aiff(1_000, u32::MAX), Some("aiff")).unwrap();
    assert_eq!(imported.info.frames, 1_000);
    assert_eq!(pcm.len(), 2_000);
    assert!(imported.layout.is_some(), "read directly, no cache");
}

#[test]
fn a_file_that_declares_nothing_is_refused_by_the_block_that_would_pass_the_cap() {
    const CAP: u64 = 16 << 20;
    // 64 MiB and 256 MiB of silence, no length declared: the cap stops both at the same point,
    // the host never receives more than the cap, and the heap of the decode is the same small
    // amount, whatever the file would decode to.
    let mut peaks = Vec::new();
    for blocks in [64u64, 256] {
        let flac = silent_flac(2, blocks * FLAC_BLOCK * 2, false);
        let (run, peak) = peak_of(|| decode_capped(flac, Some("flac"), CAP));
        assert_eq!(run.result, Err(ImportError::TooLarge(CAP)));
        assert!(!run.at_open);
        assert!(run.handed <= CAP, "{} > {CAP}", run.handed);
        // Within one block of the cap: decoding ran up to it.
        assert!(run.handed > CAP - FLAC_BLOCK * 2 * 4, "{}", run.handed);
        assert!(peak < 8 << 20, "the decode held {peak} bytes");
        peaks.push(peak);
    }
    println!("peak heap of a capped decode: {peaks:?} bytes");
    // The text says the cap in effect.
    assert_eq!(
        ImportError::TooLarge(CAP).to_string(),
        "the file decodes to more than 16 MiB of audio, the most a project keeps of one file"
    );
    // Under the cap, the same file decodes whole.
    let flac = silent_flac(2, 8 * FLAC_BLOCK, false);
    let run = decode_capped(flac, Some("flac"), CAP);
    assert_eq!(run.result.map(|i| i.frames), Ok(8 * FLAC_BLOCK));
    // A cap above the limit is the limit.
    let flac = silent_flac(2, MAX_DECODED_BYTES / 8 + 1, true);
    let run = decode_capped(flac, Some("flac"), u64::MAX);
    assert_eq!(run.result, Err(ImportError::TooLarge(MAX_DECODED_BYTES)));
}

#[test]
fn a_forged_end_padding_hides_no_frame_from_the_cap() {
    // `sine.webm` (24 000 frames of stereo, 192 000 bytes of `f32`) whose last DiscardPadding
    // says more than the whole file: every block still counts towards the cap.
    let forged = webm_with_huge_padding();
    let run = decode_capped(forged.clone(), Some("webm"), 100_000);
    assert_eq!(run.result, Err(ImportError::TooLarge(100_000)));
    assert!(run.handed <= 100_000);
    // Without a cap, the padding cuts one packet at most from the end (120 ms).
    let (honest, _) = import(read("sine.webm"), Some("webm")).unwrap();
    let (imported, _) = import(forged, Some("webm")).unwrap();
    assert!(imported.info.frames < honest.info.frames);
    assert!(honest.info.frames - imported.info.frames <= 5_760);
}

/// Measured, not run by default (`cargo test --release -p audio-import --test it -- --ignored
/// --nocapture size::`): a FLAC of silence that declares nothing and holds more than 16 GiB of
/// `f32` is refused at the cap; the time and the heap it took.
#[test]
#[ignore = "decodes 16 GiB: run in release to measure"]
fn measure_a_flac_past_the_real_cap() {
    let frames = MAX_DECODED_BYTES / 8 + 10 * FLAC_BLOCK;
    let flac = silent_flac(2, frames, false);
    let size = flac.len();
    let start = Instant::now();
    let (run, peak) = peak_of(|| decode_capped(flac, Some("flac"), MAX_DECODED_BYTES));
    assert_eq!(run.result, Err(ImportError::TooLarge(MAX_DECODED_BYTES)));
    assert!(run.handed <= MAX_DECODED_BYTES);
    println!(
        "past the cap: a FLAC of {size} bytes refused after {} bytes handed, in {:?}, peak heap {peak} bytes",
        run.handed,
        start.elapsed()
    );
}

/// Measured, not run by default: an hour of stereo 48 kHz (1.38 GB of `f32`) imports through
/// the whole pipeline (hash, decode, peaks); the time and the heap it took.
#[test]
#[ignore = "decodes an hour: run in release to measure"]
fn measure_an_hour_imports() {
    let frames = 48_000 * 3600;
    let flac = silent_flac(2, frames, true);
    let size = flac.len();
    let start = Instant::now();
    let (result, peak) = peak_of(|| {
        let mut handed = 0u64;
        let imported = audio_import::import(Cursor::new(flac), Some("flac"), |block| {
            handed += block.len() as u64 * 4
        });
        (imported, handed)
    });
    let (imported, handed) = result;
    let imported = imported.unwrap();
    assert_eq!(imported.info.frames, frames);
    assert_eq!(handed, frames * 2 * 4);
    println!(
        "an hour: a FLAC of {size} bytes, {handed} bytes of samples in {:?}, peak heap {peak} bytes",
        start.elapsed()
    );
}
