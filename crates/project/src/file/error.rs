//! The typed errors of the folder format. Reading or writing a folder never panics: a
//! file that is not what the format says is one of these.

use crate::validate::Violation;
use std::fmt;

/// Why a folder was refused, or could not be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileError {
    /// There is no `project.json`: the folder is not a Faulk project.
    NotAProject,
    /// The project was written by a newer Faulk: this build refuses it, and never
    /// rewrites it (it would lose what it does not know).
    NewerFormat { found: u32, supported: u32 },
    /// A format this build has no migration for (zero, or an older one that was dropped).
    UnsupportedFormat { found: u32 },
    /// A file the format requires is missing.
    Missing { path: String },
    /// A file is not what the format says: bad text, bad number, bad header, a count that
    /// does not add up, a blob whose hash is not its name.
    Corrupt { path: String, message: String },
    /// A file is bigger than the safety limit ([`super::MAX_TEXT_FILE_BYTES`] for text).
    TooLarge { path: String, size: u64, max: u64 },
    /// The files read fine but the project breaks its invariants ([`crate::Project::validate`]).
    Invalid { violations: Vec<Violation> },
    /// The project has a value the format cannot write (a non-finite number, a tab in a
    /// parameter id). Never happens to a valid project.
    Unwritable { path: String, message: String },
    /// The file system failed.
    Io { path: String, message: String },
    /// A migration failed.
    Migration { from: u32, message: String },
    /// A `.faulkpack` that is not a Faulk archive, or is unsafe.
    BadPack { message: String },
    /// An import wants an empty destination.
    DestinationNotEmpty,
}

impl FileError {
    pub(crate) fn corrupt(path: &str, message: impl fmt::Display) -> Self {
        Self::Corrupt {
            path: path.to_owned(),
            message: message.to_string(),
        }
    }

    pub(crate) fn io(path: &str, error: impl fmt::Display) -> Self {
        Self::Io {
            path: path.to_owned(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAProject => f.write_str("not a Faulk project: no project.json"),
            Self::NewerFormat { found, supported } => write!(
                f,
                "this project is in format {found}, newer than the {supported} this version reads"
            ),
            Self::UnsupportedFormat { found } => {
                write!(f, "no way to read a project in format {found}")
            }
            Self::Missing { path } => write!(f, "{path}: missing"),
            Self::Corrupt { path, message } => write!(f, "{path}: {message}"),
            Self::TooLarge { path, size, max } => {
                write!(f, "{path}: {size} bytes, more than the {max} allowed")
            }
            Self::Invalid { violations } => {
                write!(
                    f,
                    "the project is invalid: {} violation(s)",
                    violations.len()
                )?;
                if let Some(first) = violations.first() {
                    write!(f, ", first: {first}")?;
                }
                Ok(())
            }
            Self::Unwritable { path, message } => {
                write!(f, "{path}: cannot be written: {message}")
            }
            Self::Io { path, message } => write!(f, "{path}: {message}"),
            Self::Migration { from, message } => {
                write!(f, "migration from format {from} failed: {message}")
            }
            Self::BadPack { message } => write!(f, "bad .faulkpack: {message}"),
            Self::DestinationNotEmpty => f.write_str("the destination folder is not empty"),
        }
    }
}

impl std::error::Error for FileError {}
