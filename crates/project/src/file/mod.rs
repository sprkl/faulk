//! The folder format of a project (project model § 2, decision 2): the canonical writer
//! and reader, the dirty-file save, the cache snapshot, the session file and the
//! `.faulkpack` archive.
//!
//! Pure like the rest of the crate: every byte goes through the [`FileSystem`] trait, so
//! one implementation serves a native folder ([`StdFs`]), the browser's OPFS and a folder
//! the user picked (the hosts of SV-2 and SV-4 implement the trait), and the tests
//! ([`MemFs`]).
//!
//! ```text
//! open(fs) -> Opened { project, state, report }     reads, migrates, validates, or refuses
//! save(fs, project, state, dirty, options)          writes only what changed
//! export_pack(fs, out) / import_pack(in, fs)        .faulkpack
//! load_session(fs, project) / save_session(fs, s)   .faulk/session.json
//! ```
//!
//! What is *not* promised: an interrupted save across several files. Each file is
//! replaced atomically by the file system, the files are written in dependency order
//! (devices and clips before what refers to them) and removals come last, but a crash
//! between two files can leave a mix of old and new files. [`open`] reads the mix when it
//! keeps the invariants of the project (the half of an edit that made it to disk) and
//! refuses it with a typed error ([`FileError::Invalid`]) when it does not.

mod cache;
pub(crate) mod canon;
mod dto;
mod error;
mod fs;
mod migrate;
mod pack;
mod read;
mod session_file;
mod text;
mod unit;
mod write;

pub use cache::CacheUse;
pub use canon::fast_hash;
pub use error::FileError;
#[cfg(not(target_arch = "wasm32"))]
pub use fs::StdFs;
pub use fs::{DirEntry, FileSystem, FsError, MemFs, Op, walk};
pub use migrate::{MIGRATIONS, Migration, migrate};
pub use pack::{
    PACK_PIECE_BYTES, PackLimits, PackPlan, PackReader, PackReport, PackStep, PackWriter,
    export_pack, import_pack, plan_pack,
};
pub use read::{OpenOptions, OpenReport, Opened, open, open_with, referenced_blobs};
pub use session_file::{SessionLoad, load_session, save_session};
pub use unit::{Layout, Unit};
pub use write::{Dirty, GITATTRIBUTES, GITIGNORE, SaveOptions, SaveReport, SaveState, save};

/// The most bytes of a text file (a JSON file, a TSV file) the reader accepts. A clip of a
/// million notes is about 25 MB; anything bigger is not a project.
pub const MAX_TEXT_FILE_BYTES: u64 = 64 << 20;

/// The most bytes of a blob of dense automation points: its magic, then the points of a lane
/// at most. The reader reads such a blob when it opens the project; a bigger one is refused.
pub const MAX_LANE_BLOB_BYTES: u64 = (text::LANE_BLOB_MAGIC.len()
    + crate::limits::MAX_POINTS_PER_LANE * text::POINT_BYTES) as u64;

pub use text::LANE_BLOB_MAGIC;
