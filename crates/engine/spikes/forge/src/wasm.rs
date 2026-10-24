//! Plain C ABI exports for the WASM measurement (`bench/wasm.mjs` under Node): the same
//! workload as the native bench, and the sketches' renders for the native/WASM comparison.
//! The module needs no import. A bench driver, single-threaded, not a product module.

use std::cell::RefCell;

use crate::bench;
use crate::song::{Performance, SKETCHES};
use crate::synth::{Oversample, Synth};

thread_local! {
    static SYNTH: RefCell<Option<Synth>> = const { RefCell::new(None) };
    static RENDER: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// Prepares the bench synth: `variant` as [`bench::variant`], `voices` notes held.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_prepare(variant: u32, voices: u32) {
    let synth = bench::prepare(variant, voices as usize);
    SYNTH.with(|s| *s.borrow_mut() = Some(synth));
}

/// Renders `seconds` on the prepared synth; returns the checksum.
#[unsafe(no_mangle)]
pub extern "C" fn fk_bench_run(seconds: f32) -> f32 {
    SYNTH.with(|s| {
        s.borrow_mut()
            .as_mut()
            .map_or(0.0, |s| bench::run(s, seconds))
    })
}

/// Renders sketch `index` (0 a, 1 b, 2 c) at 2×; the interleaved stereo render is then at
/// [`fk_render_ptr`], [`fk_render_len`] floats.
#[unsafe(no_mangle)]
pub extern "C" fn fk_render(index: u32) {
    let sketch = &SKETCHES[(index as usize).min(2)];
    let (l, r) = Performance::new(sketch, bench::FS, Oversample::X2, 349).render();
    let mut out = Vec::with_capacity(l.len() * 2);
    for (a, b) in l.iter().zip(&r) {
        out.push(*a);
        out.push(*b);
    }
    RENDER.with(|v| *v.borrow_mut() = out);
}

#[unsafe(no_mangle)]
pub extern "C" fn fk_render_ptr() -> *const f32 {
    RENDER.with(|v| v.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn fk_render_len() -> u32 {
    RENDER.with(|v| v.borrow().len() as u32)
}
