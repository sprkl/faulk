//! `.faulkpack` for the page (SV-3): the zip of a project folder, built and read by
//! `project::file::{export_pack, import_pack}` so that the archive the page makes is the one
//! every host reads, and the strict import (paths of the format only, every length counted as
//! it is produced, every blob checked against its name, the result opened and validated) is
//! the one rule for a pack that comes from anyone.
//!
//! Both calls are stateless: no core, bytes in through the inbox, bytes out at `pw_out`.
//!
//! - Export takes a folder frame ([`crate::parse_folder`]) whose every entry carries its bytes
//!   and answers a status byte `0`, `u32` files in the pack, `u32` blobs the project refers to
//!   that the frame lacks (not in the pack), then the zip; or `1` and the UTF-8 message.
//! - Import takes the zip and answers the save frame's layout (status `0`, revision `0`, the
//!   files as write entries; see `save.rs`), or `1` and the message. Nothing is kept on failure.

use std::io::Cursor;

use project::file::{MemFs, PackLimits, export_pack, import_pack};

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

fn failure(message: impl std::fmt::Display) -> Vec<u8> {
    let mut out = vec![1];
    out.extend_from_slice(message.to_string().as_bytes());
    out
}

/// Whether a folder frame of `len` bytes is small enough that the pack made from it is one this
/// page imports again: a backup that the same page refuses is worse than none, so the export
/// holds to the import's total.
pub fn export_fits(len: usize) -> bool {
    (len as u64) <= import_limits().max_total
}

/// The pack of the folder in `folder_frame`.
pub fn export(folder_frame: &[u8]) -> Vec<u8> {
    if !export_fits(folder_frame.len()) {
        return failure("the project is too big to export from the page");
    }
    let staged = match parse_folder(folder_frame) {
        Ok((_, staged)) => staged,
        Err(_) => return failure("the folder frame is malformed"),
    };
    let mut zip = Cursor::new(Vec::new());
    match export_pack(&staged, &mut zip) {
        Ok(report) => {
            let mut out = vec![0];
            out.extend_from_slice(&(report.files as u32).to_le_bytes());
            out.extend_from_slice(&(report.missing_blobs.len() as u32).to_le_bytes());
            out.extend_from_slice(&zip.into_inner());
            out
        }
        Err(error) => failure(error),
    }
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
