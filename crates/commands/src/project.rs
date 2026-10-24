//! `project.*`: the project itself: create, open, save and rename (epic 11, SV).
//!
//! `project.rename` is implemented here as the reference command of the contract: an
//! `edit` with its inverse, its event and its output, exercised by the test rule.

use crate::contract::{
    CommandError, Context, FolderError, OpenedFolder, Outcome, ProjectEvent, State, Undo,
};
use crate::export::{self, ExportBitDepth, ExportRegion, ExportSettings};
use crate::macros::domain;
use project::file::{OpenOptions, SessionLoad, load_session, open};
use project::{EngineSlots, ProjectId, Tick};
use std::collections::BTreeSet;

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

    /// Start a new project: the page's default one, a single synth track, in no folder yet,
    /// with its own identity. It replaces the current project, its history and its session
    /// (the chosen MIDI input stays); the engine starts over from it. Not undoable, destructive: the caller confirms first
    /// when the current project has changes that are not saved.
    "project.new" (kind = "edit", undo = "none", destructive = true)
    New {} => {
        /// The name of the new project.
        name: String,
    };

    /// Write the project to its folder now, without waiting for the automatic save that
    /// follows each change by a moment. A project that has no folder yet asks for one, as
    /// `project.save_as` does. Changes nothing in the project, so it never enters the
    /// history.
    "project.save" (kind = "live", undo = "none", destructive = false)
    Save {} => {};

    /// Choose an empty folder and write the whole project into it; the project then lives
    /// in that folder, and the automatic save keeps writing there. The folder is chosen by
    /// the user (a browser needs a click for it), so an agent that is not driven by one gets
    /// a notice in the page instead. The old folder is left as it is. Changes nothing in the
    /// project, so it never enters the history.
    "project.save_as" (kind = "live", undo = "none", destructive = false)
    SaveAs {} => {};

    /// Export the project as a `.faulkpack`, a zip of its folder that any Faulk reads back,
    /// and hand it to the user as a download. It holds the project as it is now, saved or
    /// not, and the media the project refers to. This is how a project kept in the browser
    /// (no folder chosen) is backed up or moved. Changes nothing in the project, so it never
    /// enters the history.
    "project.export_pack" (kind = "live", undo = "none", destructive = false)
    ExportPack {} => {};

    /// Open a `.faulkpack` the user chooses: its project replaces the current one and is kept
    /// in the browser as a new project of its own (an import never overwrites a project that
    /// is already there). A file is chosen by the user (a browser needs a click for it), so
    /// an agent that is not driven by one gets a notice in the page instead. A file that is
    /// not a valid pack is refused with the reason and nothing changes. The history of the
    /// replaced project is gone; the page asks first when that project has changes that are
    /// not kept anywhere.
    "project.import_pack" (kind = "live", undo = "none", destructive = false)
    ImportPack {} => {};

    /// Render the project to an audio file (WAV) and save it, offline: the same engine
    /// that plays the song, run faster than real time, so the file is the song as it
    /// sounds, not a recording of the speakers. `region` is `song` (default: from bar 1
    /// to the end of the last placement of the arrangement, then the tail, so that an
    /// effect such as the echo rings out; the file ends where the sound falls under
    /// -90 dBFS, after 30 seconds at most), `loop` (the loop region once, then the same
    /// tail) or `bars` (from the start of bar `from_bar` to the end of bar `to_bar`, both
    /// counted from 1, required with it and refused without it, the last one included,
    /// then the same tail). `bit_depth` is `pcm16`, `pcm24` (default) or `float32`;
    /// `sample_rate` is 44100, 48000 (default) or 96000. The metronome is never in the
    /// file; the solo and mute state are. It is refused when there is nothing to render
    /// (empty arrangement, no loop region for `loop`, a `bars` range that starts after the
    /// end of the arrangement or is out of order) or the range lasts more than 30 minutes
    /// (a `limit` error). The output says what will be rendered; the page runs the render, shows its progress and asks
    /// where to save the file (a folder picker needs a click), so an agent that is not
    /// driven by a user gets the render in the page to confirm. Changes nothing in the
    /// project, so it never enters the history and has nothing to undo.
    "project.export_audio" (kind = "live", undo = "none", destructive = false)
    ExportAudio {
        #[serde(default)]
        region: Option<ExportRegion>,
        #[serde(default)]
        bit_depth: Option<ExportBitDepth>,
        #[serde(default)]
        #[schemars(range(min = 44100, max = 96000))]
        sample_rate: Option<u32>,
        #[serde(default)]
        #[schemars(range(min = 1))]
        from_bar: Option<u32>,
        #[serde(default)]
        #[schemars(range(min = 1))]
        to_bar: Option<u32>,
    } => {
        /// What is rendered: `song`, `loop` or `bars`.
        region: ExportRegion,
        /// The sample format of the file.
        bit_depth: ExportBitDepth,
        /// The sample rate of the file, in Hz.
        sample_rate: u32,
        /// Where the render starts, in ticks (960 per quarter note).
        start_tick: Tick,
        /// Where the content ends, in ticks; the tail follows.
        end_tick: Tick,
        /// Seconds of content, without the tail.
        duration_secs: f64,
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
        Command::New {} => Ok(new_project(state, ctx)),
        // The write belongs to the host (the page's storage; a folder picker needs a click):
        // the command only says it is wanted. Nothing in the project changes.
        Command::Save {} => Ok(nothing(Output::Save {})),
        Command::SaveAs {} => Ok(nothing(Output::SaveAs {})),
        Command::ExportPack {} => Ok(nothing(Output::ExportPack {})),
        Command::ImportPack {} => Ok(nothing(Output::ImportPack {})),
        // Like a save, the render belongs to the host (the page's worker and file picker):
        // the command checks the request against the project and says what will be
        // rendered.
        Command::ExportAudio {
            region,
            bit_depth,
            sample_rate,
            from_bar,
            to_bar,
        } => {
            let settings =
                ExportSettings::of(*region, *bit_depth, *sample_rate, *from_bar, *to_bar)?;
            let plan = export::plan(&state.project, settings)?;
            Ok(nothing(Output::ExportAudio {
                region: settings.region,
                bit_depth: settings.bit_depth,
                sample_rate: settings.sample_rate,
                start_tick: plan.start_tick,
                end_tick: plan.end_tick,
                duration_secs: plan.duration_secs,
            }))
        }
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

/// An outcome that changes nothing and tells the interface nothing.
fn nothing(output: Output) -> Outcome<Command, Output> {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

/// Replaces the state with a new project: the default one-synth project under a fresh
/// identity, an empty history and session, and no folder.
fn new_project(state: &mut State, ctx: &mut Context) -> Outcome<Command, Output> {
    let mut next = crate::engine_msgs::single_synth::state();
    next.project.id = ProjectId::generate(&mut ctx.ids);
    // The MIDI input is the player's device, not the song's: it stays chosen.
    next.session.midi_input = state.session.midi_input.take();
    let name = next.project.name.clone();
    let history_changed = next.history.changed_event();
    *state = next;
    ctx.group = None;
    ctx.engine = EngineSlots::default();
    // What a save knew of the old project's folder is no use to a project that has none.
    ctx.folder = None;
    ctx.opened = None;
    Outcome {
        output: Output::New { name: name.clone() },
        undo: Undo::None,
        events: vec![ProjectEvent::ProjectOpened { name }, history_changed],
        control: vec![],
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
    let files_in = |dir: &str| -> BTreeSet<String> {
        staged
            .fs()
            .list(dir)
            .map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| !entry.is_dir)
                    .map(|entry| entry.name)
                    .collect()
            })
            .unwrap_or_default()
    };
    let (blobs, root) = (files_in("blobs"), files_in(""));
    let name = opened.project.name.clone();
    let mut next = State::new(opened.project);
    next.session = session;
    let history_changed = next.history.changed_event();
    *state = next;
    // The old project's group and engine slots mean nothing to the new one.
    ctx.group = None;
    ctx.engine = EngineSlots::default();
    ctx.opened = Some(OpenedFolder {
        state: opened.state,
        blobs,
        root,
    });
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
