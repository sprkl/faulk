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

use project::{ClipId, DeviceId, IdGenerator, PlacementId, Project, SessionState, TrackId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

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

/// Who runs the command, in which history group, and where new identifiers come from.
/// The history (PR-2) stamps author and group on every entry. The generator is here,
/// not in the [`State`], so that the project stays pure (the host seeds it with its
/// clock and randomness) and so that undoing a command restores the state exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub author: Author,
    pub group: Option<GroupId>,
    pub ids: IdGenerator,
}

impl Context {
    pub fn new(author: Author, ids: IdGenerator) -> Self {
        Self {
            author,
            group: None,
            ids,
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
    /// Ephemeral, like a played note, playback, solo or the selection: changes the
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
    /// the test rule: `live` and `query` are never undoable nor destructive; an `edit`
    /// carries an inverse, or is irreversible and then declared destructive.
    pub fn check(&self) -> Result<(), String> {
        match (self.kind, self.undo, self.destructive) {
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
    /// The selected track changed.
    TrackSelected { track: Option<TrackId> },
    /// A device parameter changed.
    DeviceParamChanged {
        device: DeviceId,
        param: String,
        value: f64,
    },
    /// A track's or the master's mixer changed: its whole state, so the interface needs
    /// no merge. `track` is `null` for the master; `gain_db` is `null` for silence.
    MixerChanged {
        track: Option<TrackId>,
        gain_db: Option<f64>,
        pan: f64,
        mute: bool,
    },
    /// A track was soloed or unsoloed (session state).
    SoloChanged { track: TrackId, solo: bool },
    /// The chosen MIDI input changed; `null` when none is chosen.
    MidiInputSelected { id: Option<String> },
    /// A clip was created or brought back, with the placements it came with. The page
    /// reads the clip from its own copy of the project, to which the same command applies.
    ClipCreated {
        track: TrackId,
        clip: ClipId,
        placements: Vec<PlacementId>,
    },
    /// A clip was deleted, with its placements.
    ClipDeleted {
        track: TrackId,
        clip: ClipId,
        placements: Vec<PlacementId>,
    },
    /// Part of a clip changed.
    ClipChanged { clip: ClipId, part: ClipPart },
    /// The undo history changed: an entry was recorded, undone or redone, or the
    /// history was cleared.
    HistoryChanged { can_undo: bool, can_redo: bool },
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

/// What the project sends to the engine after a command (spec § 2.3). CT-3 (#39)
/// declares `engine::msg::ControlMsg`; until it is merged this placeholder has no
/// variant, so nothing can be sent, and the hosts translate the `live.*` and
/// `device.set_param` commands to the engine themselves. PR-3 replaces it by the
/// engine's type and compiles the project's ids to engine slots.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlMsg {}

/// What applying a command produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome<C, O> {
    /// The typed output, with the identifiers created.
    pub output: O,
    pub undo: Undo<C>,
    /// For the interface.
    pub events: Vec<ProjectEvent>,
    /// For the engine.
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
}

impl CommandError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "not_found",
            Self::Invalid { .. } => "invalid",
            Self::Limit { .. } => "limit",
            Self::Unsupported { .. } => "unsupported",
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
        }
    }
}

impl std::error::Error for CommandError {}
