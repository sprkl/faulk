//! Faulk project and commands compiled to WebAssembly for the page's main thread (PR-4).
//!
//! The page owns the project in both modes (spec § 2.1): a [`Core`] holds the project,
//! its session and history, the command context and the stream of the engine that plays
//! it (`commands::engine_msgs::Synced`), and what was last saved. Every command runs
//! here, through `commands::run`, and comes back as a JSON report (output, events) with
//! the engine bytes it produced; TypeScript never maps a command to the engine. The
//! reports' types are in `commands::host`, generated into the web app's types.
//!
//! Plain C ABI, no `wasm-bindgen` (its CLI would be one more pinned tool for a handful of
//! calls). The page writes its JSON input into the inbox ([`pw_inbox`]), calls an export,
//! and reads the JSON the call returns at [`pw_out`] (the export returns its length) and
//! the engine bytes at [`pw_engine`] ([`pw_engine_len`]; every call on a core replaces
//! them, with nothing when it produced none). One thread (the page's), so the buffers live
//! in thread-locals.

use std::cell::RefCell;

use commands::contract::Folder;
use commands::engine_msgs::Synced;
use commands::host::{ProjectView, RunReport, SnapshotReport};
use commands::{Author, Context, State};
use project::file::{DirEntry, FileSystem, FsError, MemFs};

mod save;
use project::{IdGenerator, Project, ProjectId};
pub use save::{Mode, Saver};
use serde::Serialize;

thread_local! {
    static INBOX: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static ENGINE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// No core: a null handle.
pub const ERR_NO_CORE: i32 = -1;
/// The input is not what the call takes (`pw_save`: not a save mode).
pub const ERR_BAD_INPUT: i32 = -2;
/// The call's JSON did not serialize (a value the project's writers refuse): nothing
/// at [`pw_out`] nor [`pw_engine`]. The call itself may have happened ([`pw_run`] applied
/// the command, [`pw_load`] replaced the project): the engine is then behind the project
/// until a snapshot.
pub const ERR_SERIALIZE: i32 = -3;

/// [`pw_stage`]: the folder frame is malformed (see [`parse_folder`]): nothing staged.
pub const ERR_BAD_FOLDER: i32 = -4;

/// [`pw_new`]: an empty project.
pub const KIND_EMPTY: u32 = 0;
/// [`pw_new`]: the page's one-synth project (the ids the interface uses until PR-5).
pub const KIND_SINGLE_SYNTH: u32 = 1;

/// A project in the page: the project with its engine stream, and what is known of the
/// folder it is saved into ([`Saver`]).
pub struct Core {
    synced: Synced,
    saver: Saver,
}

impl Core {
    /// A core of `kind`, whose engine is in `epoch`; the ids it creates come from the
    /// page's clock (`time_ms`) and randomness (`random`, 80 bits kept). Nothing saved
    /// yet: every file is dirty. `None` for an unknown kind.
    pub fn new(kind: u32, epoch: u16, time_ms: u64, random: u128) -> Option<Core> {
        let mut ids = IdGenerator::new(time_ms, random);
        let state = match kind {
            KIND_EMPTY => State::new(Project::new(ProjectId::generate(&mut ids), "Untitled")),
            KIND_SINGLE_SYNTH => {
                // The ids of the one synth are the interface's; the project is its own.
                let mut state = commands::engine_msgs::single_synth::state();
                state.project.id = ProjectId::generate(&mut ids);
                state
            }
            _ => return None,
        };
        Some(Core {
            synced: Synced::new(state, Context::new(Author::User, ids), epoch),
            saver: Saver::new(),
        })
    }

    /// Runs a command's JSON through `commands::run` (history included): its report, and
    /// its engine bytes (empty when none).
    pub fn run(&mut self, json: &[u8]) -> (RunReport, Vec<u8>) {
        let command = match serde_json::from_slice::<commands::Command>(json) {
            Ok(command) => command,
            Err(error) => {
                let message = error.to_string();
                return (RunReport::Invalid { message }, Vec::new());
            }
        };
        let synced = &mut self.synced;
        let kind = command.meta().kind;
        // A live command changes no project, but the selection, solo, arming and MIDI input
        // are saved with it (`.faulk/session.json`): a change of them is a change to save.
        let session = (kind == commands::Kind::Live).then(|| synced.state.session.clone());
        let outcome = match commands::run(&command, &mut synced.state, &mut synced.ctx) {
            Ok(outcome) => outcome,
            Err(error) => return (RunReport::Refused { error }, Vec::new()),
        };
        let (bytes, stream_error) = match synced.engine_bytes(&command, &outcome) {
            Ok(bytes) => (bytes, None),
            // The project changed but the engine did not get it: behind until a snapshot.
            Err(error) => (Vec::new(), Some(error.code())),
        };
        if commands::engine_msgs::replaces_project(&command) {
            // The folder of the previous project means nothing to this one: `project.open`
            // brings what its own folder holds, `project.new` has none.
            self.saver = Saver::opened(synced.ctx.opened.take());
        } else if (kind == commands::Kind::Edit && !outcome.events.is_empty())
            || session.is_some_and(|before| before != synced.state.session)
        {
            self.saver.edited();
        }
        let report = RunReport::Applied {
            output: outcome.output,
            events: outcome.events,
            stream_error,
        };
        (report, bytes)
    }

    /// A `Reset` in the current epoch and the snapshot of the project: what a fresh
    /// engine receives first.
    pub fn setup(&mut self) -> (SnapshotReport, Vec<u8>) {
        let result = self.synced.setup();
        self.snapshot_report(result)
    }

    /// The same in the next epoch: what an engine switched to, or one that refused the
    /// stream, receives to catch up with the project.
    pub fn snapshot(&mut self) -> (SnapshotReport, Vec<u8>) {
        let result = self.synced.resync();
        self.snapshot_report(result)
    }

    fn snapshot_report(
        &self,
        result: Result<Vec<u8>, engine::msg::MsgError>,
    ) -> (SnapshotReport, Vec<u8>) {
        match result {
            Ok(bytes) => (
                SnapshotReport::Encoded {
                    epoch: self.synced.epoch(),
                },
                bytes,
            ),
            Err(error) => (
                SnapshotReport::Failed {
                    stream_error: error.code(),
                },
                Vec::new(),
            ),
        }
    }

    /// The project and the session as they are now.
    pub fn view(&self) -> ProjectView {
        ProjectView {
            project: self.synced.state.project.clone(),
            session: self.synced.state.session.clone(),
            slots: commands::host::ViewSlots::of(&self.synced.ctx.engine, self.synced.epoch()),
        }
    }

    /// Stages the folder in `frame` (see [`parse_folder`]) for the next `project.open` that
    /// names it, replacing a folder staged before. Nothing is staged when the frame is
    /// malformed.
    pub fn stage(&mut self, frame: &[u8]) -> Result<(), FolderFrameError> {
        let (name, fs) = parse_folder(frame)?;
        self.synced.ctx.folder = Some(Folder::new(name, fs));
        Ok(())
    }

    /// Computes the writes that bring the project's folder up to date (see [`save`]); the
    /// frame is the call's output. Pending until [`Core::saved`].
    pub fn save(&mut self, mode: Mode) -> Vec<u8> {
        if mode == Mode::Copy {
            // A copy is another project: it must not share an identity with the folder it
            // came from. Should this save not finish, the project keeps the new identity
            // (nothing else names it) and its own folder is rewritten with it.
            self.synced.state.project.id = ProjectId::generate(&mut self.synced.ctx.ids);
            self.saver.edited();
        }
        let state = &self.synced.state;
        self.saver.start(mode, &state.project, &state.session)
    }

    /// The page applied the writes of the last [`Core::save`] to the folder, or could not.
    pub fn saved(&mut self, applied: bool) {
        self.saver.finish(applied);
    }

    /// Whether the project has changes the folder does not hold.
    pub fn dirty(&self) -> bool {
        self.saver.dirty()
    }

    /// The engine stream's epoch.
    pub fn epoch(&self) -> u16 {
        self.synced.epoch()
    }

    /// The number of entries that can be undone.
    #[cfg(test)]
    pub fn undo_depth(&self) -> usize {
        self.synced.state.history.undo.len()
    }
}

/// Why a folder frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FolderFrameError;

/// The most bytes of a folder's name.
const MAX_FOLDER_NAME: usize = 1024;
/// The most bytes of a file's path in a folder frame.
const MAX_FOLDER_PATH: usize = 1024;
/// The most bytes a folder frame may hold (the page's text files and automation blobs; the
/// audio is listed by size, never carried): the inbox grows to this much.
pub const MAX_FOLDER_FRAME: usize = 512 << 20;

/// A frame entry that carries the file's bytes.
const ENTRY_FILE: u8 = 0;
/// A frame entry that lists a blob by size, without its bytes.
const ENTRY_LISTED: u8 = 1;

/// A staged folder: the files the page carried, and the blobs it only listed. Read only.
///
/// Opening reads the project's text files and the blobs of dense automation lanes, and only
/// lists `blobs/` to find the media the project refers to that the folder lacks. So the page
/// stages the text and the lane blobs (they start with `FKAUTO01`) whole and every other blob
/// (audio, instrument state) by name and size: opening costs memory for the project, not for
/// its audio.
#[derive(Debug, Default)]
pub struct StagedFs {
    files: MemFs,
    /// `blobs/<name>` to its size.
    listed: std::collections::BTreeMap<String, u64>,
}

impl FileSystem for StagedFs {
    fn read(&self, path: &str) -> Result<Vec<u8>, FsError> {
        self.files.read(path)
    }

    fn write(&mut self, path: &str, _bytes: &[u8]) -> Result<(), FsError> {
        Err(FsError::Other(format!(
            "{path}: a staged folder is read only"
        )))
    }

    fn remove(&mut self, path: &str) -> Result<(), FsError> {
        Err(FsError::Other(format!(
            "{path}: a staged folder is read only"
        )))
    }

    fn exists(&self, path: &str) -> bool {
        self.files.exists(path) || self.listed.contains_key(path)
    }

    fn list(&self, dir: &str) -> Result<Vec<DirEntry>, FsError> {
        let mut entries = self.files.list(dir)?;
        if dir.is_empty() && !self.listed.is_empty() && !entries.iter().any(|e| e.name == "blobs") {
            entries.push(DirEntry {
                name: "blobs".to_owned(),
                is_dir: true,
                size: 0,
                stamp: 0,
            });
        }
        if dir == "blobs" {
            for (path, size) in &self.listed {
                if let Some(name) = path.strip_prefix("blobs/") {
                    entries.push(DirEntry {
                        name: name.to_owned(),
                        is_dir: false,
                        size: *size,
                        stamp: 0,
                    });
                }
            }
        }
        Ok(entries)
    }
}

/// A folder as the page hands it over: `u32` name length, the name (UTF-8), then for each
/// entry `u32` path length, the path (UTF-8, `/`-separated, relative), a kind byte
/// ([`ENTRY_FILE`]: `u64` size and that many bytes; [`ENTRY_LISTED`]: `u64` size, no bytes,
/// only for `blobs/<name>`); all integers little-endian. The page reads its storage (OPFS,
/// the folder the user picked) into this frame because the folder is read synchronously.
/// Every length is checked against what is left of the frame before anything is copied, and
/// a path that is empty, absolute, has a `..`, `.` or empty segment, a backslash or a NUL,
/// or a path given twice, refuses the whole frame; so does a frame over
/// [`MAX_FOLDER_FRAME`].
pub fn parse_folder(frame: &[u8]) -> Result<(String, StagedFs), FolderFrameError> {
    fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], FolderFrameError> {
        if n > rest.len() {
            return Err(FolderFrameError);
        }
        let (head, tail) = rest.split_at(n);
        *rest = tail;
        Ok(head)
    }
    fn u32_of(rest: &mut &[u8]) -> Result<usize, FolderFrameError> {
        let bytes: [u8; 4] = take(rest, 4)?.try_into().map_err(|_| FolderFrameError)?;
        Ok(u32::from_le_bytes(bytes) as usize)
    }
    if frame.len() > MAX_FOLDER_FRAME {
        return Err(FolderFrameError);
    }
    let mut rest = frame;
    let name_len = u32_of(&mut rest)?;
    if name_len > MAX_FOLDER_NAME {
        return Err(FolderFrameError);
    }
    let name = std::str::from_utf8(take(&mut rest, name_len)?)
        .map_err(|_| FolderFrameError)?
        .to_owned();
    let mut staged = StagedFs::default();
    while !rest.is_empty() {
        let path_len = u32_of(&mut rest)?;
        if path_len > MAX_FOLDER_PATH {
            return Err(FolderFrameError);
        }
        let path = std::str::from_utf8(take(&mut rest, path_len)?).map_err(|_| FolderFrameError)?;
        let valid = !path.is_empty()
            && !path.contains(['\\', '\0'])
            && path
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if !valid || staged.exists(path) {
            return Err(FolderFrameError);
        }
        let kind = take(&mut rest, 1)?[0];
        let size: [u8; 8] = take(&mut rest, 8)?
            .try_into()
            .map_err(|_| FolderFrameError)?;
        let size = u64::from_le_bytes(size);
        match kind {
            ENTRY_FILE => {
                let size = usize::try_from(size).map_err(|_| FolderFrameError)?;
                staged.files.insert(path, take(&mut rest, size)?);
            }
            ENTRY_LISTED => {
                let flat_blob = path
                    .strip_prefix("blobs/")
                    .is_some_and(|name| !name.contains('/'));
                if !flat_blob {
                    return Err(FolderFrameError);
                }
                staged.listed.insert(path.to_owned(), size);
            }
            _ => return Err(FolderFrameError),
        }
    }
    Ok((name, staged))
}

// ---------------------------------------------------------------- C ABI

/// The address of an inbox of at least `len` bytes (it may move between calls); null when
/// the memory is refused.
#[unsafe(no_mangle)]
pub extern "C" fn pw_inbox(len: u32) -> *mut u8 {
    INBOX.with_borrow_mut(|inbox| {
        let len = len as usize;
        if len > inbox.len() {
            if inbox.try_reserve_exact(len - inbox.len()).is_err() {
                return core::ptr::null_mut();
            }
            inbox.resize(len, 0);
        }
        inbox.as_mut_ptr()
    })
}

/// The address of the last call's JSON (valid until the next call).
#[unsafe(no_mangle)]
pub extern "C" fn pw_out() -> *const u8 {
    OUT.with_borrow(|out| out.as_ptr())
}

/// The address of the last call's engine bytes (valid until the next call).
#[unsafe(no_mangle)]
pub extern "C" fn pw_engine() -> *const u8 {
    ENGINE.with_borrow(|engine| engine.as_ptr())
}

/// The length of the last call's engine bytes (0 when it produced none).
#[unsafe(no_mangle)]
pub extern "C" fn pw_engine_len() -> u32 {
    ENGINE.with_borrow(|engine| engine.len() as u32)
}

/// A new core ([`Core::new`]): `kind` is [`KIND_EMPTY`] or [`KIND_SINGLE_SYNTH`], the
/// random seed is `r0 | r1 << 32 | r2 << 64` (80 bits kept). Null for an unknown kind.
/// The pointer is the page's handle for the other `pw_*` calls; [`pw_free`] frees it once.
#[unsafe(no_mangle)]
pub extern "C" fn pw_new(
    kind: u32,
    epoch: u32,
    time_ms: f64,
    r0: u32,
    r1: u32,
    r2: u32,
) -> *mut Core {
    // A clock before 1970 becomes 0 and NaN gives 0 (`as` saturates).
    let time_ms = time_ms as u64;
    let random = u128::from(r0) | (u128::from(r1) << 32) | (u128::from(r2) << 64);
    match Core::new(kind, epoch as u16, time_ms, random) {
        Some(core) => Box::into_raw(Box::new(core)),
        None => core::ptr::null_mut(),
    }
}

/// Frees a core.
///
/// # Safety
/// `core` must be null or a pointer from [`pw_new`], freed at most once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_free(core: *mut Core) {
    if !core.is_null() {
        // SAFETY: the caller guarantees a pointer from `pw_new`, freed once.
        drop(unsafe { Box::from_raw(core) });
    }
}

/// Publishes a call's JSON and engine bytes; returns the JSON's length, or
/// [`ERR_SERIALIZE`] (nothing published).
fn publish<T: Serialize>(report: &T, engine: Vec<u8>) -> i32 {
    let Ok(json) = serde_json::to_vec(report) else {
        OUT.with_borrow_mut(Vec::clear);
        return ERR_SERIALIZE;
    };
    let len = json.len() as i32;
    OUT.with_borrow_mut(|out| *out = json);
    ENGINE.with_borrow_mut(|out| *out = engine);
    len
}

/// The first `len` bytes of the inbox.
fn with_inbox<R>(len: u32, f: impl FnOnce(&[u8]) -> R) -> R {
    INBOX.with_borrow(|inbox| f(&inbox[..(len as usize).min(inbox.len())]))
}

/// Runs `f` on the core after clearing the engine bytes, or returns [`ERR_NO_CORE`].
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
unsafe fn with_core(core: *mut Core, f: impl FnOnce(&mut Core) -> i32) -> i32 {
    ENGINE.with_borrow_mut(Vec::clear);
    // SAFETY: forwarded from the caller's contract; one thread, so no other reference.
    match unsafe { core.as_mut() } {
        Some(core) => f(core),
        None => ERR_NO_CORE,
    }
}

/// Runs the command whose JSON is the first `len` bytes of the inbox: a [`RunReport`] at
/// [`pw_out`], the engine bytes at [`pw_engine`].
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_run(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            let (report, bytes) = with_inbox(len, |json| core.run(json));
            publish(&report, bytes)
        })
    }
}

/// [`Core::setup`]: a [`SnapshotReport`] and the engine bytes.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_setup(core: *mut Core) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            let (report, bytes) = core.setup();
            publish(&report, bytes)
        })
    }
}

/// [`Core::snapshot`]: a [`SnapshotReport`] and the engine bytes.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_snapshot(core: *mut Core) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            let (report, bytes) = core.snapshot();
            publish(&report, bytes)
        })
    }
}

/// [`Core::view`]: a [`ProjectView`].
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_project(core: *mut Core) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_core(core, |core| publish(&core.view(), Vec::new())) }
}

/// [`Core::stage`] with the folder frame in the first `len` bytes of the inbox: 0, or
/// [`ERR_BAD_FOLDER`] (nothing staged). The folder is opened by running `project.open`
/// with its name ([`pw_run`]).
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_stage(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            let staged = with_inbox(len, |frame| core.stage(frame));
            // A folder frame can be large and the inbox only grows: give its memory back.
            INBOX.with_borrow_mut(|inbox| *inbox = Vec::new());
            match staged {
                Ok(()) => 0,
                Err(_) => ERR_BAD_FOLDER,
            }
        })
    }
}

/// [`Core::save`] in `mode` ([`Mode::from_code`]: 0 changed, 1 forced, 2 everything, 3 copy, 4 rewrite): the
/// frame of [`save`] at [`pw_out`], its length returned; [`ERR_BAD_INPUT`] for another mode.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_save(core: *mut Core, mode: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            let Some(mode) = Mode::from_code(mode) else {
                return ERR_BAD_INPUT;
            };
            let frame = core.save(mode);
            let len = frame.len() as i32;
            OUT.with_borrow_mut(|out| *out = frame);
            len
        })
    }
}

/// [`Core::saved`]: `applied` is 1 when the page wrote every entry of the last frame.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_saved(core: *mut Core, applied: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            core.saved(applied == 1);
            0
        })
    }
}

/// [`Core::dirty`]: 1 when the project has changes the folder does not hold, else 0.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_dirty(core: *mut Core) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_core(core, |core| i32::from(core.dirty())) }
}

#[cfg(test)]
mod tests;
