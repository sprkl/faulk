//! The audio import pipeline (`crates/audio-import`, AU-1) as a WebAssembly module, run in
//! the page's project store Worker (`web/src/lib/audio/import/`).
//!
//! Plain C ABI (`ai_*`), no `wasm-bindgen`, like `engine-wasm` and `project-wasm`. One job
//! at a time per instance, in steps, so the Worker gets its event loop back between them:
//! it writes what a step produced with the asynchronous file APIs, reports progress, and
//! hears a cancel.
//!
//! - **Hash**: the Worker reads the file in slices into the inbox ([`ai_inbox`]) and calls
//!   [`ai_hash_push`]; [`ai_hash_end`] gives the SHA-256.
//! - **Decode**: [`ai_open`] reads the headers through the host's one import,
//!   `env.ai_read(offset, len, dst)`, which the Worker serves synchronously with
//!   `FileReaderSync` (so the file is read by ranges and never held whole); each
//!   [`ai_next`] decodes one packet into the block ([`ai_block`], interleaved `f32`, which
//!   in the little-endian wasm memory are already the bytes of the PCM cache);
//!   [`ai_finish`] checks and gives the metadata ([`ai_rate`], [`ai_channels`],
//!   [`ai_frames`]).
//! - **Layout and cache**: [`ai_layout`] says whether the file is read directly (no cache);
//!   [`ai_cache_header`] and [`ai_cache_valid`] write and check the cache's header.
//!
//! A failure returns a negative code (`-ImportError` code, below) and leaves its text at
//! [`ai_error`]. A panic aborts the instance (a trap in the Worker): the Worker reports the
//! file as damaged and makes a new instance. Natively (tests) the reader is a buffer set by
//! [`set_source`].

use audio_import::{Decoding, ImportError, MediaInfo, PcmLayout, Sha256, Source, cache};
use std::cell::RefCell;
use std::io::{Read, Seek, SeekFrom};

/// [`ai_next`]: a block is ready (it may hold no frame).
pub const BLOCK: i32 = 1;
/// [`ai_next`]: the end of the stream.
pub const END: i32 = 0;
/// No job: [`ai_next`] or [`ai_finish`] without [`ai_open`].
pub const ERR_NO_JOB: i32 = -100;
/// The input is not what the call takes (metadata that is no file's).
pub const ERR_BAD_INPUT: i32 = -101;

/// The negative code of an import error (its `code()` in this order).
pub fn error_code(error: &ImportError) -> i32 {
    -match error {
        ImportError::UnsupportedFormat => 1,
        ImportError::UnsupportedCodec(_) => 2,
        ImportError::Corrupt(_) => 3,
        ImportError::Empty => 4,
        ImportError::BadSampleRate(_) => 5,
        ImportError::BadChannels(_) => 6,
        ImportError::TooLong => 7,
        ImportError::Io(_) => 8,
    }
}

struct State {
    inbox: Vec<u8>,
    hash: Option<Sha256>,
    digest: [u8; 32],
    job: Option<Decoding>,
    info: Option<MediaInfo>,
    block: Vec<f32>,
    error: String,
    header: [u8; cache::HEADER_BYTES],
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State {
        inbox: Vec::new(),
        hash: None,
        digest: [0; 32],
        job: None,
        info: None,
        block: Vec::new(),
        error: String::new(),
        header: [0; cache::HEADER_BYTES],
    }) };
    /// The furthest byte the decoder has read: the progress of a decode.
    static POSITION: RefCell<u64> = const { RefCell::new(0) };
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Reads up to `len` bytes of the file at `offset` into `dst`: the count read (0 at the
    /// end), or a negative number when the file cannot be read.
    fn ai_read(offset: f64, len: u32, dst: *mut u8) -> i32;
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static NATIVE_SOURCE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Natively, the bytes [`ai_open`] reads (what `env.ai_read` serves in the page).
#[cfg(not(target_arch = "wasm32"))]
pub fn set_source(bytes: Vec<u8>) {
    NATIVE_SOURCE.set(bytes);
}

#[cfg(not(target_arch = "wasm32"))]
fn host_read(offset: u64, buf: &mut [u8]) -> i32 {
    NATIVE_SOURCE.with_borrow(|bytes| {
        let start = (offset as usize).min(bytes.len());
        let n = buf.len().min(bytes.len() - start);
        buf[..n].copy_from_slice(&bytes[start..start + n]);
        n as i32
    })
}

#[cfg(target_arch = "wasm32")]
fn host_read(offset: u64, buf: &mut [u8]) -> i32 {
    let len = buf.len().min(i32::MAX as usize) as u32;
    // SAFETY: `buf` is a live, writable slice of this module's memory of at least `len`
    // bytes; the host writes at most `len` bytes at `dst` and keeps no reference to it.
    unsafe { ai_read(offset as f64, len, buf.as_mut_ptr()) }
}

/// The file, read through the host.
struct HostFile {
    position: u64,
    len: u64,
}

impl Read for HostFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.position >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let want = buf.len().min((self.len - self.position) as usize);
        let n = host_read(self.position, &mut buf[..want]);
        if n < 0 {
            return Err(std::io::Error::other("the file could not be read"));
        }
        self.position += n as u64;
        POSITION.with_borrow_mut(|p| *p = (*p).max(self.position));
        Ok(n as usize)
    }
}

impl Seek for HostFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let to = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.position.checked_add_signed(d),
        };
        let to = to.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before the start")
        })?;
        self.position = to;
        Ok(to)
    }
}

impl Source for HostFile {
    fn byte_len(&self) -> u64 {
        self.len
    }
}

fn fail(state: &mut State, error: &ImportError) -> i32 {
    state.error = error.to_string();
    error_code(error)
}

fn file_len(lo: u32, hi: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

/// The address of an inbox of at least `len` bytes (it may move between calls); null when
/// the memory is refused.
#[unsafe(no_mangle)]
pub extern "C" fn ai_inbox(len: u32) -> *mut u8 {
    STATE.with_borrow_mut(|s| {
        let len = len as usize;
        if len > s.inbox.len() {
            if s.inbox.try_reserve_exact(len - s.inbox.len()).is_err() {
                return core::ptr::null_mut();
            }
            s.inbox.resize(len, 0);
        }
        s.inbox.as_mut_ptr()
    })
}

/// Starts a hash.
#[unsafe(no_mangle)]
pub extern "C" fn ai_hash_begin() {
    STATE.with_borrow_mut(|s| s.hash = Some(Sha256::new()));
}

/// Hashes the first `len` bytes of the inbox. [`ERR_NO_JOB`] without [`ai_hash_begin`].
#[unsafe(no_mangle)]
pub extern "C" fn ai_hash_push(len: u32) -> i32 {
    STATE.with_borrow_mut(|s| {
        let State { inbox, hash, .. } = s;
        match hash {
            Some(h) => {
                h.update(&inbox[..(len as usize).min(inbox.len())]);
                0
            }
            None => ERR_NO_JOB,
        }
    })
}

/// Ends the hash: the address of its 32 bytes, null without [`ai_hash_begin`].
#[unsafe(no_mangle)]
pub extern "C" fn ai_hash_end() -> *const u8 {
    STATE.with_borrow_mut(|s| match s.hash.take() {
        Some(h) => {
            s.digest = h.finish().0;
            s.digest.as_ptr()
        }
        None => core::ptr::null(),
    })
}

/// Opens the file of `len_hi:len_lo` bytes for decoding; the inbox's first `hint_len` bytes
/// are its extension (UTF-8, may be empty). 0, or a negative error code.
#[unsafe(no_mangle)]
pub extern "C" fn ai_open(len_lo: u32, len_hi: u32, hint_len: u32) -> i32 {
    STATE.with_borrow_mut(|s| {
        s.job = None;
        s.info = None;
        s.block.clear();
        POSITION.set(0);
        let hint = std::str::from_utf8(&s.inbox[..(hint_len as usize).min(s.inbox.len())])
            .ok()
            .filter(|h| !h.is_empty())
            .map(str::to_owned);
        let file = HostFile {
            position: 0,
            len: file_len(len_lo, len_hi),
        };
        match Decoding::open(Box::new(file), hint.as_deref()) {
            Ok(job) => {
                s.job = Some(job);
                0
            }
            Err(e) => fail(s, &e),
        }
    })
}

/// Decodes the next packet into the block: [`BLOCK`], [`END`], or a negative error code.
#[unsafe(no_mangle)]
pub extern "C" fn ai_next() -> i32 {
    STATE.with_borrow_mut(|s| {
        let State { job, block, .. } = s;
        let Some(job) = job.as_mut() else {
            return ERR_NO_JOB;
        };
        match job.next(block) {
            Ok(true) => BLOCK,
            Ok(false) => END,
            Err(e) => {
                s.job = None;
                fail(s, &e)
            }
        }
    })
}

/// The address of the block's samples (interleaved `f32`).
#[unsafe(no_mangle)]
pub extern "C" fn ai_block() -> *const f32 {
    STATE.with_borrow(|s| s.block.as_ptr())
}

/// How many samples the block holds (frames × channels).
#[unsafe(no_mangle)]
pub extern "C" fn ai_block_len() -> u32 {
    STATE.with_borrow(|s| s.block.len() as u32)
}

/// The furthest byte of the file the decoder has read.
#[unsafe(no_mangle)]
pub extern "C" fn ai_position() -> f64 {
    POSITION.with_borrow(|p| *p as f64)
}

/// Ends the decoding: 0 and the metadata at [`ai_rate`], [`ai_channels`], [`ai_frames`], or
/// a negative error code.
#[unsafe(no_mangle)]
pub extern "C" fn ai_finish() -> i32 {
    STATE.with_borrow_mut(|s| {
        let Some(job) = s.job.take() else {
            return ERR_NO_JOB;
        };
        match job.finish() {
            Ok(info) => {
                s.info = Some(info);
                0
            }
            Err(e) => fail(s, &e),
        }
    })
}

/// The sample rate of the decoded file (or of the open job's headers, 0 when unknown).
#[unsafe(no_mangle)]
pub extern "C" fn ai_rate() -> u32 {
    STATE.with_borrow(|s| {
        s.info
            .map(|i| i.sample_rate)
            .or(s.job.as_ref().map(Decoding::sample_rate))
            .unwrap_or(0)
    })
}

/// The channels of the decoded file (or of the open job's headers, 0 when unknown).
#[unsafe(no_mangle)]
pub extern "C" fn ai_channels() -> u32 {
    STATE.with_borrow(|s| {
        u32::from(
            s.info
                .map(|i| i.channels)
                .or(s.job.as_ref().map(Decoding::channels))
                .unwrap_or(0),
        )
    })
}

/// Frames of the decoded file (or decoded so far by the open job).
#[unsafe(no_mangle)]
pub extern "C" fn ai_frames() -> f64 {
    STATE.with_borrow(|s| {
        s.info
            .map(|i| i.frames)
            .or(s.job.as_ref().map(Decoding::frames))
            .unwrap_or(0) as f64
    })
}

/// Whether a file of this metadata (as decoding gave it) is read directly (1: no PCM cache)
/// or not (0), from its first `head_len` bytes in the inbox and its length;
/// [`ERR_BAD_INPUT`] for metadata that is no file's.
#[unsafe(no_mangle)]
pub extern "C" fn ai_layout(
    head_len: u32,
    len_lo: u32,
    len_hi: u32,
    rate: u32,
    channels: u32,
    frames: f64,
) -> i32 {
    STATE.with_borrow(|s| {
        let Some(info) = info_of(rate, channels, frames) else {
            return ERR_BAD_INPUT;
        };
        let head = &s.inbox[..(head_len as usize).min(s.inbox.len())];
        i32::from(
            PcmLayout::parse(head, file_len(len_lo, len_hi)).is_some_and(|l| l.matches(&info)),
        )
    })
}

/// Whether a file whose first `head_len` bytes are in the inbox, of this length, looks read
/// directly (1) or not (0), before decoding: a WAV or AIFF of plain samples. The page then
/// decodes it without writing a cache, and checks with [`ai_layout`] afterwards.
#[unsafe(no_mangle)]
pub extern "C" fn ai_layout_candidate(head_len: u32, len_lo: u32, len_hi: u32) -> i32 {
    STATE.with_borrow(|s| {
        let head = &s.inbox[..(head_len as usize).min(s.inbox.len())];
        i32::from(PcmLayout::parse(head, file_len(len_lo, len_hi)).is_some())
    })
}

fn info_of(rate: u32, channels: u32, frames: f64) -> Option<MediaInfo> {
    Some(MediaInfo {
        sample_rate: rate,
        channels: u16::try_from(channels).ok()?,
        frames: (frames >= 0.0 && frames.fract() == 0.0 && frames <= 9_007_199_254_740_992.0)
            .then_some(frames as u64)?,
    })
}

/// The address of the 32-byte header of the PCM cache of such a file; null for metadata
/// that is not one.
#[unsafe(no_mangle)]
pub extern "C" fn ai_cache_header(rate: u32, channels: u32, frames: f64) -> *const u8 {
    STATE.with_borrow_mut(|s| match info_of(rate, channels, frames) {
        Some(info) => {
            s.header = cache::header(&info);
            s.header.as_ptr()
        }
        None => core::ptr::null(),
    })
}

/// Whether a cache file whose first `head_len` bytes are in the inbox, of `file_len` bytes,
/// is the whole cache of such a file (1) or not (0).
#[unsafe(no_mangle)]
pub extern "C" fn ai_cache_valid(
    head_len: u32,
    file_len: f64,
    rate: u32,
    channels: u32,
    frames: f64,
) -> i32 {
    STATE.with_borrow(|s| {
        let head = &s.inbox[..(head_len as usize).min(s.inbox.len())];
        let ok = info_of(rate, channels, frames)
            .is_some_and(|info| file_len >= 0.0 && cache::is_valid(head, file_len as u64, &info));
        i32::from(ok)
    })
}

/// The address of the last failure's text ([`ai_error_len`] bytes, UTF-8).
#[unsafe(no_mangle)]
pub extern "C" fn ai_error() -> *const u8 {
    STATE.with_borrow(|s| s.error.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn ai_error_len() -> u32 {
    STATE.with_borrow(|s| s.error.len() as u32)
}
