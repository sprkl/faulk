//! `.faulkpack`: a zip of the project folder, for export and sharing only (§ 2.1), never
//! the working format.
//!
//! The archive holds the project's files (not `.faulk/`, not git's), and the blobs the
//! project refers to (not the ones left over by old versions). It is deterministic: the
//! entries are sorted, their dates fixed. Import is strict, since a pack comes from
//! anyone: only the paths of the format, no entry twice, every length counted as it is
//! produced (a header that lies about its size is no bomb), every blob checked against its
//! name, and the result opened and validated; on any failure nothing is left behind.

use super::MAX_TEXT_FILE_BYTES;
use super::error::FileError;
use super::fs::FileSystem;
use super::read::{OpenOptions, Opened, open, referenced_blobs, text_path};
use super::unit::{GITATTRIBUTES_FILE, GITIGNORE_FILE, PROJECT_FILE, blob_file, sha256};
use crate::blob::Hash;
use std::collections::BTreeSet;
use std::io::{Read, Seek, Write};
use zip::write::{SimpleFileOptions, StreamWriter};
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

/// What an import accepts.
#[derive(Clone, Copy, Debug)]
pub struct PackLimits {
    pub max_entries: usize,
    /// The most bytes of a text file.
    pub max_text: u64,
    /// The most bytes of a blob.
    pub max_blob: u64,
    /// The most bytes of everything.
    pub max_total: u64,
}

impl Default for PackLimits {
    fn default() -> Self {
        Self {
            max_entries: 1_000_000,
            max_text: MAX_TEXT_FILE_BYTES,
            max_blob: 2 << 30,
            max_total: 16 << 30,
        }
    }
}

/// What an export did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackReport {
    pub files: usize,
    /// Blobs the project refers to that the folder does not have: not in the pack.
    pub missing_blobs: Vec<Hash>,
}

fn bad(message: impl std::fmt::Display) -> FileError {
    FileError::BadPack {
        message: message.to_string(),
    }
}

/// Whether a path may appear in a pack.
fn pack_path(path: &str) -> bool {
    if path == PROJECT_FILE || path == GITATTRIBUTES_FILE || path == GITIGNORE_FILE {
        return true;
    }
    if let Some(hex) = path.strip_prefix("blobs/") {
        return Hash::parse(hex).is_ok();
    }
    text_path(path)
}

/// What an export will put in the pack, found before any byte is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackPlan {
    /// The files of the pack, in the order they are written (sorted).
    pub paths: Vec<String>,
    /// Blobs the project refers to that the folder does not have: not in the pack.
    pub missing_blobs: Vec<Hash>,
}

/// Opens (and so validates) the project in `fs` and lists the files its pack holds. An
/// invalid project is not exported. Reads the project's own files, never a blob's bytes: a
/// blob only has to exist (`FileSystem::exists`), so `fs` may know a blob by its name and size
/// alone.
pub fn plan_pack(fs: &dyn FileSystem) -> Result<PackPlan, FileError> {
    let opened = open(fs, &OpenOptions::default())?;
    let mut paths: BTreeSet<String> = opened.state.known_files().map(str::to_owned).collect();
    for support in [GITATTRIBUTES_FILE, GITIGNORE_FILE] {
        if fs.exists(support) {
            paths.insert(support.to_owned());
        }
    }
    let mut blobs: BTreeSet<Hash> = referenced_blobs(&opened.project);
    blobs.extend(opened.state.blob_lanes.values().copied());
    let mut missing = Vec::new();
    for hash in blobs {
        if fs.exists(&blob_file(&hash)) {
            paths.insert(blob_file(&hash));
        } else {
            missing.push(hash);
        }
    }
    Ok(PackPlan {
        paths: paths.into_iter().collect(),
        missing_blobs: missing,
    })
}

/// A file of this size or more is written with the 64-bit zip fields.
const LARGE_FILE_BYTES: u64 = 1 << 30;

/// Writes a pack one file at a time, in whatever pieces they come in, to a writer that need not
/// seek: the memory it holds is the compressor's buffers and the central directory, whatever the
/// size of the files. The bytes are the same for the same files, whatever the pieces.
pub struct PackWriter<W: Write> {
    zip: ZipWriter<StreamWriter<W>>,
}

impl<W: Write> PackWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            zip: ZipWriter::new_stream(out),
        }
    }

    /// Starts the file `path`, whose `size` the caller knows (the next [`write`](Self::write)s
    /// add up to it). The previous file is closed.
    pub fn start(&mut self, path: &str, size: u64) -> Result<(), FileError> {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(DateTime::default())
            .unix_permissions(0o644)
            .large_file(size >= LARGE_FILE_BYTES);
        self.zip.start_file(path, options).map_err(bad)
    }

    /// Adds `bytes` to the file started last.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), FileError> {
        self.zip.write_all(bytes).map_err(bad)
    }

    /// Closes the last file, writes the directory and gives the writer back.
    pub fn finish(self) -> Result<W, FileError> {
        let out = self.zip.finish().map_err(bad)?;
        Ok(out.into_inner())
    }
}

/// Writes the pack of the project in `fs` into `out` (see [`plan_pack`]), reading one file at
/// a time.
pub fn export_pack<W: Write>(fs: &dyn FileSystem, out: W) -> Result<PackReport, FileError> {
    let plan = plan_pack(fs)?;
    let mut pack = PackWriter::new(out);
    for path in &plan.paths {
        let bytes = fs.read(path).map_err(|e| FileError::io(path, e))?;
        pack.start(path, bytes.len() as u64)?;
        pack.write(&bytes)?;
    }
    pack.finish()?;
    Ok(PackReport {
        files: plan.paths.len(),
        missing_blobs: plan.missing_blobs,
    })
}

/// Reads a pack into the empty folder `dest` and opens it. On any failure the files
/// written are removed again.
pub fn import_pack<R: Read + Seek>(
    input: R,
    dest: &mut dyn FileSystem,
    limits: &PackLimits,
) -> Result<Opened, FileError> {
    if !dest.list("").map_err(|e| FileError::io("", e))?.is_empty() {
        return Err(FileError::DestinationNotEmpty);
    }
    let mut zip = ZipArchive::new(input).map_err(bad)?;
    if zip.len() > limits.max_entries {
        return Err(bad("too many entries"));
    }
    let mut written: Vec<String> = Vec::new();
    let result = extract(&mut zip, dest, limits, &mut written)
        .and_then(|()| open(dest, &OpenOptions::default()));
    if result.is_err() {
        for path in &written {
            let _ = dest.remove(path);
        }
    }
    result
}

fn extract<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    dest: &mut dyn FileSystem,
    limits: &PackLimits,
    written: &mut Vec<String>,
) -> Result<(), FileError> {
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let file = zip.by_index(i).map_err(bad)?;
        let name = file.name().map_err(bad)?.into_owned();
        if file.is_dir() {
            continue;
        }
        if !pack_path(&name) {
            return Err(bad(format!("{name:?} is not a file of a Faulk project")));
        }
        if !seen.insert(name.clone()) {
            return Err(bad(format!("{name:?} is there twice")));
        }
        let max = if name.starts_with("blobs/") {
            limits.max_blob
        } else {
            limits.max_text
        };
        if file.size() > max {
            return Err(bad(format!("{name:?} is bigger than {max} bytes")));
        }
        // The length that counts is the one produced, not the one declared.
        let mut bytes = Vec::new();
        file.take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| bad(format!("{name:?}: {e}")))?;
        if bytes.len() as u64 > max {
            return Err(bad(format!("{name:?} is bigger than {max} bytes")));
        }
        total += bytes.len() as u64;
        if total > limits.max_total {
            return Err(bad("the pack is too big"));
        }
        if let Some(hex) = name.strip_prefix("blobs/")
            && sha256(&bytes).to_string() != hex
        {
            return Err(bad(format!("{name:?} is not the blob its name says")));
        }
        dest.write(&name, &bytes)
            .map_err(|e| FileError::io(&name, e))?;
        written.push(name);
    }
    Ok(())
}
