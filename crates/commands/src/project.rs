//! `project.*`: the project itself: create, open, save and rename (epic 11, SV).
//!
//! `project.rename` is implemented here as the reference command of the contract: an
//! `edit` with its inverse, its event and its output, exercised by the test rule.

use crate::contract::{
    CommandError, Context, FolderError, OpenedFolder, Outcome, ProjectEvent, State,
    StoredBrowserProject, StoredFolder, StoredProjects, Undo,
};
use crate::export::{self, ExportBitDepth, ExportRegion, ExportSettings};
use crate::levels::{self, Level, TrackLevel};
use crate::macros::domain;
use crate::query::DeviceSummary;
use project::file::{OpenOptions, SessionLoad, load_session, open};
use project::scale::MAX_ROOT;
use project::{EngineSlots, Mixer, ProjectId, Scale, ScaleKind, Tick, TrackId};
use std::collections::BTreeSet;

/// Longest project name accepted, in characters: the project's own limit.
pub use ::project::limits::MAX_NAME_CHARS;

/// The name of a new project when no name is given: the page gives its drafts a name of
/// their own (the time they were started), an agent may give one to `project.new`.
pub const DEFAULT_NAME: &str = "Sketch";

domain! {
    domain = "project", schema = ("ProjectCommand", "ProjectOutput");

    /// Rename the project. The name is trimmed, not empty, at most 200 characters.
    /// Undo: rename it back to its previous name.
    "project.rename" (kind = "edit", undo = "inverse", destructive = false)
    Rename { name: String } => {
        /// The name before the command.
        previous: String,
    };

    /// Set the project's key and scale: `root` is the tonic's pitch class, 0 (C) to 11 (B), and
    /// `mode` one of `major`, `natural_minor`, `harmonic_minor`, `melodic_minor` (its rising
    /// form), `dorian`, `phrygian`, `lydian`, `mixolydian`, `locrian`, `major_pentatonic`,
    /// `minor_pentatonic`, `blues` (the six-note minor blues), or `chromatic` for no scale (the
    /// default: every tool works on all twelve notes; `root` is then not kept). F minor is
    /// `{"root": 5, "mode": "natural_minor"}`. The piano roll shows the scale's notes, and
    /// `project.get` answers the scale and its pitch classes. One scale for the whole project.
    /// A root outside 0 to 11 is refused; the scale the project already has changes nothing.
    /// Undo: set the previous scale back.
    "project.set_scale" (kind = "edit", undo = "inverse", destructive = false)
    SetScale {
        #[schemars(range(min = 0, max = 11))]
        root: u8,
        mode: ScaleKind,
    } => {
        /// The tonic before the command (0 when there was no scale).
        previous_root: u8,
        /// The scale before the command; `chromatic` when there was none.
        previous_mode: ScaleKind,
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
    /// (the chosen MIDI input stays); the engine starts over from it. Not undoable, destructive:
    /// the page first keeps the current project's changes where it can (written into its folder,
    /// or kept as a draft in the browser's storage, which stays listed). Only when they cannot
    /// be kept anywhere does it ask the user, whoever sends the command, and nothing changes if
    /// the user cancels. The question is a dialog of the page, so it needs no click. `name`
    /// is optional (trimmed, at most 200 characters): without one the project is called
    /// "Sketch" (the page names its drafts by the time they were started, #473).
    "project.new" (kind = "edit", undo = "none", destructive = true)
    New {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    } => {
        /// The name of the new project.
        name: String,
    };

    /// Write the project to its folder now, without waiting for the automatic save that
    /// follows each change by a moment. A project that has no folder yet asks for one, as
    /// `project.save_as` does. Changes nothing in the project, so it never enters the
    /// history.
    "project.save" (kind = "live", undo = "none", destructive = false)
    Save {} => {};

    /// Ask the user for the project's name (a field prefilled with the current one), then for
    /// a folder, and write the whole project into a folder of that name inside it; the project
    /// takes that name (a `project.rename`, undoable) and then lives in that folder, and the
    /// automatic save keeps writing there. The name and the folder are chosen by the user (a
    /// browser needs a click for it), so an agent that is not driven by one gets a notice in the
    /// page instead, and nothing is chosen. A folder of that name that already holds files is
    /// refused. The old folder is left as it is. Changes nothing else in the project, so the save
    /// itself never enters the history.
    "project.save_as" (kind = "live", undo = "none", destructive = false)
    SaveAs {} => {};

    /// Export the project as a `.faulkpack`, a zip of its folder that any Faulk reads back,
    /// and hand it to the user as a download. It holds the project as it is now, saved or
    /// not, and the media the project refers to. This is how a project kept in the browser
    /// (no folder chosen) is backed up or moved. Changes nothing in the project, so it never
    /// enters the history.
    "project.export_pack" (kind = "live", undo = "none", destructive = false)
    ExportPack {} => {};

    /// Stop the `.faulkpack` export under way, from the moment it was asked for: nothing is handed
    /// over (a download is not started; a file the user chose is left empty). Nothing happens when
    /// no export is running. Changes nothing in the project, so
    /// it never enters the history.
    "project.cancel_export_pack" (kind = "live", undo = "none", destructive = false)
    CancelExportPack {} => {};

    /// Open a `.faulkpack` the user chooses: its project replaces the current one and is kept
    /// in the browser as a new project of its own (an import never overwrites a project that
    /// is already there). A file is chosen by the user (a browser needs a click for it), so
    /// an agent that is not driven by one gets a notice in the page instead. A file that is
    /// not a valid pack is refused with the reason and nothing changes. The history of the
    /// replaced project is gone; the page asks first when that project has changes that are
    /// not kept anywhere.
    "project.import_pack" (kind = "live", undo = "none", destructive = false)
    ImportPack {} => {};

    /// Stop the `.faulkpack` imports under way: a pack being downloaded (`project.open_url`) or
    /// read into the browser's storage, and those waiting their turn, until their project starts
    /// opening, wherever the browser keeps the project (its storage, or the tab alone, where the
    /// pack is read whole). Nothing of them is kept and the current project is untouched. The
    /// answer says what was stopped: `cancelled` imports, and `too_late` imports whose project had
    /// started to open (they are left to finish; when nothing could be stopped and some were too
    /// late, the call is refused instead). Both are 0 when no import was running (none asked, or
    /// the last one is over). Changes nothing in the project, so it never enters the history.
    "project.cancel_import_pack" (kind = "live", undo = "none", destructive = false)
    CancelImportPack {} => {
        /// How many imports (a pack being downloaded, read or waiting its turn) were stopped.
        cancelled: u32,
        /// How many imports were left to finish because their project had started to open.
        too_late: u32,
    };

    /// Open a `.faulkpack` hosted at `url`: `https`, with no user name or password in it (plain
    /// `http` only to `localhost`, `127.0.0.1` or `[::1]`, and the page takes it only when Faulk
    /// itself runs on this computer). The page downloads it without cookies or credentials,
    /// checks it whole (size, files, format) and opens its project as a new project of its own,
    /// as `project.import_pack` does with a file: kept in the browser's storage where the
    /// browser has one, in the tab alone where it has none. Nothing in the pack runs as code.
    /// When the browser already keeps the project of that pack (the same link opened before), the
    /// page opens the copy it keeps instead of a second one. The current project is never
    /// overwritten: it stays where it is kept (its folder, or the browser), and
    /// `project.list_stored` then `project.open_browser_project` or `project.open_recent` bring
    /// it back; the page asks first when that project has changes that are kept nowhere. A link
    /// that cannot be downloaded, a file that is not a valid pack and a pack that is too big are
    /// refused in the page with a notice, and nothing changes. The result only says the command
    /// was accepted, not how the download ended. Not undoable: opening a project is not an edit
    /// of one; go back to the previous project through the project list.
    "project.open_url" (kind = "live", undo = "none", destructive = false)
    OpenUrl {
        #[schemars(length(min = 1, max = 2048))]
        url: String,
    } => {};

    /// List the projects this page can open: the folders opened recently (each with an `id`)
    /// and the projects kept in the browser's own storage (each with a `dir`), newest first.
    /// The page reads its storage again before it answers. The `id` and the `dir` are the
    /// names `project.open_recent`, `project.open_browser_project` and
    /// `project.delete_browser_project` take. Changes nothing in the project.
    "project.list_stored" (kind = "query", undo = "none", destructive = false)
    ListStored {} => {
        /// The folders opened recently, newest first.
        recent: Vec<StoredFolder>,
        /// The projects kept in the browser's storage, newest first.
        browser: Vec<StoredBrowserProject>,
    };

    /// Bring back the project that was open last time, the one the page offers after a reload
    /// (a folder, which asks the browser for its permission again and so needs a click by the
    /// user, or a draft kept in the browser), or the one that the page tried to reopen by itself
    /// and could not (locked by another tab, unreadable): the notice then offers this command
    /// as its retry. A folder still needs the user's click when the browser withdrew its
    /// permission: an agent that is not driven by one gets a notice in the page. Nothing
    /// happens, with a notice, when the page offers nothing. The current project is replaced;
    /// the page asks first when it has changes that are kept nowhere. Not undoable: it replaces
    /// the history.
    "project.reopen_last" (kind = "live", undo = "none", destructive = false)
    ReopenLast {} => {};

    /// Choose a project folder and open it, replacing the current project. The folder is
    /// chosen by the user (a browser needs a click for it), so an agent that is not driven by
    /// one gets a notice in the page instead. A folder that is not a project is refused with
    /// the reason and nothing changes. The page asks first when the current project has
    /// changes that are kept nowhere. Not undoable: it replaces the history.
    "project.open_folder" (kind = "live", undo = "none", destructive = false)
    OpenFolder {} => {};

    /// Open a folder from the recent projects, by the `id` that `project.list_stored` gives,
    /// replacing the current project. When the browser withdrew its permission on the folder,
    /// it asks again, which needs a click by the user: an agent that is not driven by one gets
    /// a notice in the page. An `id` the page does not know is refused with a notice. The page
    /// asks first when the current project has changes that are kept nowhere. Not undoable: it
    /// replaces the history.
    "project.open_recent" (kind = "live", undo = "none", destructive = false)
    OpenRecent {
        #[schemars(length(min = 1, max = 128))]
        id: String,
    } => {};

    /// Open a project kept in the browser's storage (a draft, or a project where the browser
    /// has no folders), by the `dir` that `project.list_stored` gives, replacing the current
    /// project. A `dir` the page does not list is refused with a notice, and so is a project
    /// that another tab has open. The page asks first when the current project has changes
    /// that are kept nowhere. Not undoable: it replaces the history.
    "project.open_browser_project" (kind = "live", undo = "none", destructive = false)
    OpenBrowserProject {
        #[schemars(length(min = 1, max = 128))]
        dir: String,
    } => {};

    /// Delete a project kept in the browser's storage, for good, by the `dir` that
    /// `project.list_stored` gives. The page always asks the user to confirm first, whoever
    /// sends the command, and nothing is deleted until the user says yes. The project that is
    /// open, a project another tab has open and a `dir` the page does not list are refused
    /// with a notice. The result only says the command was accepted, not what the user
    /// answered. Not undoable: the files are gone (`project.export_pack` makes a copy first).
    "project.delete_browser_project" (kind = "live", undo = "none", destructive = false)
    DeleteBrowserProject {
        #[schemars(length(min = 1, max = 128))]
        dir: String,
    } => {};

    /// When the project's folder was edited outside Faulk while the project has unsaved
    /// changes, keep this page's version: the folder's files are replaced by the project right
    /// away, and the outside edits are lost. The page always asks the user to
    /// confirm first, whoever sends the command. Refused with a notice when the folder is not
    /// in conflict. The result only says the command was accepted, not what the user answered.
    /// Changes nothing in the project, so it never enters the history.
    "project.keep_mine" (kind = "live", undo = "none", destructive = false)
    KeepMine {} => {};

    /// When the project's folder was edited outside Faulk while the project has unsaved
    /// changes, take the folder's version: the project is replaced by what the folder holds,
    /// and the changes made since the last write are lost. The page always asks the user to
    /// confirm first, whoever sends the command. Refused with a notice when the folder is not
    /// in conflict. The result only says the command was accepted, not what the user answered.
    /// Not undoable: it replaces the history.
    "project.load_folder_version" (kind = "live", undo = "none", destructive = false)
    LoadFolderVersion {} => {};

    /// After the browser withdrew the right to write in the project's folder, ask for it
    /// again (the browser shows a prompt, which needs a click by the user: an agent that is
    /// not driven by one gets a notice in the page), then write the changes that waited.
    /// Changes nothing in the project, so it never enters the history.
    "project.allow_access" (kind = "live", undo = "none", destructive = false)
    AllowAccess {} => {};

    /// Measure how loud the song is without playing it nor writing a file: the same
    /// offline render as `project.export_audio` (the engine that plays the song, run
    /// faster than real time), but only the levels come back, per track and for the
    /// master, so an agent without ears learns whether its song sounds, whether it is too
    /// loud, and whether a track is silent. The range is the whole song by default, or
    /// bars `from_bar` to `to_bar` (both, counted from 1, the last one included); the
    /// tail after the range is not measured. Levels are in dBFS (0 is full scale), `null`
    /// is silence; a peak is the larger of the two channels, an RMS the power mean of
    /// both, over the whole range (not the loudest short window that `mixer.levels`
    /// gives). The metronome and the loop are off, the solo and mute state are on. Audio
    /// clips are measured as they play: the render reads their files from the project's
    /// folder, and a file that cannot be read leaves its clip silent. Refused, as an export
    /// is, when there is nothing to render (empty arrangement, a range that starts after
    /// the end of the arrangement or is out of order), and when the range lasts more than
    /// 10 minutes (a `limit` error). The page runs the render in the background (a few
    /// seconds for a few minutes of music), one at a time; it answers `unavailable` when
    /// another analysis runs or the page's engine is not ready. A query: changes nothing
    /// in the project and never enters the history.
    "project.analyze_render" (kind = "query", undo = "none", destructive = false)
    AnalyzeRender {
        #[serde(default)]
        #[schemars(range(min = 1))]
        from_bar: Option<u32>,
        #[serde(default)]
        #[schemars(range(min = 1))]
        to_bar: Option<u32>,
    } => {
        /// Where the measured range starts, in ticks (960 per quarter note).
        start_tick: Tick,
        /// Where it ends, in ticks.
        end_tick: Tick,
        /// Seconds of content measured.
        duration_secs: f64,
        /// The level of each track over the range, in display order.
        tracks: Vec<TrackLevel>,
        /// The level of the output over the range, after the master and before the output
        /// protection: above 0 dBFS it is the protection that holds the sound.
        master: Level,
        /// Frames the output protection turned to silence because a sample was not a
        /// number (NaN or infinity): 0 on a healthy mix.
        silenced_frames: u64,
        /// How deep the output protection attenuated the mix at its worst, in dB (0 when
        /// it never engaged): above a dB or two, the mix is too loud.
        limiter_reduction_db: f64,
    };

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
    /// file; the solo and mute state are. Audio clips are in the file as they play: the
    /// render reads their files from the project's folder, in the page in both modes (so in
    /// Boost too, where they are not heard yet), and waits for each read; a file that cannot
    /// be read leaves its clips silent and the page names it when the render ends. It is
    /// refused when there is
    /// nothing to render (empty arrangement, no loop region for `loop`, a `bars` range that
    /// starts after the end of the arrangement or is out of order) or the range lasts more
    /// than 30 minutes
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

    /// Read the project's summary: its id, name, file format version, tempo in BPM, time
    /// signature, key and scale (as `project.set_scale` takes them, with the scale's pitch
    /// classes, 0 = C to 11 = B), master mixer (gain in dB, `null` is silence; pan; mute), how many tracks,
    /// scenes, clips, placements, automation lanes it holds, the master's effect chain (each device with its id, role, bypass state, source and English name), the selected
    /// track, and whether there is something to undo or redo. It lists nothing: use
    /// `track.list`, `scene.list` and `transport.get` for the rest. A query: changes nothing,
    /// never enters the history and never marks the project as having changes to save.
    "project.get" (kind = "query", undo = "none", destructive = false)
    Get {} => {
        /// The project's stable identity.
        id: ProjectId,
        /// The project's name.
        name: String,
        /// The on-disk format version.
        format: u32,
        /// The tempo in BPM.
        tempo_bpm: f64,
        /// The time signature's numerator.
        numerator: u8,
        /// The time signature's denominator.
        denominator: u8,
        /// The scale's tonic, 0 (C) to 11 (B); 0 when there is no scale.
        scale_root: u8,
        /// The scale; `chromatic` when the project has none.
        scale_mode: ScaleKind,
        /// The pitch classes of the scale (0 = C to 11 = B), rising; all twelve when there is
        /// no scale.
        scale_pitch_classes: Vec<u8>,
        /// The master's gain (dB, `null` is silence), pan and mute.
        master: Mixer,
        /// How many tracks.
        track_count: u32,
        /// How many scenes.
        scene_count: u32,
        /// How many clips, all tracks together.
        clip_count: u32,
        /// How many arrangement placements, all tracks together.
        placement_count: u32,
        /// How many automation lanes, the master's included.
        lane_count: u32,
        /// The master's effect chain, in order (`device.describe` reads one).
        master_devices: Vec<DeviceSummary>,
        /// The selected track (session state); `null` when none.
        selected_track: Option<TrackId>,
        /// Whether `history.undo` has an entry to undo.
        can_undo: bool,
        /// Whether `history.redo` has an entry to redo.
        can_redo: bool,
    };
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::Get {} => {
            let p = crate::query::project_data(state);
            Ok(crate::query::answer(Output::Get {
                id: p.id,
                name: p.name,
                format: p.format,
                tempo_bpm: p.tempo_bpm,
                numerator: p.numerator,
                denominator: p.denominator,
                scale_root: p.scale_root,
                scale_mode: p.scale_mode,
                scale_pitch_classes: p.scale_pitch_classes,
                master: p.master,
                track_count: p.track_count,
                scene_count: p.scene_count,
                clip_count: p.clip_count,
                placement_count: p.placement_count,
                lane_count: p.lane_count,
                master_devices: p.master_devices,
                selected_track: p.selected_track,
                can_undo: p.can_undo,
                can_redo: p.can_redo,
            }))
        }
        Command::SetScale { root, mode } => set_scale(state, *root, *mode),
        Command::Open { folder } => open_folder(folder, state, ctx),
        Command::New { name } => {
            let name = name.as_deref().map(check_project_name).transpose()?;
            Ok(new_project(state, ctx, name))
        }
        // The write belongs to the host (the page's storage; a folder picker needs a click):
        // the command only says it is wanted. Nothing in the project changes.
        Command::Save {} => Ok(nothing(Output::Save {})),
        Command::SaveAs {} => Ok(nothing(Output::SaveAs {})),
        Command::ExportPack {} => Ok(nothing(Output::ExportPack {})),
        Command::CancelExportPack {} => Ok(nothing(Output::CancelExportPack {})),
        // The imports belong to the page, which answers with what it stopped (`runForAgent`).
        Command::CancelImportPack {} => Ok(nothing(Output::CancelImportPack {
            cancelled: 0,
            too_late: 0,
        })),
        Command::ImportPack {} => Ok(nothing(Output::ImportPack {})),
        // The download and the pack belong to the page (the core checks the pack itself when the
        // page hands it over): the command only checks that the link may be fetched at all.
        Command::OpenUrl { url } => {
            check_pack_url(url)?;
            Ok(nothing(Output::OpenUrl {}))
        }
        // The host stages what it keeps just before (`Context::stored`); with nothing staged the
        // answer is empty rather than an old list.
        Command::ListStored {} => {
            let StoredProjects { recent, browser } = ctx.stored.take().unwrap_or_default();
            Ok(nothing(Output::ListStored { recent, browser }))
        }
        Command::ReopenLast {} => Ok(nothing(Output::ReopenLast {})),
        Command::OpenFolder {} => Ok(nothing(Output::OpenFolder {})),
        Command::KeepMine {} => Ok(nothing(Output::KeepMine {})),
        Command::LoadFolderVersion {} => Ok(nothing(Output::LoadFolderVersion {})),
        Command::AllowAccess {} => Ok(nothing(Output::AllowAccess {})),
        // Which stored project is named belongs to the page (it knows what it keeps): the
        // command only checks that something is named.
        Command::OpenRecent { id } => {
            stored_name("id", id)?;
            Ok(nothing(Output::OpenRecent {}))
        }
        Command::OpenBrowserProject { dir } => {
            stored_name("dir", dir)?;
            Ok(nothing(Output::OpenBrowserProject {}))
        }
        Command::DeleteBrowserProject { dir } => {
            stored_name("dir", dir)?;
            Ok(nothing(Output::DeleteBrowserProject {}))
        }
        // Like a save, the render belongs to the host (the page's worker and file picker):
        // the command checks the request against the project and says what will be
        // rendered.
        // The numbers belong to the host (the page's offline render): the command checks the
        // range against the project, takes what the host staged and names the tracks.
        Command::AnalyzeRender { from_bar, to_bar } => {
            let plan = export::analysis_plan(&state.project, *from_bar, *to_bar)?;
            let Some(stage) = ctx.analysis.take() else {
                return Err(CommandError::Invalid {
                    field: "render".into(),
                    message:
                        "this host measured nothing: the page renders the range before it answers"
                            .into(),
                });
            };
            if (stage.start_tick, stage.end_tick) != (plan.start_tick, plan.end_tick) {
                return Err(CommandError::Invalid {
                    field: "render".into(),
                    message: "the project changed while it was measured: ask again".into(),
                });
            }
            if export::track_slots(&state.project, &state.session) != stage.slots {
                return Err(CommandError::Invalid {
                    field: "render".into(),
                    message: "the tracks changed while the project was measured: ask again".into(),
                });
            }
            let slot_of = |id: &TrackId| stage.slots.iter().position(|s| s == id).map(|i| i as u32);
            Ok(crate::query::answer(Output::AnalyzeRender {
                start_tick: plan.start_tick,
                end_tick: plan.end_tick,
                duration_secs: plan.duration_secs,
                tracks: levels::track_levels(&state.project, slot_of, &stage.tracks),
                master: levels::level_of(&stage.master),
                silenced_frames: stage.silenced_frames,
                limiter_reduction_db: levels::reduction_db(stage.reduction),
            }))
        }
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
            let name = check_project_name(name)?;
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

/// `project.set_scale`: `chromatic` is no scale, whatever the root.
fn set_scale(
    state: &mut State,
    root: u8,
    mode: ScaleKind,
) -> Result<Outcome<Command, Output>, CommandError> {
    if root > MAX_ROOT {
        return Err(CommandError::Invalid {
            field: "root".into(),
            message: format!("a root is a pitch class, 0 (C) to {MAX_ROOT} (B), not {root}"),
        });
    }
    let new = mode.mode().map(|mode| Scale { root, mode });
    let previous = state.project.scale;
    let (previous_root, previous_mode) = kind_of(previous);
    let output = Output::SetScale {
        previous_root,
        previous_mode,
    };
    if previous == new {
        return Ok(nothing(output));
    }
    state.project.scale = new;
    let (root, mode) = kind_of(new);
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::SetScale {
            root: previous_root,
            mode: previous_mode,
        })),
        events: vec![ProjectEvent::ScaleChanged { root, mode }],
        control: vec![],
    })
}

/// A project's scale as the commands name it: `(0, chromatic)` for none.
pub(crate) fn kind_of(scale: Option<Scale>) -> (u8, ScaleKind) {
    match scale {
        None => (0, ScaleKind::Chromatic),
        Some(s) => (s.root, ScaleKind::from(Some(s.mode))),
    }
}

/// Longest `id` or `dir` the page's lists give (identifiers it generated itself).
const MAX_STORED_NAME_CHARS: usize = 128;

/// Checks that a command names a stored project: not empty, not longer than anything the page
/// lists. What the page does with a name it does not know is its own refusal.
fn stored_name(field: &str, value: &str) -> Result<(), CommandError> {
    if value.is_empty() {
        return Err(CommandError::Invalid {
            field: field.into(),
            message: format!("`{field}` names a stored project, it is not empty"),
        });
    }
    if value.chars().count() > MAX_STORED_NAME_CHARS {
        return Err(CommandError::Invalid {
            field: field.into(),
            message: format!("`{field}` has at most {MAX_STORED_NAME_CHARS} characters"),
        });
    }
    Ok(())
}

/// Longest link `project.open_url` takes, in characters.
pub const MAX_PACK_URL_CHARS: usize = 2048;

/// The hosts plain `http` may reach: the computer itself (a pack served while developing).
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// Checks the shape of a link to a `.faulkpack`: `https`, or `http` to the computer itself, a
/// host, and no user name or password. Stricter than a browser's parser on purpose (a host the
/// page would rewrite, such as `127.1`, is refused rather than guessed); the page parses the
/// link again before it downloads anything.
fn check_pack_url(url: &str) -> Result<(), CommandError> {
    let refuse = |message: String| CommandError::Invalid {
        field: "url".into(),
        message,
    };
    if url.is_empty() {
        return Err(refuse(
            "`url` is the link to a .faulkpack, it is not empty".into(),
        ));
    }
    if url.chars().count() > MAX_PACK_URL_CHARS {
        return Err(refuse(format!(
            "`url` has at most {MAX_PACK_URL_CHARS} characters"
        )));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refuse(
            "`url` holds no space or control character (percent-encode them)".into(),
        ));
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(refuse(
            "`url` is an absolute link that starts with https://".into(),
        ));
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err(refuse(format!("`url` uses https (not `{scheme}`)")));
    }
    // A browser ends the authority at any of these; a backslash counts as a slash in http(s).
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(refuse(
            "`url` carries no user name or password: the page downloads without credentials".into(),
        ));
    }
    let (host, port) = match authority.strip_prefix('[') {
        Some(inner) => match inner.split_once(']') {
            Some((address, after)) => (&authority[..address.len() + 2], after),
            None => return Err(refuse("`url` has an unclosed `[` in its host".into())),
        },
        None => authority
            .find(':')
            .map_or((authority, ""), |at| authority.split_at(at)),
    };
    if host.is_empty() {
        return Err(refuse("`url` names a host".into()));
    }
    if let Some(digits) = port.strip_prefix(':') {
        if !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(refuse("`url` has a port made of digits".into()));
        }
    } else if !port.is_empty() {
        return Err(refuse("`url` has a malformed host".into()));
    }
    if scheme == "http"
        && !LOOPBACK_HOSTS
            .iter()
            .any(|loopback| host.eq_ignore_ascii_case(loopback))
    {
        return Err(refuse(
            "`url` uses https; plain http is only for this computer (localhost)".into(),
        ));
    }
    Ok(())
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

/// A project name as the commands take it: trimmed, not empty, at most [`MAX_NAME_CHARS`].
pub fn check_project_name(name: &str) -> Result<&str, CommandError> {
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
    Ok(name)
}

/// Replaces the state with a new project: the default one-synth project under a fresh
/// identity, an empty history and session, and no folder.
fn new_project(
    state: &mut State,
    ctx: &mut Context,
    name: Option<&str>,
) -> Outcome<Command, Output> {
    let mut next = crate::engine_msgs::single_synth::state();
    next.project.id = ProjectId::generate(&mut ctx.ids);
    if let Some(name) = name {
        name.clone_into(&mut next.project.name);
    }
    // The MIDI input is the player's device, not the song's: it stays chosen. So does the
    // user's preset library, which every project shares.
    next.session.midi_input = state.session.midi_input.take();
    next.library = std::mem::take(&mut state.library);
    let name = next.project.name.clone();
    let history_changed = next.history.changed_event();
    *state = next;
    ctx.group = None;
    ctx.engine = EngineSlots::default();
    // What a save knew of the old project's folder is no use to a project that has none.
    ctx.folder = None;
    ctx.opened = None;
    // The files imported into the old project are in its folder, not in the new one's.
    ctx.media.clear();
    ctx.missing_media.clear();
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
    // The user's preset library is every project's.
    next.library = std::mem::take(&mut state.library);
    let history_changed = next.history.changed_event();
    *state = next;
    // The old project's group and engine slots mean nothing to the new one, and the files
    // imported into it are in its folder, not in this one.
    ctx.group = None;
    ctx.engine = EngineSlots::default();
    ctx.media.clear();
    ctx.missing_media = opened.report.missing_blobs.iter().copied().collect();
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
