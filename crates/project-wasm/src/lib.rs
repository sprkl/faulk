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
use commands::host::{ExportReport, LiveRouting, ProjectView, RunReport, SnapshotReport};
use commands::{Author, Context, State};
use project::file::{DirEntry, FileSystem, FsError, MemFs};

mod pack;
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

/// [`pw_preset_file`]: no such preset (deleted, or never there).
pub const ERR_NO_PRESET: i32 = -5;
/// [`pw_preset_file`]: a preset of the project.
pub const LOCATION_PROJECT: u32 = 1;
/// [`pw_preset_file`]: a preset of the user's library.
pub const LOCATION_LIBRARY: u32 = 2;

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
            KIND_EMPTY => State::new(Project::new(
                ProjectId::generate(&mut ids),
                commands::project::DEFAULT_NAME,
            )),
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

    /// Names the project of a new core before any command ran: the page names its new draft
    /// (#473) as it creates it, so the name is no edit to undo. A name that is not valid
    /// (empty once trimmed, too long) leaves the default one. Returns whether it was taken.
    pub fn name_new(&mut self, name: &str) -> bool {
        match commands::project::check_project_name(name) {
            Ok(name) => {
                name.clone_into(&mut self.synced.state.project.name);
                true
            }
            Err(_) => false,
        }
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
        // The selection, solo, arming and MIDI input are saved with the project
        // (`.faulk/session.json`): a change of them is a change to save. Live commands change
        // them, and so do a solo (an undoable edit since #473) and its undo or redo.
        let session = (kind != commands::Kind::Query).then(|| synced.state.session.clone());
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
        } else if matches!(kind, commands::Kind::Edit | commands::Kind::Setting)
            && outcome
                .events
                .iter()
                .any(commands::ProjectEvent::changes_project)
        {
            // A gesture that left a control as it was (`GestureUnchanged`) is no edit.
            self.saver.edited();
        } else if session.is_some_and(|before| before != synced.state.session) {
            self.saver.session_edited();
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

    /// [`Core::snapshot`] for an engine the page did not start, in epoch `other` (the
    /// helper's, from its `hello`, BO-1): the `Reset` is newer than both
    /// (`Synced::resync_after`).
    pub fn snapshot_after(&mut self, other: u16) -> (SnapshotReport, Vec<u8>) {
        let result = self.synced.resync_after(other);
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

    /// Prepares the render of `project.export_audio` for these settings (the command's
    /// JSON, or just its fields): the report, and the setup and stop streams of an engine
    /// of its own, one after the other. Changes nothing in the project nor in the page's
    /// engine stream.
    pub fn export_job(&self, json: &[u8]) -> (ExportReport, Vec<u8>) {
        use commands::export::{ExportBitDepth, ExportError, ExportRegion, ExportSettings, job};
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Request {
            #[serde(default)]
            region: Option<ExportRegion>,
            #[serde(default)]
            bit_depth: Option<ExportBitDepth>,
            #[serde(default)]
            sample_rate: Option<u32>,
            #[serde(default)]
            from_bar: Option<u32>,
            #[serde(default)]
            to_bar: Option<u32>,
        }
        let refused = |error| (ExportReport::Refused { error }, Vec::new());
        let request = match serde_json::from_slice::<Request>(json) {
            Ok(request) => request,
            Err(error) => {
                return refused(commands::CommandError::Invalid {
                    field: "export".into(),
                    message: error.to_string(),
                });
            }
        };
        let settings = match ExportSettings::of(
            request.region,
            request.bit_depth,
            request.sample_rate,
            request.from_bar,
            request.to_bar,
        ) {
            Ok(settings) => settings,
            Err(error) => return refused(error),
        };
        let state = &self.synced.state;
        match job(&state.project, &state.session, settings) {
            Ok(job) => {
                let setup_len = job.setup.len() as u32;
                let start_len = job.start.len() as u32;
                let mut bytes = job.setup;
                bytes.extend(job.start);
                bytes.extend(job.stop);
                (
                    ExportReport::Ready {
                        plan: job.plan,
                        setup_len,
                        start_len,
                    },
                    bytes,
                )
            }
            Err(ExportError::Refused(error)) => refused(error),
            Err(ExportError::Stream(error)) => (
                ExportReport::Failed {
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

    /// Where the live input plays now, by engine slot, in the stream's epoch: what the page
    /// tells the helper in Boost (`live.routing`, #598).
    pub fn live_routing(&self) -> LiveRouting {
        LiveRouting::of(
            &self.synced.state,
            &self.synced.ctx.engine,
            self.synced.epoch(),
        )
    }

    /// The undo history as the history panel shows it.
    pub fn history(&self) -> commands::host::HistoryView {
        self.synced.state.history.view()
    }

    /// Who the commands run next are attributed to (`user` or `agent:<name>`): the history
    /// stamps it on their entries. A string that is neither changes nothing.
    pub fn set_author(&mut self, author: &[u8]) -> Result<(), AuthorError> {
        let text = core::str::from_utf8(author).map_err(|_| AuthorError)?;
        self.synced.ctx.author = Author::try_from(text.to_owned()).map_err(|_| AuthorError)?;
        Ok(())
    }

    /// The agent `author` (`agent:<name>`) went away (#641, the helper's `agent.left`): the
    /// group it left open is closed and its turn ends (`History::agent_left`). Whether
    /// anything changed; a string that is no agent's author changes nothing.
    pub fn agent_left(&mut self, author: &[u8]) -> Result<bool, AuthorError> {
        let text = core::str::from_utf8(author).map_err(|_| AuthorError)?;
        let author = Author::try_from(text.to_owned()).map_err(|_| AuthorError)?;
        if author == Author::User {
            return Err(AuthorError);
        }
        Ok(self.synced.state.history.agent_left(&author))
    }

    /// The page lost every link to the helper (#641): no agent can reach it nor be heard
    /// leaving, so an agent's open group is closed and any agent's turn ends
    /// (`History::agents_gone`). Whether anything changed.
    pub fn agents_gone(&mut self) -> bool {
        self.synced.state.history.agents_gone()
    }

    /// The host's clock for the commands run next, in milliseconds since the Unix epoch:
    /// the history stamps it on the entries it records.
    pub fn set_clock(&mut self, now_ms: u64) {
        self.synced.ctx.now_ms = now_ms;
    }

    /// Stages the folder in `frame` (see [`parse_folder`]) for the next `project.open` that
    /// names it, replacing a folder staged before. Nothing is staged when the frame is
    /// malformed.
    pub fn stage(&mut self, frame: &[u8]) -> Result<(), FolderFrameError> {
        let (name, fs) = parse_folder(frame)?;
        self.synced.ctx.folder = Some(Folder::new(name, fs));
        Ok(())
    }

    /// Stages the projects the page keeps, as the JSON of a [`commands::contract::StoredProjects`],
    /// for the next `project.list_stored` to answer with (#323), replacing a list staged before.
    /// Nothing is staged when the JSON is not that.
    pub fn stage_stored(&mut self, json: &[u8]) -> Result<(), StoredJsonError> {
        let stored = serde_json::from_slice(json).map_err(|_| StoredJsonError)?;
        self.synced.ctx.stored = Some(stored);
        Ok(())
    }

    /// Replaces the user's preset library the core holds with the files of `json` (a list of
    /// [`commands::host::LibraryFile`]), as the page's storage reads them (#191). A file whose
    /// id is not a preset id or whose text is not a preset is left out and named in the report;
    /// so is any file past [`project::preset::MAX_LIBRARY_PRESETS`]. Not an edit: the history
    /// is untouched. `Err` when the JSON is not such a list: nothing changes.
    pub fn stage_library(
        &mut self,
        json: &[u8],
    ) -> Result<commands::host::LibraryReport, StoredJsonError> {
        use commands::host::{LibraryFile, LibraryReport, SkippedPreset};
        use project::preset::{MAX_LIBRARY_PRESETS, Preset, PresetId};
        let files: Vec<LibraryFile> = serde_json::from_slice(json).map_err(|_| StoredJsonError)?;
        let mut library = std::collections::BTreeMap::new();
        let mut report = LibraryReport::default();
        for file in files {
            let skip = |reason: String| SkippedPreset {
                id: file.id.clone(),
                reason,
            };
            let parsed = PresetId::parse(&file.id)
                .map_err(|e| e.to_string())
                .and_then(|id| {
                    Preset::parse(file.text.as_bytes())
                        .map(|p| (id, p))
                        .map_err(|e| e.to_string())
                });
            match parsed {
                Ok(_) if library.len() >= MAX_LIBRARY_PRESETS => report.skipped.push(skip(
                    format!("the library holds at most {MAX_LIBRARY_PRESETS} presets"),
                )),
                Ok((id, preset)) => {
                    if library.insert(id, preset).is_some() {
                        report.skipped.push(skip("the same id twice".into()));
                    }
                }
                Err(reason) => report.skipped.push(skip(reason)),
            }
        }
        report.loaded = library.len() as u32;
        self.synced.state.library = library;
        Ok(report)
    }

    /// The file of a preset of the library or of the project (#191), in the common preset
    /// format: what the page writes after a `preset_changed` event. `None` when there is no
    /// such preset (it was deleted: the page removes its file), for the factory location and
    /// for an id that is not a preset id.
    pub fn preset_file(&self, location: u32, id: &[u8]) -> Option<Vec<u8>> {
        use project::preset::PresetId;
        let id = PresetId::parse(std::str::from_utf8(id).ok()?).ok()?;
        let state = &self.synced.state;
        let slots = match location {
            LOCATION_PROJECT => &state.project.presets,
            LOCATION_LIBRARY => &state.library,
            _ => return None,
        };
        slots.get(&id)?.to_file().ok()
    }

    /// Stages an audio file the page imported (AU-1), as the JSON of a `project::AudioMedia`
    /// (its SHA-256, rate, channels, frames and name, from decoding the file's bytes), for
    /// `clip.import_audio` to make clips of. Kept for the session; a second staging of the same
    /// file replaces the first. Nothing is staged when the JSON is not that.
    pub fn stage_media(&mut self, json: &[u8]) -> Result<(), StoredJsonError> {
        let media: project::AudioMedia =
            serde_json::from_slice(json).map_err(|_| StoredJsonError)?;
        // The page writes the blob before it stages the media: the file is there now.
        self.synced.ctx.missing_media.remove(&media.hash);
        self.synced.ctx.media.insert(media.hash, media);
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

    /// Whether the project itself, not only its session, has changes the folder does not hold.
    pub fn project_dirty(&self) -> bool {
        self.saver.project_dirty()
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

/// [`Core::stage_stored`]: the JSON is not a list of stored projects: nothing staged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoredJsonError;

/// [`Core::set_author`]: the string is not `user` nor `agent:<name>`: the author is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorError;

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

impl StagedFs {
    /// The size of a blob the folder lists without its bytes.
    pub(crate) fn listed_size(&self, path: &str) -> Option<u64> {
        self.listed.get(path).copied()
    }
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
/// `name_len` (#473): when not 0, the first `name_len` bytes of the inbox name the project
/// ([`Core::name_new`]); 0, or a name that is not valid, leaves the default name.
/// The pointer is the page's handle for the other `pw_*` calls; [`pw_free`] frees it once.
#[unsafe(no_mangle)]
pub extern "C" fn pw_new(
    kind: u32,
    epoch: u32,
    time_ms: f64,
    r0: u32,
    r1: u32,
    r2: u32,
    name_len: u32,
) -> *mut Core {
    // A clock before 1970 becomes 0 and NaN gives 0 (`as` saturates).
    let time_ms = time_ms as u64;
    let random = u128::from(r0) | (u128::from(r1) << 32) | (u128::from(r2) << 64);
    match Core::new(kind, epoch as u16, time_ms, random) {
        Some(mut made) => {
            if name_len > 0 {
                with_inbox(name_len, |bytes| {
                    if let Ok(name) = core::str::from_utf8(bytes) {
                        made.name_new(name);
                    }
                });
            }
            Box::into_raw(Box::new(made))
        }
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

/// [`Core::snapshot_after`] the engine epoch `other` (its low 16 bits): a
/// [`SnapshotReport`] and the engine bytes.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_snapshot_after(core: *mut Core, other: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            let (report, bytes) = core.snapshot_after(other as u16);
            publish(&report, bytes)
        })
    }
}

/// [`Core::export_job`] for the JSON in the first `len` bytes of the inbox: an
/// [`ExportReport`] at [`pw_out`], the engine bytes at [`pw_engine`].
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_export_job(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            let (report, bytes) = with_inbox(len, |json| core.export_job(json));
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

/// [`Core::live_routing`]: a `LiveRouting`.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_live_routing(core: *mut Core) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_core(core, |core| publish(&core.live_routing(), Vec::new())) }
}

/// [`Core::history`]: a `HistoryView`.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_history(core: *mut Core) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_core(core, |core| publish(&core.history(), Vec::new())) }
}

/// [`Core::set_clock`]: the page's `Date.now()`, a whole number of milliseconds (a negative or
/// non-finite value is 0).
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_set_clock(core: *mut Core, now_ms: f64) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            core.set_clock(if now_ms.is_finite() && now_ms > 0.0 {
                now_ms as u64
            } else {
                0
            });
            0
        })
    }
}

/// [`Core::set_author`] with the author in the first `len` bytes of the inbox: 0, or
/// [`ERR_BAD_INPUT`] (the author is unchanged).
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_set_author(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| match with_inbox(len, |a| core.set_author(a)) {
            Ok(()) => 0,
            Err(AuthorError) => ERR_BAD_INPUT,
        })
    }
}

/// [`Core::agent_left`] with the agent's author (`agent:<name>`) in the first `len` bytes of
/// the inbox: 1 when its group or its turn ended, 0 when it had neither, or
/// [`ERR_BAD_INPUT`] (not an agent's author; nothing changed).
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_agent_left(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| match with_inbox(len, |a| core.agent_left(a)) {
            Ok(changed) => i32::from(changed),
            Err(AuthorError) => ERR_BAD_INPUT,
        })
    }
}

/// [`Core::agents_gone`]: 1 when an agent's group or turn ended, else 0.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_agents_gone(core: *mut Core) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_core(core, |core| i32::from(core.agents_gone())) }
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

/// [`Core::stage_stored`] with the JSON in the first `len` bytes of the inbox: 0, or
/// [`ERR_BAD_INPUT`] (nothing staged). The list is read by running `project.list_stored`
/// ([`pw_run`]).
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_stage_stored(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            match with_inbox(len, |json| core.stage_stored(json)) {
                Ok(()) => 0,
                Err(StoredJsonError) => ERR_BAD_INPUT,
            }
        })
    }
}

/// [`Core::stage_library`] with the JSON in the first `len` bytes of the inbox: the
/// `LibraryReport` at [`pw_out`], its length returned; [`ERR_BAD_INPUT`] (nothing changed).
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_stage_library(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            match with_inbox(len, |json| core.stage_library(json)) {
                Ok(report) => publish(&report, Vec::new()),
                Err(StoredJsonError) => ERR_BAD_INPUT,
            }
        })
    }
}

/// [`Core::preset_file`] for `location` ([`LOCATION_PROJECT`] or [`LOCATION_LIBRARY`]) and the
/// id in the first `len` bytes of the inbox: the file's bytes at [`pw_out`], their length
/// returned; [`ERR_NO_PRESET`] when there is no such preset.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_preset_file(core: *mut Core, location: u32, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            match with_inbox(len, |id| core.preset_file(location, id)) {
                Some(bytes) => {
                    let len = bytes.len() as i32;
                    OUT.with_borrow_mut(|out| *out = bytes);
                    len
                }
                None => ERR_NO_PRESET,
            }
        })
    }
}

/// [`Core::stage_media`] with the first `len` bytes of the inbox: 0, or [`ERR_BAD_INPUT`]
/// (nothing staged). The clips are made by running `clip.import_audio` ([`pw_run`]).
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_stage_media(core: *mut Core, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_core(core, |core| {
            match with_inbox(len, |json| core.stage_media(json)) {
                Ok(()) => 0,
                Err(StoredJsonError) => ERR_BAD_INPUT,
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

/// Publishes the bytes of a pack call at [`pw_out`] and, with `free_inbox`, gives the inbox's
/// memory back (a pack can be large and the inbox only grows). The length returned, or
/// [`ERR_SERIALIZE`] when it does not fit the return type.
fn publish_pack(bytes: Vec<u8>, free_inbox: bool) -> i32 {
    ENGINE.with_borrow_mut(Vec::clear);
    if free_inbox {
        INBOX.with_borrow_mut(|inbox| *inbox = Vec::new());
    }
    let Ok(len) = i32::try_from(bytes.len()) else {
        OUT.with_borrow_mut(Vec::clear);
        return ERR_SERIALIZE;
    };
    OUT.with_borrow_mut(|out| *out = bytes);
    len
}

/// Starts the export of a `.faulkpack` of the folder frame in the first `len` bytes of the inbox
/// (see [`pack`]): the answer at [`pw_out`], its length returned.
#[unsafe(no_mangle)]
pub extern "C" fn pw_pack_begin(len: u32) -> i32 {
    publish_pack(with_inbox(len, pack::begin), true)
}

/// The next file of the export under way that the page must supply, or its end (see [`pack`]).
#[unsafe(no_mangle)]
pub extern "C" fn pw_pack_next() -> i32 {
    publish_pack(pack::next(), false)
}

/// The first `len` bytes of the inbox as the next bytes of the file the export asked for (see
/// [`pack`]).
#[unsafe(no_mangle)]
pub extern "C" fn pw_pack_feed(len: u32) -> i32 {
    publish_pack(with_inbox(len, pack::feed), false)
}

/// Ends the export under way, if any, and frees what it holds.
#[unsafe(no_mangle)]
pub extern "C" fn pw_pack_cancel() {
    pack::cancel();
    INBOX.with_borrow_mut(|inbox| *inbox = Vec::new());
}

/// The files of the `.faulkpack` in the first `len` bytes of the inbox, validated as a project
/// folder (see [`pack`]): the answer at [`pw_out`], its length returned.
#[unsafe(no_mangle)]
pub extern "C" fn pw_pack_import(len: u32) -> i32 {
    publish_pack(with_inbox(len, pack::import), true)
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

/// [`Core::project_dirty`]: 1 when the project itself (not only its session) has changes the
/// folder does not hold, else 0.
///
/// # Safety
/// `core` must be null or a live pointer from [`pw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_project_dirty(core: *mut Core) -> i32 {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_core(core, |core| i32::from(core.project_dirty())) }
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
