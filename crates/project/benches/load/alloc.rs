//! A counting allocator for the memory figures of the bench: live and peak heap bytes.
//! Counting is off unless a measure asks for it, so that the timings are not charged for it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

pub struct Counting;

static ON: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is forwarded unchanged to the system allocator; the counters are
// atomics and never touch the allocation.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: same contract as the caller's.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() && ON.load(Relaxed) {
            add(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ON.load(Relaxed) {
            LIVE.fetch_sub(layout.size().min(LIVE.load(Relaxed)), Relaxed);
        }
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: same contract as the caller's.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() && ON.load(Relaxed) {
            LIVE.fetch_sub(layout.size().min(LIVE.load(Relaxed)), Relaxed);
            add(new_size);
        }
        p
    }
}

fn add(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
}

/// Starts counting from the heap as it is now: later `live` is the growth since this call.
pub fn start() {
    LIVE.store(0, Relaxed);
    PEAK.store(0, Relaxed);
    ON.store(true, Relaxed);
}

pub fn stop() {
    ON.store(false, Relaxed);
}

/// Heap bytes allocated since `start` and still held.
pub fn live() -> usize {
    LIVE.load(Relaxed)
}

/// The most heap bytes held at once since `start`.
pub fn peak() -> usize {
    PEAK.load(Relaxed)
}

/// The process's peak working set in bytes, from the OS (Windows: `PeakWorkingSetSize`).
pub fn peak_working_set() -> Option<u64> {
    let pid = std::process::id();
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).PeakWorkingSet64"),
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}
