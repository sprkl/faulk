//! Faulk audio engine core.
//!
//! Pure DSP: no I/O, no allocation after construction, no panic on any input.
//! The same code runs natively in the helper and as WebAssembly in an `AudioWorklet`.

mod synth;

pub use synth::{MAX_VOICES, Param, Synth, Waveform};
