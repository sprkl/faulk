//! Faulk audio engine core.
//!
//! Pure DSP: no I/O, no allocation after construction, no panic on any input.
//! The same code runs natively in the helper and as WebAssembly in an `AudioWorklet`.

pub mod arrangement;
pub mod audio_source;
pub mod automation;
pub mod clip_player;
pub mod device;
pub mod devices;
pub mod dsp;
pub mod time;

pub use devices::osc_synth::{DESCRIPTOR as OSC_SYNTH, MAX_VOICES, Param, Synth, Waveform};
pub mod engine;
pub mod events;
pub mod fader;
pub mod graph;
pub mod host;
pub mod latency;
pub mod launcher;
pub mod limiter;
pub mod metronome;
pub mod mixer;
pub mod msg;
pub mod offline;
pub mod param;
pub mod recorder;
pub mod transport;
pub mod warm;
