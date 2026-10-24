//! Spike #349 (epic #348, Forge): the DSP building blocks of a flagship subtractive synth,
//! prototyped outside the engine, with three character sketches and their cost.
//!
//! Not a device: no descriptor, no parameter contract, no commands. The report
//! (`docs/spikes/349-forge-character.md`) says what the Forge tasks keep from it.
//!
//! The audio path ([`synth::Synth::render`] and below) never allocates, locks or panics on
//! its inputs: buffers are sized at construction, values are clamped, filter states flushed
//! below 1e-15. `cargo test` checks it under the allocation detector.

pub mod analysis;
pub mod bench;
pub mod chorus;
pub mod env;
pub mod filter;
pub mod halfband;
pub mod math;
pub mod osc;
pub mod song;
pub mod synth;
pub mod voice;

#[cfg(target_arch = "wasm32")]
mod wasm;
