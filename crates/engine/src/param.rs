//! The shared parameter smoother (AM-3).
//!
//! Every smoothed parameter of a device, and every level of the mixer, follows the rule
//! of the device contract ([`crate::device`], "Parameters and smoothing"): a parameter
//! declares a smoothing time ([`crate::device::ParamDescriptor::smoothing_seconds`]); a
//! value set at frame `k` is a **target**, and the value at frame `k + i` is
//! `start + (target − start) · min(i, n) / n`, with `n = round(smoothing_seconds ×
//! sample_rate)` and `start` the value at frame `k`. A linear ramp reaches its target
//! exactly `n` frames after the event, so "the declared time ± one block" holds whatever
//! the block size, and the render does not depend on how the host cuts blocks. A new target
//! during a ramp starts a new ramp from where the value is, so nothing jumps. With `n = 0`
//! the target is applied at the exact frame.
//!
//! Two ways to read a [`Smoother`]:
//!
//! - **per sample** ([`Smoother::next_value`], [`Smoother::fill`]) for a parameter that is
//!   heard sample by sample (a gain, a mix, a cutoff): this is what removes the zipper
//!   noise and the click of a jump;
//! - **per block** ([`Smoother::block`]) for a parameter whose steps cannot be heard (it
//!   feeds a coefficient computed once per block): the value at the first frame of the
//!   block, then the ramp moves by the whole block, still reaching its target at the same
//!   frame as the per-sample reading. A device that reads per block must still split the
//!   block at the events it receives (as the per-sample devices do), or a value with a zero
//!   smoothing time would land at the next block instead of at its exact offset; reading
//!   per block is for ramps of several blocks, where a block of delay cannot be heard.
//!
//! The smoother holds four numbers: it never allocates, locks or panics, and is built in a
//! `const` context. A non-finite target is ignored.

/// A linear ramp towards a target, under the contract's smoothing rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smoother {
    start: f32,
    target: f32,
    frames: u32,
    done: u32,
}

impl Smoother {
    /// A still smoother at `value`.
    pub const fn new(value: f32) -> Self {
        Smoother {
            start: value,
            target: value,
            frames: 0,
            done: 0,
        }
    }

    /// The value of the next frame, without moving.
    #[inline]
    pub fn current(&self) -> f32 {
        self.at(0)
    }

    /// The value `k` frames after the next one, without moving.
    #[inline]
    pub fn at(&self, k: u32) -> f32 {
        let i = self.done.saturating_add(k);
        if i >= self.frames {
            self.target
        } else {
            self.start + (self.target - self.start) * (i as f32 / self.frames as f32)
        }
    }

    /// The value the smoother is heading to.
    #[inline]
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Whether the ramp has reached its target.
    #[inline]
    pub fn is_still(&self) -> bool {
        self.done >= self.frames
    }

    /// The contract's rule: aims at `target` over `frames` frames from the current value
    /// (0 frames: the target at the next frame). A target set during a ramp starts a new
    /// ramp, even towards the same target. A non-finite target is ignored.
    pub fn set(&mut self, target: f32, frames: u32) {
        if !target.is_finite() {
            return;
        }
        self.start = self.current();
        self.target = target;
        self.frames = frames;
        self.done = 0;
    }

    /// Like [`Self::set`], but the same target again changes nothing: for a level that a
    /// message restates at every change (the mixer), where restarting would stretch the
    /// ramp.
    pub fn aim(&mut self, target: f32, frames: u32) {
        if target != self.target {
            self.set(target, frames);
        }
    }

    /// Sets the value at once and leaves the smoother still. A non-finite value is ignored.
    pub fn jump(&mut self, value: f32) {
        if value.is_finite() {
            *self = Smoother::new(value);
        }
    }

    /// Ends the ramp at its target ([`crate::device::Device::reset`]).
    pub fn snap(&mut self) {
        self.start = self.target;
        self.frames = 0;
        self.done = 0;
    }

    /// Moves `frames` frames on without reading them.
    pub fn advance(&mut self, frames: u32) {
        self.done = self.done.saturating_add(frames).min(self.frames);
    }

    /// The value of this frame, then one frame on.
    #[inline]
    pub fn next_value(&mut self) -> f32 {
        let value = self.current();
        if self.done < self.frames {
            self.done += 1;
        }
        value
    }

    /// Writes the next `out.len()` values, one per frame.
    pub fn fill(&mut self, out: &mut [f32]) {
        for slot in out {
            *slot = self.next_value();
        }
    }

    /// The per-block reading: the value at the first frame of a block of `frames` frames,
    /// then the smoother moves by the whole block.
    pub fn block(&mut self, frames: u32) -> f32 {
        let value = self.current();
        self.advance(frames);
        value
    }
}
