//! Prototype of the engine channel (CT-3): the real engine (the old `engine-wasm` `fk_*` exports,
//! frozen in [`old_instance`]) plus the rt ring, a runtime table that grows (a slot array), a
//! content table swapped by pointer, and the garbage ring of `engine::msg`.
//!
//! Way (a): one instance in the worklet, which calls the prepare exports between two quanta.
//! Way (b): two instances on one shared memory; the Worker calls the prepare exports, the
//! worklet only `fk_proto_pump` and `fk_render`.
//!
//! The state is split in two halves with one owner each: [`PrepareSide`] (the Worker, or
//! the worklet between quanta in way (a)) and [`RenderSide`] (the worklet's `process`).
//! Every export forms a `&mut` to its own half only, through the raw pointer, never to the
//! whole [`Proto`]: the two threads never alias. Allocations on each instance are counted
//! per role (a thread-local, so that two instances on one memory count apart) and read by
//! the worklet to prove that the render side never allocates nor frees.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

use engine::msg::{Note, ParamId, Table, TrackSlot, limits};
use engine::time::Tick;
use old_instance::Instance;

pub mod old_instance;
use proto_msg::{Garbage, Opaque, Prepared, RtMsg, Swap, TableKind};
use rtrb::{Consumer, Producer, RingBuffer};

/// The rt message types CT-3 measured with, kept here as they were: MO-1 (#54) replaced
/// them in `engine::msg` by concrete ones, and this prototype stays the reference of CT-3's
/// numbers.
#[allow(dead_code)]
mod proto_msg {
    use engine::msg::{
        AutomationPoint, ClipSlot, DeviceSlot, LaneSlot, Note, ParamId, Placement, SceneSlot,
        SourceSlot, Table, TrackSlot,
    };
    use engine::time::{Frame, LoopRegion, Quantize, Tempo, Tick, TimeSignature};

    /// An opaque engine structure built by `prepare` (a track with its devices, a device, the
    /// whole engine state): the concrete types are MO-3's and CT-5's.
    pub type Opaque = Box<dyn core::any::Any + Send>;

    /// Which engine structure a [`Swap`] or a [`RtMsg::Remove`] targets.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TableKind {
        /// The whole engine state (`Reset`).
        Everything,
        /// The runtime table of the tracks (grown when full).
        Tracks,
        Track(TrackSlot),
        Device(TrackSlot, DeviceSlot),
        /// The notes of one clip (content snapshot).
        ClipNotes(ClipSlot),
        /// The points of one lane (content snapshot).
        LanePoints(LaneSlot),
        /// The arrangement of one track (content snapshot).
        TrackPlacements(TrackSlot),
        /// The runtime table of the audio sources (grown when full).
        AudioSources,
        AudioSource(SourceSlot),
    }

    /// A structure `prepare` built, carried into `apply` by [`RtMsg::Swap`]. Runtime tables
    /// hold one boxed entry per slot ([`Opaque`]); content tables hold the values.
    #[derive(Debug)]
    pub enum Prepared {
        Everything(Opaque),
        Tracks(Table<Option<Opaque>>),
        Track(Opaque),
        Device(Opaque),
        ClipNotes(Table<Note>),
        LanePoints(Table<AutomationPoint>),
        TrackPlacements(Table<Placement>),
        AudioSources(Table<Option<Opaque>>),
        AudioSource(Opaque),
    }

    /// What `apply` sends back on the garbage ring, one per rt message, dropped on the
    /// prepare side only. A removed track carries its devices and buffers inside its box.
    #[derive(Debug)]
    pub enum Garbage {
        Everything(Opaque),
        Tracks(Table<Option<Opaque>>),
        Track(Opaque),
        Device(Opaque),
        ClipNotes(Table<Note>),
        LanePoints(Table<AutomationPoint>),
        TrackPlacements(Table<Placement>),
        AudioSources(Table<Option<Opaque>>),
        AudioSource(Opaque),
    }

    /// Replaces one engine structure in one block.
    #[derive(Debug)]
    pub struct Swap {
        pub kind: TableKind,
        pub next: Prepared,
    }

    /// Transport order carried to the audio thread.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TransportMsg {
        Play { from: Tick },
        Stop,
        Seek { tick: Tick },
        SetLoop { region: Option<LoopRegion> },
        SetRecording { recording: bool },
    }

    /// Launch order carried to the audio thread (executed at the next quantization boundary).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum LaunchMsg {
        Clip { track: TrackSlot, scene: SceneSlot },
        Scene { scene: SceneSlot },
        StopTrack { track: TrackSlot },
        StopAll,
        BackToArrangement,
    }

    /// What `prepare` hands to `apply`: a Rust value moved through the ring, never serialized.
    /// `offset` is the frame in the next block where the message takes effect (0: block start).
    #[derive(Debug)]
    pub enum RtMsg {
        /// Replaces a structure (a grown table, a new device); the old one goes to [`Garbage`].
        Swap(Swap),
        /// Removes a structure; what it held goes to [`Garbage`].
        Remove(TableKind),
        SetTransport {
            tempo: Tempo,
            signature: TimeSignature,
            launch_quantize: Quantize,
        },
        SetMetronome {
            enabled: bool,
            volume: f32,
            count_in_bars: u8,
        },
        SetMaster {
            gain: f32,
            mute: bool,
        },
        TrackSetMix {
            track: TrackSlot,
            gain: f32,
            pan: f32,
            mute: bool,
        },
        TrackSetSolo {
            track: TrackSlot,
            solo: bool,
        },
        TrackArm {
            track: TrackSlot,
            armed: bool,
        },
        TrackSetMonitor {
            track: TrackSlot,
            monitor: bool,
        },
        SelectTrack {
            track: Option<TrackSlot>,
        },
        DeviceSetEnabled {
            track: TrackSlot,
            device: DeviceSlot,
            enabled: bool,
        },
        ParamSet {
            track: TrackSlot,
            device: DeviceSlot,
            param: ParamId,
            value: f32,
            offset: u16,
        },
        SceneSetSlot {
            track: TrackSlot,
            scene: SceneSlot,
            clip: Option<ClipSlot>,
        },
        ClipAudioSet {
            clip: ClipSlot,
            source: SourceSlot,
            start: Frame,
            length: Frame,
        },
        Transport(TransportMsg),
        Launch(LaunchMsg),
        /// Live note, from the live queue.
        NoteOn {
            track: TrackSlot,
            pitch: u8,
            velocity: u8,
            offset: u16,
        },
        NoteOff {
            track: TrackSlot,
            pitch: u8,
            offset: u16,
        },
        AllNotesOff {
            track: TrackSlot,
        },
    }
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

/// The prepare side's half: the rt producer, the garbage consumer, its counters.
pub struct PrepareSide {
    rt_tx: Producer<RtMsg>,
    gc_rx: Consumer<Garbage>,
    busy: u32,
}

/// The render side's half: the rt consumer, the garbage producer, the tables.
pub struct RenderSide {
    rt_rx: Consumer<RtMsg>,
    gc_tx: Producer<Garbage>,
    tracks: Table<Option<Opaque>>,
    notes: Table<Note>,
    applied: u32,
    /// Messages naming a slot beyond the track table (a contract violation, no panic).
    skipped: u32,
    /// Garbage the ring refused (the invariant broken); parked, never dropped here.
    garbage_refused: u32,
    parked: Option<Garbage>,
}

/// Both halves, allocated once by the prepare side.
pub struct Proto {
    prepare: PrepareSide,
    render: RenderSide,
}

/// The prepare half, or `None` when null.
///
/// # Safety
/// `ptr` must be null or a pointer returned by [`fk_proto_new`] and not yet freed, and
/// only the prepare thread may call the exports that use this half.
unsafe fn prepare_side<'a>(ptr: *mut Proto) -> Option<&'a mut PrepareSide> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees a live pointer; the reference covers the `prepare`
    // field only, through a place projection on the raw pointer (no `&mut Proto` is ever
    // formed), and that field has one owner thread.
    Some(unsafe { &mut (*ptr).prepare })
}

/// The render half, or `None` when null.
///
/// # Safety
/// As [`prepare_side`], for the render thread.
unsafe fn render_side<'a>(ptr: *mut Proto) -> Option<&'a mut RenderSide> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: as in `prepare_side`, for the `render` field.
    Some(unsafe { &mut (*ptr).render })
}

/// Sets the role of this instance (1 prepare, 2 render) for the allocation counters.
#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_set_role(role: u32) {
    ROLE.with(|r| r.set(role));
}

/// The role of this instance, to check that two instances keep their own thread-locals.
#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_role() -> u32 {
    ROLE.with(|r| r.get())
}

#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_allocs(role: u32) -> u32 {
    ALLOCS[(role as usize).min(2)].load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_frees(role: u32) -> u32 {
    FREES[(role as usize).min(2)].load(Ordering::Relaxed)
}

/// Creates the rings and the tables (prepare side, allocates). The track table is a slot
/// array of `track_capacity` slots.
#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_new(track_capacity: u32) -> *mut Proto {
    let (rt_tx, rt_rx) = RingBuffer::new(limits::RT_RING_CAPACITY);
    let (gc_tx, gc_rx) = RingBuffer::new(limits::GARBAGE_RING_CAPACITY);
    let Ok(tracks) = Table::try_filled(track_capacity as usize) else {
        return core::ptr::null_mut();
    };
    let Ok(notes) = Table::try_with_capacity(0) else {
        return core::ptr::null_mut();
    };
    Box::into_raw(Box::new(Proto {
        prepare: PrepareSide {
            rt_tx,
            gc_rx,
            busy: 0,
        },
        render: RenderSide {
            rt_rx,
            gc_tx,
            tracks,
            notes,
            applied: 0,
            skipped: 0,
            garbage_refused: 0,
            parked: None,
        },
    }))
}

/// Leaks `bytes` of the shared heap: the stack of a second instance.
#[unsafe(no_mangle)]
pub extern "C" fn fk_proto_alloc(bytes: u32) -> *mut u8 {
    let mut v: Vec<u8> = Vec::new();
    if v.try_reserve_exact(bytes as usize).is_err() {
        return core::ptr::null_mut();
    }
    v.resize(bytes as usize, 0);
    Box::into_raw(v.into_boxed_slice()) as *mut u8
}

fn collect(p: &mut PrepareSide) -> u32 {
    let mut n = 0;
    while let Ok(garbage) = p.gc_rx.pop() {
        drop(garbage);
        n += 1;
    }
    n
}

/// Drains the garbage first (the invariant), then pushes; 0 ok, 2 ring full.
fn send(p: &mut PrepareSide, msg: RtMsg) -> u32 {
    collect(p);
    if p.rt_tx.push(msg).is_err() {
        p.busy += 1;
        2
    } else {
        0
    }
}

/// Prepare side: one `ParamSet` on the rt ring. Returns 2 when the ring is full.
///
/// # Safety
/// `ptr` as in [`prepare_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_push_param(ptr: *mut Proto, param: u32, value: f32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { prepare_side(ptr) }) else {
        return 2;
    };
    send(
        p,
        RtMsg::ParamSet {
            track: TrackSlot(0),
            device: engine::msg::DeviceSlot(0),
            param: ParamId(param),
            value,
            offset: 0,
        },
    )
}

/// Prepare side: a content snapshot of `n` notes, swapped by pointer (0 ok, 1 out of
/// memory, 2 ring full).
///
/// # Safety
/// `ptr` as in [`prepare_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_prepare_notes(ptr: *mut Proto, n: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { prepare_side(ptr) }) else {
        return 2;
    };
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
    send(
        p,
        RtMsg::Swap(Swap {
            kind: TableKind::ClipNotes(engine::msg::ClipSlot(0)),
            next: Prepared::ClipNotes(next),
        }),
    )
}

/// Prepare side: the next track slot array, `capacity` slots (0 ok, 1 out of memory,
/// 2 full).
///
/// # Safety
/// `ptr` as in [`prepare_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_prepare_tracks(ptr: *mut Proto, capacity: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { prepare_side(ptr) }) else {
        return 2;
    };
    let Ok(next) = Table::<Option<Opaque>>::try_filled(capacity as usize) else {
        return 1;
    };
    send(
        p,
        RtMsg::Swap(Swap {
            kind: TableKind::Tracks,
            next: Prepared::Tracks(next),
        }),
    )
}

/// Prepare side: a boxed track for `slot` (0 ok, 1 out of memory, 2 full). The caller
/// grows the table first when the slot is beyond it.
///
/// # Safety
/// `ptr` as in [`prepare_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_prepare_track(ptr: *mut Proto, slot: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { prepare_side(ptr) }) else {
        return 2;
    };
    let Ok(track) = engine::msg::try_box([slot as u8; 256]) else {
        return 1;
    };
    send(
        p,
        RtMsg::Swap(Swap {
            kind: TableKind::Track(TrackSlot(slot)),
            next: Prepared::Track(track as Opaque),
        }),
    )
}

/// Prepare side: drops what the render side replaced; returns how many.
///
/// # Safety
/// `ptr` as in [`prepare_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_collect(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { prepare_side(ptr) }.map_or(0, collect)
}

/// # Safety
/// `ptr` as in [`prepare_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_busy(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { prepare_side(ptr) }.map_or(0, |p| p.busy)
}

/// Render side: pops up to `MAX_APPLY_PER_BLOCK` messages and applies them to the engine
/// instance (null allowed: parameters are then dropped). No allocation, no free. Returns
/// how many were applied.
///
/// # Safety
/// `ptr` as in [`render_side`]; `inst` null or a live pointer from `fk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_pump(ptr: *mut Proto, inst: *mut Instance) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(r) = (unsafe { render_side(ptr) }) else {
        return 0;
    };
    // SAFETY: forwarded from this function's contract (single render thread).
    let mut inst = unsafe { inst.as_mut() };
    if let Some(parked) = r.parked.take()
        && let Err(rtrb::PushError::Full(again)) = r.gc_tx.push(parked)
    {
        r.parked = Some(again);
    }
    let mut n = 0;
    for _ in 0..limits::MAX_APPLY_PER_BLOCK {
        let Ok(msg) = r.rt_rx.pop() else { break };
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
            }) => Some(Garbage::ClipNotes(core::mem::replace(&mut r.notes, next))),
            RtMsg::Swap(Swap {
                kind: TableKind::Tracks,
                next: Prepared::Tracks(mut next),
            }) => {
                next.take_slots_from(&mut r.tracks);
                Some(Garbage::Tracks(core::mem::replace(&mut r.tracks, next)))
            }
            RtMsg::Swap(Swap {
                kind: TableKind::Track(slot),
                next: Prepared::Track(track),
            }) => match r.tracks.as_mut_slice().get_mut(slot.0 as usize) {
                Some(entry) => entry.replace(track).map(Garbage::Track),
                None => {
                    // Beyond the slot array: skipped, the box goes to the garbage.
                    r.skipped += 1;
                    Some(Garbage::Track(track))
                }
            },
            // The other variants carry no heap in this prototype.
            _ => None,
        };
        if let Some(garbage) = garbage {
            // The invariant (garbage drained before each push) makes this slot free; if it
            // ever is not, the item is parked for the next pump, never dropped here.
            if let Err(rtrb::PushError::Full(refused)) = r.gc_tx.push(garbage) {
                r.garbage_refused += 1;
                if r.parked.is_none() {
                    r.parked = Some(refused);
                } else {
                    // A second refusal while one is parked: leaked rather than freed on
                    // the render thread, and counted (the e2e asserts the counter is 0).
                    core::mem::forget(refused);
                }
            }
        }
    }
    r.applied += n;
    n
}

/// # Safety
/// `ptr` as in [`render_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_applied(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { render_side(ptr) }.map_or(0, |r| r.applied)
}

/// # Safety
/// `ptr` as in [`render_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_skipped(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { render_side(ptr) }.map_or(0, |r| r.skipped)
}

/// # Safety
/// `ptr` as in [`render_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_garbage_refused(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { render_side(ptr) }.map_or(0, |r| r.garbage_refused)
}

/// Filled slots of the track table.
///
/// # Safety
/// `ptr` as in [`render_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_tracks_len(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { render_side(ptr) }.map_or(0, |r| {
        r.tracks.as_slice().iter().filter(|e| e.is_some()).count() as u32
    })
}

/// # Safety
/// `ptr` as in [`render_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_tracks_capacity(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { render_side(ptr) }.map_or(0, |r| r.tracks.capacity() as u32)
}

/// # Safety
/// `ptr` as in [`render_side`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_proto_notes_len(ptr: *mut Proto) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { render_side(ptr) }.map_or(0, |r| r.notes.len() as u32)
}
