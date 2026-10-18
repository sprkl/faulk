//! Faulk engine as a WebAssembly module with a plain C ABI (MO-2): one module, two
//! instances on one shared memory.
//!
//! The shipped build targets `wasm32-wasip1-threads` (CT-3's choice: atomics and a shared,
//! imported memory on the pinned stable toolchain; link arguments in `.cargo/config.toml`).
//! No `wasm-bindgen`: its glue needs `TextDecoder` and `fetch`, which an
//! `AudioWorkletGlobalScope` does not provide.
//!
//! - The **prepare instance** runs in a dedicated Worker. It creates the channel
//!   ([`fk_channel_new`]: `engine::host::Prepare` and `engine::host::Host`), allocates the
//!   stack and the thread-local block of the render instance ([`fk_alloc`]), and feeds the
//!   `ControlMsg` byte stream ([`fk_inbox`], [`fk_push`]) and the regular [`fk_flush`].
//! - The **render instance** runs in the `AudioWorklet`, on the same memory, with its own
//!   stack (`__stack_pointer`) and thread-local block (`__wasm_init_tls`). Its `process`
//!   calls [`fk_render`] only and copies [`fk_left`] / [`fk_right`]; the page reads the
//!   report region of [`fk_feedback`] from the same memory. It never allocates nor frees:
//!   a counting allocator keyed by a per-instance role proves it ([`fk_allocs`]).
//!
//! Each half of a [`Channel`] has one owner thread and is reached through its own field,
//! never through a `&mut Channel`: the two instances never alias. The crate also builds
//! for `wasm32-unknown-unknown`, where one instance holds both halves (the parity tests
//! run that build in an interpreter).
//!
//! Every export taking a channel pointer accepts null (ignored, never dereferenced) or a
//! live pointer returned by [`fk_channel_new`].

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

use engine::host::{Host, Prepare, channel};
use engine::msg::{MsgError, limits};

/// Largest render quantum accepted (Web Audio allows render sizes up to 2048 frames).
pub const MAX_FRAMES: usize = limits::MAX_BLOCK_FRAMES;

// ---------------------------------------------------------------- allocation counters

thread_local! {
    /// 0: not set, 1: prepare instance, 2: render instance ([`fk_set_role`]).
    static ROLE: Cell<u32> = const { Cell::new(0) };
}

/// Allocations and frees, by role.
static ALLOCS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static FREES: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

fn role() -> usize {
    ROLE.try_with(|r| r.get()).unwrap_or(0).min(2) as usize
}

/// The global allocator of the module: the system's, counted per role.
pub struct Counting;

// SAFETY: forwards every call to the system allocator unchanged; the counters are atomics
// and the role a thread-local `Cell` read without allocating.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS[role()].fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS[role()].fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        FREES[role()].fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS[role()].fetch_add(1, Ordering::Relaxed);
        FREES[role()].fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[cfg(target_family = "wasm")]
#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Sets the role of this instance (1 prepare, 2 render) for the allocation counters;
/// first call of each instance, after its stack and thread-local block are set.
#[unsafe(no_mangle)]
pub extern "C" fn fk_set_role(role: u32) {
    ROLE.with(|r| r.set(role.min(2)));
}

/// The role of this instance (two instances keep their own thread-locals).
#[unsafe(no_mangle)]
pub extern "C" fn fk_role() -> u32 {
    ROLE.with(|r| r.get())
}

/// Allocations made by the instances of `role` so far (0 for an unknown role).
#[unsafe(no_mangle)]
pub extern "C" fn fk_allocs(role: u32) -> u32 {
    ALLOCS
        .get(role as usize)
        .map_or(0, |c| c.load(Ordering::Relaxed))
}

/// Frees made by the instances of `role` so far.
#[unsafe(no_mangle)]
pub extern "C" fn fk_frees(role: u32) -> u32 {
    FREES
        .get(role as usize)
        .map_or(0, |c| c.load(Ordering::Relaxed))
}

/// Leaks `bytes` of zeroed heap aligned to `align` (rounded up to a power of two, at
/// most 64 KiB): the stack and thread-local block of the render instance. Null when
/// refused.
#[unsafe(no_mangle)]
pub extern "C" fn fk_alloc(bytes: u32, align: u32) -> *mut u8 {
    let align = (align as usize).clamp(1, 1 << 16).next_power_of_two();
    let Ok(layout) = Layout::from_size_align(bytes.max(1) as usize, align) else {
        return core::ptr::null_mut();
    };
    // SAFETY: the layout has a non-zero size.
    unsafe { std::alloc::alloc_zeroed(layout) }
}

// ---------------------------------------------------------------- the channel

/// Both halves of the engine channel.
pub struct Channel {
    prepare: PrepareHalf,
    host: Host,
}

/// The prepare instance's half: the prepare side and its inbox.
pub struct PrepareHalf {
    prepare: Prepare,
    inbox: Vec<u8>,
}

/// The prepare half, or `None` when null.
///
/// # Safety
/// `ptr` must be null or a live pointer from [`fk_channel_new`], and only the prepare
/// instance's thread may call the exports that use this half.
unsafe fn prepare_half<'a>(ptr: *mut Channel) -> Option<&'a mut PrepareHalf> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: a live pointer by the contract; the reference covers the `prepare` field
    // only, through a place projection on the raw pointer (no `&mut Channel` is formed),
    // and that field has one owner thread.
    Some(unsafe { &mut (*ptr).prepare })
}

/// The render half, or `None` when null.
///
/// # Safety
/// As [`prepare_half`], for the render instance's thread.
unsafe fn host<'a>(ptr: *mut Channel) -> Option<&'a mut Host> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: as in `prepare_half`, for the `host` field.
    Some(unsafe { &mut (*ptr).host })
}

/// Creates the channel at `sample_rate` (prepare instance; allocates). Null when the
/// memory is refused. A new sample rate is a new channel.
#[unsafe(no_mangle)]
pub extern "C" fn fk_channel_new(sample_rate: f32) -> *mut Channel {
    let built = channel(sample_rate).and_then(|(prepare, host)| {
        engine::msg::try_box(Channel {
            prepare: PrepareHalf {
                prepare,
                inbox: Vec::new(),
            },
            host,
        })
    });
    match built {
        Ok(ch) => Box::into_raw(ch),
        Err(_) => core::ptr::null_mut(),
    }
}

/// The address of an inbox of at least `len` bytes to write a stream into (prepare
/// instance; grows fallibly, so the address may change from one call to the next). Null
/// when the memory is refused or for a null channel.
///
/// # Safety
/// `ch` as in [`prepare_half`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_inbox(ch: *mut Channel, len: u32) -> *mut u8 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { prepare_half(ch) }) else {
        return core::ptr::null_mut();
    };
    let len = len as usize;
    if len > p.inbox.len() {
        if p.inbox.try_reserve_exact(len - p.inbox.len()).is_err() {
            return core::ptr::null_mut();
        }
        p.inbox.resize(len, 0);
    }
    p.inbox.as_mut_ptr()
}

/// Submits the `len` bytes written at `ptr`, which must be the address [`fk_inbox`] last
/// returned (prepare instance). Returns 0, or the `MsgError::code` that stopped the stream
/// (-1 truncated, also for a `ptr`/`len` outside the inbox; -8 out of memory; -9 engine
/// busy); messages rejected one by one are counted ([`fk_prepare_counter`]), not returned.
///
/// # Safety
/// `ch` as in [`prepare_half`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_push(ch: *mut Channel, ptr: *const u8, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { prepare_half(ch) }) else {
        return MsgError::NoSuchTarget.code();
    };
    let len = len as usize;
    if !core::ptr::eq(ptr, p.inbox.as_ptr()) || len > p.inbox.len() {
        return MsgError::Truncated {
            needed: len,
            available: p.inbox.len(),
        }
        .code();
    }
    match p.prepare.submit(&p.inbox[..len]) {
        Ok(()) => 0,
        Err(error) => error.code(),
    }
}

/// The byte offset of the message that stopped the last [`fk_push`], -1 when it went
/// through whole (prepare instance).
///
/// # Safety
/// `ch` as in [`prepare_half`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_stopped_at(ch: *mut Channel) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { prepare_half(ch) }
        .and_then(|p| p.prepare.preparer().stopped_at())
        .map_or(-1, |at| at.min(i32::MAX as usize) as i32)
}

/// Pushes what waited on the rt ring and drops the garbage (prepare instance, called
/// regularly by the Worker).
///
/// # Safety
/// `ch` as in [`prepare_half`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_flush(ch: *mut Channel) {
    // SAFETY: forwarded from this function's contract.
    if let Some(p) = unsafe { prepare_half(ch) } {
        p.prepare.flush();
    }
}

/// A counter of the prepare side: 0 rejected, 1 pending, 2 garbage collected (low 32
/// bits), 3 epoch; `u32::MAX` for a null channel or an unknown number.
///
/// # Safety
/// `ch` as in [`prepare_half`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_prepare_counter(ch: *mut Channel, which: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(p) = (unsafe { prepare_half(ch) }) else {
        return u32::MAX;
    };
    let p = p.prepare.preparer();
    match which {
        0 => p.rejected(),
        1 => p.pending() as u32,
        2 => p.collected() as u32,
        3 => u32::from(p.epoch()),
        _ => u32::MAX,
    }
}

/// Applies what arrived on the rt ring and takes the live events (render instance; the
/// first half of [`fk_render`]). Returns the rt messages applied.
///
/// # Safety
/// `ch` as in [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_pump(ch: *mut Channel) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { host(ch) }.map_or(0, Host::pump)
}

/// Renders one block of `frames` (clamped to [`MAX_FRAMES`]) without applying anything
/// (render instance; the second half of [`fk_render`]). Returns the frames rendered.
///
/// # Safety
/// `ch` as in [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_render_block(ch: *mut Channel, frames: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { host(ch) }.map_or(0, |h| h.render_block(frames as usize) as u32)
}

/// One block: [`fk_pump`] then [`fk_render_block`] (render instance, the worklet's only
/// call per quantum). Returns the frames rendered (0 for a null channel).
///
/// # Safety
/// `ch` as in [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_render(ch: *mut Channel, frames: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { host(ch) }.map_or(0, |h| h.render(frames as usize) as u32)
}

/// Address of the left output ([`MAX_FRAMES`] floats); null for a null channel.
///
/// # Safety
/// `ch` as in [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_left(ch: *mut Channel) -> *const f32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { host(ch) }.map_or(core::ptr::null(), |h| h.left().as_ptr())
}

/// Address of the right output ([`MAX_FRAMES`] floats); null for a null channel.
///
/// # Safety
/// `ch` as in [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_right(ch: *mut Channel) -> *const f32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { host(ch) }.map_or(core::ptr::null(), |h| h.right().as_ptr())
}

/// Address of the report region (the `engine::msg::Feedback` layout, written about 30
/// times per second under a seqlock); its address and size never change. Null for a null
/// channel. The page reads it through the shared memory, never through an export.
///
/// # Safety
/// `ch` as in [`host`]; the prepare instance may call it once, before audio starts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_feedback(ch: *mut Channel) -> *const u8 {
    // SAFETY: forwarded from this function's contract.
    unsafe { host(ch) }.map_or(core::ptr::null(), |h| h.feedback().as_ptr().cast())
}

/// Bytes of the report region (0 for a null channel).
///
/// # Safety
/// As [`fk_feedback`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_feedback_len(ch: *mut Channel) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { host(ch) }.map_or(0, |h| (h.feedback().len() * 4) as u32)
}

/// A counter of the render side: 0 applied (low 32 bits), 1 skipped, 2 leaked, 3 track
/// table capacity, 4 tracks held; `u32::MAX` for a null channel or an unknown number.
///
/// # Safety
/// `ch` as in [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_host_counter(ch: *mut Channel, which: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    let Some(h) = (unsafe { host(ch) }) else {
        return u32::MAX;
    };
    let e = h.engine();
    match which {
        0 => e.applied() as u32,
        1 => e.skipped(),
        2 => e.leaked(),
        3 => e.state().tracks.capacity() as u32,
        4 => e.state().tracks.as_slice().iter().flatten().count() as u32,
        _ => u32::MAX,
    }
}

/// Renders the device contract's scenario (`engine::device::testing::scenario`) once and
/// returns the address of its output ([`fk_contract_len`] floats: left, then right), null
/// if it could not render. Only in the test build (`--features device-contract`), for the
/// native/WASM parity test of the device contract; never in the worklet's module.
#[cfg(feature = "device-contract")]
#[unsafe(no_mangle)]
pub extern "C" fn fk_contract_scenario() -> *const f32 {
    let mut out = vec![0.0f32; 2 * engine::device::testing::SCENARIO_FRAMES].into_boxed_slice();
    if engine::device::testing::scenario(&mut out) == 0 {
        return core::ptr::null();
    }
    Box::leak(out).as_ptr()
}

/// Floats [`fk_contract_scenario`] writes.
#[cfg(feature = "device-contract")]
#[unsafe(no_mangle)]
pub extern "C" fn fk_contract_len() -> u32 {
    (2 * engine::device::testing::SCENARIO_FRAMES) as u32
}

/// The prepare/apply cycle's test host (`engine::engine::cycle::Cycle`, MO-1): both sides
/// of the channel in this one instance. Only in the test build (`--features engine-cycle`),
/// for the native/WASM parity and memory growth tests; never in the worklet's module, whose
/// channel is MO-2's.
#[cfg(feature = "engine-cycle")]
pub mod cycle_exports {
    use engine::engine::cycle::Cycle;

    /// Converts an exported pointer back to the cycle, or `None` when null.
    ///
    /// # Safety
    /// `ptr` must be null or a pointer returned by [`fk_cycle_new`].
    unsafe fn cycle<'a>(ptr: *mut Cycle) -> Option<&'a mut Cycle> {
        // SAFETY: the caller guarantees `ptr` is null or a live pointer from
        // `fk_cycle_new`; the test calls the exports from one thread.
        unsafe { ptr.as_mut() }
    }

    /// A cycle at `sample_rate`; null when the memory is refused.
    #[unsafe(no_mangle)]
    pub extern "C" fn fk_cycle_new(sample_rate: f32) -> *mut Cycle {
        match Cycle::new(sample_rate).and_then(engine::msg::try_box) {
            Ok(cycle) => Box::into_raw(cycle),
            Err(_) => core::ptr::null_mut(),
        }
    }

    /// The address of an inbox of `len` bytes to write a stream into; null when the memory
    /// is refused (or for a null cycle). The address may change from one call to the next.
    ///
    /// # Safety
    /// `ptr` must be null or a live pointer returned by [`fk_cycle_new`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fk_cycle_inbox(ptr: *mut Cycle, len: u32) -> *mut u8 {
        // SAFETY: forwarded from this function's contract.
        match unsafe { cycle(ptr) }.and_then(|c| c.inbox(len as usize)) {
            Some(inbox) => inbox.as_mut_ptr(),
            None => core::ptr::null_mut(),
        }
    }

    /// Submits the first `len` bytes of the inbox: 0, or the `MsgError::code` that stopped
    /// the stream (-8: out of memory).
    ///
    /// # Safety
    /// `ptr` must be null or a live pointer returned by [`fk_cycle_new`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fk_cycle_submit(ptr: *mut Cycle, len: u32) -> i32 {
        // SAFETY: forwarded from this function's contract.
        match unsafe { cycle(ptr) }.map(|c| c.submit_inbox(len as usize)) {
            Some(Ok(())) => 0,
            Some(Err(error)) => error.code(),
            None => engine::msg::MsgError::NoSuchTarget.code(),
        }
    }

    /// Renders one block of `frames`; returns the frames rendered.
    ///
    /// # Safety
    /// `ptr` must be null or a live pointer returned by [`fk_cycle_new`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fk_cycle_render(ptr: *mut Cycle, frames: u32) -> u32 {
        // SAFETY: forwarded from this function's contract.
        unsafe { cycle(ptr) }.map_or(0, |c| c.render(frames as usize) as u32)
    }

    /// Address of the left output of the last block.
    ///
    /// # Safety
    /// `ptr` must be null or a live pointer returned by [`fk_cycle_new`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fk_cycle_left(ptr: *mut Cycle) -> *const f32 {
        // SAFETY: forwarded from this function's contract.
        unsafe { cycle(ptr) }.map_or(core::ptr::null(), |c| c.left().as_ptr())
    }

    /// Address of the right output of the last block.
    ///
    /// # Safety
    /// `ptr` must be null or a live pointer returned by [`fk_cycle_new`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fk_cycle_right(ptr: *mut Cycle) -> *const f32 {
        // SAFETY: forwarded from this function's contract.
        unsafe { cycle(ptr) }.map_or(core::ptr::null(), |c| c.right().as_ptr())
    }

    /// A counter of the cycle, by number (`Cycle::counter`); `u32::MAX` for a null cycle
    /// or an unknown number.
    ///
    /// # Safety
    /// `ptr` must be null or a live pointer returned by [`fk_cycle_new`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fk_cycle_counter(ptr: *mut Cycle, which: u32) -> u32 {
        // SAFETY: forwarded from this function's contract.
        let Some(c) = (unsafe { cycle(ptr) }) else {
            return u32::MAX;
        };
        c.counter(which)
    }
}

/// The transport clock's test scenario (`engine::transport::testing::scenario`), computed
/// once and kept: the address of its `i64` output ([`fk_transport_len`] values), null when
/// empty. Only in the test build (`--features transport-scenario`), for the native/WASM
/// parity test; never in the worklet's module.
#[cfg(feature = "transport-scenario")]
pub mod transport_exports {
    use std::sync::OnceLock;

    fn scenario() -> &'static [i64] {
        static OUT: OnceLock<Box<[i64]>> = OnceLock::new();
        OUT.get_or_init(|| engine::transport::testing::scenario().into_boxed_slice())
    }

    /// Address of the scenario's output, null when it is empty.
    #[unsafe(no_mangle)]
    pub extern "C" fn fk_transport_scenario() -> *const i64 {
        let out = scenario();
        if out.is_empty() {
            core::ptr::null()
        } else {
            out.as_ptr()
        }
    }

    /// Number of `i64` values [`fk_transport_scenario`] points to.
    #[unsafe(no_mangle)]
    pub extern "C" fn fk_transport_len() -> u32 {
        scenario().len() as u32
    }
}

/// The timed events' test scenario (`engine::events::testing::scenario`, MO-4), rendered
/// once and kept: the address of its `f32` output ([`fk_events_len`] values), null when it
/// could not render. Only in the test build (`--features events-scenario`), for the
/// native/WASM parity test; never in the worklet's module.
#[cfg(feature = "events-scenario")]
pub mod events_exports {
    use std::sync::OnceLock;

    fn scenario() -> &'static [f32] {
        static OUT: OnceLock<Box<[f32]>> = OnceLock::new();
        OUT.get_or_init(|| {
            engine::events::testing::scenario()
                .unwrap_or_default()
                .into_boxed_slice()
        })
    }

    /// Address of the scenario's output, null when it is empty.
    #[unsafe(no_mangle)]
    pub extern "C" fn fk_events_scenario() -> *const f32 {
        let out = scenario();
        if out.is_empty() {
            core::ptr::null()
        } else {
            out.as_ptr()
        }
    }

    /// Number of `f32` values [`fk_events_scenario`] points to.
    #[unsafe(no_mangle)]
    pub extern "C" fn fk_events_len() -> u32 {
        scenario().len() as u32
    }
}
