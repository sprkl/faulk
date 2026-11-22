//! The C ABI the page's Worker drives, run natively (the reader is a buffer instead of the
//! Worker's `FileReaderSync`): hash, decode in steps, layout, the cache header, the errors,
//! and that a failed job leaves the next one working. `web/src/lib/audio/import/` runs the
//! same steps on the WebAssembly build.

use audio_import_wasm::*;
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../audio-import/tests/fixtures")
        .join(name);
    std::fs::read(path).unwrap()
}

fn to_inbox(bytes: &[u8]) {
    let ptr = ai_inbox(bytes.len() as u32);
    assert!(!ptr.is_null());
    // SAFETY: the inbox holds at least `bytes.len()` bytes.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
}

fn hash(bytes: &[u8]) -> String {
    ai_hash_begin();
    for chunk in bytes.chunks(1000) {
        to_inbox(chunk);
        assert_eq!(ai_hash_push(chunk.len() as u32), 0);
    }
    let ptr = ai_hash_end();
    assert!(!ptr.is_null());
    // SAFETY: `ai_hash_end` returns the address of 32 bytes.
    let digest = unsafe { core::slice::from_raw_parts(ptr, 32) };
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes the whole file through the steps: the samples, or the error code.
fn decode(bytes: &[u8], hint: &str) -> Result<Vec<f32>, i32> {
    set_source(bytes.to_vec());
    to_inbox(hint.as_bytes());
    let len = bytes.len() as u64;
    let status = ai_open(len as u32, (len >> 32) as u32, hint.len() as u32);
    if status != 0 {
        return Err(status);
    }
    // The reads of the opening (up to the end of the file for some) are not progress.
    assert_eq!(ai_position(), 0.0, "the progress starts with the decoding");
    let mut pcm = Vec::new();
    loop {
        match ai_next() {
            BLOCK => {
                // SAFETY: the block holds `ai_block_len` samples until the next call.
                let block =
                    unsafe { core::slice::from_raw_parts(ai_block(), ai_block_len() as usize) };
                pcm.extend_from_slice(block);
            }
            END => break,
            error => return Err(error),
        }
    }
    assert_eq!(ai_position(), len as f64, "the whole file was read");
    match ai_finish() {
        0 => Ok(pcm),
        error => Err(error),
    }
}

fn error_text() -> String {
    // SAFETY: `ai_error` holds `ai_error_len` bytes of UTF-8.
    let bytes = unsafe { core::slice::from_raw_parts(ai_error(), ai_error_len() as usize) };
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn the_steps_give_the_hash_the_samples_and_the_metadata_of_the_library() {
    for (name, hint) in [
        ("sine-s16.wav", "wav"),
        ("sine.mp3", "mp3"),
        ("sine.m4a", "m4a"),
        ("sine-48k-mono.flac", "flac"),
        ("sine.opus", "opus"),
        ("sine.webm", "webm"),
    ] {
        let bytes = fixture(name);
        let mut expected = Vec::new();
        let imported = audio_import::import(std::io::Cursor::new(bytes.clone()), Some(hint), |b| {
            expected.extend_from_slice(b)
        })
        .unwrap();
        assert_eq!(hash(&bytes), imported.hash.to_string(), "{name}");
        let pcm = decode(&bytes, hint).unwrap();
        assert!(pcm == expected, "{name}");
        assert_eq!(ai_rate(), imported.info.sample_rate, "{name}");
        assert_eq!(ai_channels(), u32::from(imported.info.channels), "{name}");
        assert_eq!(ai_frames(), imported.info.frames as f64, "{name}");
        let head = &bytes[..bytes.len().min(64 * 1024)];
        to_inbox(head);
        let len = bytes.len() as u64;
        assert_eq!(
            ai_layout(
                head.len() as u32,
                len as u32,
                (len >> 32) as u32,
                ai_rate(),
                ai_channels(),
                ai_frames()
            ),
            i32::from(imported.layout.is_some()),
            "{name}"
        );
    }
}

#[test]
fn a_failure_is_a_code_and_a_text_and_the_next_job_works() {
    assert_eq!(decode(&fixture("not-audio.txt"), "txt"), Err(-1));
    assert!(!error_text().is_empty());
    // An Opus stream whose channels Faulk does not model (mapping family 3, ambisonics).
    let mut ambisonic = fixture("sine.webm");
    let head = ambisonic.windows(8).position(|w| w == b"OpusHead").unwrap();
    ambisonic[head + 18] = 3;
    assert_eq!(decode(&ambisonic, "webm"), Err(-2));
    assert!(error_text().contains("Opus"), "{}", error_text());
    let wav = fixture("sine-s16.wav");
    assert_eq!(
        decode(&wav[..wav.len() / 2 + 1], "wav").map(|_| ()),
        Err(-3)
    );
    assert_eq!(decode(&[], "wav"), Err(-4));
    // Without a job, the steps say so.
    assert_eq!(ai_next(), ERR_NO_JOB);
    assert_eq!(ai_finish(), ERR_NO_JOB);
    assert_eq!(ai_hash_push(0), ERR_NO_JOB);
    assert!(ai_hash_end().is_null());
    // And a job after the failures decodes as ever.
    assert!(decode(&wav, "wav").is_ok());
}

#[test]
fn the_decoded_size_is_capped_and_a_lower_cap_holds_for_one_job_only() {
    assert_eq!(ai_max_decoded_bytes(), (16u64 << 30) as f64);
    assert_eq!(
        ai_max_decoded_bytes(),
        audio_import::MAX_DECODED_BYTES as f64
    );
    // `sine.mp3` decodes to 176 400 bytes of `f32` and declares no length: refused by the block
    // that would pass a cap of 100 000, with the cap in its text.
    let mp3 = fixture("sine.mp3");
    ai_limit(100_000.0);
    assert_eq!(decode(&mp3, "mp3").map(|_| ()), Err(-9));
    assert_eq!(
        error_text(),
        "the file decodes to more than 100000 bytes of audio, the most a project keeps of one file"
    );
    // The next job has the limit again.
    assert!(decode(&mp3, "mp3").is_ok());
    // `sine.flac` declares its length: refused at the opening.
    let flac = fixture("sine.flac");
    ai_limit(1_000.0);
    assert_eq!(decode(&flac, "flac").map(|_| ()), Err(-9));
    assert!(decode(&flac, "flac").is_ok());
    // A failed opening takes the lower cap too: the job after it has the limit.
    ai_limit(1_000.0);
    assert_eq!(decode(&fixture("not-audio.txt"), "txt"), Err(-1));
    assert!(decode(&mp3, "mp3").is_ok());
    // A cap that is no number of bytes is the limit; 0 refuses any sample.
    for cap in [f64::NAN, -1.0, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
        ai_limit(cap);
        assert!(decode(&mp3, "mp3").is_ok(), "{cap}");
    }
    ai_limit(0.0);
    assert_eq!(decode(&mp3, "mp3").map(|_| ()), Err(-9));
}

#[test]
fn the_cache_header_and_its_check_are_the_librarys() {
    let info = audio_import::MediaInfo {
        sample_rate: 44_100,
        channels: 2,
        frames: 22_050,
    };
    let ptr = ai_cache_header(44_100, 2, 22_050.0);
    assert!(!ptr.is_null());
    // SAFETY: `ai_cache_header` returns the address of the 32-byte header.
    let header = unsafe { core::slice::from_raw_parts(ptr, 32) }.to_vec();
    assert_eq!(header, audio_import::cache::header(&info));
    let size = audio_import::cache::size(&info).unwrap() as f64;
    to_inbox(&header);
    assert_eq!(ai_cache_valid(32, size, 44_100, 2, 22_050.0), 1);
    assert_eq!(ai_cache_valid(32, size - 4.0, 44_100, 2, 22_050.0), 0);
    assert_eq!(ai_cache_valid(31, size, 44_100, 2, 22_050.0), 0);
    assert_eq!(ai_cache_valid(32, size, 48_000, 2, 22_050.0), 0);
    // Metadata that is no file's: no header.
    assert!(ai_cache_header(44_100, 70_000, 1.0).is_null());
    assert!(ai_cache_header(44_100, 2, 0.5).is_null());
    assert!(ai_cache_header(44_100, 2, -1.0).is_null());
    assert_eq!(ai_layout(0, 0, 0, 44_100, 2, 0.5), ERR_BAD_INPUT);
}

fn peaks_file() -> Vec<u8> {
    // SAFETY: `ai_peaks` holds `ai_peaks_len` bytes until the next job.
    unsafe { core::slice::from_raw_parts(ai_peaks(), ai_peaks_len() as usize) }.to_vec()
}

#[test]
fn the_steps_give_the_peaks_of_the_library_and_a_failed_job_none() {
    for (name, hint) in [
        ("sine-s16.wav", "wav"),
        ("sine.mp3", "mp3"),
        ("sine.m4a", "m4a"),
        ("sine.ogg", "ogg"),
        ("sine-48k-mono.flac", "flac"),
    ] {
        let bytes = fixture(name);
        let imported =
            audio_import::import(std::io::Cursor::new(bytes.clone()), Some(hint), |_| ()).unwrap();
        decode(&bytes, hint).unwrap();
        let file = peaks_file();
        assert_eq!(Some(&file), imported.peaks.as_ref(), "{name}");
        let (rate, channels, frames) = (ai_rate(), ai_channels(), ai_frames());
        let len = file.len() as f64;
        to_inbox(&file[..32]);
        assert_eq!(ai_peaks_valid(32, len, rate, channels, frames), 1, "{name}");
        assert_eq!(ai_peaks_valid(32, len - 1.0, rate, channels, frames), 0);
        assert_eq!(ai_peaks_valid(31, len, rate, channels, frames), 0);
        assert_eq!(ai_peaks_valid(32, len, rate, channels, 1.5), 0);
        assert_eq!(ai_peaks_valid(32, len, rate + 1, channels, frames), 0);
    }
    // A job that fails leaves no peaks of the job before it.
    let wav = fixture("sine-s16.wav");
    assert!(decode(&wav, "wav").is_ok());
    assert!(ai_peaks_len() > 0);
    assert_eq!(
        decode(&wav[..wav.len() / 2 + 1], "wav").map(|_| ()),
        Err(-3)
    );
    assert_eq!(ai_peaks_len(), 0);
    assert!(decode(&wav, "wav").is_ok());
    assert_eq!(decode(&fixture("not-audio.txt"), "txt"), Err(-1));
    assert_eq!(ai_peaks_len(), 0);
}
