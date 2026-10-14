//! Prototype of the engine channel (CT-3): the real engine (`engine-wasm`'s `fk_*` exports
//! come along with the rlib) plus the rt ring, a runtime table that grows, a content table
//! swapped by pointer, and the garbage ring of `engine::msg`.
//!
//! Way (a): one instance in the worklet, which calls the prepare exports between two quanta.
//! Way (b): two instances on one shared memory; the Worker calls the prepare exports, the
//! worklet only `fk_proto_pump` and `fk_render`.
//!
//! Every export takes a `Proto` pointer that must be null or a live pointer from
//! [`fk_proto_new`]; a null pointer is ignored. Allocations on each instance are counted
//! per role (a thread-local, so that two instances on one memory count apart) and read by
//! the worklet to prove that the render side never allocates nor frees.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

use engine::msg::{
    Garbage, Note, Opaque, ParamId, Prepared, RtMsg, Swap, Table, TableFull, TableKind, TrackSlot,
    limits,
};
use engine::time::Tick;
use engine_wasm::Instance;
use rtrb::{Consumer, Producer, RingBuffer};

// Keep engine-wasm's exports linked in: the rlib's `#[no_mangle]` symbols are exported by
// this cdylib as long as the crate is used.
#[allow(dead_code)]
fn link_engine_wasm(sample_rate: f32) -> *mut Instance {
    engine_wasm::fk_new(sample_rate)
}

thread_local! {
    /// 0: unknown, 1: prepare side, 2: render side (set by `fk_proto_set_role`).
    static ROLE: Cell<u32> = const { Cell::new(0) };
}

/// Allocations and frees, by role (index 1: prepare, 2: render).
static ALLOCS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static FREES: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

struct Counting;

// SAFETY: forwards every call to the system allocator unchanged; the counters are atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let role = ROLE.try_with(|r| r.get()).unwrap_or(0) as usize;
        ALLOCS[role.min(2)].fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let role = ROLE.try_with(|r| r.get()).unwrap_or(0) as usize;
        FREES[role.min(2)].fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The two rings and the audio side's tables.
pub struct Proto {
    rt_tx: Producer<RtMsg>,
    rt_rx: Consumer<RtMsg>,
    gc_tx: Producer<Garbage>,
    gc_rx: Consumer<Garbage>,
    tracks: Table<Option<Opaque>>,
    notes: Table<Note>,
    applied: u32,
    busy: u32,
}

/// Converts an exported pointer back, or `None` when null.
///
/// # Safety
/// `ptr` must be null or a pointer returned by [`fk_proto_new`] and not yet freed.
unsafe fn proto<'a>(ptr: *mut Proto) -> Option<&'a mut Proto> {
    // SAFETY: the caller guarantees `ptr` is null or a live pointer from `fk_proto_new`.
    // The prepare side touches only `rt_tx`, `gc_rx`, `busy`; the render side only
    // `rt_rx`, `gc_tx`, `tracks`, `notes`, `applied`: disjoint fields, one writer each.
    unsafe { ptr.as_mut() }
}

/// Sets the role of this instance (1 prepare, 2 render) for the allocation counters.
#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_set_role(role: u32) {
    ROLE.with(|r| r.set(role));
}

#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_allocs(role: u32) -> u32 {
    ALLOCS[(role as usize).min(2)].load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_frees(role: u32) -> u32 {
    FREES[(role as usize).min(2)].load(Ordering::Relaxed)
}

/// Creates the rings and the tables (prepare side, allocates).
#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_new(track_capacity: u32) -> *mut Proto {
    let (rt_tx, rt_rx) = RingBuffer::new(limits::RT_RING_CAPACITY);
    let (gc_tx, gc_rx) = RingBuffer::new(limits::RT_RING_CAPACITY);
    let Ok(tracks) = Table::try_with_capacity(track_capacity as usize) else {
        return core::ptr::null_mut();
    };
    let Ok(notes) = Table::try_with_capacity(0) else {
        return core::ptr::null_mut();
    };
    Box::into_raw(Box::new(Proto {
        rt_tx,
        rt_rx,
        gc_tx,
        gc_rx,
        tracks,
        notes,
        applied: 0,
        busy: 0,
    }))
}

/// Leaks `bytes` of the shared heap: the stack and TLS area of a second instance.
#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_alloc(bytes: u32) -> *mut u8 {
    let mut v: Vec<u8> = Vec::new();
    if v.try_reserve_exact(bytes as usize).is_err() {
        return core::ptr::null_mut();
    }
    v.resize(bytes as usize, 0);
    Box::into_raw(v.into_boxed_slice()) as *mut u8
}

/// Prepare side: one `ParamSet` on the rt ring. Returns 1 when the ring is full.
///
/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_push_param(ptr: *mut Proto, param: u32, value: f32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { proto(ptr) }) else {
        return 1;
    };
    let msg = RtMsg::ParamSet {
        track: TrackSlot(0),
        device: engine::msg::DeviceSlot(0),
        param: ParamId(param),
        value,
        offset: 0,
    };
    if p.rt_tx.push(msg).is_err() {
        p.busy += 1;
        1
    } else {
        0
    }
}

/// Prepare side: a content snapshot of `n` notes, swapped by pointer (0 ok, 1 out of
/// memory, 2 ring full).
///
/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_prepare_notes(ptr: *mut Proto, n: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { proto(ptr) }) else {
        return 2;
    };
    collect(p);
    let Ok(mut next) = Table::<Note>::try_with_capacity(n as usize) else {
        return 1;
    };
    for i in 0..n {
        let _ = next.push(Note {
            start: Tick(i as i64 * 240),
            length: Tick(240),
            pitch: (i % 128) as u8,
            velocity: 100,
        });
    }
    let swap = RtMsg::Swap(Swap {
        kind: TableKind::ClipNotes(engine::msg::ClipSlot(0)),
        next: Prepared::ClipNotes(next),
    });
    if p.rt_tx.push(swap).is_err() {
        p.busy += 1;
        2
    } else {
        0
    }
}

/// Prepare side: the next track table, `capacity` slots (0 ok, 1 out of memory, 2 full).
///
/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_prepare_tracks(ptr: *mut Proto, capacity: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { proto(ptr) }) else {
        return 2;
    };
    collect(p);
    let Ok(next) = Table::<Option<Opaque>>::try_with_capacity(capacity as usize) else {
        return 1;
    };
    let swap = RtMsg::Swap(Swap {
        kind: TableKind::Tracks,
        next: Prepared::Tracks(next),
    });
    if p.rt_tx.push(swap).is_err() {
        p.busy += 1;
        2
    } else {
        0
    }
}

/// Prepare side: a boxed track in `slot` (0 ok, 1 out of memory, 2 full).
///
/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_prepare_track(ptr: *mut Proto, slot: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { proto(ptr) }) else {
        return 2;
    };
    collect(p);
    let Ok(track) = engine::msg::try_box([slot as u8; 256]) else {
        return 1;
    };
    let swap = RtMsg::Swap(Swap {
        kind: TableKind::Track(TrackSlot(slot)),
        next: Prepared::Track(track as Opaque),
    });
    if p.rt_tx.push(swap).is_err() {
        p.busy += 1;
        2
    } else {
        0
    }
}

fn collect(p: &mut Proto) -> u32 {
    let mut n = 0;
    while let Ok(garbage) = p.gc_rx.pop() {
        drop(garbage);
        n += 1;
    }
    n
}

/// Prepare side: drops what the render side replaced; returns how many.
///
/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_collect(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { proto(ptr) }.map_or(0, collect)
}

/// Render side: pops up to `MAX_APPLY_PER_BLOCK` messages and applies them to the engine
/// instance (null allowed: parameters are then dropped). No allocation, no free. Returns
/// how many were applied.
///
/// # Safety
/// `ptr` as in [`proto`]; `inst` null or a live pointer from `fk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_pump(ptr: *mut Proto, inst: *mut Instance) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { proto(ptr) }) else {
        return 0;
    };
    // SAFETY: forwarded from this function's contract (single render thread).
    let mut inst = unsafe { inst.as_mut() };
    let mut n = 0;
    for _ in 0..limits::MAX_APPLY_PER_BLOCK {
        let Ok(msg) = p.rt_rx.pop() else { break };
        n += 1;
        let garbage = match msg {
            RtMsg::ParamSet { param, value, .. } => {
                if let (Some(inst), Some(param)) =
                    (inst.as_deref_mut(), engine::Param::from_id(param.0))
                {
                    inst.synth_mut().set_param(param, value);
                }
                None
            }
            RtMsg::Swap(Swap {
                kind: TableKind::ClipNotes(_),
                next: Prepared::ClipNotes(next),
            }) => Some(Garbage::ClipNotes(core::mem::replace(&mut p.notes, next))),
            RtMsg::Swap(Swap {
                kind: TableKind::Tracks,
                next: Prepared::Tracks(mut next),
            }) => {
                next.take_from(&mut p.tracks);
                Some(Garbage::Tracks(core::mem::replace(&mut p.tracks, next)))
            }
            RtMsg::Swap(Swap {
                kind: TableKind::Track(slot),
                next: Prepared::Track(track),
            }) => {
                let i = slot.0 as usize;
                if i < p.tracks.len() {
                    p.tracks.as_mut_slice()[i]
                        .replace(track)
                        .map(Garbage::Track)
                } else {
                    // A new slot; a full table refuses it (never happens when the prepare
                    // side grows the table first), and the boxed track is then dropped by
                    // the prepare side as garbage.
                    p.tracks
                        .push(Some(track))
                        .err()
                        .and_then(|TableFull(t)| t.map(Garbage::Track))
                }
            }
            _ => None,
        };
        if let Some(garbage) = garbage {
            // The invariant (garbage drained before each push) makes this slot free.
            let _ = p.gc_tx.push(garbage);
        }
    }
    p.applied += n;
    n
}

/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_applied(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { proto(ptr) }.map_or(0, |p| p.applied)
}

/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_busy(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { proto(ptr) }.map_or(0, |p| p.busy)
}

/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_tracks_len(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { proto(ptr) }.map_or(0, |p| p.tracks.len() as u32)
}

/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_tracks_capacity(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { proto(ptr) }.map_or(0, |p| p.tracks.capacity() as u32)
}

/// # Safety
/// `ptr` as in [`proto`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_notes_len(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { proto(ptr) }.map_or(0, |p| p.notes.len() as u32)
}

/// The role of this instance, to check that two instances keep their own thread-locals.
#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_role() -> u32 {
    ROLE.with(|r| r.get())
}
