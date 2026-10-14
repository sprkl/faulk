//! Faulk engine as a WebAssembly module, for an `AudioWorklet`.
//!
//! The exports are a plain C ABI, without `wasm-bindgen`: its JavaScript glue needs
//! `TextDecoder` and `fetch`, which an `AudioWorkletGlobalScope` does not provide.
//! The worklet instantiates the module, creates an instance with [`fk_new`], then for
//! each render quantum calls [`fk_render`] and copies the two output buffers, whose
//! addresses [`fk_left_ptr`] and [`fk_right_ptr`] give, from the module's memory.
//!
//! Every export takes an instance pointer that must be either null or a live pointer
//! returned by [`fk_new`]; a null pointer is ignored, never dereferenced.

use engine::{Param, Synth};

/// Largest render quantum accepted (Web Audio allows render sizes up to 2048 frames).
pub const MAX_FRAMES: usize = 2048;

pub struct Instance {
    synth: Synth,
    left: Box<[f32; MAX_FRAMES]>,
    right: Box<[f32; MAX_FRAMES]>,
}

impl Instance {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            synth: Synth::new(sample_rate),
            left: Box::new([0.0; MAX_FRAMES]),
            right: Box::new([0.0; MAX_FRAMES]),
        }
    }

    /// Renders `frames` frames (clamped to [`MAX_FRAMES`]) into the instance buffers.
    pub fn render(&mut self, frames: usize) -> usize {
        let frames = frames.min(MAX_FRAMES);
        self.synth
            .render(&mut self.left[..frames], &mut self.right[..frames]);
        frames
    }

    pub fn left(&self) -> &[f32] {
        &self.left[..]
    }

    pub fn right(&self) -> &[f32] {
        &self.right[..]
    }

    pub fn synth_mut(&mut self) -> &mut Synth {
        &mut self.synth
    }
}

/// Converts an exported pointer back to the instance, or `None` when null.
///
/// # Safety
/// `ptr` must be null or a pointer returned by [`fk_new`] and not yet freed.
unsafe fn instance<'a>(ptr: *mut Instance) -> Option<&'a mut Instance> {
    // SAFETY: the caller guarantees `ptr` is null or a live pointer from `fk_new`;
    // the worklet calls the exports from a single thread, so no alias exists.
    unsafe { ptr.as_mut() }
}

/// Creates an engine instance. Allocates; call it once, outside the render loop.
#[unsafe(no_mangle)]
pub extern "C" fn fk_new(sample_rate: f32) -> *mut Instance {
    Box::into_raw(Box::new(Instance::new(sample_rate)))
}

/// Frees an instance created by [`fk_new`].
///
/// # Safety
/// `ptr` must be null or a pointer returned by [`fk_new`], freed at most once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_free(ptr: *mut Instance) {
    if !ptr.is_null() {
        // SAFETY: the caller guarantees `ptr` came from `fk_new` and is freed once.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

/// Renders `frames` frames; returns the number rendered (0 for a null instance).
///
/// # Safety
/// `ptr` must be null or a live pointer returned by [`fk_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_render(ptr: *mut Instance, frames: u32) -> u32 {
    // SAFETY: forwarded from this function's contract.
    match unsafe { instance(ptr) } {
        Some(inst) => inst.render(frames as usize) as u32,
        None => 0,
    }
}

/// Address of the left output buffer ([`MAX_FRAMES`] floats), null for a null instance.
///
/// # Safety
/// `ptr` must be null or a live pointer returned by [`fk_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_left_ptr(ptr: *mut Instance) -> *const f32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { instance(ptr) }.map_or(core::ptr::null(), |i| i.left.as_ptr())
}

/// Address of the right output buffer ([`MAX_FRAMES`] floats), null for a null instance.
///
/// # Safety
/// `ptr` must be null or a live pointer returned by [`fk_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_right_ptr(ptr: *mut Instance) -> *const f32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { instance(ptr) }.map_or(core::ptr::null(), |i| i.right.as_ptr())
}

/// Starts a note (MIDI note number and velocity, both clamped to 0–127).
///
/// # Safety
/// `ptr` must be null or a live pointer returned by [`fk_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_note_on(ptr: *mut Instance, note: u32, velocity: u32) {
    // SAFETY: forwarded from this function's contract.
    if let Some(inst) = unsafe { instance(ptr) } {
        inst.synth
            .note_on(note.min(127) as u8, velocity.min(127) as u8);
    }
}

/// Releases a note.
///
/// # Safety
/// `ptr` must be null or a live pointer returned by [`fk_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_note_off(ptr: *mut Instance, note: u32) {
    // SAFETY: forwarded from this function's contract.
    if let Some(inst) = unsafe { instance(ptr) } {
        inst.synth.note_off(note.min(127) as u8);
    }
}

/// Releases every note.
///
/// # Safety
/// `ptr` must be null or a live pointer returned by [`fk_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_all_notes_off(ptr: *mut Instance) {
    // SAFETY: forwarded from this function's contract.
    if let Some(inst) = unsafe { instance(ptr) } {
        inst.synth.all_notes_off();
    }
}

/// Sets a parameter by its id (see `engine::Param::id`). Unknown ids are ignored.
///
/// # Safety
/// `ptr` must be null or a live pointer returned by [`fk_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_set_param(ptr: *mut Instance, id: u32, value: f32) {
    // SAFETY: forwarded from this function's contract.
    if let (Some(inst), Some(param)) = (unsafe { instance(ptr) }, Param::from_id(id)) {
        inst.synth.set_param(param, value);
    }
}

/// Number of voices producing sound (for the UI's activity display).
///
/// # Safety
/// `ptr` must be null or a live pointer returned by [`fk_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fk_active_voices(ptr: *mut Instance) -> u32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { instance(ptr) }.map_or(0, |i| i.synth.active_voices() as u32)
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
