//! Format versions and migrations (§ 2.6). Faulk reads every version it has a migration
//! for and writes only the current one; a newer version is refused.
//!
//! A migration is a pure function on the text files of the folder held in memory (the
//! blobs are never touched: they are named by their content). The host saves a migrated
//! project as a version of its own, so that the diff of the migration is not mixed with
//! musical changes.

use super::error::FileError;
use super::fs::MemFs;

/// One step: the folder in format `from` becomes the folder in format `from + 1`. The
/// function must also set `format` in `project.json`.
#[derive(Clone, Copy)]
pub struct Migration {
    pub from: u32,
    pub apply: fn(&mut MemFs) -> Result<(), String>,
}

/// The migrations of this build, one per format below [`crate::FORMAT_VERSION`]. Format 1
/// is the first: there is nothing to migrate from.
pub const MIGRATIONS: &[Migration] = &[];

/// Applies the steps from `found` up to `target`. A step that is not in `table` is an
/// unsupported format; a step that fails is a typed error, and `mem` must then be dropped.
pub fn migrate(
    table: &[Migration],
    found: u32,
    target: u32,
    mem: &mut MemFs,
) -> Result<(), FileError> {
    let mut version = found;
    while version < target {
        let step = table
            .iter()
            .find(|m| m.from == version)
            .ok_or(FileError::UnsupportedFormat { found })?;
        (step.apply)(mem).map_err(|message| FileError::Migration {
            from: version,
            message,
        })?;
        version += 1;
    }
    Ok(())
}
