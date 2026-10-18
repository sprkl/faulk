// demo: no-op comment
//! Faulk audio engine core.
//!
//! Pure DSP: no I/O, no allocation after construction, no panic on any input.
//! The same code runs natively in the helper and as WebAssembly in an `AudioWorklet`.

pub mod device;
pub mod devices;
pub mod time;

pub use devices::osc_synth::{DESCRIPTOR as OSC_SYNTH, MAX_VOICES, Param, Synth, Waveform};
pub mod engine;
pub mod events;
pub mod graph;
pub mod host;
pub mod metronome;
pub mod msg;
pub mod transport;
