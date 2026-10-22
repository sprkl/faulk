//! The folder as the format sees it: an abstract file system, so that the same reader and
//! writer serve a native folder, the browser's OPFS and a folder the user picked.
//!
//! The trait is synchronous (OPFS has synchronous access handles in a worker); a host with
//! only asynchronous access fills a [`MemFs`] first and replays the writes it records.
//! Paths are relative to the project folder, `/`-separated, with no leading slash.

use std::collections::BTreeMap;
use std::fmt;

/// Why a file-system call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    Other(String),
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for FsError {}

impl From<std::io::Error> for FsError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::Other(e.to_string())
        }
    }
}

/// One entry of a directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// The name, without its directory.
    pub name: String,
    pub is_dir: bool,
    /// Size in bytes (0 for a directory).
    pub size: u64,
    /// Anything that changes when the file does: a modification time in any unit. The
    /// cache snapshot trusts the pair (size, stamp) to say a file did not change.
    pub stamp: u64,
}

/// A project folder.
pub trait FileSystem {
    /// The bytes of a file.
    fn read(&self, path: &str) -> Result<Vec<u8>, FsError>;
    /// Replaces a file, creating its directories. Atomic per file: a crash leaves the
    /// old content or the new one, never half of it.
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), FsError>;
    /// Removes a file; removing a file that is not there is not an error. A directory
    /// left empty goes with it.
    fn remove(&mut self, path: &str) -> Result<(), FsError>;
    /// Whether a file exists.
    fn exists(&self, path: &str) -> bool;
    /// What is directly under a directory (`""` is the root), in no particular order; a
    /// directory that does not exist is empty.
    fn list(&self, dir: &str) -> Result<Vec<DirEntry>, FsError>;
}

/// A folder in memory: path to bytes. What the browser build fills from OPFS, what the
/// tests write into, and what records the writes a host must replay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemFs {
    files: BTreeMap<String, Vec<u8>>,
    stamps: BTreeMap<String, u64>,
    /// Every `write` and `remove` since the last [`MemFs::take_log`].
    log: Vec<Op>,
}

/// A change recorded by a [`MemFs`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Write(String),
    Remove(String),
}

impl MemFs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts a file without recording it (a folder as the host read it). Its stamp is a
    /// hash of its bytes, so that a file that changes is never mistaken for the one the
    /// cache snapshot knew.
    pub fn insert(&mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        let stamp = super::canon::fast_hash(&bytes);
        self.insert_with_stamp(path, bytes, stamp);
    }

    /// Puts a file with the stamp the host's own file system gave it (its modification
    /// time): the cache snapshot trusts (size, stamp) to say a file did not change.
    pub fn insert_with_stamp(
        &mut self,
        path: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
        stamp: u64,
    ) {
        let path = path.into();
        self.stamps.insert(path.clone(), stamp);
        self.files.insert(path, bytes.into());
    }

    pub fn files(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.files
    }

    /// The recorded changes, and forgets them.
    pub fn take_log(&mut self) -> Vec<Op> {
        std::mem::take(&mut self.log)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl FileSystem for MemFs {
    fn read(&self, path: &str) -> Result<Vec<u8>, FsError> {
        self.files.get(path).cloned().ok_or(FsError::NotFound)
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), FsError> {
        self.insert(path, bytes);
        self.log.push(Op::Write(path.to_owned()));
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<(), FsError> {
        self.files.remove(path);
        self.stamps.remove(path);
        self.log.push(Op::Remove(path.to_owned()));
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    fn list(&self, dir: &str) -> Result<Vec<DirEntry>, FsError> {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut out: BTreeMap<String, DirEntry> = BTreeMap::new();
        for (path, bytes) in self.files.range(prefix.clone()..) {
            let Some(rest) = path.strip_prefix(&prefix) else {
                break;
            };
            match rest.split_once('/') {
                Some((name, _)) => {
                    out.entry(name.to_owned()).or_insert_with(|| DirEntry {
                        name: name.to_owned(),
                        is_dir: true,
                        size: 0,
                        stamp: 0,
                    });
                }
                None => {
                    out.insert(
                        rest.to_owned(),
                        DirEntry {
                            name: rest.to_owned(),
                            is_dir: false,
                            size: bytes.len() as u64,
                            stamp: self.stamps.get(path).copied().unwrap_or(0),
                        },
                    );
                }
            }
        }
        Ok(out.into_values().collect())
    }
}

/// The directories a walk of the project does not enter: the session, caches and temporary
/// files, git's own, and the blobs (named by their content, never listed by a walk of the
/// text).
const WALK_SKIP: [&str; 3] = [".faulk", ".git", "blobs"];

/// The most files a walk accepts: far above any project, so a hostile folder cannot make a
/// host list without end.
pub const MAX_WALK_FILES: usize = 4_000_000;

/// Every file of the folder outside `.faulk/`, `.git/` and `blobs/`, with its size and
/// stamp, by path.
pub fn walk(fs: &dyn FileSystem) -> Result<Vec<(String, DirEntry)>, FsError> {
    fn go(
        fs: &dyn FileSystem,
        dir: &str,
        depth: usize,
        out: &mut Vec<(String, DirEntry)>,
    ) -> Result<(), FsError> {
        if depth > 8 {
            return Err(FsError::Other("folders nested too deep".into()));
        }
        for entry in fs.list(dir)? {
            let path = if dir.is_empty() {
                entry.name.clone()
            } else {
                format!("{dir}/{}", entry.name)
            };
            if entry.is_dir {
                if depth == 0 && WALK_SKIP.contains(&entry.name.as_str()) {
                    continue;
                }
                go(fs, &path, depth + 1, out)?;
            } else {
                if out.len() >= MAX_WALK_FILES {
                    return Err(FsError::Other("too many files".into()));
                }
                out.push((path, entry));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    go(fs, "", 0, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// A real folder, for native hosts (the helper, tests). Writes go through a temporary
/// file and a rename.
#[cfg(not(target_arch = "wasm32"))]
pub struct StdFs {
    root: std::path::PathBuf,
    /// The temporary files a crash left are removed before the first write.
    cleaned: bool,
    counter: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl StdFs {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            root: root.into(),
            cleaned: false,
            counter: 0,
        }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The path inside the folder; a component that could lead out of it (`..`, a drive,
    /// a separator of this platform) is refused, whatever the name a pack or a project file
    /// gave.
    fn join(&self, path: &str) -> Result<std::path::PathBuf, FsError> {
        let mut p = self.root.clone();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            if part == ".."
                || part == "."
                || part.contains(['\\', ':', '\0'])
                || std::path::Path::new(part).components().count() != 1
            {
                return Err(FsError::Other(format!("unsafe path {path:?}")));
            }
            p.push(part);
        }
        Ok(p)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl FileSystem for StdFs {
    fn read(&self, path: &str) -> Result<Vec<u8>, FsError> {
        Ok(std::fs::read(self.join(path)?)?)
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), FsError> {
        let target = self.join(path)?;
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Through `.faulk/tmp/`, which git ignores, so that a crash leaves nothing a
        // `git add` would pick up; the rename stays on one volume.
        let tmp_dir = self.root.join(".faulk").join("tmp");
        if !self.cleaned {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            self.cleaned = true;
        }
        std::fs::create_dir_all(&tmp_dir)?;
        self.counter += 1;
        let tmp = tmp_dir.join(format!("{}-{}", std::process::id(), self.counter));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &target).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        // The folders are only there for the write: leave none behind, so that a folder the
        // import of a pack cleaned up is empty again (a non-empty one is left alone).
        let _ = std::fs::remove_dir(&tmp_dir);
        let _ = std::fs::remove_dir(self.root.join(".faulk"));
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<(), FsError> {
        let target = self.join(path)?;
        match std::fs::remove_file(&target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        // Empty directories are not versioned and only clutter: drop those left behind,
        // up to the root.
        let mut dir = target.parent().map(std::path::Path::to_owned);
        while let Some(d) = dir {
            if d == self.root || std::fs::remove_dir(&d).is_err() {
                break;
            }
            dir = d.parent().map(std::path::Path::to_owned);
        }
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        self.join(path).is_ok_and(|p| p.is_file())
    }

    fn list(&self, dir: &str) -> Result<Vec<DirEntry>, FsError> {
        let rd = match std::fs::read_dir(self.join(dir)?) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for entry in rd {
            let entry = entry?;
            let meta = entry.metadata()?;
            let stamp = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos() as u64);
            out.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
                stamp,
            });
        }
        Ok(out)
    }
}
