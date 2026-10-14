//! Faulk project document.
//!
//! The page owns the project in both modes (web and Boost); the helper relays and never
//! owns it. This crate is pure Rust with no I/O. It holds the musical time contract
//! (re-exported from `engine::time`, CT-2) and is otherwise an empty skeleton for now.

/// Musical time: the contract shared by the project and the engine (`engine::time`).
pub use engine::time;
pub use engine::time::{Frame, LoopRegion, PPQ, Quantize, SampleRate, Tempo, Tick, TimeSignature};
