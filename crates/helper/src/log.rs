//! The helper's journal: one line per event, to a file in the config directory and to
//! stderr. A release build on Windows has no console, so the file is where its messages
//! go.
//!
//! Never call it from the audio callback: it locks and does I/O.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Above this size the log is moved to `helper.old.log` when the helper starts.
pub const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// Bytes one run may add to the file. Any page in the browser can make the helper log
/// (refused connections), so the file stops growing past this point until a restart.
pub const MAX_RUN_BYTES: u64 = 4 * 1024 * 1024;
const FULL_LINE: &str = "log full: further lines go to stderr only until the helper restarts";

static FILE: OnceLock<Mutex<File>> = OnceLock::new();
static PATH: OnceLock<PathBuf> = OnceLock::new();
static WRITTEN: AtomicU64 = AtomicU64::new(0);

/// Opens `<dir>/helper.log` for appending (after rotating it if too large, when
/// `rotate_first` is set) and logs panics. Without it, lines only go to stderr.
pub fn init(dir: &Path, rotate_first: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("helper.log");
    if rotate_first {
        rotate(&path, MAX_LOG_BYTES)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let _ = FILE.set(Mutex::new(file));
    let _ = PATH.set(path);
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_line(&format!("panic: {info}"));
        default_hook(info);
    }));
    Ok(())
}

/// Path of the log file, once [`init`] succeeded.
pub fn path() -> Option<&'static Path> {
    PATH.get().map(PathBuf::as_path)
}

/// Moves `path` to `helper.old.log` beside it when it is larger than `max` bytes,
/// replacing an older one (`rename` replaces an existing target on every platform).
pub fn rotate(path: &Path, max: u64) -> std::io::Result<()> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max => {
            std::fs::rename(path, path.with_file_name("helper.old.log"))
        }
        _ => Ok(()),
    }
}

/// Writes one line, prefixed with the time in seconds since the Unix epoch (UTC).
pub fn write_line(message: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{secs}] {message}");
    eprintln!("{line}");
    if let Some(file) = FILE.get()
        && let Ok(mut file) = file.lock()
    {
        match budget(&WRITTEN, line.len() as u64 + 1, MAX_RUN_BYTES) {
            Budget::Write => {
                let _ = writeln!(file, "{line}");
            }
            Budget::Last => {
                let _ = writeln!(file, "{FULL_LINE}");
            }
            Budget::Full => {}
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Budget {
    Write,
    /// The line would cross the limit: write the "log full" line instead, once.
    Last,
    Full,
}

/// Accounts for `len` more bytes against `max`.
fn budget(written: &AtomicU64, len: u64, max: u64) -> Budget {
    let before = written.fetch_add(len, Ordering::Relaxed);
    if before + len <= max {
        Budget::Write
    } else if before <= max {
        Budget::Last
    } else {
        Budget::Full
    }
}

/// Logs a formatted line: `log!("listening on {port}")`.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write_line(&format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_writing_past_the_run_budget_with_one_last_line() {
        let written = AtomicU64::new(0);
        assert_eq!(budget(&written, 6, 10), Budget::Write);
        assert_eq!(budget(&written, 4, 10), Budget::Write);
        assert_eq!(budget(&written, 1, 10), Budget::Last);
        assert_eq!(budget(&written, 1, 10), Budget::Full);
        assert_eq!(budget(&written, 1000, 10), Budget::Full);
    }

    #[test]
    fn rotates_only_above_the_limit() {
        let dir = std::env::temp_dir().join(format!("faulk-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("helper.log");
        let old = dir.join("helper.old.log");
        let _ = std::fs::remove_file(&old);

        std::fs::write(&log, b"0123456789").unwrap();
        rotate(&log, 10).unwrap();
        assert!(log.exists() && !old.exists(), "at the limit: kept");

        std::fs::write(&log, b"0123456789A").unwrap();
        rotate(&log, 10).unwrap();
        assert!(!log.exists() && old.exists(), "above the limit: moved");

        std::fs::write(&log, b"second run, above").unwrap();
        rotate(&log, 10).expect("a second rotation replaces the old file");
        assert_eq!(std::fs::read(&old).unwrap(), b"second run, above");
        assert!(!log.exists());

        rotate(&log, 10).unwrap(); // missing file: nothing to do
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
