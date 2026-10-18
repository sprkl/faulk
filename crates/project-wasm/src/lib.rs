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

use commands::engine_msgs::Synced;
use commands::host::{
    LoadError, LoadReport, ProjectChanges, ProjectFiles, ProjectView, RunReport, SaveReport,
    SnapshotReport, load_files, save_files,
};
use commands::{Author, Context, State};
use project::{IdGenerator, Project, ProjectId};
use serde::Serialize;

thread_local! {
    static INBOX: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static ENGINE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// No core: a null handle.
pub const ERR_NO_CORE: i32 = -1;
/// The input is not what the call takes (`pw_saved`: not a `ProjectChanges`).
pub const ERR_BAD_INPUT: i32 = -2;
/// The call's JSON did not serialize (a value the project's writers refuse): nothing
/// at [`pw_out`] nor [`pw_engine`]. The core is untouched.
pub const ERR_SERIALIZE: i32 = -3;

/// [`pw_new`]: an empty project.
pub const KIND_EMPTY: u32 = 0;
/// [`pw_new`]: the page's one-synth project (the ids the interface uses until PR-5).
pub const KIND_SINGLE_SYNTH: u32 = 1;

/// A project in the page: the project with its engine stream, and the text of each file
/// as last written (`save` gives the files that differ from it).
pub struct Core {
    synced: Synced,
    saved: ProjectFiles,
}

impl Core {
    /// A core of `kind`, whose engine is in `epoch`; the ids it creates come from the
    /// page's clock (`time_ms`) and randomness (`random`, 80 bits kept). Nothing saved
    /// yet: every file is dirty. `None` for an unknown kind.
    pub fn new(kind: u32, epoch: u16, time_ms: u64, random: u128) -> Option<Core> {
        let mut ids = IdGenerator::new(time_ms, random);
        let state = match kind {
            KIND_EMPTY => State::new(Project::new(ProjectId::generate(&mut ids), "Untitled")),
            KIND_SINGLE_SYNTH => commands::engine_msgs::single_synth::state(),
            _ => return None,
        };
        Some(Core {
            synced: Synced::new(state, Context::new(Author::User, ids), epoch),
            saved: ProjectFiles::new(),
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
        let outcome = match commands::run(&command, &mut synced.state, &mut synced.ctx) {
            Ok(outcome) => outcome,
            Err(error) => return (RunReport::Refused { error }, Vec::new()),
        };
        let (bytes, stream_error) = if outcome.control.is_empty() {
            (Vec::new(), None)
        } else {
            match synced.encode(&outcome.control) {
                Ok(bytes) => (bytes, None),
                // The project changed but the engine did not get it: behind until a
                // snapshot.
                Err(error) => (Vec::new(), Some(error.code())),
            }
        };
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
        }
    }

    /// Replaces the project with the one in `json` (a map of path → text), with an empty
    /// history, and snapshots it for the engine in the next epoch. The loaded project is
    /// clean. Refused files change nothing.
    pub fn load(&mut self, json: &[u8]) -> (LoadReport, Vec<u8>) {
        let loaded = serde_json::from_slice::<ProjectFiles>(json)
            .map_err(|error| LoadError::Parse {
                file: String::new(),
                message: error.to_string(),
            })
            .and_then(|files| load_files(&files));
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(error) => return (LoadReport::Refused { error }, Vec::new()),
        };
        // A loaded project serializes (it was just read from these writers' text); if
        // it did not, it stays fully dirty rather than wrongly clean.
        self.saved = save_files(&loaded.state).unwrap_or_default();
        self.synced.state = loaded.state;
        // A group opened on the previous project's history is gone with it.
        self.synced.ctx.group = None;
        let (snapshot, bytes) = self.snapshot();
        let report = LoadReport::Loaded {
            snapshot,
            session_reset: loaded.session_reset,
        };
        (report, bytes)
    }

    /// The changes since the last recorded write: the files whose text differs from it.
    /// Does not record them as written: the host calls [`Core::saved`] once the write
    /// succeeded.
    pub fn save(&self) -> SaveReport {
        match save_files(&self.synced.state) {
            Ok(files) => SaveReport::Changes {
                changes: ProjectChanges {
                    write: files
                        .into_iter()
                        .filter(|(path, text)| self.saved.get(path) != Some(text))
                        .collect(),
                    remove: Vec::new(),
                },
            },
            Err(error) => SaveReport::Failed {
                message: error.to_string(),
            },
        }
    }

    /// Records the changes the host made to the folder, with the text it wrote (what
    /// [`Core::save`] gave): an edit made since stays dirty. Paths that are not project
    /// files are ignored.
    pub fn saved(&mut self, json: &[u8]) -> Result<(), serde_json::Error> {
        let done = serde_json::from_slice::<ProjectChanges>(json)?;
        for path in done.remove {
            self.saved.remove(&path);
        }
        for (path, text) in done.write {
            if is_project_file(&path) {
                self.saved.insert(path, text);
            }
        }
        Ok(())
    }

    /// The engine stream's epoch.
    pub fn epoch(&self) -> u16 {
        self.synced.epoch()
    }

    /// The number of entries that can be undone (tests).
    pub fn undo_depth(&self) -> usize {
        self.synced.state.history.undo.len()
    }
}

/// The paths [`save_files`] writes.
fn is_project_file(path: &str) -> bool {
    path == commands::host::PROJECT_FILE || path == commands::host::SESSION_FILE
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

/// [`Core::load`] with the files' JSON in the first `len` bytes of the inbox: a
/// [`LoadReport`] and the engine bytes.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_load(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            let (report, bytes) = with_inbox(len, |json| core.load(json));
            publish(&report, bytes)
        })
    }
}

/// [`Core::save`]: a [`SaveReport`].
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_save(core: *mut Core) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_core(core, |core| publish(&core.save(), Vec::new())) }
}

/// [`Core::saved`] with the `ProjectChanges` JSON the host applied in the first `len`
/// bytes of the inbox: 0, or [`ERR_BAD_INPUT`].
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_saved(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            match with_inbox(len, |json| core.saved(json)) {
                Ok(()) => 0,
                Err(_) => ERR_BAD_INPUT,
            }
        })
    }
}

#[cfg(test)]
mod tests;
