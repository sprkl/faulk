//! The command contract v2 (CT-4): what every command receives, returns and declares.
//!
//! Decisions frozen here (spec § 2.4, project model § 5.2):
//!
//! - **Mutable model, explicit inverses.** A command mutates the [`State`] in place and
//!   returns its inverse command in [`Outcome::undo`]. Undo is `apply(inverse)` through
//!   the same path, so it yields its own [`ProjectEvent`]s and [`ControlMsg`]s: the
//!   interface and the engine follow an undo like any edit. Snapshot undo is not
//!   retained: it would need a project diff to drive the engine.
//! - **Metadata** (exported in the schema as `x-kind`, `x-undo`, `x-destructive`):
//!   [`Kind`], [`UndoKind`] and the destructive flag, with the consistency rules of
//!   [`Meta::check`] which the test rule enforces on every command.
//! - **Typed input and output**: each command has an `Output` variant of the same name.

use project::file::{FileError, FileSystem, SaveState};
use project::{
    AutomationTarget, ClipId, DeviceId, EngineSlots, IdGenerator, LaneId, PlacementId, Project,
    Quantize, SceneId, SessionState, TickLoop, TrackId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

/// What a command applies to: the project document, the session state (founder's
/// decision 10: solo, selected MIDI input, selection live in the session, never in the
/// versioned project) and the undo history, which only [`crate::run`] and the
/// `history.*` commands change. No `Default`: a project is created with its id.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub project: Project,
    pub session: SessionState,
    pub history: crate::History,
}

impl State {
    /// A state on a project, with an empty session and an empty history.
    pub fn new(project: Project) -> Self {
        Self {
            project,
            session: SessionState::default(),
            history: crate::History::default(),
        }
    }
}

/// Who runs a command. Serialized as `user` or `agent:<name>`: the form of the
/// `Faulk-Author` trailer in version messages and of the history entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub enum Author {
    User,
    Agent { name: String },
}

impl From<Author> for String {
    fn from(author: Author) -> String {
        match author {
            Author::User => "user".to_owned(),
            Author::Agent { name } => format!("agent:{name}"),
        }
    }
}

impl TryFrom<String> for Author {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        if s == "user" {
            return Ok(Self::User);
        }
        match s.strip_prefix("agent:") {
            Some(name) if !name.is_empty() => Ok(Self::Agent {
                name: name.to_owned(),
            }),
            _ => Err(format!("an author is `user` or `agent:<name>`, not {s:?}")),
        }
    }
}

/// Identifier of a history group: the commands of one mouse drag or one agent turn
/// share a group and are undone together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct GroupId(pub u64);

/// Who runs the command, in which history group, where new identifiers come from, and
/// what the engine holds. The history (PR-2) stamps author and group on every entry. The
/// generator and the engine's slot table are here, not in the [`State`], so that the
/// project stays pure (the host seeds the generator with its clock and randomness) and so
/// that undoing a command restores the state exactly: the slot table depends on the
/// order things were created and deleted, and belongs to one engine connection, rebuilt
/// by every snapshot ([`crate::engine_msgs::resync`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub author: Author,
    pub group: Option<GroupId>,
    /// The host's clock in milliseconds since the Unix epoch, set before each command: the
    /// history stamps it on the entries it records (0 when the host has none). The commands
    /// never read it, so the project stays pure.
    pub now_ms: u64,
    pub ids: IdGenerator,
    /// The engine's slots of the project's entities (PR-3).
    pub engine: EngineSlots,
    /// The project folder the host staged for `project.open` (SV-4): taken by the command,
    /// so a folder serves one opening.
    pub folder: Option<Folder>,
    /// What a later save needs of the folder `project.open` last read (SV-2): set by the
    /// command, taken by the host that keeps saving into that folder.
    pub opened: Option<OpenedFolder>,
    /// The projects the host keeps, staged for `project.list_stored` (#323): taken by the
    /// command, so a list serves one answer and a later one never shows an old list.
    pub stored: Option<StoredProjects>,
}

/// The projects a host can open, as it lists them: what `project.list_stored` answers and the
/// names `project.open_recent`, `project.open_browser_project` and
/// `project.delete_browser_project` take.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredProjects {
    /// The folders opened recently, newest first.
    pub recent: Vec<StoredFolder>,
    /// The projects kept in the browser's own storage, newest first.
    pub browser: Vec<StoredBrowserProject>,
}

/// A folder opened recently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredFolder {
    /// What `project.open_recent` takes.
    pub id: String,
    /// The folder's name.
    pub name: String,
}

/// A project kept in the browser's own storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredBrowserProject {
    /// What `project.open_browser_project` and `project.delete_browser_project` take.
    pub dir: String,
    /// The project's name.
    pub name: String,
}

/// What a save into a folder that was opened needs to know of it: the hash of each file as
/// read (so that a save writes only what changed) and the blobs it holds (so that a lane
/// blob already there is not written again).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedFolder {
    pub state: SaveState,
    /// The names of the files under `blobs/`, without the directory.
    pub blobs: BTreeSet<String>,
    /// The names of the files at the folder's root (`.gitignore` and `.gitattributes` are
    /// written only when absent).
    pub root: BTreeSet<String>,
}

/// A project folder a host read access to, staged in the [`Context`] for `project.open`.
/// The host decides where it lives: the page fills a `MemFs` from the browser's storage,
/// the helper wraps the disk. The command reads it and never writes.
#[derive(Clone)]
pub struct Folder {
    /// What the host calls this folder; `project.open` names it, so a folder staged for
    /// another opening is never taken by mistake.
    pub name: String,
    fs: Arc<dyn FileSystem + Send + Sync>,
}

impl Folder {
    pub fn new(name: impl Into<String>, fs: impl FileSystem + Send + Sync + 'static) -> Self {
        Self {
            name: name.into(),
            fs: Arc::new(fs),
        }
    }

    /// The folder's files.
    pub fn fs(&self) -> &dyn FileSystem {
        &*self.fs
    }
}

impl fmt::Debug for Folder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Folder").field("name", &self.name).finish()
    }
}

impl PartialEq for Folder {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && Arc::ptr_eq(&self.fs, &other.fs)
    }
}

impl Eq for Folder {}

impl Context {
    /// A context for an engine that holds nothing yet: the host sends a snapshot
    /// ([`crate::engine_msgs::resync`]) before the commands.
    pub fn new(author: Author, ids: IdGenerator) -> Self {
        Self {
            author,
            group: None,
            now_ms: 0,
            ids,
            engine: EngineSlots::default(),
            folder: None,
            opened: None,
            stored: None,
        }
    }
}

/// Whether a command enters the history (`x-kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Changes the project: enters the history. An edit that changes nothing (a value
    /// already in place) returns no inverse and no event, and the history records
    /// nothing.
    Edit,
    /// A setting saved with the project that is not part of the work being undone, like
    /// the loop switch or the metronome: changes the project but never enters the undo
    /// history; the history's activity log lists it. Never undoable nor destructive.
    Setting,
    /// Ephemeral, like a played note, playback, arming or the selection: changes the
    /// session or the engine, never the project, and never enters the history.
    Live,
    /// Read-only: changes nothing.
    Query,
}

/// How a command is undone (`x-undo`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UndoKind {
    /// [`Outcome::undo`] carries the inverse command.
    Inverse,
    /// Not undoable.
    None,
}

/// A command's metadata, as declared in its domain and exported in its schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Meta {
    /// The command's name, `domain.verb`.
    pub name: &'static str,
    pub kind: Kind,
    pub undo: UndoKind,
    /// Irreversible: the interface and the MCP server ask for a confirmation first.
    pub destructive: bool,
}

impl Meta {
    /// The consistency rules between the three metadata, checked on every command by
    /// the test rule: `setting`, `live` and `query` are never undoable nor destructive;
    /// an `edit` carries an inverse, or is irreversible and then declared destructive.
    pub fn check(&self) -> Result<(), String> {
        match (self.kind, self.undo, self.destructive) {
            (Kind::Setting, UndoKind::Inverse, _) => Err(format!(
                "{}: a setting never enters the undo history, so it cannot carry an inverse",
                self.name
            )),
            (Kind::Setting, UndoKind::None, true) => {
                Err(format!("{}: only an edit can be destructive", self.name))
            }
            (Kind::Live | Kind::Query, UndoKind::Inverse, _) => Err(format!(
                "{}: a {:?} command never enters the history, so it cannot carry an inverse",
                self.name, self.kind
            )),
            (Kind::Live | Kind::Query, UndoKind::None, true) => {
                Err(format!("{}: only an edit can be destructive", self.name))
            }
            (Kind::Edit, UndoKind::None, false) => Err(format!(
                "{}: an edit without an inverse is irreversible and must be declared destructive",
                self.name
            )),
            (Kind::Edit, UndoKind::Inverse, true) => Err(format!(
                "{}: an undoable edit is not destructive",
                self.name
            )),
            _ => Ok(()),
        }
    }
}

/// The undo part of an [`Outcome`].
///
/// An inverse is an ordinary command: undo applies it through the same path, so it
/// yields its own events and engine messages, and it is in the catalog like any other
/// (the inverse of a delete is a restore command that carries the entity with its
/// identifiers, refused if they collide with existing ones). An inverse is built from
/// what the command actually changed, never from its arguments alone: adding a note
/// that already exists changes nothing, so its inverse removes nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum Undo<C> {
    /// Applying this command restores the state as it was before the command, with
    /// its own events for the interface. Exact because a state is always valid: the
    /// previous value passes the command's checks again.
    Inverse(Box<C>),
    /// Nothing to undo.
    None,
}

/// What the interface learns from an applied command: its mirror of the project is fed
/// by these events only (spec § 2.6). Each task adds the events its commands emit, next
/// to them in this enum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectEvent {
    /// The project was renamed.
    ProjectRenamed { name: String },
    /// Another project replaced this one (`project.open`): the page reads the project, its
    /// session and its slots again.
    ProjectOpened { name: String },
    /// The selected track changed.
    TrackSelected { track: Option<TrackId> },
    /// A device parameter changed.
    DeviceParamChanged {
        device: DeviceId,
        param: String,
        value: f64,
    },
    /// A track's chain of devices changed: a device was inserted, removed, moved or
    /// brought back, or the instrument was replaced. `track` is `null` for the master's
    /// chain. The page reads the chain from its own copy of the project, to which the same
    /// command applies.
    DeviceChainChanged { track: Option<TrackId> },
    /// A device was bypassed or put back in the chain.
    DeviceBypassChanged { device: DeviceId, bypass: bool },
    /// A track's or the master's mixer changed: its whole state, so the interface needs
    /// no merge. `track` is `null` for the master; `gain_db` is `null` for silence.
    MixerChanged {
        track: Option<TrackId>,
        gain_db: Option<f64>,
        pan: f64,
        mute: bool,
    },
    /// A manual gesture on a control that an automation lane drives set it to the value the
    /// project already holds (#417): the command changed nothing and recorded no history
    /// entry, but the lane still gives way to the gesture (AM-2). `target` is the control;
    /// `track` owns it: the track of a mixer field or of a device, `null` for the master's
    /// fader, pan and chain. Emitted only when a lane with points drives the control. It
    /// changes no state: the page does not mark the project as edited for it.
    GestureUnchanged {
        track: Option<TrackId>,
        target: AutomationTarget,
    },
    /// A track was soloed or unsoloed (session state).
    SoloChanged { track: TrackId, solo: bool },
    /// A track was armed or disarmed (session state).
    ArmChanged { track: TrackId, armed: bool },
    /// A track's monitoring was switched on or off (session state).
    MonitorChanged { track: TrackId, monitor: bool },
    /// A control change (CC) came in on the live input: `controller` 0 to 127, `value`
    /// 0 to 127, channels merged. For the automation (AM-5); it changes no state.
    ControlChange { controller: u8, value: u8 },
    /// The tempo changed (the project's one tempo, in BPM).
    TempoChanged { bpm: f64 },
    /// The time signature changed (the project's one signature).
    TimeSignatureChanged { numerator: u8, denominator: u8 },
    /// The launch quantization changed (the project's one value).
    LaunchQuantizeChanged { quantize: Quantize },
    /// The loop region or whether the transport loops changed; `region` is `null` when
    /// none is set.
    LoopChanged {
        region: Option<TickLoop>,
        enabled: bool,
    },
    /// The metronome settings changed: the whole state; `gain_db` is `null` for silence.
    MetronomeChanged {
        enabled: bool,
        gain_db: Option<f64>,
        count_in_bars: u8,
    },
    /// The chosen MIDI input changed; `null` when none is chosen.
    MidiInputSelected { id: Option<String> },
    /// A clip was created or brought back, with the placements it came with. The page
    /// reads the clip from its own copy of the project, to which the same command applies.
    ClipCreated {
        track: TrackId,
        clip: ClipId,
        placements: Vec<PlacementId>,
    },
    /// A clip was deleted, with its placements. `clip.move` also reports it (with no
    /// placements, which the clip keeps) before the `ClipCreated` that follows.
    ClipDeleted {
        track: TrackId,
        clip: ClipId,
        placements: Vec<PlacementId>,
    },
    /// Part of a clip changed.
    ClipChanged { clip: ClipId, part: ClipPart },
    /// The placements of a track changed: some were placed, moved, resized, split,
    /// carved by the overlap rule, deleted or brought back (AR-1). The page reads them
    /// from its own copy of the project, to which the same command applies. A command
    /// that also creates or removes clips reports them with `ClipCreated` and
    /// `ClipDeleted` first.
    ArrangementChanged { track: TrackId },
    /// A track was created or brought back, with everything it holds. The page reads it
    /// from its own copy of the project, to which the same command applies.
    TrackCreated { track: TrackId },
    /// A track was deleted, with everything it holds.
    TrackDeleted { track: TrackId },
    /// Part of a track changed.
    TrackChanged { track: TrackId, part: TrackPart },
    /// A scene was created or brought back, with the clips of its slots. The page reads
    /// them from its own copy of the project, to which the same command applies.
    SceneCreated { scene: SceneId },
    /// A scene was deleted; the clips of its slots were deleted before (`ClipDeleted`).
    SceneDeleted { scene: SceneId },
    /// Part of a scene changed.
    SceneChanged { scene: SceneId, part: ScenePart },
    /// The section markers changed (#473): added, renamed, moved, deleted, brought back, or
    /// shifted by a time edit. The page reads them from its own copy of the project.
    MarkersChanged {},
    /// An automation lane was created or brought back, with its points (AM-1). The page
    /// reads the lane from its own copy of the project, to which the same command applies.
    /// `track` is `null` for a lane of the master (#378).
    AutomationLaneCreated {
        track: Option<TrackId>,
        lane: LaneId,
    },
    /// An automation lane was deleted, with its points: by `automation.lane.delete`, or
    /// because the device it targets was removed. `track` is `null` for a lane of the master.
    AutomationLaneDeleted {
        track: Option<TrackId>,
        lane: LaneId,
    },
    /// The points of a lane changed: some were added, removed, changed or cleared. `track` is
    /// `null` for a lane of the master.
    AutomationPointsChanged {
        track: Option<TrackId>,
        lane: LaneId,
    },
    /// The undo history changed: an entry was recorded, undone or redone, or the
    /// history was cleared.
    HistoryChanged { can_undo: bool, can_redo: bool },
    /// A take of MIDI recording changed state (`record.*`, and `transport.stop` or
    /// `transport.pause` during a take). Session state: the project does not change.
    RecordingChanged { take: u64, state: TakeState },
    /// The settings of MIDI recording changed (session state).
    RecordSettingsChanged {
        offset_ms: i32,
        replace: bool,
        quantize_on_input: project::InputGrid,
        quantize_strength: u8,
    },
}

/// Where a take of MIDI recording stands ([`ProjectEvent::RecordingChanged`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TakeState {
    /// The engine captures it.
    Recording,
    /// The capture ended; the take waits to be written (`record.commit_take`) or dropped.
    Stopped,
    /// Written into the project.
    Committed,
    /// Dropped without writing anything.
    Discarded,
}

impl ProjectEvent {
    /// Whether the event reports a change of the project, as opposed to a gesture that left
    /// it as it was ([`ProjectEvent::GestureUnchanged`]): the hosts mark the project as
    /// edited (unsaved, autosave) for the first only.
    pub fn changes_project(&self) -> bool {
        // The history and the session are not the project's files: a solo, or an undo
        // that only moves the history, leaves the project as it was (#473).
        !matches!(
            self,
            ProjectEvent::GestureUnchanged { .. }
                | ProjectEvent::RecordingChanged { .. }
                | ProjectEvent::RecordSettingsChanged { .. }
                | ProjectEvent::HistoryChanged { .. }
                | ProjectEvent::SoloChanged { .. }
                | ProjectEvent::ArmChanged { .. }
                | ProjectEvent::MonitorChanged { .. }
                | ProjectEvent::TrackSelected { .. }
                | ProjectEvent::MidiInputSelected { .. }
                | ProjectEvent::ControlChange { .. }
        )
    }
}

/// The event of a manual gesture that left a control as it was (#417): `Some` when a lane
/// with points drives the control (`owner` is the track that owns it, `None` for the
/// master), else nothing, so that every other unchanged command stays silent. A lane with
/// no point does nothing (AM-1) and the engine never receives it.
pub(crate) fn gesture_unchanged(
    project: &Project,
    owner: Option<&TrackId>,
    target: AutomationTarget,
) -> Option<ProjectEvent> {
    let drives = project.lanes(owner).is_some_and(|lanes| {
        lanes
            .values()
            .any(|lane| lane.target == target && !lane.points.is_empty())
    });
    drives.then(|| ProjectEvent::GestureUnchanged {
        track: owner.cloned(),
        target,
    })
}

/// Which part of a track a [`ProjectEvent::TrackChanged`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackPart {
    Name,
    Color,
    /// The track's position in the list changed.
    Order,
}

/// Which part of a scene a [`ProjectEvent::SceneChanged`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScenePart {
    Name,
    /// The scene's position in the list changed.
    Order,
}

/// Which part of a clip a [`ProjectEvent::ClipChanged`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClipPart {
    Name,
    Length,
    Loop,
    Notes,
    /// Length, loop and notes at once.
    Content,
}

/// What the project sends to the engine after a command (spec § 2.3): the engine's own
/// message type (CT-3), whose slots the project assigns ([`project::EngineSlots`]).
pub use engine::msg::ControlMsg;

/// What applying a command produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome<C, O> {
    /// The typed output, with the identifiers created.
    pub output: O,
    pub undo: Undo<C>,
    /// For the interface.
    pub events: Vec<ProjectEvent>,
    /// For the engine: what the change means for it, in order (PR-3). A domain leaves it
    /// empty; the root [`crate::Command::apply`] fills it from the command and its events
    /// ([`crate::engine_msgs::compile`]).
    pub control: Vec<ControlMsg>,
}

/// Why a command was refused. `code()` is stable for the interface and the agents;
/// the message is for developers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandError {
    /// The entity the command addresses does not exist.
    NotFound { entity: String, id: String },
    /// An input value is invalid.
    Invalid { field: String, message: String },
    /// A safety limit of the project would be exceeded (spec § 2.3: the engine never
    /// fails in real time, the project refuses first).
    Limit { entity: String, max: u64 },
    /// The command is declared but its domain does not implement it yet.
    Unsupported { command: String },
    /// `project.open` refused the folder: nothing changed.
    Folder { error: FolderError },
}

/// Why a project folder was refused (`project.open`). The page shows a text per variant;
/// `message`s are for developers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "reason", rename_all = "snake_case", deny_unknown_fields)]
pub enum FolderError {
    /// The host staged no folder of that name.
    NotStaged { folder: String },
    /// There is no `project.json`: not a Faulk project.
    NotAProject,
    /// Written by a newer Faulk than this build reads; never rewritten.
    NewerFormat { found: u32, supported: u32 },
    /// A format with no way to read it.
    UnsupportedFormat { found: u32 },
    /// A file the format requires is missing.
    Missing { path: String },
    /// A file is not what the format says.
    Corrupt { path: String, message: String },
    /// A file is bigger than the safety limit.
    TooLarge { path: String, size: u64, max: u64 },
    /// The files read fine but the project breaks its invariants: the first violations
    /// (`total` in all).
    Invalid { violations: Vec<String>, total: u32 },
    /// The storage failed.
    Io { path: String, message: String },
    /// Migrating an older format failed.
    Migration { from: u32, message: String },
}

/// The most violations a [`FolderError::Invalid`] lists.
const MAX_LISTED_VIOLATIONS: usize = 20;

impl From<FileError> for FolderError {
    fn from(error: FileError) -> Self {
        match error {
            FileError::NotAProject => Self::NotAProject,
            FileError::NewerFormat { found, supported } => Self::NewerFormat { found, supported },
            FileError::UnsupportedFormat { found } => Self::UnsupportedFormat { found },
            FileError::Missing { path } => Self::Missing { path },
            FileError::Corrupt { path, message } => Self::Corrupt { path, message },
            FileError::TooLarge { path, size, max } => Self::TooLarge { path, size, max },
            FileError::Invalid { violations } => Self::Invalid {
                total: u32::try_from(violations.len()).unwrap_or(u32::MAX),
                violations: violations
                    .iter()
                    .take(MAX_LISTED_VIOLATIONS)
                    .map(ToString::to_string)
                    .collect(),
            },
            FileError::Io { path, message } => Self::Io { path, message },
            FileError::Migration { from, message } => Self::Migration { from, message },
            // Writing and archives are not part of opening a folder; kept total anyway.
            FileError::Unwritable { path, message } => Self::Io { path, message },
            FileError::BadPack { message } => Self::Corrupt {
                path: String::new(),
                message,
            },
            FileError::DestinationNotEmpty => Self::Io {
                path: String::new(),
                message: "the destination is not empty".into(),
            },
        }
    }
}

impl CommandError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "not_found",
            Self::Invalid { .. } => "invalid",
            Self::Limit { .. } => "limit",
            Self::Unsupported { .. } => "unsupported",
            Self::Folder { .. } => "folder",
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { entity, id } => write!(f, "{entity} {id} not found"),
            Self::Invalid { field, message } => write!(f, "invalid {field}: {message}"),
            Self::Limit { entity, max } => write!(f, "at most {max} {entity}"),
            Self::Unsupported { command } => write!(f, "{command} is not implemented"),
            Self::Folder { error } => write!(f, "project folder refused: {error:?}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// Schema transform of a required `Option<T>` field (`#[schemars(required, transform = ...)]`):
/// the field accepts `null`, as the command does, and stays required. schemars drops the
/// `null` of an `Option` it is told to require, so an agent reading the schema would believe
/// `null` is refused (#561). The field's description stays on the field, beside the new
/// `anyOf`, not inside it.
pub(crate) fn allow_null(schema: &mut schemars::Schema) {
    let Some(inner) = schema.as_object_mut() else {
        return;
    };
    // Already nullable (`type: [.., "null"]`, or an `anyOf` with a `null` branch): nothing to add.
    let is_null = |v: &serde_json::Value| v == "null";
    let nullable = match (inner.get("type"), inner.get("anyOf")) {
        (Some(t), _) if is_null(t) => true,
        (Some(serde_json::Value::Array(types)), _) if types.iter().any(is_null) => true,
        (_, Some(serde_json::Value::Array(any))) => any.iter().any(|b| b["type"] == "null"),
        _ => false,
    };
    if nullable {
        return;
    }
    let mut outer = serde_json::Map::new();
    for key in ["title", "description", "default", "examples", "deprecated"] {
        if let Some(value) = inner.remove(key) {
            outer.insert(key.to_owned(), value);
        }
    }
    let branches = vec![
        serde_json::Value::Object(std::mem::take(inner)),
        serde_json::json!({ "type": "null" }),
    ];
    outer.insert("anyOf".to_owned(), branches.into());
    *inner = outer;
}

#[cfg(test)]
mod allow_null_tests {
    use super::allow_null;
    use schemars::json_schema;

    fn run(mut schema: schemars::Schema) -> serde_json::Value {
        allow_null(&mut schema);
        schema.into()
    }

    #[test]
    fn wraps_a_reference_and_keeps_the_description_outside() {
        let out = run(json_schema!({ "$ref": "#/$defs/Id", "description": "d" }));
        assert_eq!(
            out,
            serde_json::json!({
                "description": "d",
                "anyOf": [{ "$ref": "#/$defs/Id" }, { "type": "null" }]
            })
        );
    }

    #[test]
    fn does_not_wrap_a_nullable_schema_twice() {
        for nullable in [
            json_schema!({ "type": ["number", "null"] }),
            json_schema!({ "type": "null" }),
            json_schema!({ "anyOf": [{ "type": "string" }, { "type": "null" }] }),
        ] {
            let before: serde_json::Value = nullable.clone().into();
            assert_eq!(run(nullable), before);
        }
    }

    #[test]
    fn leaves_a_boolean_schema_alone() {
        assert_eq!(run(true.into()), serde_json::json!(true));
    }
}
