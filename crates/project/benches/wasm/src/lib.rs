//! The prototype folder reader and writer of the load bench, compiled to wasm32 (QA-7), so
//! that the browser measures the Rust code that will run in the page (parse, build the
//! model, validate, serialise) and not a stand-in. Plain C ABI, no `wasm-bindgen` (as the
//! engine's build). The page copies the folder's files into this module's memory with
//! `fk_bench_alloc` and `fk_bench_put`, then calls `fk_bench_open`.
//!
//! A bench driver, not a product crate: it is not in the workspace and SV-1 does not build
//! on it.
#![allow(dead_code)]

#[path = "../../load/folder.rs"]
mod folder;
#[path = "../../load/synth.rs"]
mod synth;

use std::cell::RefCell;

thread_local! {
    static FILES: RefCell<folder::MemFiles> = RefCell::new(folder::MemFiles::default());
    static PROJECT: RefCell<Option<project::Project>> = const { RefCell::new(None) };
    static ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn fail(message: String) -> i32 {
    ERROR.with(|e| *e.borrow_mut() = message);
    -1
}

/// A buffer of `len` bytes the page fills (exact capacity, so that it can be taken back).
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

/// Takes back a buffer made by [`fk_bench_alloc`].
///
/// # Safety
/// `ptr` and `len` come from one call of `fk_bench_alloc` not yet given back.
unsafe fn take(ptr: *mut u8, len: usize) -> Box<[u8]> {
    // SAFETY: the caller's contract: the pointer and length of a boxed slice made by
    // `fk_bench_alloc`, owned once.
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) }
}

/// Registers a file of the folder: its path and its bytes, both made by `fk_bench_alloc`
/// and handed over.
///
/// # Safety
/// Both buffers come from `fk_bench_alloc` and are not used by the caller afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_bench_put(
    path_ptr: *mut u8,
    path_len: usize,
    data_ptr: *mut u8,
    data_len: usize,
) {
    // SAFETY: the function's contract.
    let (path, data) = unsafe { (take(path_ptr, path_len), take(data_ptr, data_len)) };
    let path = String::from_utf8_lossy(&path).into_owned();
    FILES.with(|f| f.borrow_mut().0.insert(path, data.into_vec()));
}

/// Forgets the registered files (the page frees the copies it made).
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_drop_files() {
    FILES.with(|f| f.borrow_mut().0.clear());
}

/// Forgets the project.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_drop_project() {
    PROJECT.with(|p| *p.borrow_mut() = None);
}

/// Reads the registered folder into a project; the number of tracks, or -1 (see
/// `fk_bench_error`).
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_open() -> i32 {
    let result = FILES.with(|f| folder::read_project(&*f.borrow(), 1));
    match result {
        Ok((project, _)) => {
            let n = project.tracks.len() as i32;
            PROJECT.with(|p| *p.borrow_mut() = Some(project));
            n
        }
        Err(e) => fail(e),
    }
}

/// Decodes a cache snapshot (bytes made by `fk_bench_alloc`, handed over) into the project.
///
/// # Safety
/// The buffer comes from `fk_bench_alloc` and is not used by the caller afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_bench_open_snapshot(ptr: *mut u8, len: usize) -> i32 {
    // SAFETY: the function's contract.
    let bytes = unsafe { take(ptr, len) };
    match folder::decode_snapshot(&bytes) {
        Ok(project) => {
            let n = project.tracks.len() as i32;
            PROJECT.with(|p| *p.borrow_mut() = Some(project));
            n
        }
        Err(e) => fail(format!("{e:?}")),
    }
}

/// Builds, in this module's memory, a project at the safety limits of notes and points (CT-3:
/// 16 777 216 notes and 16 777 216 points; 17 clips of 986 895 notes and 17 lanes of 986 895
/// points here, 16 777 215 each), as the page would hold it. The number of tracks.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_generate_limits() -> i32 {
    let profile = synth::Profile {
        seed: synth::SEED,
        midi_tracks: 17,
        audio_tracks: 0,
        clips_per_midi_track: 1,
        effects: 0,
        notes_per_clip: 986_895,
        lanes_per_midi_track: 1,
        lanes_per_audio_track: 0,
        total_points: 17 * 986_895,
        clips_per_audio_track: 0,
        scenes: 1,
    };
    let (project, _) = synth::generate(&profile, 0);
    let n = project.tracks.len() as i32;
    PROJECT.with(|p| *p.borrow_mut() = Some(project));
    n
}

/// Validates the open project: 0 when it is valid, else the number of violations.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_validate() -> i32 {
    PROJECT.with(|p| match p.borrow().as_ref() {
        Some(project) => match project.validate() {
            Ok(()) => 0,
            Err(v) => v.len() as i32,
        },
        None => -1,
    })
}

/// Serialises every file of the open project (text layout) without writing: the bytes
/// produced, which is what a save pays before it knows what changed.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_serialize_all() -> f64 {
    PROJECT.with(|p| {
        let mut total = 0u64;
        if let Some(project) = p.borrow().as_ref() {
            folder::emit_all(project, folder::Layout::TEXT, &mut |o| {
                total += o.bytes.len() as u64;
            });
        }
        total as f64
    })
}

/// Serialises the notes of the first MIDI clip: the bytes of the one file a note edit rewrites.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_serialize_one_clip() -> u32 {
    PROJECT.with(|p| {
        let guard = p.borrow();
        let Some(project) = guard.as_ref() else {
            return 0;
        };
        let Some((t, c)) = folder::first_midi_clip(project) else {
            return 0;
        };
        match &project.tracks[&t].clips[&c].content {
            project::ClipContent::Midi { notes, .. } => folder::notes_tsv(notes).len() as u32,
            _ => 0,
        }
    })
}

/// Notes in the open project.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_notes() -> u32 {
    PROJECT.with(|p| {
        p.borrow().as_ref().map_or(0, |project| {
            project
                .tracks
                .values()
                .flat_map(|t| t.clips.values())
                .map(|c| match &c.content {
                    project::ClipContent::Midi { notes, .. } => notes.len() as u32,
                    _ => 0,
                })
                .sum()
        })
    })
}

/// Automation points in the open project.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_points() -> u32 {
    PROJECT.with(|p| {
        p.borrow().as_ref().map_or(0, |project| {
            project
                .tracks
                .values()
                .flat_map(|t| t.automation.values())
                .map(|l| l.points.len() as u32)
                .sum()
        })
    })
}

/// The size of the linear memory in bytes (it never shrinks).
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_memory() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        (core::arch::wasm32::memory_size(0) * 65536) as f64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0.0
    }
}

/// The last error, written into a buffer the caller reads: returns its length; the bytes
/// are at `fk_bench_error_ptr`.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_error_len() -> usize {
    ERROR.with(|e| e.borrow().len())
}

#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_error_ptr() -> *const u8 {
    ERROR.with(|e| e.borrow().as_ptr())
}
