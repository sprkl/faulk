//! `.faulkpack` for the page (SV-3, #617): the zip of a project folder, built and read by
//! `project::file::{PackWriter, import_pack}` so that the archive the page makes is the one
//! every host reads, and the strict import (paths of the format only, every length counted as
//! it is produced, every blob checked against its name, the result opened and validated) is
//! the one rule for a pack that comes from anyone.
//!
//! Import of a whole pack is stateless: the zip in the inbox, the save frame's layout out at
//! `pw_out` (status `0`, revision `0`, the files as write entries; see `save.rs`), or `1` and the
//! message. The page uses it only where the browser keeps no projects (the project then lives in
//! the tab's memory anyway).
//!
//! Import in pieces (#682) is a session too, apart from the export's, run by the store's Worker
//! on a pack of any size: the reader (`project::file::PackReader`, the one rule for a pack) asks
//! for ranges of the pack, the Worker reads them from the file and writes the blobs the session
//! hands it piece by piece; the project's own files are kept and come back at the end, once the
//! project they make is opened and valid. Every answer starts with a status byte:
//!
//! - [`unpack_begin`] and [`unpack_step`]: `0`, `u64` offset, `u32` length: the bytes of the
//!   pack the next step takes; `2`, `u64` size, `u32` path length, the path: a blob starts;
//!   `3` and bytes of that blob; `4`: the blob is whole and checked; `5`, `u64` bytes of the
//!   blobs as the directory declares them, and the save frame of the project's own files: the
//!   pack is read and its project valid; or `1` and the message.
//!   A failure, the end and [`unpack_cancel`] end the session.
//! - [`unpack_texts`]: the same `5` answer for the project's own files read so far (they come
//!   before the blobs), unchecked; the session goes on. The session keeps the project's own files
//!   and its lane blobs, `MAX_UNPACK_KEPT` bytes at most, never another blob.

//!
//! Export is a session that writes the pack file by file, so the memory it holds does not
//! depend on the size of the project: the page gives the folder (a folder frame, see
//! [`crate::parse_folder`]: the project's own files with their bytes, the blobs by name and
//! size), the session validates it and plans the pack, then the page asks for the zip piece by
//! piece and reads each listed blob in chunks from its store. Every answer starts with a status
//! byte, then the zip bytes written so far (the session keeps none):
//!
//! - [`begin`]: `0`, `u32` files in the pack, `u32` blobs the project refers to that the folder
//!   lacks (not in the pack); or `1` and the UTF-8 message.
//! - [`next`]: the files with their bytes in the frame are written; at the first file the page
//!   must supply, `2`, `u32` path length, the path (UTF-8), the zip bytes; `3` and the last zip
//!   bytes when the pack is whole; or `1` and the message. A supplied file stays open until the
//!   next call, which checks that it got exactly its size.
//! - [`feed`]: the next bytes of the open file; `0` and the zip bytes, or `1` and the message.
//!
//! A failure ends the session, and so does [`cancel`].

use std::cell::RefCell;
use std::io::{self, Cursor, Write};
use std::rc::Rc;

use project::file::{
    FileSystem, LANE_BLOB_MAGIC, MAX_LANE_BLOB_BYTES, MemFs, OpenOptions, PackLimits, PackReader,
    PackStep, PackWriter, import_pack, open, plan_pack,
};

use crate::StagedFs;
use crate::parse_folder;
use crate::save::{Entry, frame};

/// What an import accepts. The page holds the pack, its files and the frame it answers with
/// in the 32-bit memory of the module, so these are far under the format's own limits.
pub(crate) fn import_limits() -> PackLimits {
    PackLimits {
        max_entries: 200_000,
        max_blob: 192 << 20,
        max_total: 384 << 20,
        ..PackLimits::default()
    }
}

/// What an export accepts: the format's own limits, since the memory it holds does not grow
/// with the project. A pack over what the page itself reads back (`MAX_PACK_BYTES` in
/// `storage.svelte.ts`) opens in the other hosts.
pub(crate) fn export_limits() -> PackLimits {
    PackLimits {
        max_entries: 200_000,
        ..PackLimits::default()
    }
}

const STATUS_OK: u8 = 0;
const STATUS_FAILED: u8 = 1;
const STATUS_SUPPLY: u8 = 2;
const STATUS_DONE: u8 = 3;

fn failure(message: impl std::fmt::Display) -> Vec<u8> {
    let mut out = vec![STATUS_FAILED];
    out.extend_from_slice(message.to_string().as_bytes());
    out
}

fn answer(status: u8, head: &[u8], zip: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + head.len() + zip.len());
    out.push(status);
    out.extend_from_slice(head);
    out.extend_from_slice(zip);
    out
}

/// The zip bytes the writer made since the last answer.
#[derive(Clone, Default)]
struct Sink(Rc<RefCell<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Sink {
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

/// A file the page supplies: its announced size and what it gave so far.
struct Supplied {
    size: u64,
    fed: u64,
}

struct Export {
    staged: StagedFs,
    paths: Vec<String>,
    next: usize,
    /// `None` once the pack is finished.
    pack: Option<PackWriter<Sink>>,
    sink: Sink,
    supplied: Option<Supplied>,
    total: u64,
}

thread_local! {
    static EXPORT: RefCell<Option<Export>> = const { RefCell::new(None) };
}

/// Starts an export of the folder in `folder_frame`, ending the one under way.
pub fn begin(folder_frame: &[u8]) -> Vec<u8> {
    cancel();
    let staged = match parse_folder(folder_frame) {
        Ok((_, staged)) => staged,
        Err(_) => return failure("the folder frame is malformed"),
    };
    let plan = match plan_pack(&staged) {
        Ok(plan) => plan,
        Err(error) => return failure(error),
    };
    if plan.paths.len() > export_limits().max_entries {
        return failure("the project has too many files to export");
    }
    let mut head = (plan.paths.len() as u32).to_le_bytes().to_vec();
    head.extend_from_slice(&(plan.missing_blobs.len() as u32).to_le_bytes());
    let sink = Sink::default();
    EXPORT.with_borrow_mut(|slot| {
        *slot = Some(Export {
            staged,
            paths: plan.paths,
            next: 0,
            pack: Some(PackWriter::new(sink.clone())),
            sink,
            supplied: None,
            total: 0,
        });
    });
    answer(STATUS_OK, &head, &[])
}

/// Ends the export under way, if any.
pub fn cancel() {
    EXPORT.with_borrow_mut(|slot| *slot = None);
}

/// Runs `f` on the session. The session is kept unless `f` failed or the pack is whole.
fn with_export(f: impl FnOnce(&mut Export) -> Result<Vec<u8>, String>) -> Vec<u8> {
    let Some(mut export) = EXPORT.with_borrow_mut(Option::take) else {
        return failure("no export is under way");
    };
    match f(&mut export) {
        Ok(out) => {
            if out[0] != STATUS_DONE {
                EXPORT.with_borrow_mut(|slot| *slot = Some(export));
            }
            out
        }
        Err(message) => failure(message),
    }
}

/// The next file the page must supply, or the end of the pack.
pub fn next() -> Vec<u8> {
    with_export(|export| {
        if let Some(open) = export.supplied.take()
            && open.fed != open.size
        {
            return Err(format!(
                "a file changed while it was exported ({} bytes given, {} announced)",
                open.fed, open.size
            ));
        }
        let limits = export_limits();
        loop {
            let Some(pack) = export.pack.as_mut() else {
                return Err("the pack is already whole".to_owned());
            };
            let Some(path) = export.paths.get(export.next).cloned() else {
                let Some(pack) = export.pack.take() else {
                    return Err("the pack is already whole".to_owned());
                };
                pack.finish().map_err(|e| e.to_string())?;
                return Ok(answer(STATUS_DONE, &[], &export.sink.take()));
            };
            export.next += 1;
            let max = if path.starts_with("blobs/") {
                limits.max_blob
            } else {
                limits.max_text
            };
            let listed = export.staged.listed_size(&path);
            let bytes = match listed {
                Some(_) => None,
                None => Some(
                    export
                        .staged
                        .read(&path)
                        .map_err(|e| format!("{path:?}: {e}"))?,
                ),
            };
            let size = listed.unwrap_or_else(|| bytes.as_ref().map_or(0, |b| b.len() as u64));
            if size > max {
                return Err(format!("{path:?} is bigger than {max} bytes"));
            }
            export.total += size;
            if export.total > limits.max_total {
                return Err("the project is too big to export".to_owned());
            }
            pack.start(&path, size).map_err(|e| e.to_string())?;
            match bytes {
                Some(bytes) => pack.write(&bytes).map_err(|e| e.to_string())?,
                None => {
                    export.supplied = Some(Supplied { size, fed: 0 });
                    let mut head = (path.len() as u32).to_le_bytes().to_vec();
                    head.extend_from_slice(path.as_bytes());
                    return Ok(answer(STATUS_SUPPLY, &head, &export.sink.take()));
                }
            }
        }
    })
}

/// The next bytes of the file the page supplies.
pub fn feed(chunk: &[u8]) -> Vec<u8> {
    with_export(|export| {
        let (Some(open), Some(pack)) = (export.supplied.as_mut(), export.pack.as_mut()) else {
            return Err("no file is waiting for bytes".to_owned());
        };
        open.fed += chunk.len() as u64;
        if open.fed > open.size {
            return Err("a file is bigger than it was announced".to_owned());
        }
        pack.write(chunk).map_err(|e| e.to_string())?;
        Ok(answer(STATUS_OK, &[], &export.sink.take()))
    })
}

/// The files of the pack `zip`, as a save frame.
pub fn import(zip: &[u8]) -> Vec<u8> {
    let mut dest = MemFs::new();
    match import_pack(Cursor::new(zip), &mut dest, &import_limits()) {
        Ok(_) => {
            let entries: Vec<Entry> = dest
                .files()
                .iter()
                .map(|(path, bytes)| Entry::Write(path.clone(), bytes.clone()))
                .collect();
            frame(0, &entries)
        }
        Err(error) => failure(error),
    }
}

// ------------------------------------------------------------------ import in pieces (#682)

const UNPACK_NEED: u8 = 0;
const UNPACK_START: u8 = 2;
const UNPACK_BYTES: u8 = 3;
const UNPACK_END: u8 = 4;
const UNPACK_DONE: u8 = 5;

/// What an import in pieces accepts: the format's limits, since the media pass through, but for
/// the project's own files, which the session keeps and gives back whole at the end, and the page
/// then stages to open the project (under `MAX_FOLDER_FRAME`).
pub(crate) fn unpack_limits() -> PackLimits {
    PackLimits {
        max_entries: 200_000,
        max_text_total: 128 << 20,
        ..PackLimits::default()
    }
}

/// The file the reader gives now.
enum Current {
    /// One of the project's own files: kept, given at the end.
    Text { path: String, bytes: Vec<u8> },
    /// A blob, handed to the page piece by piece. Its bytes are kept while it may be a blob of
    /// dense automation points (opening reads those): until its first bytes say otherwise, or
    /// when it is bigger than such a blob may be.
    Blob {
        path: String,
        size: u64,
        kept: Option<Vec<u8>>,
    },
}

struct Unpack {
    reader: PackReader,
    current: Option<Current>,
    /// The project's own files and the lane blobs with their bytes, the other blobs by size:
    /// what opening the project at the end reads.
    staged: StagedFs,
    /// The bytes of the project's own files and lane blobs kept so far, and the most it keeps.
    kept: u64,
    keep_max: u64,
}

/// The most bytes of the project's own files and lane blobs an import keeps to open the project: the
/// page stages them in one folder frame at the opening ([`crate::MAX_FOLDER_FRAME`]).
pub(crate) const MAX_UNPACK_KEPT: u64 = crate::MAX_FOLDER_FRAME as u64;

thread_local! {
    static UNPACK: RefCell<Option<Unpack>> = const { RefCell::new(None) };
}

/// Starts the import of a pack of `len` bytes, ending the one under way: `0`, `u64` offset and
/// `u32` length of the first bytes to give.
pub fn unpack_begin(len: u64) -> Vec<u8> {
    unpack_begin_keeping(len, MAX_UNPACK_KEPT)
}

/// [`unpack_begin`] keeping at most `keep_max` bytes of the project's own files and lane blobs.
pub(crate) fn unpack_begin_keeping(len: u64, keep_max: u64) -> Vec<u8> {
    unpack_cancel();
    let reader = PackReader::new(len, unpack_limits());
    let Some((offset, want)) = reader.wants() else {
        return failure("the reader wants nothing");
    };
    UNPACK.with_borrow_mut(|slot| {
        *slot = Some(Unpack {
            reader,
            current: None,
            staged: StagedFs::default(),
            kept: 0,
            keep_max,
        });
    });
    need(offset, want)
}

/// The project's own files the import under way has read so far, unchecked, as `5` and a save
/// frame (they all come before the first blob): the page learns the project of a pack (its
/// identity) without reading its media. The session goes on.
pub fn unpack_texts() -> Vec<u8> {
    UNPACK.with_borrow(|slot| match slot {
        Some(unpack) => unpack.texts(),
        None => failure("no import is under way"),
    })
}

/// Ends the import under way, if any.
pub fn unpack_cancel() {
    UNPACK.with_borrow_mut(|slot| *slot = None);
}

fn need(offset: u64, len: u64) -> Vec<u8> {
    let mut out = vec![UNPACK_NEED];
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out
}

/// The next step of the import under way, `input` being the bytes it asked for last (or
/// nothing): the bytes to give next, a blob to write (its start, its pieces, its end), or the
/// end with the project's own files as a save frame, once the project they make is opened and
/// valid. A failure ends the session.
pub fn unpack_step(input: &[u8]) -> Vec<u8> {
    let Some(mut unpack) = UNPACK.with_borrow_mut(Option::take) else {
        return failure("no import is under way");
    };
    match unpack.step(input) {
        Ok(out) => {
            if out[0] != UNPACK_DONE {
                UNPACK.with_borrow_mut(|slot| *slot = Some(unpack));
            }
            out
        }
        Err(message) => failure(message),
    }
}

impl Unpack {
    fn step(&mut self, input: &[u8]) -> Result<Vec<u8>, String> {
        let mut input = input;
        loop {
            let step = self.reader.step(input).map_err(|e| e.to_string())?;
            input = &[];
            match step {
                PackStep::Need { offset, len } => return Ok(need(offset, len)),
                PackStep::Start { path, size } => {
                    if path.starts_with("blobs/") {
                        let mut out = vec![UNPACK_START];
                        out.extend_from_slice(&size.to_le_bytes());
                        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
                        out.extend_from_slice(path.as_bytes());
                        let kept = (size <= MAX_LANE_BLOB_BYTES).then(Vec::new);
                        self.current = Some(Current::Blob { path, size, kept });
                        return Ok(out);
                    }
                    self.current = Some(Current::Text {
                        path,
                        bytes: Vec::with_capacity(size as usize),
                    });
                }
                PackStep::Bytes(bytes) => match self.current.as_mut() {
                    Some(Current::Text { bytes: body, .. }) => {
                        body.extend_from_slice(&bytes);
                        self.keep(bytes.len())?;
                    }
                    Some(Current::Blob { kept, .. }) => {
                        if let Some(head) = kept.as_mut() {
                            head.extend_from_slice(&bytes);
                            let n = head.len().min(LANE_BLOB_MAGIC.len());
                            if head[..n] != LANE_BLOB_MAGIC[..n] {
                                *kept = None;
                            } else {
                                self.keep(bytes.len())?;
                            }
                        }
                        let mut out = Vec::with_capacity(1 + bytes.len());
                        out.push(UNPACK_BYTES);
                        out.extend_from_slice(&bytes);
                        return Ok(out);
                    }
                    None => return Err("bytes of no file".into()),
                },
                PackStep::End => match self.current.take() {
                    Some(Current::Text { path, bytes }) => {
                        self.staged.files.insert(path, bytes);
                    }
                    Some(Current::Blob { path, size, kept }) => {
                        match kept {
                            // A blob shorter than the magic is no lane blob.
                            Some(bytes) if bytes.len() >= LANE_BLOB_MAGIC.len() => {
                                self.staged.files.insert(path, bytes);
                            }
                            _ => {
                                self.staged.listed.insert(path, size);
                            }
                        }
                        return Ok(vec![UNPACK_END]);
                    }
                    None => return Err("the end of no file".into()),
                },
                PackStep::Done => {
                    // The project the files make, opened and validated as any other.
                    open(&self.staged, &OpenOptions::default()).map_err(|e| e.to_string())?;
                    return Ok(self.texts());
                }
            }
        }
    }

    /// Counts `n` more bytes kept, within the bound.
    fn keep(&mut self, n: usize) -> Result<(), String> {
        self.kept += n as u64;
        if self.kept > self.keep_max {
            return Err(
                "the project's own files and automation are too big to open in the page".into(),
            );
        }
        Ok(())
    }

    /// `5`, `u64` bytes of the blobs (as the directory declares them), and the save frame of the
    /// project's own files read so far.
    fn texts(&self) -> Vec<u8> {
        let entries: Vec<Entry> = self
            .staged
            .files
            .files()
            .iter()
            .filter(|(path, _)| !path.starts_with("blobs/"))
            .map(|(path, bytes)| Entry::Write(path.clone(), bytes.clone()))
            .collect();
        let mut out = vec![UNPACK_DONE];
        out.extend_from_slice(&self.reader.blob_bytes().to_le_bytes());
        out.extend_from_slice(&frame(0, &entries));
        out
    }
}
