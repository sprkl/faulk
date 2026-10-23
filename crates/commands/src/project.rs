//! `project.*`: the project itself: create, open, save and rename (epic 11, SV).
//!
//! `project.rename` is implemented here as the reference command of the contract: an
//! `edit` with its inverse, its event and its output, exercised by the test rule.

use crate::contract::{CommandError, Context, FolderError, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use project::EngineSlots;
use project::file::{OpenOptions, SessionLoad, load_session, open};

/// Longest project name accepted, in characters: the project's own limit.
pub use ::project::limits::MAX_NAME_CHARS;

domain! {
    domain = "project", schema = ("ProjectCommand", "ProjectOutput");

    /// Rename the project. The name is trimmed, not empty, at most 200 characters.
    /// Undo: rename it back to its previous name.
    "project.rename" (kind = "edit", undo = "inverse", destructive = false)
    Rename { name: String } => {
        /// The name before the command.
        previous: String,
    };

    /// Open the project folder the host staged under `folder` (the host reads the folder
    /// first; named any other way, the command is refused as `not_staged`), replacing the
    /// current project: the engine starts over from it (stopped, nothing held, the saved solo and
    /// selection restored from `.faulk/session.json`). A folder that is not a valid
    /// project of this version is refused with the reason and nothing changes. Replaces
    /// the history: not undoable, destructive (the caller confirms first when the current
    /// project has unsaved changes).
    "project.open" (kind = "edit", undo = "none", destructive = true)
    Open { folder: String } => {
        /// The name of the project opened.
        name: String,
        /// The format version the folder was in, when it was migrated in memory.
        migrated_from: Option<u32>,
        /// The session file was there but unreadable: the default session is used.
        session_reset: bool,
        /// Blobs the project refers to that the folder does not have (shown as missing
        /// media).
        missing_blobs: u32,
    };
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::Open { folder } => open_folder(folder, state, ctx),
        Command::Rename { name } => {
            let name = name.trim();
            if name.is_empty() {
                return Err(CommandError::Invalid {
                    field: "name".into(),
                    message: "a project name is not empty".into(),
                });
            }
            if name.chars().count() > MAX_NAME_CHARS {
                return Err(CommandError::Invalid {
                    field: "name".into(),
                    message: format!("a project name has at most {MAX_NAME_CHARS} characters"),
                });
            }
            if state.project.name == name {
                // Nothing changed: nothing to undo, nothing to tell the interface.
                return Ok(Outcome {
                    output: Output::Rename {
                        previous: name.to_owned(),
                    },
                    undo: Undo::None,
                    events: vec![],
                    control: vec![],
                });
            }
            let previous = std::mem::replace(&mut state.project.name, name.to_owned());
            Ok(Outcome {
                output: Output::Rename {
                    previous: previous.clone(),
                },
                undo: Undo::Inverse(Box::new(Command::Rename { name: previous })),
                events: vec![ProjectEvent::ProjectRenamed {
                    name: name.to_owned(),
                }],
                control: vec![],
            })
        }
    }
}

/// Replaces the state with the project of the staged folder. Everything is read, checked
/// and built before the state is touched, so a refusal leaves the current project, its
/// history and its session exactly as they were.
fn open_folder(
    folder: &str,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    let refuse = |error| CommandError::Folder { error };
    // Only a folder staged under this name is taken, and it serves one opening, whatever
    // comes of it: a refusal for any other name leaves the staged folder where it is.
    let Some(staged) = ctx.folder.take_if(|staged| staged.name == folder) else {
        return Err(refuse(FolderError::NotStaged {
            folder: folder.to_owned(),
        }));
    };
    // The cache snapshot is for the browser build (the host that stages a `MemFs`); a
    // folder without one reads its text.
    let opened = open(staged.fs(), &OpenOptions { use_cache: true })
        .map_err(|error| refuse(FolderError::from(error)))?;
    let (session, how) = load_session(staged.fs(), &opened.project);
    let name = opened.project.name.clone();
    let mut next = State::new(opened.project);
    next.session = session;
    let history_changed = next.history.changed_event();
    *state = next;
    // The old project's group and engine slots mean nothing to the new one.
    ctx.group = None;
    ctx.engine = EngineSlots::default();
    Ok(Outcome {
        output: Output::Open {
            name: name.clone(),
            migrated_from: opened.report.migrated_from,
            session_reset: how == SessionLoad::Reset,
            missing_blobs: u32::try_from(opened.report.missing_blobs.len()).unwrap_or(u32::MAX),
        },
        undo: Undo::None,
        events: vec![ProjectEvent::ProjectOpened { name }, history_changed],
        control: vec![],
    })
}
