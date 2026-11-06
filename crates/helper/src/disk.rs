//! The project's folder in Boost mode (AU-7, #116): the only place the helper reads or writes
//! project data.
//!
//! Founder's decision (2026-11-06): in Boost the helper reads and writes in the open project's
//! folder only, nowhere else, and the page still owns the project. The page keeps writing the
//! project's files in both modes (File System Access, BO-1 and AU-1); what the helper needs is:
//!
//! - **to reach the folder the page opened** without the page naming a path (a browser never
//!   gives one): the user picks it in the helper's own folder dialog ([`FolderPicker`]), and the
//!   helper opens it only when its `project.json` holds the id of the project the page has open.
//!   To tell the folder the page has open from a copy of it (a copy made outside Faulk keeps the
//!   id), the page writes a random marker of its own in that folder (`.faulk/link.json`, a nonce
//!   per attach) and names the nonce: the helper opens only a folder that holds both the id and
//!   that nonce. It remembers the folders picked ([`Remembered`], in its config folder: its own
//!   configuration, the one file it writes outside a project folder) and opens one by itself
//!   only when exactly one of them holds the page's marker; otherwise it asks, never choosing
//!   silently;
//! - **to read** the audio files of that folder for its engine's disk planner (AU-3, AU-4):
//!   `blobs/<sha256>` and `.faulk/cache/pcm/<sha256>.pcm`, built from the media's hash, never from
//!   a string ([`ProjectFolder::read_media`], served by [`spawn_disk`]);
//! - **to write** what the helper itself produces there (the export of #319, a native import),
//!   atomically through `.faulk/tmp/` ([`ProjectFolder::write`]); never the project's own files,
//!   which are the page's.
//!
//! Every path is checked twice: as text first (no `..`, no absolute path, drive letter, UNC path,
//! backslash, colon, device name or trailing dot), before any disk access; then on disk, where the
//! canonical path (every link followed) must stay inside the canonical root. Each check fails with
//! a typed [`PathError`] and leaves the disk untouched. Nothing is cached between two operations,
//! so a link made since the last one is seen. What stays possible is stated in the module's tests and
//! in the storage architecture (§ 5.13): a local process that swaps a link between the check and
//! the open already has the user's rights on the folder; a hard link is the same file and cannot be
//! told apart by its path.

use engine::audio_source::{Failure, FileKind, Job};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// The most components a caller-given path may have.
pub const MAX_COMPONENTS: usize = 32;
/// The longest caller-given path, in bytes.
pub const MAX_PATH_BYTES: usize = 1024;
/// The longest read the disk thread does for one job (the planner asks for half a second of a
/// source at most: 768 kHz × 64 channels × 4 bytes is 98 MiB, far above any real one; the cap
/// only bounds a planner gone wrong).
pub const MAX_READ_BYTES: u32 = 16 << 20;
/// The largest `project.json` the helper reads to learn a folder's project id.
pub const MAX_PROJECT_FILE: u64 = 16 << 20;
/// How many picked folders the helper remembers.
pub const MAX_REMEMBERED: usize = 32;
/// The file of the remembered folders, in the helper's config folder (its own state, like its
/// token: not project data).
pub const REMEMBERED_FILE: &str = "folders.json";
/// How often the disk thread polls the engine's planner (the page's executor polls every 10 ms).
pub const DISK_POLL: Duration = Duration::from_millis(10);

/// Why a path is refused. Every variant is found before the disk is changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    /// An empty path, or an empty component (`a//b`, a trailing `/`).
    Empty,
    /// Longer than [`MAX_PATH_BYTES`] or deeper than [`MAX_COMPONENTS`].
    TooLong,
    /// Starts with `/`: an absolute path.
    Absolute,
    /// Holds a `\`: another separator (Windows), never taken as part of a name.
    Backslash,
    /// Holds a `:`: a drive letter (`C:`), or an NTFS alternate stream (`file:stream`).
    Colon,
    /// A `.` or `..` component.
    Dot,
    /// A NUL or another control character.
    Control,
    /// A component ending in `.` or a space, which Windows strips (`blobs.` is `blobs`).
    Trailing,
    /// A Windows device name (`CON`, `NUL`, `COM1`...), with or without an extension.
    Device,
    /// A path of the project's own files or of the folder's own data (`project.json`, `tracks/`,
    /// `master/`, `presets/`, `blobs/`, `.faulk/`, `.git*`): the page's, never written by the
    /// helper through a path.
    Reserved,
    /// Its canonical path (links followed) leaves the project's folder.
    Escapes,
    /// The folder is not the one the operation began in any more: moved, replaced, or holding
    /// another project.
    FolderChanged,
    /// A directory stands where the file goes.
    IsDirectory,
    /// The disk refused (permission, full disk...). `String` is for developers.
    Io(String),
}

impl PathError {
    /// A stable code, for logs and for the page.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Empty => "path.empty",
            Self::TooLong => "path.too_long",
            Self::Absolute => "path.absolute",
            Self::Backslash => "path.backslash",
            Self::Colon => "path.colon",
            Self::Dot => "path.dot",
            Self::Control => "path.control",
            Self::Trailing => "path.trailing",
            Self::Device => "path.device",
            Self::Reserved => "path.reserved",
            Self::Escapes => "path.escapes",
            Self::FolderChanged => "path.folder_changed",
            Self::IsDirectory => "path.is_directory",
            Self::Io(_) => "path.io",
        }
    }
}

fn io(error: std::io::Error) -> PathError {
    PathError::Io(error.to_string())
}

/// Windows device names: a file of that name (any extension) is the device, in any folder.
const DEVICES: &[&str] = &[
    "con", "prn", "aux", "nul", "conin$", "conout$", "com1", "com2", "com3", "com4", "com5",
    "com6", "com7", "com8", "com9", "com0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7",
    "lpt8", "lpt9", "lpt0", "com¹", "com²", "com³", "lpt¹", "lpt²", "lpt³",
];

/// The first components the helper never writes through a path (compared without case: the
/// folder may be on a case-insensitive disk).
const RESERVED_TOP: &[&str] = &[
    "project.json",
    "tracks",
    "master",
    "presets",
    "blobs",
    ".faulk",
    ".git",
    ".gitignore",
    ".gitattributes",
];

/// Checks a caller-given relative path as text, before any disk access: `/` separated, each
/// component a plain name. Returns its components.
pub fn check_relative(path: &str) -> Result<Vec<&str>, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(PathError::TooLong);
    }
    if path.starts_with('/') {
        return Err(PathError::Absolute);
    }
    if path.contains('\\') {
        return Err(PathError::Backslash);
    }
    if path.contains(':') {
        return Err(PathError::Colon);
    }
    if path.chars().any(char::is_control) {
        return Err(PathError::Control);
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > MAX_COMPONENTS {
        return Err(PathError::TooLong);
    }
    for part in &parts {
        check_component(part)?;
    }
    Ok(parts)
}

fn check_component(part: &str) -> Result<(), PathError> {
    if part.is_empty() {
        return Err(PathError::Empty);
    }
    if part == "." || part == ".." {
        return Err(PathError::Dot);
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return Err(PathError::Trailing);
    }
    // `CON`, `con.txt`, `COM1.tar.gz` and `nul ` (spaces before the extension are stripped too).
    let stem = part
        .split('.')
        .next()
        .unwrap_or(part)
        .trim_end()
        .to_lowercase();
    if DEVICES.contains(&stem.as_str()) {
        return Err(PathError::Device);
    }
    Ok(())
}

/// Whether components (from the folder's root) name the project's own files or the folder's
/// own data.
fn reserved<S: AsRef<str>>(parts: &[S]) -> bool {
    parts.first().is_some_and(|first| {
        let first = first.as_ref().to_lowercase();
        RESERVED_TOP.contains(&first.as_str()) || first.starts_with(".git")
    })
}

/// Canonicalizes `path`, which must exist.
fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    fs::canonicalize(path)
}

/// The components of `path` below `root`, as text: `None` when `path` is not below it.
fn below(root: &Path, path: &Path) -> Option<Vec<String>> {
    let rest = path.strip_prefix(root).ok()?;
    rest.components()
        .map(|c| match c {
            Component::Normal(name) => name.to_str().map(str::to_owned),
            _ => None,
        })
        .collect()
}

/// Reads the `id` of the `project.json` in `dir`; `None` when there is none, or none valid.
pub fn project_id_of(dir: &Path) -> Option<String> {
    // Read only inside the folder: a `project.json` that is a link leading out is no project.
    let root = canonical(dir).ok()?;
    let real = canonical(&root.join("project.json")).ok()?;
    if !real.starts_with(&root) {
        return None;
    }
    let file = File::open(&real).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    /// The one field read; the others are skipped, not built.
    #[derive(serde::Deserialize)]
    struct Head {
        id: String,
    }
    let head: Head =
        serde_json::from_reader(std::io::BufReader::new(file.take(MAX_PROJECT_FILE))).ok()?;
    project::ProjectId::parse(&head.id).ok()?;
    Some(head.id)
}

/// The page's link marker, in the folder's `.faulk/` (written by the page, never by the helper).
pub const LINK_FILE: &str = "link.json";

/// A link nonce: 32 lowercase hex digits (128 bits), the page's random per attach.
pub fn valid_nonce(nonce: &str) -> bool {
    nonce.len() == 32
        && nonce
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A media read: its bytes, the file's length, and when it last changed.
#[derive(Debug, PartialEq, Eq)]
pub struct MediaRead {
    pub bytes: Vec<u8>,
    pub file_len: u64,
    pub modified: Option<std::time::SystemTime>,
}

/// Why a folder could not be opened as a project's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Not a directory the helper can read.
    Unreadable,
    /// No `project.json` with a valid id.
    NoProject,
    /// Its `project.json` holds another project's id.
    OtherProject,
}

/// A project's folder, opened: its canonical root and the project it holds. Every operation
/// resolves its path again from the root (module doc).
#[derive(Debug)]
pub struct ProjectFolder {
    /// The canonical root (every link followed).
    root: PathBuf,
    /// The path it was opened by, canonicalized again to tell whether the folder moved.
    opened_as: PathBuf,
    project: String,
    name: String,
}

impl ProjectFolder {
    /// Opens `path` as the folder of project `expected` (any project when `None`).
    pub fn open(path: &Path, expected: Option<&str>) -> Result<ProjectFolder, OpenError> {
        let root = canonical(path).map_err(|_| OpenError::Unreadable)?;
        if !fs::metadata(&root).is_ok_and(|m| m.is_dir()) {
            return Err(OpenError::Unreadable);
        }
        let project = project_id_of(&root).ok_or(OpenError::NoProject)?;
        if expected.is_some_and(|e| e != project) {
            return Err(OpenError::OtherProject);
        }
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(ProjectFolder {
            root,
            opened_as: path.to_path_buf(),
            project,
            name,
        })
    }

    /// The canonical root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The id of the project it holds (when it was opened).
    pub fn project(&self) -> &str {
        &self.project
    }

    /// The folder's name, as the page's File System Access handle names it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The path of a media file, built from its hash.
    fn media_path(&self, kind: FileKind, media: &[u8; 32]) -> PathBuf {
        let hex: String = media.iter().map(|b| format!("{b:02x}")).collect();
        let mut path = self.root.clone();
        match kind {
            FileKind::Blob => {
                path.push("blobs");
                path.push(hex);
            }
            FileKind::Cache => {
                path.push(".faulk");
                path.push("cache");
                path.push("pcm");
                path.push(format!("{hex}.pcm"));
            }
        }
        path
    }

    /// The nonce of the page's link marker (`.faulk/link.json`, `{"nonce": "<32 hex>"}`): the
    /// page writes it in the folder it has open, so a folder that holds the nonce the page names
    /// is that folder, not a copy. `None` when there is none, or none valid.
    pub fn link_nonce(&self) -> Option<String> {
        let mut path = self.root.clone();
        path.push(".faulk");
        path.push(LINK_FILE);
        let real = canonical(&path).ok()?;
        if !real.starts_with(&self.root) {
            return None;
        }
        let file = File::open(&real).ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let value: serde_json::Value = serde_json::from_reader(file.take(4096)).ok()?;
        let nonce = value.get("nonce")?.as_str()?;
        valid_nonce(nonce).then(|| nonce.to_owned())
    }

    /// Whether the folder holds the page's link marker of `nonce`.
    pub fn holds_link(&self, nonce: &str) -> bool {
        self.link_nonce()
            .is_some_and(|n| crate::security::constant_time_eq(n.as_bytes(), nonce.as_bytes()))
    }

    /// Reads `len` bytes (at most [`MAX_READ_BYTES`]) at `offset` of a media file (the blob or
    /// the PCM cache of `media`), with the file's length and time of change. A read past the end
    /// is short, as a `File.slice` in the page. A file whose canonical path leaves the folder is
    /// not opened.
    pub fn read_media(
        &self,
        kind: FileKind,
        media: &[u8; 32],
        offset: u64,
        len: u32,
    ) -> Result<MediaRead, Failure> {
        let path = self.media_path(kind, media);
        let real = match canonical(&path) {
            Ok(real) => real,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Failure::NotFound),
            Err(_) => return Err(Failure::Unreadable),
        };
        if !real.starts_with(&self.root) {
            crate::log!("refused a read whose path leaves the project folder");
            return Err(Failure::Unreadable);
        }
        let mut file = File::open(&real).map_err(|_| Failure::Unreadable)?;
        let meta = file.metadata().map_err(|_| Failure::Unreadable)?;
        if !meta.is_file() {
            return Err(Failure::Unreadable);
        }
        let file_len = meta.len();
        let modified = meta.modified().ok();
        let want = u64::from(len.min(MAX_READ_BYTES)).min(file_len.saturating_sub(offset));
        let mut bytes = Vec::with_capacity(want as usize);
        if want > 0 {
            file.seek(SeekFrom::Start(offset))
                .map_err(|_| Failure::Unreadable)?;
            file.take(want)
                .read_to_end(&mut bytes)
                .map_err(|_| Failure::Unreadable)?;
        }
        Ok(MediaRead {
            bytes,
            file_len,
            modified,
        })
    }

    /// Whether the folder is still the one opened: the path it was opened by leads to the same
    /// canonical root, which still holds the same project.
    fn still_the_same(&self) -> Result<(), PathError> {
        let now = canonical(&self.opened_as).map_err(|_| PathError::FolderChanged)?;
        if now != self.root || project_id_of(&self.root).as_deref() != Some(&self.project) {
            return Err(PathError::FolderChanged);
        }
        Ok(())
    }

    /// The components of the directory at `parts` below the root as they will be once made, links
    /// followed, without making anything: the canonical path of its deepest existing ancestor
    /// (which must stay inside the root), then the components still missing.
    fn probe(&self, parts: &[&str]) -> Result<Vec<String>, PathError> {
        let mut existing = self.root.clone();
        let mut depth = 0;
        for part in parts {
            let next = existing.join(part);
            match fs::symlink_metadata(&next) {
                Ok(_) => {
                    existing = next;
                    depth += 1;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(io(e)),
            }
        }
        let real = canonical(&existing).map_err(io)?;
        let mut out = below(&self.root, &real).ok_or(PathError::Escapes)?;
        out.extend(parts[depth..].iter().map(|p| (*p).to_owned()));
        Ok(out)
    }

    /// The canonical directory at `parts` below the root, made on the way; each component must
    /// stay inside the root once made (a link inside the folder is followed, one leading out
    /// refused).
    fn directory(&self, parts: &[&str]) -> Result<PathBuf, PathError> {
        let mut dir = self.root.clone();
        for part in parts {
            let next = dir.join(part);
            match fs::symlink_metadata(&next) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&next) {
                        Ok(()) => {}
                        // Made meanwhile by another writer: checked below like any other.
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(e) => return Err(io(e)),
                    }
                }
                Err(e) => return Err(io(e)),
            }
            let real = canonical(&next).map_err(io)?;
            if !real.starts_with(&self.root) {
                return Err(PathError::Escapes);
            }
            if !fs::metadata(&real).map_err(io)?.is_dir() {
                return Err(PathError::IsDirectory);
            }
            dir = real;
        }
        Ok(dir)
    }

    /// Writes `bytes` to the file at `path` (relative, `/` separated) of the folder, atomically:
    /// into a new file of `.faulk/tmp/`, flushed to the disk, then moved over the target. Refuses a
    /// path the text check refuses, a path of the project's own files or of the folder's own data
    /// ([`PathError::Reserved`], before and after the links are followed), a path that leaves the
    /// folder, and a folder that is not the one opened any more. Every check of the path comes
    /// before anything is made, so a refused path makes no directory (`.faulk/tmp/` itself aside,
    /// made once the target passed). On a refusal or a failure no temporary file is left and the
    /// target is as it was.
    pub fn write(&self, path: &str, bytes: &[u8]) -> Result<(), PathError> {
        self.write_with(path, bytes, || {})
    }

    /// [`write`](Self::write), with `before_commit` run between the temporary file and the move
    /// (the tests change the folder there).
    pub(crate) fn write_with(
        &self,
        path: &str,
        bytes: &[u8],
        before_commit: impl FnOnce(),
    ) -> Result<(), PathError> {
        let parts = check_relative(path)?;
        if reserved(&parts) {
            return Err(PathError::Reserved);
        }
        self.still_the_same()?;
        let (name, dirs) = parts.split_last().ok_or(PathError::Empty)?;
        // Every check before anything is made: the path after the links (`Blobs/x`, an 8.3
        // alias of `.faulk`, a link inside the folder to `blobs/`), then the temporary folder.
        let mut real = self.probe(dirs)?;
        real.push((*name).to_owned());
        if reserved(&real) {
            return Err(PathError::Reserved);
        }
        let tmp_dir = self.directory(&[".faulk", "tmp"])?;
        if tmp_dir != self.root.join(".faulk").join("tmp") {
            // `.faulk` or `tmp` is a link, even to inside the folder: the temporary file would
            // not be where the cleaning of `.faulk/tmp/` looks.
            return Err(PathError::Escapes);
        }
        let parent = self.directory(dirs)?;
        let target = parent.join(name);
        if fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir()) {
            return Err(PathError::IsDirectory);
        }
        let tmp = tmp_dir.join(format!("helper-{}.tmp", crate::security::random_hex(8)));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .map_err(io)?;
            file.write_all(bytes).map_err(io)?;
            file.sync_all().map_err(io)?;
            drop(file);
            before_commit();
            self.still_the_same()?;
            // The parent again: a link made in its place since is seen.
            let parent_now = canonical(&parent).map_err(|_| PathError::FolderChanged)?;
            if parent_now != parent || !parent_now.starts_with(&self.root) {
                return Err(PathError::FolderChanged);
            }
            if fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir()) {
                return Err(PathError::IsDirectory);
            }
            fs::rename(&tmp, &target).map_err(io)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }
}

// ---------------------------------------------------------------- the open folder

/// The folder the session's page has open, shared by the server and the disk thread. Each change
/// counts a generation: a read begun in a folder that was replaced since is answered as not
/// found (the rule of the page's `DiskExecutor`).
#[derive(Default)]
pub struct OpenFolder {
    inner: Mutex<(u64, Option<Arc<ProjectFolder>>)>,
}

impl OpenFolder {
    /// Opens `folder` (`None`: no folder), as a new generation.
    pub fn set(&self, folder: Option<Arc<ProjectFolder>>) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.0 = inner.0.wrapping_add(1);
        inner.1 = folder;
    }

    /// The generation and the folder open now.
    pub fn get(&self) -> (u64, Option<Arc<ProjectFolder>>) {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        (inner.0, inner.1.clone())
    }

    /// The generation now.
    pub fn generation(&self) -> u64 {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).0
    }
}

/// What a read job came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done { bytes: Vec<u8>, file_len: u64 },
    Failed(Failure),
}

/// The disk side of the helper's engine: runs read jobs against the folder open now. Like the
/// page's executor, which keeps one `File` per path (a snapshot), it remembers each file's length
/// and time of change at its first read in a folder: a file found changed at a later read (a PCM
/// cache the page made again) is answered [`Failure::Changed`], so the planner probes it again.
pub struct DiskReader {
    open: Arc<OpenFolder>,
    /// The folder generation the snapshots below are of.
    generation: u64,
    seen: HashMap<(u8, [u8; 32]), (u64, Option<std::time::SystemTime>)>,
}

impl DiskReader {
    pub fn new(open: Arc<OpenFolder>) -> DiskReader {
        DiskReader {
            open,
            generation: 0,
            seen: HashMap::new(),
        }
    }

    /// Runs one read job; `after_read` runs between the read and the generation check (tests
    /// switch the folder there). A read begun in a folder replaced since is answered as not found.
    pub(crate) fn run(&mut self, job: &Job, after_read: impl FnOnce()) -> Outcome {
        let (generation, folder) = self.open.get();
        if generation != self.generation {
            self.generation = generation;
            self.seen.clear();
        }
        let read = match folder {
            Some(folder) => folder.read_media(job.file, &job.media, job.offset, job.len),
            None => Err(Failure::NotFound),
        };
        after_read();
        if self.open.generation() != generation {
            return Outcome::Failed(Failure::NotFound);
        }
        let key = (job.file as u8, job.media);
        match read {
            Ok(read) => {
                let now = (read.file_len, read.modified);
                match self.seen.insert(key, now) {
                    Some(before) if before != now => Outcome::Failed(Failure::Changed),
                    _ => Outcome::Done {
                        bytes: read.bytes,
                        file_len: read.file_len,
                    },
                }
            }
            Err(why) => {
                self.seen.remove(&key);
                Outcome::Failed(why)
            }
        }
    }
}

/// One round of the disk thread: polls the planner of the open output's engine, runs each job
/// outside the engine's lock (the flushes and the page's streams never wait for the disk), and
/// hands each answer back to the engine it was polled from only: an engine installed meanwhile
/// (another output) numbers its jobs from 1 again, and never gets an answer meant for another.
/// `between` runs after the poll (tests replace the engine there).
pub(crate) fn disk_round(
    slot: &crate::audio::EngineSlot,
    reader: &mut DiskReader,
    now_ms: u64,
    jobs: &mut Vec<Job>,
    between: impl FnOnce(),
) {
    let generation = slot.with_generation_of(|sender| {
        jobs.clear();
        jobs.extend_from_slice(sender.disk_poll(now_ms));
    });
    between();
    let Some(generation) = generation else {
        return;
    };
    for job in jobs.iter() {
        let outcome = reader.run(job, || {});
        slot.with_generation(generation, |sender| match outcome {
            Outcome::Done { bytes, file_len } => sender.disk_done(job.id, &bytes, file_len),
            Outcome::Failed(why) => sender.disk_failed(job.id, why),
        });
    }
}

/// The disk thread (AU-7): a [`disk_round`] every [`DISK_POLL`], until the slot is dropped.
pub fn spawn_disk(
    slot: &Arc<crate::audio::EngineSlot>,
    open: &Arc<OpenFolder>,
) -> std::thread::JoinHandle<()> {
    let slot: Weak<crate::audio::EngineSlot> = Arc::downgrade(slot);
    let mut reader = DiskReader::new(open.clone());
    let start = Instant::now();
    std::thread::spawn(move || {
        let mut jobs = Vec::new();
        while let Some(slot) = slot.upgrade() {
            let now_ms = start.elapsed().as_millis() as u64;
            disk_round(&slot, &mut reader, now_ms, &mut jobs, || {});
            drop(slot);
            std::thread::sleep(DISK_POLL);
        }
    })
}

// ---------------------------------------------------------------- the remembered folders

/// The folders the user picked in the helper's dialog, most recent first, at most
/// [`MAX_REMEMBERED`]. Kept in the helper's config folder ([`REMEMBERED_FILE`]); read leniently
/// (a file that does not parse is an empty list), and each one checked again at every lookup.
#[derive(Debug, Default)]
pub struct Remembered {
    file: Option<PathBuf>,
    paths: Vec<PathBuf>,
}

/// What the lookup of a project among the remembered folders found.
#[derive(Debug)]
pub enum Lookup {
    /// Exactly one remembered folder holds the project and the page's link marker: the folder
    /// the page has open.
    Found(ProjectFolder),
    /// None does (never shown to the helper, or only a copy of it was): the user shows it.
    Unknown,
    /// Several do (a copy made outside Faulk since the page wrote its marker): the user chooses.
    Ambiguous(u32),
}

impl Remembered {
    /// The list kept in `file` (an empty one when it is missing or does not parse).
    pub fn load(file: PathBuf) -> Remembered {
        let paths = fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .take(MAX_REMEMBERED)
            .collect();
        Remembered {
            file: Some(file),
            paths,
        }
    }

    /// A list kept in memory only (tests, and a helper without a config folder).
    pub fn in_memory(paths: Vec<PathBuf>) -> Remembered {
        Remembered {
            file: None,
            paths: paths.into_iter().take(MAX_REMEMBERED).collect(),
        }
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Puts `path` (a canonical root) first, and keeps the list.
    pub fn remember(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX_REMEMBERED);
        let Some(file) = &self.file else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec_pretty(&self.paths) else {
            return;
        };
        // Written whole then moved, so a crash never leaves half a list.
        let tmp = file.with_extension("json.tmp");
        let written = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, file));
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            crate::log!("could not keep the remembered folders: {e}");
        }
    }

    /// Looks up the folder of project `project` that holds the page's link marker `nonce`. Two
    /// folders of one id are told apart by the marker, which only the folder the page has open
    /// holds; a copy the helper remembers alone is never taken for it.
    pub fn lookup(&self, project: &str, nonce: &str) -> Lookup {
        lookup(&self.paths, project, nonce)
    }
}

/// [`Remembered::lookup`] on a copy of the list: the disk is read with no lock held.
pub fn lookup(paths: &[PathBuf], project: &str, nonce: &str) -> Lookup {
    let mut found: Vec<ProjectFolder> = Vec::new();
    for path in paths {
        if let Ok(folder) = ProjectFolder::open(path, Some(project))
            && folder.holds_link(nonce)
            && !found.iter().any(|f| f.root == folder.root)
        {
            found.push(folder);
        }
    }
    match found.len() {
        0 => Lookup::Unknown,
        1 => Lookup::Found(found.remove(0)),
        n => Lookup::Ambiguous(u32::try_from(n).unwrap_or(u32::MAX)),
    }
}

// ---------------------------------------------------------------- the folder dialog

/// What the user did in the helper's folder dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picked {
    Folder(PathBuf),
    Cancelled,
    /// This system has no dialog the helper can show.
    Unavailable,
}

/// The helper's folder dialog. Blocking: called off the async runtime.
pub trait FolderPicker: Send + Sync {
    /// Shows the dialog titled `title`.
    fn pick(&self, title: &str) -> Picked;
}

/// No dialog (a helper started without one, and the default in tests).
pub struct NoPicker;

impl FolderPicker for NoPicker {
    fn pick(&self, _title: &str) -> Picked {
        Picked::Unavailable
    }
}

/// The system's folder dialog: the common dialog on Windows, `osascript` on macOS, `zenity`
/// or `kdialog` on Linux.
pub struct SystemPicker;

impl FolderPicker for SystemPicker {
    #[cfg(windows)]
    fn pick(&self, title: &str) -> Picked {
        match rfd::FileDialog::new().set_title(title).pick_folder() {
            Some(path) => Picked::Folder(path),
            None => Picked::Cancelled,
        }
    }

    #[cfg(target_os = "macos")]
    fn pick(&self, title: &str) -> Picked {
        // The title goes in an AppleScript string: its quotes and backslashes are escaped.
        let prompt = title.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("POSIX path of (choose folder with prompt \"{prompt}\")");
        run_picker("osascript", &["-e", script.as_str()])
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn pick(&self, title: &str) -> Picked {
        match run_picker(
            "zenity",
            &["--file-selection", "--directory", "--title", title],
        ) {
            Picked::Unavailable => run_picker(
                "kdialog",
                &["--getexistingdirectory", ".", "--title", title],
            ),
            picked => picked,
        }
    }
}

/// Runs a dialog program that prints the chosen path: its absence is `Unavailable`, a failure
/// (the user cancelled) `Cancelled`.
#[cfg(unix)]
fn run_picker(program: &str, args: &[&str]) -> Picked {
    match std::process::Command::new(program).args(args).output() {
        Err(_) => Picked::Unavailable,
        Ok(out) if !out.status.success() => Picked::Cancelled,
        Ok(out) => {
            let path = String::from_utf8_lossy(&out.stdout)
                .trim_end_matches(['\n', '\r'])
                .to_owned();
            if path.is_empty() {
                Picked::Cancelled
            } else {
                Picked::Folder(PathBuf::from(path))
            }
        }
    }
}

/// One dialog at a time: a second request while one is open shows nothing more.
#[derive(Default)]
pub struct DialogGate(AtomicBool);

impl DialogGate {
    /// Takes the gate; `None` when a dialog is open already. The gate opens again when the
    /// guard is dropped, however the dialog ends (a picker that panics included).
    pub fn take(self: &Arc<Self>) -> Option<DialogTaken> {
        (!self.0.swap(true, Ordering::AcqRel)).then(|| DialogTaken(self.clone()))
    }
}

/// The dialog gate, held: released on drop.
pub struct DialogTaken(Arc<DialogGate>);

impl Drop for DialogTaken {
    fn drop(&mut self) {
        self.0.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
