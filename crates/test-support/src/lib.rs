//! The offline project render harness (QA-1).
//!
//! A test builds a project through commands ([`Session`]), which records the engine's
//! message stream and the blocks to render as a [`Scenario`]; renders it natively
//! ([`render_native`]) and in the WASM module ([`render_wasm`], [`parity`]); then asserts
//! on the [`Render`]: level, silence, spectrum, onset time, NaN and denormals, and writes
//! a WAV file for listening. Reference projects live in [`reference`], recorded in
//! `tests/fixtures/`. How to use it: `docs/agents/verification.md`.

mod render;
mod scenario;
mod signal;
mod wasm;

pub mod reference;

pub use render::{PARITY_TOLERANCE, QUANTUM, SAMPLE_RATE, parity, render_native};
pub use scenario::{Scenario, Session, Step};
pub use signal::{Render, Spectrum};
pub use wasm::render_wasm;
