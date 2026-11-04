//! The `.faulkpack` export of a project with a large audio blob (#617): the page reads the blob
//! in chunks and the session writes the pack as they come, so what the export holds does not
//! depend on the size of the blob. Generated data, nothing committed.
//!
//! This binary counts every byte its allocator holds (the data generator and the checks
//! included, which is why they work in chunks too) and asserts that the peak stays far under
//! the size of the blob. The pack is written to a temporary file and read back by the zip
//! reader, which checks the blob's CRC and length. Peak memory and time are printed:
//! `cargo test -p project-wasm --release --test pack_stream -- --nocapture`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use project_wasm::{pw_inbox, pw_out, pw_pack_begin, pw_pack_feed, pw_pack_next};

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards every call to the system allocator and only counts the bytes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded from the caller's contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: forwarded from the caller's contract.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded from the caller's contract.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            if new_size >= layout.size() {
                let now = CURRENT.fetch_add(new_size - layout.size(), Ordering::Relaxed)
                    + (new_size - layout.size());
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const BLOB_BYTES: u64 = 600 << 20;
const CHUNK: usize = 4 << 20;

/// The `index`th chunk of the generated blob: a noisy but compressible signal, the same every time.
fn chunk_at(index: u64, out: &mut Vec<u8>, size: usize) {
    out.clear();
    let mut state = 0x9E37_79B9_7F4A_7C15u64 ^ index.wrapping_mul(0xD1B5_4A32_D192_ED03);
    out.extend((0..size).map(|i| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        // Mostly a slow ramp, a little noise: like audio, it deflates somewhat.
        ((i / 8) as u8).wrapping_add((state & 0x1f) as u8)
    }));
}

fn put_inbox(bytes: &[u8]) {
    let ptr = pw_inbox(bytes.len() as u32);
    assert!(!ptr.is_null());
    // SAFETY: the inbox holds at least `bytes.len()` bytes.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
}

fn out_of(len: i32) -> &'static [u8] {
    assert!(len > 0, "error code {len}");
    // SAFETY: `pw_out` holds the `len` bytes of the last call, valid until the next one.
    unsafe { core::slice::from_raw_parts(pw_out(), len as usize) }
}

fn folder_frame(files: &[(&str, &[u8])], listed: &[(&str, u64)]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend(4u32.to_le_bytes());
    frame.extend(b"pack");
    for (path, bytes) in files {
        frame.extend((path.len() as u32).to_le_bytes());
        frame.extend(path.as_bytes());
        frame.push(0);
        frame.extend((bytes.len() as u64).to_le_bytes());
        frame.extend(*bytes);
    }
    for (path, size) in listed {
        frame.extend((path.len() as u32).to_le_bytes());
        frame.extend(path.as_bytes());
        frame.push(1);
        frame.extend(size.to_le_bytes());
    }
    frame
}

/// The folder of the format's reference song, with its audio blob: `(files, blob name)`.
fn song() -> (Vec<(String, Vec<u8>)>, String) {
    fn walk(dir: &std::path::Path, base: &str, files: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            let path = if base.is_empty() {
                name
            } else {
                format!("{base}/{name}")
            };
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), &path, files);
            } else {
                files.push((path, std::fs::read(entry.path()).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../project/tests/reference/song");
    walk(&dir, "", &mut files);
    // The song names the audio blob `ab` * 32 and its folder lacks it.
    (files, format!("blobs/{}", "ab".repeat(32)))
}

#[test]
fn a_pack_with_a_600_mib_blob_is_written_in_bounded_memory() {
    let (files, blob) = song();
    let owned: Vec<(&str, &[u8])> = files
        .iter()
        // Blobs of the song other than the audio one are small: they travel whole.
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let frame = folder_frame(&owned, &[(blob.as_str(), BLOB_BYTES)]);
    let pack_path =
        std::env::temp_dir().join(format!("faulk-pack-stream-{}.zip", std::process::id()));
    let mut pack = std::fs::File::create(&pack_path).unwrap();

    let baseline = CURRENT.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let started = Instant::now();
    put_inbox(&frame);
    let begun = out_of(pw_pack_begin(frame.len() as u32));
    assert_eq!(begun[0], 0, "{}", String::from_utf8_lossy(&begun[1..]));

    let mut chunk = Vec::with_capacity(CHUNK);
    let mut pack_bytes = 0u64;
    loop {
        let out = out_of(pw_pack_next());
        match out[0] {
            3 => {
                pack.write_all(&out[1..]).unwrap();
                pack_bytes += out.len() as u64 - 1;
                break;
            }
            2 => {
                let n = u32::from_le_bytes(out[1..5].try_into().unwrap()) as usize;
                assert_eq!(std::str::from_utf8(&out[5..5 + n]).unwrap(), blob);
                pack.write_all(&out[5 + n..]).unwrap();
                pack_bytes += (out.len() - 5 - n) as u64;
                let mut done = 0u64;
                let mut index = 0u64;
                while done < BLOB_BYTES {
                    let size = CHUNK.min((BLOB_BYTES - done) as usize);
                    chunk_at(index, &mut chunk, size);
                    put_inbox(&chunk);
                    let fed = out_of(pw_pack_feed(size as u32));
                    assert_eq!(fed[0], 0, "{}", String::from_utf8_lossy(&fed[1..]));
                    pack.write_all(&fed[1..]).unwrap();
                    pack_bytes += fed.len() as u64 - 1;
                    done += size as u64;
                    index += 1;
                }
            }
            _ => panic!("{}", String::from_utf8_lossy(&out[1..])),
        }
    }
    pack.flush().unwrap();
    drop(pack);
    let export_ms = started.elapsed().as_millis();
    let peak = PEAK.load(Ordering::Relaxed) - baseline;

    // The reader reads the pack as it reads any other: the blob has its length and its CRC
    // (the reader checks it at the end of the entry), and its content is the generated one.
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&pack_path).unwrap()).unwrap();
    let mut entry = zip.by_name(&blob).unwrap();
    assert_eq!(entry.size(), BLOB_BYTES);
    let mut index = 0u64;
    let mut expected = Vec::new();
    let mut got = vec![0u8; CHUNK];
    let mut read = 0u64;
    while read < BLOB_BYTES {
        let size = CHUNK.min((BLOB_BYTES - read) as usize);
        chunk_at(index, &mut expected, size);
        entry.read_exact(&mut got[..size]).unwrap();
        assert!(got[..size] == expected[..], "chunk {index} differs");
        read += size as u64;
        index += 1;
    }
    let mut rest = Vec::new();
    entry.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
    drop(entry);
    drop(zip);
    let _ = std::fs::remove_file(&pack_path);

    println!(
        "pack: blob {} MiB, zip {} MiB, export {export_ms} ms, peak {:.1} MiB over the baseline",
        BLOB_BYTES >> 20,
        pack_bytes >> 20,
        peak as f64 / (1 << 20) as f64
    );
    // The blob, or the pack, held whole would be hundreds of MiB.
    assert!(
        peak < 48 << 20,
        "the export held {} MiB for a {} MiB blob",
        peak >> 20,
        BLOB_BYTES >> 20
    );
}
