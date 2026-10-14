//! Faulk native helper: the local bridge for Boost mode.

pub mod audio;
pub mod instance;
pub mod log;
pub mod midi;
pub mod security;
pub mod server;
#[cfg(windows)]
pub mod tray;
pub mod tray_text;
pub mod updater;

/// The library's own tests run under the allocation detector too (the binary installs it
/// in `main.rs`): without it, `assert_no_alloc` around `AudioCore::process` could not fail.
#[cfg(all(test, debug_assertions))]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;
