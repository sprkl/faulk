//! Faulk native helper: the local bridge for Boost mode.

pub mod agents;
pub mod audio;
pub mod disk;
pub mod instance;
pub mod live;
pub mod log;
pub mod mcp;
pub mod mcp_stdio;
pub mod midi;
pub mod output;
pub mod security;
pub mod server;
#[cfg(windows)]
pub mod tray;
pub mod tray_text;
pub mod updater;

/// A debug build reads its config folder from this variable when it is set, so that a test can
/// run the binary without touching the user's folder. A release build ignores it.
pub const CONFIG_DIR_ENV: &str = "FAULK_HELPER_CONFIG_DIR";

/// The helper's folder in the user's config directory (`%APPDATA%\faulk` on Windows): the
/// journal, the session token and the instance lock.
pub fn config_dir() -> std::path::PathBuf {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV).filter(|d| !d.is_empty()) {
        return std::path::PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("faulk")
}

/// The library's own tests run under the allocation detector too (the binary installs it
/// in `main.rs`): without it, `assert_no_alloc` around `AudioCore::process` could not fail.
#[cfg(all(test, debug_assertions))]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;
