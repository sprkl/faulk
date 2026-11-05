//! `.faulkpack` for the page (SV-3, #617): the zip of a project folder, built and read by
//! `project::file::{PackWriter, import_pack}` so that the archive the page makes is the one
//! every host reads, and the strict import (paths of the format only, every length counted as
//! it is produced, every blob checked against its name, the result opened and validated) is
//! the one rule for a pack that comes from anyone.
//!
//! Import is stateless: the zip in the inbox, the save frame's layout out at `pw_out` (status
//! `0`, revision `0`, the files as write entries; see `save.rs`), or `1` and the message.
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

use project::file::{FileSystem, MemFs, PackLimits, PackWriter, import_pack, plan_pack};

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
