//! Saving the page's project into its folder (SV-2).
//!
//! The folder is reached through asynchronous storage (File System Access, in a Worker),
//! and `project::file::save` writes through a synchronous [`FileSystem`]. So the save runs
//! into a [`Recorder`], which keeps the writes in order, and hands them to the page as one
//! binary frame; the page applies them to the folder (one atomic file after another) and
//! tells the core how it went.
//!
//! The result of a save is *pending*: the core does not believe the folder holds the files
//! until the page says so ([`Saver::finish`]). A failed or abandoned write therefore costs
//! nothing: the next save computes the same changes again, and writing a file twice is
//! harmless. A project that was edited meanwhile stays dirty: the revision a save covers is
//! the one it read.
//!
//! Frame layout, little endian: a status byte. `0`: `u32` revision the save covers, `u32`
//! entry count, then for each entry a kind byte (`0` write: `u32` path length, the path,
//! `u64` length, the bytes; `1` remove: `u32` path length, the path). `1`: the rest is the
//! UTF-8 message of the failure (a bug: the commands keep the project valid). Entries come
//! in the order they must be applied: blobs, then the files of each unit with `project.json`
//! last, then removals, then the session.

use std::collections::BTreeSet;

use commands::contract::OpenedFolder;
use project::Project;
use project::SessionState;
use project::file::{DirEntry, FileSystem, FsError, SaveOptions, SaveState, save, save_session};

/// What the file holds, for a file whose bytes the recorder has to answer.
const SESSION_FILE: &str = ".faulk/session.json";

/// Which files a save looks at, and which it takes for granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Nothing is done when nothing changed since the last save; otherwise only the files
    /// whose bytes differ from the folder's are written.
    Changed,
    /// The same, even when nothing is known to have changed (a project edited outside a
    /// command, the session): the page's explicit save.
    Forced,
    /// The whole project, as if the folder were empty: the first write into a new folder.
    Everything,
    /// The same into a folder that is a copy: the project takes a new identity first (the
    /// project model: a save as makes a new one). The core does that before it saves.
    Copy,
    /// Every file again, into the folder the project is in, whatever the core believes it
    /// holds: for a folder edited from outside whose version the user decided to replace. What
    /// the folder is known to hold apart from the project files (support files, blobs) is left.
    Rewrite,
}

impl Mode {
    pub fn from_code(code: u32) -> Option<Mode> {
        match code {
            0 => Some(Mode::Changed),
            1 => Some(Mode::Forced),
            2 => Some(Mode::Everything),
            3 => Some(Mode::Copy),
            4 => Some(Mode::Rewrite),
            _ => None,
        }
    }
}

/// A write or a removal the folder must apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Write(String, Vec<u8>),
    Remove(String),
}

/// A [`FileSystem`] that keeps what it is asked to write. It answers `exists` and `read`
/// from what the core knows of the folder, and nothing else.
struct Recorder {
    entries: Vec<Entry>,
    blobs: BTreeSet<String>,
    root: BTreeSet<String>,
    session: Option<Vec<u8>>,
}

impl FileSystem for Recorder {
    fn read(&self, path: &str) -> Result<Vec<u8>, FsError> {
        match (path == SESSION_FILE, &self.session) {
            (true, Some(bytes)) => Ok(bytes.clone()),
            _ => Err(FsError::NotFound),
        }
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), FsError> {
        if let Some(name) = path.strip_prefix("blobs/") {
            self.blobs.insert(name.to_owned());
        } else if !path.contains('/') {
            self.root.insert(path.to_owned());
        } else if path == SESSION_FILE {
            self.session = Some(bytes.to_vec());
        }
        self.entries
            .push(Entry::Write(path.to_owned(), bytes.to_vec()));
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<(), FsError> {
        self.entries.push(Entry::Remove(path.to_owned()));
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        match path.strip_prefix("blobs/") {
            Some(name) => self.blobs.contains(name),
            None => !path.contains('/') && self.root.contains(path),
        }
    }

    fn list(&self, _dir: &str) -> Result<Vec<DirEntry>, FsError> {
        Ok(Vec::new())
    }
}

/// What a save computed and the core adopts once the folder holds it.
struct Pending {
    state: SaveState,
    blobs: BTreeSet<String>,
    root: BTreeSet<String>,
    session: Option<Vec<u8>>,
    revision: u32,
}

/// What the core knows of the folder its project is saved into, and of how far the project
/// is from it.
#[derive(Default)]
pub struct Saver {
    state: SaveState,
    blobs: BTreeSet<String>,
    root: BTreeSet<String>,
    session: Option<Vec<u8>>,
    /// Counts the edits applied to the project since the core was made or the project
    /// replaced.
    revision: u32,
    /// The revision the folder holds.
    saved: u32,
    pending: Option<Pending>,
}

impl Saver {
    /// A project that has no folder: every file is new.
    pub fn new() -> Self {
        Self::default()
    }

    /// A project just read from a folder: the folder holds exactly what was read. Without
    /// the folder's facts (`None`), nothing is known of it.
    pub fn opened(folder: Option<OpenedFolder>) -> Self {
        match folder {
            Some(folder) => Self {
                state: folder.state,
                blobs: folder.blobs,
                // An opened folder that lacks the two support files is given them, one that has
                // them is left alone.
                root: folder.root,
                ..Self::default()
            },
            None => Self::default(),
        }
    }

    /// An edit was applied: the folder no longer holds the project.
    pub fn edited(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Whether the project has changes the folder does not hold.
    pub fn dirty(&self) -> bool {
        self.revision != self.saved
    }

    /// Runs the save of `project` and `session` against what the folder holds and returns
    /// its frame (see the module docs). The result stays pending until [`Saver::finish`].
    pub fn start(&mut self, mode: Mode, project: &Project, session: &SessionState) -> Vec<u8> {
        if mode == Mode::Changed && !self.dirty() {
            self.pending = Some(Pending {
                state: self.state.clone(),
                blobs: self.blobs.clone(),
                root: self.root.clone(),
                session: self.session.clone(),
                revision: self.revision,
            });
            return frame(self.revision, &[]);
        }
        let (mut state, blobs, root, session_bytes) =
            if matches!(mode, Mode::Everything | Mode::Copy) {
                (SaveState::new(), BTreeSet::new(), BTreeSet::new(), None)
            } else if mode == Mode::Rewrite {
                (
                    SaveState::new(),
                    self.blobs.clone(),
                    self.root.clone(),
                    None,
                )
            } else {
                (
                    self.state.clone(),
                    self.blobs.clone(),
                    self.root.clone(),
                    self.session.clone(),
                )
            };
        let mut recorder = Recorder {
            entries: Vec::new(),
            blobs,
            root,
            session: session_bytes,
        };
        let options = SaveOptions::default();
        let result = save(&mut recorder, project, &mut state, None, &options)
            .and_then(|_| save_session(&mut recorder, session));
        if let Err(error) = result {
            self.pending = None;
            let mut out = vec![1];
            out.extend_from_slice(error.to_string().as_bytes());
            return out;
        }
        let out = frame(self.revision, &recorder.entries);
        self.pending = Some(Pending {
            state,
            blobs: recorder.blobs,
            root: recorder.root,
            session: recorder.session,
            revision: self.revision,
        });
        out
    }

    /// The page applied the last frame (`true`) or could not (`false`). Without a pending
    /// save, nothing happens.
    pub fn finish(&mut self, applied: bool) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if !applied {
            return;
        }
        self.state = pending.state;
        self.blobs = pending.blobs;
        self.root = pending.root;
        self.session = pending.session;
        self.saved = pending.revision;
    }
}

fn frame(revision: u32, entries: &[Entry]) -> Vec<u8> {
    let mut out = vec![0];
    out.extend_from_slice(&revision.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        match entry {
            Entry::Write(path, bytes) => {
                out.push(0);
                out.extend_from_slice(&(path.len() as u32).to_le_bytes());
                out.extend_from_slice(path.as_bytes());
                out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
                out.extend_from_slice(bytes);
            }
            Entry::Remove(path) => {
                out.push(1);
                out.extend_from_slice(&(path.len() as u32).to_le_bytes());
                out.extend_from_slice(path.as_bytes());
            }
        }
    }
    out
}

/// Reads a frame back: the revision it covers and its entries, or the failure message. For
/// the tests; the page parses the same layout in TypeScript.
#[cfg(test)]
pub fn parse_frame(bytes: &[u8]) -> Result<(u32, Vec<Entry>), String> {
    fn take<'a>(rest: &mut &'a [u8], n: usize) -> &'a [u8] {
        let (head, tail) = rest.split_at(n);
        *rest = tail;
        head
    }
    let mut rest = bytes;
    if take(&mut rest, 1)[0] != 0 {
        return Err(String::from_utf8_lossy(rest).into_owned());
    }
    let revision = u32::from_le_bytes(take(&mut rest, 4).try_into().unwrap());
    let count = u32::from_le_bytes(take(&mut rest, 4).try_into().unwrap());
    let mut entries = Vec::new();
    for _ in 0..count {
        let kind = take(&mut rest, 1)[0];
        let len = u32::from_le_bytes(take(&mut rest, 4).try_into().unwrap()) as usize;
        let path = String::from_utf8(take(&mut rest, len).to_vec()).unwrap();
        if kind == 0 {
            let len = u64::from_le_bytes(take(&mut rest, 8).try_into().unwrap()) as usize;
            entries.push(Entry::Write(path, take(&mut rest, len).to_vec()));
        } else {
            entries.push(Entry::Remove(path));
        }
    }
    assert!(rest.is_empty(), "bytes after the last entry");
    Ok((revision, entries))
}
