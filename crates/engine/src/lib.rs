//! Faulk audio engine core.
//!
//! Pure DSP: no I/O, no allocation after construction, no panic on any input.
//! The same code runs natively in the helper and as WebAssembly in an `AudioWorklet`.

pub mod device;
mod synth;
pub mod time;

pub use synth::{DESCRIPTOR as OSC_SYNTH, MAX_VOICES, Param, Synth, Waveform};
pub mod engine;
pub mod graph;
pub mod msg;
pub mod transport;
