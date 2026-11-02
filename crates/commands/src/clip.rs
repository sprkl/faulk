//! `clip.*`: MIDI clips and their notes: create, delete, restore, duplicate, rename,
//! length, loop, content and the note batches (CL-1; epics 5 and 8, CL and PN). Notes are
//! addressed by value: there is no `NoteId` (CT-4).
//!
//! The contract of the clip commands, written once here and repeated in their
//! descriptions for agents:
//!
//! - A clip lives in a MIDI track and is addressed by its `ClipId`; only `clip.create`
//!   and `clip.restore` name the track. Audio clips come from import (AU): the commands
//!   that read or write notes, and `clip.delete` and `clip.duplicate`, refuse them.
//! - One command is one history entry, whatever the size of its batch, and it is atomic:
//!   everything is checked before anything changes, so a refusal leaves the project as
//!   it was.
//! - **A note outside `[0, clip length]` is refused, never silently truncated.** Cutting
//!   notes is explicit: `clip.set_length` with `trim: true`.
//! - Adding a note already in the clip changes nothing; removing an absent note changes
//!   nothing. Updating an absent note is refused, since there is nothing to move.
//! - No cap below the safety limits of `project::limits` (`MAX_NOTES_PER_CLIP`,
//!   `MAX_NOTES`, `MAX_CLIPS_PER_TRACK`): beyond them the command is refused with
//!   `limit`.
//! - An inverse is built from what actually changed, and is an ordinary command.
//!
//! Overlapping placements are the arrangement's rule (AR-1): `clip.create` into the
//! arrangement refuses an overlap instead of choosing a rule.

use crate::contract::{ClipPart, CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use crate::query::{ClipContentInfo, ClipSummary, PageInfo, PlacementInfo};
use project::limits::{
    MAX_CLIPS, MAX_CLIPS_PER_TRACK, MAX_NAME_CHARS, MAX_NOTES, MAX_NOTES_PER_CLIP, MAX_PLACEMENTS,
    MAX_PLACEMENTS_PER_TRACK, MAX_TICK,
};
use project::{
    Clip, ClipContent, ClipId, Note, Placement, PlacementId, Project, Quantize, Rgb, SceneId, Span,
    Tick, TickLoop, Track, TrackId, TrackKind,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

/// Where `clip.create` puts the new clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "at", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipTarget {
    /// In the launch grid, at the intersection of the track and a scene.
    Slot { scene: SceneId },
    /// In the arrangement: a placement of the whole clip from `start` (in ticks).
    Arrangement { start: Tick },
}

/// A note to replace by another: `clip.notes.update` addresses a note by its whole value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoteChange {
    /// The note as it is in the clip now.
    pub from: Note,
    /// The note it becomes.
    pub to: Note,
}

/// The length of one placement of a clip, in ticks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacementLength {
    pub placement: PlacementId,
    pub length: Tick,
}

/// A placement of a MIDI clip, with its identifier, as `clip.restore` brings it back. It
/// belongs to the clip restored: a snapshot cannot place another clip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacementSnapshot {
    pub id: PlacementId,
    /// Start in the arrangement, in ticks.
    pub start: Tick,
    /// Length on the timeline, in ticks.
    pub length: Tick,
    /// Where in the clip the placement starts, in ticks, before the clip's length.
    pub offset: Tick,
}

/// A MIDI clip as `clip.restore` brings it back (everything but its identifier).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipSnapshot {
    pub name: String,
    pub color: Option<Rgb>,
    /// The scene of its launch-grid slot, when it sits in one.
    pub slot: Option<SceneId>,
    /// A per-clip launch quantization, when one was set (the MVP never sets one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_quantize: Option<Quantize>,
    /// Length in ticks.
    pub length: Tick,
    pub looping: Option<TickLoop>,
    pub notes: Vec<Note>,
}

domain! {
    domain = "clip", schema = ("ClipCommand", "ClipOutput");

    /// Create an empty MIDI clip in a MIDI track, in the launch grid (`{"at": "slot",
    /// "scene": <scene id>}`: the slot must exist and be free) or in the arrangement
    /// (`{"at": "arrangement", "start": <tick>}`: also places the whole clip there, and
    /// is refused when it would overlap another placement of the track). `length` is in
    /// ticks (960 per quarter note), one bar of the project's time signature by default.
    /// `name` is optional (trimmed, at most 200 characters): without one the clip gets the
    /// next default name of its track, the track's name and the first number no clip of
    /// the track has with it ("Bass 3"); it is stored and never changes when other clips
    /// move, are copied or deleted, nor when the track is renamed. Undo:
    /// `clip.delete`.
    "clip.create" (kind = "edit", undo = "inverse", destructive = false)
    Create {
        track: TrackId,
        at: ClipTarget,
        length: Option<Tick>,
        name: Option<String>,
    } => {
        /// The clip created.
        clip: ClipId,
        /// Its placement, when it was created in the arrangement.
        placement: Option<PlacementId>,
    };

    /// Delete a MIDI clip and its placements in the arrangement. Undo: `clip.restore`
    /// brings back the clip, its notes and its placements with their identifiers.
    "clip.delete" (kind = "edit", undo = "inverse", destructive = false)
    Delete { clip: ClipId } => {
        /// How many placements were removed with the clip.
        placements: u32,
    };

    /// Bring back a MIDI clip with its identifier, as `clip.delete` removed it: the
    /// inverse of a delete. Refused when the track is not a MIDI track, when the clip's
    /// identifier or a placement's already exists in the project, when the slot is
    /// taken or its scene is gone, or when the snapshot breaks a rule of the clip
    /// commands (a note outside the length, a placement outside the timeline). Undo:
    /// `clip.delete`.
    "clip.restore" (kind = "edit", undo = "inverse", destructive = false)
    Restore {
        track: TrackId,
        clip: ClipId,
        snapshot: ClipSnapshot,
        placements: Vec<PlacementSnapshot>,
    } => {};

    /// Copy a MIDI clip into a new, independent clip with the same name ("Bass 2" stays
    /// "Bass 2": a copy plays the same part), colour, length, loop and notes; editing one never changes the other. The copy always sits in a
    /// launch-grid slot: of the source's track, or of `track` when given (another MIDI
    /// track). `slot` when given must be free in that track, otherwise the copy takes
    /// the first free slot (below the source's scene in the same track, from the top in
    /// another). Refused, with no effect, when the track has no free slot to give. A
    /// second placement of the same clip is an option of the arrangement commands, not
    /// of this one. Undo: `clip.delete` of the copy.
    "clip.duplicate" (kind = "edit", undo = "inverse", destructive = false)
    Duplicate {
        clip: ClipId,
        slot: Option<SceneId>,
        track: Option<TrackId>,
    } => {
        /// The new clip.
        clip: ClipId,
    };

    /// Move a MIDI clip to another launch-grid slot, of the same track or of `track`
    /// (another MIDI track), keeping its identifier, its name and its content. `slot`
    /// is the scene of the target slot, which must be free; `null` takes the clip out of
    /// the grid, which is refused when no placement of the arrangement plays it. A clip
    /// that has placements stays in its track: moving it to another track is refused.
    /// A move that changes nothing does nothing and leaves no history entry. Leaving
    /// `slot` out is the same as `null`. The engine frees the clip and takes it again, so
    /// a clip that is playing stops, and so does the arrangement's playback of its
    /// placements (they play again from the next note). Undo: move it back.
    "clip.move" (kind = "edit", undo = "inverse", destructive = false)
    Move {
        clip: ClipId,
        track: Option<TrackId>,
        slot: Option<SceneId>,
    } => {
        /// The track of the clip before the command.
        previous_track: TrackId,
        /// Its slot before the command.
        previous_slot: Option<SceneId>,
    };

    /// Rename a clip (MIDI or audio). The name is trimmed, at most 200 characters; an
    /// empty name gives a MIDI clip the next default name of its track ("Bass 3"; one that
    /// already has a default name of its track keeps it) and clears an audio clip's. Undo: rename it back.
    "clip.rename" (kind = "edit", undo = "inverse", destructive = false)
    Rename { clip: ClipId, name: String } => {
        /// The name before the command.
        previous: String,
    };

    /// Set the length of a MIDI clip, in ticks (1 to 2^40). Making it shorter than a
    /// note's end or the loop's end is refused, unless `trim` is true: then notes that
    /// start at or after the new end are removed, notes that cross it are shortened
    /// (two notes that become identical merge into one), and the loop is cut at it
    /// (removed when nothing is left of it). Refused too when a placement of the clip
    /// starts inside it past the new length. Every placement of the clip that plays it
    /// whole (offset 0, length equal to the old length of the clip) takes the new length
    /// in the same command, so notes added past the old end are heard in the
    /// arrangement; a growing placement stops at the start of the next placement of its
    /// track, and a placement that shows only part of the clip, or repeats it, is left
    /// alone. Undo: set the length back, restoring what `trim` cut and the lengths of the
    /// placements.
    "clip.set_length" (kind = "edit", undo = "inverse", destructive = false)
    SetLength {
        clip: ClipId,
        length: Tick,
        #[serde(default)]
        trim: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        // Internal: the exact lengths an undo gives back, not in the schema (agents never
        // see it; it is not a placement resize, AR-1 owns that).
        #[schemars(skip)]
        placements: Option<Vec<PlacementLength>>,
    } => {
        /// The length before the command.
        previous: Tick,
    };

    /// Set the loop of a MIDI clip: `{"start": <tick>, "end": <tick>}` with
    /// `0 <= start < end <= length`, or `null` for no loop. Undo: set the previous loop
    /// back.
    "clip.set_loop" (kind = "edit", undo = "inverse", destructive = false)
    SetLoop {
        clip: ClipId,
        looping: Option<TickLoop>,
    } => {
        /// The loop before the command.
        previous: Option<TickLoop>,
    };

    /// Replace the length, the loop and all the notes of a MIDI clip at once: one
    /// history entry for a whole clip written in one go. The same rules as
    /// `clip.set_length`, `clip.set_loop` and `clip.notes.add` apply (a note outside
    /// `[0, length]` is refused), and the placements follow the length in the same way
    /// as `clip.set_length`. Undo: set the previous content and the previous placement
    /// lengths back.
    "clip.set_content" (kind = "edit", undo = "inverse", destructive = false)
    SetContent {
        clip: ClipId,
        length: Tick,
        looping: Option<TickLoop>,
        notes: Vec<Note>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        // Internal: the exact lengths an undo gives back, not in the schema (agents never
        // see it; it is not a placement resize, AR-1 owns that).
        #[schemars(skip)]
        placements: Option<Vec<PlacementLength>>,
    } => {
        /// How many notes the clip has now.
        notes: u32,
    };

    /// Add notes to a MIDI clip, in one batch (one history entry, all or nothing).
    /// A note has `start` and `length` in ticks (the note must lie inside
    /// `[0, clip length]`, length at least 1), `pitch` 0 to 127, `velocity` 1 to 127,
    /// and optionally `cents` (fine tuning, -50 to 50), `chance` (0 to 100 percent,
    /// default 100) and `repeat` (1 to 16, default 1). A note outside the clip is
    /// refused, never truncated. A note already in the clip changes nothing. Undo:
    /// remove the notes actually added.
    "clip.notes.add" (kind = "edit", undo = "inverse", destructive = false)
    NotesAdd { clip: ClipId, notes: Vec<Note> } => {
        /// How many notes were new.
        added: u32,
    };

    /// Remove notes from a MIDI clip, in one batch (one history entry), each addressed
    /// by its whole value (start, pitch, length, velocity and the optional fields). A
    /// note that is not in the clip is ignored; the output counts those removed. Undo:
    /// add the notes actually removed.
    "clip.notes.remove" (kind = "edit", undo = "inverse", destructive = false)
    NotesRemove { clip: ClipId, notes: Vec<Note> } => {
        /// How many notes were in the clip.
        removed: u32,
    };

    /// Change notes of a MIDI clip in one batch (one history entry, all or nothing):
    /// position, length, pitch, velocity, cents, chance or repeat. Each change gives
    /// the note as it is now (`from`) and as it becomes (`to`, the same rules as
    /// `clip.notes.add`). All the `from` notes leave and all the `to` notes arrive at
    /// once, so a chain or a swap is fine. Refused when a `from` is not in the clip,
    /// when two changes share a `from` or a `to`, or when a `to` equals a note that
    /// stays. A change that ends with the same notes changes nothing. Undo: the
    /// opposite changes.
    "clip.notes.update" (kind = "edit", undo = "inverse", destructive = false)
    NotesUpdate {
        clip: ClipId,
        changes: Vec<NoteChange>,
    } => {
        /// How many notes changed.
        updated: u32,
    };

    /// Quantize the start of notes of a MIDI clip toward a grid, in one batch (one history
    /// entry, all or nothing). `grid` is the grid step in ticks (960 per quarter note, at
    /// least 1); `strength` is 0 to 100 percent: 100 puts a note on the nearest grid line
    /// (halfway goes to the later line), 50 moves it half of the way, 0 changes nothing.
    /// `notes` lists the notes to quantize, each by its whole value; leave it out to
    /// quantize every note of the clip. A listed note that is not in the clip is refused.
    /// Only the start moves: the length never changes. A note that the move would push past
    /// the end of the clip stops where it still fits (its end on the clip's end), so it may
    /// be off the grid. Notes that end up equal merge into one. Undo: the opposite moves
    /// (the previous content of the clip when notes merged).
    "clip.notes.quantize" (kind = "edit", undo = "inverse", destructive = false)
    NotesQuantize {
        clip: ClipId,
        notes: Option<Vec<Note>>,
        grid: Tick,
        #[schemars(range(min = 0, max = 100))]
        strength: u8,
    } => {
        /// How many notes moved.
        quantized: u32,
    };

    /// Transpose notes of a MIDI clip by `semitones` (positive up, negative down), in one
    /// batch (one history entry, all or nothing). Each note is addressed by its whole
    /// value and must be in the clip. A transposition that would take one note outside
    /// pitch 0 to 127 is refused for the whole batch, never clamped. Notes that end up
    /// equal merge into one. 0 semitones changes nothing. Undo: transpose back (the
    /// previous content of the clip when notes merged).
    "clip.notes.transpose" (kind = "edit", undo = "inverse", destructive = false)
    NotesTranspose {
        clip: ClipId,
        notes: Vec<Note>,
        semitones: i32,
    } => {
        /// How many notes moved.
        transposed: u32,
    };

    /// Copy notes of a MIDI clip, shifted in time by `offset` ticks (negative: earlier), in
    /// one batch (one history entry, all or nothing). Each source note is addressed by its
    /// whole value and must be in the clip. Without `offset`, the copies follow the whole
    /// selection: the shift is the span from the start of the bar of the first note to the last
    /// end, rounded up to whole bars (at least one bar), so a selection of one bar or less is
    /// copied to the next bar and a selection of two bars two bars later. An `offset` of 0
    /// is refused, and so is a copy that would lie outside `[0, clip length]` (never
    /// truncated). A copy equal to a note already in the clip changes nothing. Undo: remove
    /// the notes actually added.
    "clip.notes.duplicate" (kind = "edit", undo = "inverse", destructive = false)
    NotesDuplicate {
        clip: ClipId,
        notes: Vec<Note>,
        offset: Option<Tick>,
    } => {
        /// How many copies were new.
        added: u32,
    };

    /// Read one clip, MIDI or audio, without its notes (`clip.notes.list` pages them): the
    /// track that holds it, its summary (name, colour, launch-grid slot, kind, length and
    /// note count of a MIDI clip), its own launch quantization when it has one, its content
    /// (a MIDI clip's length in ticks, loop and note count; an audio clip's media, range of
    /// frames, gain in dB with `null` for silence, and loop) and a page of its placements
    /// in the track's arrangement, in the order of their identifiers. Paged on the
    /// placements: `offset` (placements to skip, 0 by default) and `limit` (placements to
    /// return, 1 to 1000, 100 by default; any other limit is refused with `invalid`, never
    /// clamped); the answer's `page` gives the `total`, the `offset` used and the
    /// `next_offset` of the next page, `null` after the last one. Refused with `not_found`
    /// for an unknown clip. A query: changes nothing and never enters the history.
    "clip.get" (kind = "query", undo = "none", destructive = false)
    Get {
        /// The clip to read.
        clip: ClipId,
        /// Placements to skip, 0 by default.
        offset: Option<u32>,
        /// Placements to return, 1 to 1000, 100 by default.
        limit: Option<u32>,
    } => {
        /// The track that holds the clip.
        track: TrackId,
        /// The clip's summary.
        summary: ClipSummary,
        /// Its own launch quantization; `null` follows the project's.
        launch_quantize: Option<Quantize>,
        /// What the clip holds, without the notes.
        content: Box<ClipContentInfo>,
        /// The placements of this page, in the order of their identifiers.
        placements: Vec<PlacementInfo>,
        /// Where this page sits in the clip's placement list.
        page: PageInfo,
    };

    /// List the notes of a MIDI clip in time order (start, then pitch, then length, then
    /// the other fields of the note): each note has `start` and `length` in ticks (960 per
    /// quarter note, from the start of the clip), `pitch` (MIDI key 0 to 127) and
    /// `velocity` (1 to 127), and its `cents`, `chance` and `repeat` when they are not the
    /// default. A note is addressed in the edit commands by this whole value. Paged: `offset`
    /// (notes to skip, 0 by default) and `limit` (notes to return, 1 to 1000, 100 by
    /// default; any other limit is refused with `invalid`, never clamped); the answer's
    /// `page` gives the `total`, the `offset` used and the `next_offset` of the next page,
    /// `null` after the last one. An `offset` past the end gives an empty page. Refused with
    /// `not_found` for an unknown clip and with `invalid` for an audio clip, which has no
    /// notes. A query: changes nothing and never enters the history.
    "clip.notes.list" (kind = "query", undo = "none", destructive = false)
    NotesList {
        /// The MIDI clip whose notes are listed.
        clip: ClipId,
        /// Notes to skip, 0 by default.
        offset: Option<u32>,
        /// Notes to return, 1 to 1000, 100 by default.
        limit: Option<u32>,
    } => {
        /// The notes of this page, in time order.
        notes: Vec<Note>,
        /// Where this page sits in the clip's note list.
        page: PageInfo,
    };
}

type Out = Outcome<Command, Output>;

pub(crate) fn invalid(field: impl Into<String>, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

pub(crate) fn not_found(entity: &str, id: impl ToString) -> CommandError {
    CommandError::NotFound {
        entity: entity.into(),
        id: id.to_string(),
    }
}

/// An edit that changed nothing: no inverse, no event, nothing in the history.
fn unchanged(output: Output) -> Out {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

fn edited(output: Output, inverse: Command, events: Vec<ProjectEvent>) -> Out {
    Outcome {
        output,
        undo: Undo::Inverse(Box::new(inverse)),
        events,
        control: vec![],
    }
}

fn changed(clip: &ClipId, part: ClipPart) -> Vec<ProjectEvent> {
    vec![ProjectEvent::ClipChanged {
        clip: clip.clone(),
        part,
    }]
}

/// The count of a collection as the `u32` of an output; a batch is at most the safety
/// limits, far below `u32::MAX`.
pub(crate) fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

pub(crate) fn midi_track<'a>(
    project: &'a Project,
    track: &TrackId,
) -> Result<&'a Track, CommandError> {
    let found = project
        .tracks
        .get(track)
        .ok_or_else(|| not_found("track", track))?;
    if found.kind != TrackKind::Midi {
        return Err(invalid(
            "track",
            "a MIDI clip lives in a MIDI track; audio clips come from import",
        ));
    }
    Ok(found)
}

pub(crate) fn locate(project: &Project, clip: &ClipId) -> Result<TrackId, CommandError> {
    project
        .tracks
        .iter()
        .find(|(_, t)| t.clips.contains_key(clip))
        .map(|(id, _)| id.clone())
        .ok_or_else(|| not_found("clip", clip))
}

/// A MIDI clip's parts, or a refusal for an audio clip.
type MidiRef<'a> = (
    &'a mut Tick,
    &'a mut Option<TickLoop>,
    &'a mut BTreeSet<Note>,
);

const AUDIO: &str = "an audio clip has no notes, length in ticks or loop in ticks";

fn midi_mut(clip: &mut Clip) -> Result<MidiRef<'_>, CommandError> {
    match &mut clip.content {
        ClipContent::Midi {
            length,
            looping,
            notes,
        } => Ok((length, looping, notes)),
        ClipContent::Audio { .. } => Err(invalid("clip", AUDIO)),
    }
}

pub(crate) fn midi_ref(
    clip: &Clip,
) -> Result<(Tick, Option<TickLoop>, &BTreeSet<Note>), CommandError> {
    match &clip.content {
        ClipContent::Midi {
            length,
            looping,
            notes,
        } => Ok((*length, *looping, notes)),
        ClipContent::Audio { .. } => Err(invalid("clip", AUDIO)),
    }
}

pub(crate) fn project_notes(project: &Project) -> usize {
    project
        .tracks
        .values()
        .flat_map(|t| t.clips.values())
        .map(|c| match &c.content {
            ClipContent::Midi { notes, .. } => notes.len(),
            ClipContent::Audio { .. } => 0,
        })
        .sum()
}

/// Refuses a count beyond the safety limits: `in_clip` notes in the clip after the
/// command, `in_project` in the whole project.
pub(crate) fn check_note_limits(in_clip: usize, in_project: usize) -> Result<(), CommandError> {
    if in_clip > MAX_NOTES_PER_CLIP {
        return Err(CommandError::Limit {
            entity: "notes per clip".into(),
            max: MAX_NOTES_PER_CLIP as u64,
        });
    }
    if in_project > MAX_NOTES {
        return Err(CommandError::Limit {
            entity: "notes".into(),
            max: MAX_NOTES as u64,
        });
    }
    Ok(())
}

/// Refuses `count + more` beyond `max` with a typed limit error.
pub(crate) fn check_total(
    entity: &str,
    count: usize,
    more: usize,
    max: usize,
) -> Result<(), CommandError> {
    if count.saturating_add(more) > max {
        return Err(CommandError::Limit {
            entity: entity.into(),
            max: max as u64,
        });
    }
    Ok(())
}

/// Refuses one more clip beyond the track's limit or the project's (the engine's clip
/// slots are project-wide).
pub(crate) fn check_clips(project: &Project, track: &Track) -> Result<(), CommandError> {
    check_total("clips per track", track.clips.len(), 1, MAX_CLIPS_PER_TRACK)?;
    check_total("clips", project.counts().clips, 1, MAX_CLIPS)
}

/// Refuses `more` placements beyond the track's limit or the project's.
pub(crate) fn check_placements(
    project: &Project,
    track: &Track,
    more: usize,
) -> Result<(), CommandError> {
    check_total(
        "placements per track",
        track.placements.len(),
        more,
        MAX_PLACEMENTS_PER_TRACK,
    )?;
    check_total(
        "placements",
        project.counts().placements,
        more,
        MAX_PLACEMENTS,
    )
}

pub(crate) fn check_length(length: Tick, field: &str) -> Result<(), CommandError> {
    if (1..=MAX_TICK).contains(&length.get()) {
        Ok(())
    } else {
        Err(invalid(
            field,
            format!(
                "a clip length is 1 to {MAX_TICK} ticks, not {}",
                length.get()
            ),
        ))
    }
}

pub(crate) fn check_loop(
    looping: &Option<TickLoop>,
    length: Tick,
    field: &str,
) -> Result<(), CommandError> {
    match looping {
        Some(l) if !(0 <= l.start.get() && l.start < l.end && l.end <= length) => Err(invalid(
            field,
            format!(
                "a loop is 0 <= start < end <= the clip length ({}), not {} to {}",
                length.get(),
                l.start.get(),
                l.end.get()
            ),
        )),
        _ => Ok(()),
    }
}

pub(crate) fn check_name(name: &str, field: &str) -> Result<(), CommandError> {
    if name.trim() != name {
        return Err(invalid(
            field,
            "a name has no leading or trailing white space",
        ));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(
            field,
            format!("a clip name has at most {MAX_NAME_CHARS} characters"),
        ));
    }
    Ok(())
}

/// A note must have every field in its range and lie inside `[0, length]`.
fn check_note(note: &Note, length: Tick, field: &str) -> Result<(), CommandError> {
    if !note.fields_in_range() {
        return Err(invalid(
            field,
            format!(
                "pitch 0 to 127, velocity 1 to 127, cents -50 to 50, chance 0 to 100 and \
                 repeat 1 to 16 are required, got {note:?}"
            ),
        ));
    }
    let inside = note.start.get() >= 0
        && note.length.get() > 0
        && note
            .start
            .get()
            .checked_add(note.length.get())
            .is_some_and(|end| end <= length.get());
    if inside {
        Ok(())
    } else {
        Err(invalid(
            field,
            format!(
                "a note lies inside the clip, [0, {}] ticks with a length of at least 1; \
                 it is not truncated: start {}, length {}",
                length.get(),
                note.start.get(),
                note.length.get()
            ),
        ))
    }
}

pub(crate) fn check_notes(notes: &[Note], length: Tick, field: &str) -> Result<(), CommandError> {
    notes
        .iter()
        .enumerate()
        .try_for_each(|(i, n)| check_note(n, length, &format!("{field}[{i}]")))
}

/// A placement of the clip may not start at or past its length: the invariant that
/// `Project::validate` calls `BadRange`.
fn check_offsets(track: &Track, clip: &ClipId, length: Tick) -> Result<(), CommandError> {
    let past = track.placements.iter().find(|(_, p)| {
        p.clip == *clip && matches!(p.span, Span::Ticks { offset, .. } if offset >= length)
    });
    match past {
        Some((id, _)) => Err(invalid(
            "length",
            format!(
                "placement {id} starts inside the clip past that length; move or resize the \
                 placement first"
            ),
        )),
        None => Ok(()),
    }
}

/// The placements of `clip` in the track whose length changes with the clip, as
/// `(id, length before, length after)`. Without `explicit`: those that play the clip whole
/// (offset 0, length equal to `old`) take `new`; a growing one stops at the start of the
/// next placement of the track, and is left as it is when that next one already starts
/// inside it (a note edit never makes an overlap: that rule is the arrangement's). With
/// `explicit` (undo): exactly those lengths, refused when one is listed twice, is not a
/// placement of the clip or leaves the timeline.
fn placement_changes(
    track: &Track,
    clip: &ClipId,
    old: Tick,
    new: Tick,
    explicit: Option<&[PlacementLength]>,
) -> Result<Vec<(PlacementId, Tick, Tick)>, CommandError> {
    if let Some(list) = explicit {
        return explicit_changes(track, clip, list);
    }
    let mut out = Vec::new();
    if old == new {
        return Ok(out);
    }
    // The starts in order, for the next placement of each one in O(log n).
    let mut starts: Vec<i64> = track.placements.values().map(|q| q.start.get()).collect();
    starts.sort_unstable();
    for (id, p) in &track.placements {
        let Span::Ticks { length, offset } = p.span else {
            continue;
        };
        if p.clip != *clip || offset.get() != 0 || length != old {
            continue;
        }
        let start = p.start.get();
        let mut target = new.get().min(MAX_TICK.saturating_sub(start));
        if new > old {
            let after = starts.partition_point(|s| *s <= start);
            if let Some(next) = starts.get(after) {
                target = target.min(next - start);
            }
            if target <= length.get() {
                // Already blocked (or overlapping) there: it does not shrink on a grow.
                continue;
            }
        }
        if target >= 1 && target != length.get() {
            out.push((id.clone(), length, Tick(target)));
        }
    }
    Ok(out)
}

/// The exact lengths of `list`, for undo: what [`placement_changes`] checks about them.
fn explicit_changes(
    track: &Track,
    clip: &ClipId,
    list: &[PlacementLength],
) -> Result<Vec<(PlacementId, Tick, Tick)>, CommandError> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in list {
        if !seen.insert(&item.placement) {
            return Err(invalid("placements", "a placement is listed twice"));
        }
        let p = track
            .placements
            .get(&item.placement)
            .filter(|p| p.clip == *clip)
            .ok_or_else(|| not_found("placement", &item.placement))?;
        let Span::Ticks { length, .. } = p.span else {
            return Err(invalid("placements", "an audio placement has no ticks"));
        };
        let end = p.start.get().checked_add(item.length.get());
        if item.length.get() < 1 || end.is_none_or(|e| e > MAX_TICK) {
            return Err(invalid(
                "placements",
                format!("a placement lies in [0, {MAX_TICK}] ticks, with a length of at least 1"),
            ));
        }
        if length != item.length {
            out.push((item.placement.clone(), length, item.length));
        }
    }
    // No overlap check: the lengths are those of a state the project already held (the
    // inverse of a length change), which may hold overlaps the model allows.
    Ok(out)
}

fn apply_placement_changes(
    placements: &mut std::collections::BTreeMap<PlacementId, Placement>,
    changes: &[(PlacementId, Tick, Tick)],
) {
    for (id, _, new) in changes {
        if let Some(Span::Ticks { length, .. }) = placements.get_mut(id).map(|p| &mut p.span) {
            *length = *new;
        }
    }
}

/// The exact lengths that put the placements back as they were.
fn previous_lengths(changes: &[(PlacementId, Tick, Tick)]) -> Vec<PlacementLength> {
    changes
        .iter()
        .map(|(id, before, _)| PlacementLength {
            placement: id.clone(),
            length: *before,
        })
        .collect()
}

/// A launch-grid slot must name an existing scene and be free in the track.
pub(crate) fn check_slot(
    project: &Project,
    track: &Track,
    scene: &SceneId,
    field: &str,
) -> Result<(), CommandError> {
    if !project.scenes.contains_key(scene) {
        return Err(not_found("scene", scene));
    }
    if track.clips.values().any(|c| c.slot.as_ref() == Some(scene)) {
        return Err(invalid(
            field,
            "the slot already holds a clip in this track",
        ));
    }
    Ok(())
}

/// The first scene, in display order, after `after` (from the top when the source has no
/// slot) whose slot is free in the track.
fn next_free_slot(
    project: &Project,
    track: &Track,
    after: Option<&SceneId>,
) -> Result<SceneId, CommandError> {
    let scenes = project.scenes_in_order();
    let from = after
        .and_then(|a| scenes.iter().position(|(id, _)| *id == a))
        .map_or(0, |i| i + 1);
    scenes[from..]
        .iter()
        .map(|(id, _)| (*id).clone())
        .find(|id| !track.clips.values().any(|c| c.slot.as_ref() == Some(id)))
        .ok_or_else(|| invalid("slot", "the track has no free slot below the source"))
}

/// A placement on the timeline: inside `[0, MAX_TICK]`, overlapping none of `taken`,
/// which it joins.
fn check_span(
    start: Tick,
    length: Tick,
    taken: &mut Vec<(i64, i64)>,
    field: &str,
) -> Result<(), CommandError> {
    match start.get().checked_add(length.get()) {
        Some(end) if start.get() >= 0 && length.get() > 0 && end <= MAX_TICK => {
            if taken.iter().any(|&(s, e)| s < end && start.get() < e) {
                return Err(invalid(
                    field,
                    "the placement would overlap another placement of the track; the overlap \
                     rule belongs to the arrangement commands",
                ));
            }
            taken.push((start.get(), end));
            Ok(())
        }
        _ => Err(invalid(
            field,
            format!("a placement lies in [0, {MAX_TICK}] ticks, with a length of at least 1"),
        )),
    }
}

fn taken_spans(track: &Track) -> Vec<(i64, i64)> {
    track
        .placements
        .values()
        .filter_map(|p| match p.span {
            Span::Ticks { length, .. } => {
                Some((p.start.get(), p.start.get().saturating_add(length.get())))
            }
            Span::Frames { .. } => None,
        })
        .collect()
}

/// The snapshot `clip.restore` needs to bring a MIDI clip back.
pub(crate) fn snapshot(clip: &Clip) -> Result<ClipSnapshot, CommandError> {
    let (length, looping, notes) = midi_ref(clip)?;
    Ok(ClipSnapshot {
        name: clip.name.clone(),
        color: clip.color,
        slot: clip.slot.clone(),
        launch_quantize: clip.launch_quantize,
        length,
        looping,
        notes: notes.iter().copied().collect(),
    })
}

/// Applies a command of this domain.
pub fn apply(command: &Command, state: &mut State, ctx: &mut Context) -> Result<Out, CommandError> {
    match command {
        Command::Get {
            clip,
            offset,
            limit,
        } => {
            let d = crate::query::clip_get(&state.project, clip, *offset, *limit)?;
            Ok(crate::query::answer(Output::Get {
                track: d.track,
                summary: d.summary,
                launch_quantize: d.launch_quantize,
                content: Box::new(d.content),
                placements: d.placements,
                page: d.page,
            }))
        }
        Command::NotesList {
            clip,
            offset,
            limit,
        } => {
            let (notes, page) = crate::query::clip_notes(&state.project, clip, *offset, *limit)?;
            Ok(crate::query::answer(Output::NotesList { notes, page }))
        }
        Command::Create {
            track,
            at,
            length,
            name,
        } => create(state, ctx, track, at, *length, name.as_deref()),
        Command::Delete { clip } => delete(state, clip),
        Command::Restore {
            track,
            clip,
            snapshot,
            placements,
        } => restore(state, track, clip, snapshot, placements),
        Command::Duplicate { clip, slot, track } => {
            duplicate(state, ctx, clip, slot.as_ref(), track.as_ref())
        }
        Command::Move { clip, track, slot } => {
            move_clip(state, clip, track.as_ref(), slot.as_ref())
        }
        Command::Rename { clip, name } => rename(state, clip, name),
        Command::SetLength {
            clip,
            length,
            trim,
            placements,
        } => set_length(state, clip, *length, *trim, placements.as_deref()),
        Command::SetLoop { clip, looping } => set_loop(state, clip, *looping),
        Command::SetContent {
            clip,
            length,
            looping,
            notes,
            placements,
        } => set_content(state, clip, *length, *looping, notes, placements.as_deref()),
        Command::NotesAdd { clip, notes } => notes_add(state, clip, notes),
        Command::NotesRemove { clip, notes } => notes_remove(state, clip, notes),
        Command::NotesUpdate { clip, changes } => notes_update(state, clip, changes),
        Command::NotesQuantize {
            clip,
            notes,
            grid,
            strength,
        } => notes_quantize(state, clip, notes.as_deref(), *grid, *strength),
        Command::NotesTranspose {
            clip,
            notes,
            semitones,
        } => notes_transpose(state, clip, notes, *semitones),
        Command::NotesDuplicate {
            clip,
            notes,
            offset,
        } => notes_duplicate(state, clip, notes, *offset),
    }
}

fn create(
    state: &mut State,
    ctx: &mut Context,
    track_id: &TrackId,
    at: &ClipTarget,
    length: Option<Tick>,
    name: Option<&str>,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let track = midi_track(project, track_id)?;
    let length = length.unwrap_or_else(|| project.time_signature().ticks_per_bar());
    check_length(length, "length")?;
    let name = name.map(str::trim).unwrap_or_default();
    check_name(name, "name")?;
    let name = if name.is_empty() {
        project.next_clip_name(track_id)
    } else {
        name.to_owned()
    };
    check_clips(project, track)?;
    let (slot, start) = match at {
        ClipTarget::Slot { scene } => {
            check_slot(project, track, scene, "at.scene")?;
            (Some(scene.clone()), None)
        }
        ClipTarget::Arrangement { start } => {
            check_placements(project, track, 1)?;
            check_span(*start, length, &mut taken_spans(track), "at.start")?;
            (None, Some(*start))
        }
    };
    let clip_id = ClipId::generate(&mut ctx.ids);
    let placement_id = start.map(|_| PlacementId::generate(&mut ctx.ids));
    let track = state
        .project
        .tracks
        .get_mut(track_id)
        .ok_or_else(|| not_found("track", track_id))?;
    track.clips.insert(
        clip_id.clone(),
        Clip {
            name,
            color: None,
            slot,
            launch_quantize: None,
            content: ClipContent::Midi {
                length,
                looping: None,
                notes: BTreeSet::new(),
            },
        },
    );
    if let (Some(start), Some(id)) = (start, &placement_id) {
        track.placements.insert(
            id.clone(),
            Placement {
                clip: clip_id.clone(),
                start,
                span: Span::Ticks {
                    length,
                    offset: Tick::ZERO,
                },
            },
        );
    }
    Ok(edited(
        Output::Create {
            clip: clip_id.clone(),
            placement: placement_id.clone(),
        },
        Command::Delete {
            clip: clip_id.clone(),
        },
        vec![ProjectEvent::ClipCreated {
            track: track_id.clone(),
            clip: clip_id,
            placements: placement_id.into_iter().collect(),
        }],
    ))
}

fn delete(state: &mut State, clip_id: &ClipId) -> Result<Out, CommandError> {
    let track_id = locate(&state.project, clip_id)?;
    let track = state
        .project
        .tracks
        .get_mut(&track_id)
        .ok_or_else(|| not_found("track", &track_id))?;
    let clip = track
        .clips
        .get(clip_id)
        .ok_or_else(|| not_found("clip", clip_id))?;
    let snap = snapshot(clip)?;
    let mut placements = Vec::new();
    for (id, p) in &track.placements {
        if p.clip != *clip_id {
            continue;
        }
        let Span::Ticks { length, offset } = p.span else {
            return Err(invalid("clip", "a MIDI clip has a placement in frames"));
        };
        placements.push(PlacementSnapshot {
            id: id.clone(),
            start: p.start,
            length,
            offset,
        });
    }
    track.clips.remove(clip_id);
    track.placements.retain(|_, p| p.clip != *clip_id);
    let ids = placements.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
    Ok(edited(
        Output::Delete {
            placements: count(placements.len()),
        },
        Command::Restore {
            track: track_id.clone(),
            clip: clip_id.clone(),
            snapshot: snap,
            placements,
        },
        vec![ProjectEvent::ClipDeleted {
            track: track_id,
            clip: clip_id.clone(),
            placements: ids,
        }],
    ))
}

fn restore(
    state: &mut State,
    track_id: &TrackId,
    clip_id: &ClipId,
    snap: &ClipSnapshot,
    placements: &[PlacementSnapshot],
) -> Result<Out, CommandError> {
    let project = &state.project;
    let track = midi_track(project, track_id)?;
    if project.contains_id(&clip_id.0) {
        return Err(invalid(
            "clip",
            format!("the identifier {clip_id} is already in use"),
        ));
    }
    let mut seen = HashSet::new();
    for p in placements {
        if project.contains_id(&p.id.0) || p.id.0 == clip_id.0 || !seen.insert(&p.id) {
            return Err(invalid(
                "placements",
                format!("the identifier {} is already in use", p.id),
            ));
        }
    }
    check_name(&snap.name, "snapshot.name")?;
    check_length(snap.length, "snapshot.length")?;
    check_loop(&snap.looping, snap.length, "snapshot.looping")?;
    check_notes(&snap.notes, snap.length, "snapshot.notes")?;
    let notes: BTreeSet<Note> = snap.notes.iter().copied().collect();
    if notes.len() != snap.notes.len() {
        return Err(invalid("snapshot.notes", "a note appears twice"));
    }
    check_note_limits(
        notes.len(),
        project_notes(project).saturating_add(notes.len()),
    )?;
    check_clips(project, track)?;
    check_placements(project, track, placements.len())?;
    if let Some(scene) = &snap.slot {
        check_slot(project, track, scene, "snapshot.slot")?;
    }
    // No overlap check: a restore brings back what existed, and the project does not
    // forbid overlapping placements (the rule is the arrangement's, AR-1), so the undo
    // of a delete never fails on them.
    for (i, p) in placements.iter().enumerate() {
        let field = format!("placements[{i}]");
        check_span(p.start, p.length, &mut Vec::new(), &field)?;
        if !(0 <= p.offset.get() && p.offset < snap.length) {
            return Err(invalid(field, "a placement's offset is inside the clip"));
        }
    }
    let track = state
        .project
        .tracks
        .get_mut(track_id)
        .ok_or_else(|| not_found("track", track_id))?;
    track.clips.insert(
        clip_id.clone(),
        Clip {
            name: snap.name.clone(),
            color: snap.color,
            slot: snap.slot.clone(),
            launch_quantize: snap.launch_quantize,
            content: ClipContent::Midi {
                length: snap.length,
                looping: snap.looping,
                notes,
            },
        },
    );
    for p in placements {
        track.placements.insert(
            p.id.clone(),
            Placement {
                clip: clip_id.clone(),
                start: p.start,
                span: Span::Ticks {
                    length: p.length,
                    offset: p.offset,
                },
            },
        );
    }
    Ok(edited(
        Output::Restore {},
        Command::Delete {
            clip: clip_id.clone(),
        },
        vec![ProjectEvent::ClipCreated {
            track: track_id.clone(),
            clip: clip_id.clone(),
            placements: placements.iter().map(|p| p.id.clone()).collect(),
        }],
    ))
}

fn duplicate(
    state: &mut State,
    ctx: &mut Context,
    clip_id: &ClipId,
    slot: Option<&SceneId>,
    to: Option<&TrackId>,
) -> Result<Out, CommandError> {
    let from_id = locate(&state.project, clip_id)?;
    let track_id = to.cloned().unwrap_or_else(|| from_id.clone());
    let project = &state.project;
    let from = midi_track(project, &from_id)?;
    let track = midi_track(project, &track_id)?;
    let source = from
        .clips
        .get(clip_id)
        .ok_or_else(|| not_found("clip", clip_id))?;
    let (_, _, notes) = midi_ref(source)?;
    check_clips(project, track)?;
    check_note_limits(
        notes.len(),
        project_notes(project).saturating_add(notes.len()),
    )?;
    let target = match slot {
        Some(scene) => {
            check_slot(project, track, scene, "slot")?;
            scene.clone()
        }
        None => {
            let after = source.slot.as_ref().filter(|_| track_id == from_id);
            next_free_slot(project, track, after)?
        }
    };
    let mut copy = source.clone();
    copy.slot = Some(target);
    let new_id = ClipId::generate(&mut ctx.ids);
    state
        .project
        .tracks
        .get_mut(&track_id)
        .ok_or_else(|| not_found("track", &track_id))?
        .clips
        .insert(new_id.clone(), copy);
    Ok(edited(
        Output::Duplicate {
            clip: new_id.clone(),
        },
        Command::Delete {
            clip: new_id.clone(),
        },
        vec![ProjectEvent::ClipCreated {
            track: track_id,
            clip: new_id,
            placements: vec![],
        }],
    ))
}

fn move_clip(
    state: &mut State,
    clip_id: &ClipId,
    to: Option<&TrackId>,
    slot: Option<&SceneId>,
) -> Result<Out, CommandError> {
    let from_id = locate(&state.project, clip_id)?;
    let to_id = to.cloned().unwrap_or_else(|| from_id.clone());
    let project = &state.project;
    let from = midi_track(project, &from_id)?;
    let target = midi_track(project, &to_id)?;
    let clip = from
        .clips
        .get(clip_id)
        .ok_or_else(|| not_found("clip", clip_id))?;
    midi_ref(clip)?;
    let previous_slot = clip.slot.clone();
    let placed = from.placements.values().any(|p| p.clip == *clip_id);
    let output = Output::Move {
        previous_track: from_id.clone(),
        previous_slot: previous_slot.clone(),
    };
    if to_id == from_id && previous_slot.as_ref() == slot {
        return Ok(unchanged(output));
    }
    if to_id != from_id && placed {
        return Err(invalid(
            "track",
            "a clip placed in the arrangement stays in its track",
        ));
    }
    match slot {
        Some(scene) => check_slot(project, target, scene, "slot")?,
        None if !placed => {
            return Err(invalid(
                "slot",
                "a clip outside the grid needs a placement in the arrangement",
            ));
        }
        None => {}
    }
    if to_id != from_id {
        check_total(
            "clips per track",
            target.clips.len(),
            1,
            MAX_CLIPS_PER_TRACK,
        )?;
    }
    let tracks = &mut state.project.tracks;
    let mut moved = tracks
        .get_mut(&from_id)
        .and_then(|t| t.clips.remove(clip_id))
        .ok_or_else(|| not_found("clip", clip_id))?;
    moved.slot = slot.cloned();
    tracks
        .get_mut(&to_id)
        .ok_or_else(|| not_found("track", &to_id))?
        .clips
        .insert(clip_id.clone(), moved);
    Ok(edited(
        output,
        Command::Move {
            clip: clip_id.clone(),
            track: Some(from_id.clone()),
            slot: previous_slot,
        },
        vec![
            ProjectEvent::ClipDeleted {
                track: from_id,
                clip: clip_id.clone(),
                placements: vec![],
            },
            ProjectEvent::ClipCreated {
                track: to_id,
                clip: clip_id.clone(),
                placements: vec![],
            },
        ],
    ))
}

/// The clip, mutably, wherever it is.
fn clip_mut<'a>(project: &'a mut Project, id: &ClipId) -> Result<&'a mut Clip, CommandError> {
    project
        .tracks
        .values_mut()
        .find_map(|t| t.clips.get_mut(id))
        .ok_or_else(|| not_found("clip", id))
}

fn rename(state: &mut State, clip_id: &ClipId, name: &str) -> Result<Out, CommandError> {
    let name = name.trim();
    check_name(name, "name")?;
    let midi = locate(&state.project, clip_id)
        .ok()
        .and_then(|t| state.project.tracks.get(&t))
        .and_then(|t| t.clips.get(clip_id))
        .is_some_and(|c| c.content.is_midi());
    let track_id = locate(&state.project, clip_id)?;
    let own_default = clip_mut(&mut state.project, clip_id)
        .map(|c| c.name.clone())
        .is_ok_and(|n| state.project.is_default_clip_name(&track_id, &n));
    let name = if name.is_empty() && midi {
        // A clip that already carries a default name of its track keeps it.
        if own_default {
            clip_mut(&mut state.project, clip_id)?.name.clone()
        } else {
            state.project.next_clip_name(&track_id)
        }
    } else {
        name.to_owned()
    };
    let clip = clip_mut(&mut state.project, clip_id)?;
    if clip.name == name {
        return Ok(unchanged(Output::Rename { previous: name }));
    }
    let previous = std::mem::replace(&mut clip.name, name);
    Ok(edited(
        Output::Rename {
            previous: previous.clone(),
        },
        Command::Rename {
            clip: clip_id.clone(),
            name: previous,
        },
        changed(clip_id, ClipPart::Name),
    ))
}

/// What shortening a clip does to its content when `trim` is set.
fn trimmed(
    notes: &BTreeSet<Note>,
    looping: Option<TickLoop>,
    length: Tick,
) -> (BTreeSet<Note>, Option<TickLoop>) {
    let notes = notes
        .iter()
        .filter(|n| n.start < length)
        .map(|n| {
            let mut n = *n;
            n.length = Tick(n.length.get().min(length.get() - n.start.get()));
            n
        })
        .collect();
    let looping = looping.and_then(|l| {
        (l.start < length).then(|| TickLoop {
            start: l.start,
            end: Tick(l.end.get().min(length.get())),
        })
    });
    (notes, looping)
}

fn set_length(
    state: &mut State,
    clip_id: &ClipId,
    length: Tick,
    trim: bool,
    placements: Option<&[PlacementLength]>,
) -> Result<Out, CommandError> {
    check_length(length, "length")?;
    let track_id = locate(&state.project, clip_id)?;
    let track = state
        .project
        .tracks
        .get_mut(&track_id)
        .ok_or_else(|| not_found("track", &track_id))?;
    check_offsets(track, clip_id, length)?;
    let old = midi_ref(
        track
            .clips
            .get(clip_id)
            .ok_or_else(|| not_found("clip", clip_id))?,
    )?
    .0;
    let follow = placement_changes(track, clip_id, old, length, placements)?;
    let clip = track
        .clips
        .get_mut(clip_id)
        .ok_or_else(|| not_found("clip", clip_id))?;
    let (len, looping, notes) = midi_mut(clip)?;
    let previous = *len;
    if previous == length && follow.is_empty() {
        return Ok(unchanged(Output::SetLength { previous }));
    }
    let past_notes = notes
        .iter()
        .filter(|n| n.start.get().saturating_add(n.length.get()) > length.get())
        .count();
    let past_loop = looping.is_some_and(|l| l.end > length);
    let cut = past_notes > 0 || past_loop;
    if cut && !trim {
        return Err(invalid(
            "length",
            format!(
                "{past_notes} note(s){} go past the new end; pass trim: true to cut them",
                if past_loop { " and the loop" } else { "" }
            ),
        ));
    }
    let restore = Some(previous_lengths(&follow));
    let inverse = if cut {
        let (new_notes, new_loop) = trimmed(notes, *looping, length);
        let inverse = Command::SetContent {
            clip: clip_id.clone(),
            length: previous,
            looping: *looping,
            notes: notes.iter().copied().collect(),
            placements: restore,
        };
        *notes = new_notes;
        *looping = new_loop;
        inverse
    } else {
        Command::SetLength {
            clip: clip_id.clone(),
            length: previous,
            trim: false,
            placements: restore,
        }
    };
    *len = length;
    apply_placement_changes(&mut track.placements, &follow);
    Ok(edited(
        Output::SetLength { previous },
        inverse,
        changed(clip_id, ClipPart::Length),
    ))
}

fn set_loop(
    state: &mut State,
    clip_id: &ClipId,
    new: Option<TickLoop>,
) -> Result<Out, CommandError> {
    let clip = clip_mut(&mut state.project, clip_id)?;
    let (length, looping, _) = midi_mut(clip)?;
    check_loop(&new, *length, "looping")?;
    let previous = *looping;
    if previous == new {
        return Ok(unchanged(Output::SetLoop { previous }));
    }
    *looping = new;
    Ok(edited(
        Output::SetLoop { previous },
        Command::SetLoop {
            clip: clip_id.clone(),
            looping: previous,
        },
        changed(clip_id, ClipPart::Loop),
    ))
}

fn set_content(
    state: &mut State,
    clip_id: &ClipId,
    length: Tick,
    new_loop: Option<TickLoop>,
    new_notes: &[Note],
    placements: Option<&[PlacementLength]>,
) -> Result<Out, CommandError> {
    check_length(length, "length")?;
    check_loop(&new_loop, length, "looping")?;
    check_notes(new_notes, length, "notes")?;
    let new_notes: BTreeSet<Note> = new_notes.iter().copied().collect();
    let in_project = project_notes(&state.project);
    let track_id = locate(&state.project, clip_id)?;
    let track = state
        .project
        .tracks
        .get_mut(&track_id)
        .ok_or_else(|| not_found("track", &track_id))?;
    check_offsets(track, clip_id, length)?;
    let old = midi_ref(
        track
            .clips
            .get(clip_id)
            .ok_or_else(|| not_found("clip", clip_id))?,
    )?
    .0;
    let follow = placement_changes(track, clip_id, old, length, placements)?;
    let clip = track
        .clips
        .get_mut(clip_id)
        .ok_or_else(|| not_found("clip", clip_id))?;
    let (len, looping, notes) = midi_mut(clip)?;
    check_note_limits(
        new_notes.len(),
        in_project
            .saturating_sub(notes.len())
            .saturating_add(new_notes.len()),
    )?;
    if *len == length && *looping == new_loop && *notes == new_notes && follow.is_empty() {
        return Ok(unchanged(Output::SetContent {
            notes: count(notes.len()),
        }));
    }
    let inverse = Command::SetContent {
        clip: clip_id.clone(),
        length: *len,
        looping: *looping,
        notes: notes.iter().copied().collect(),
        placements: Some(previous_lengths(&follow)),
    };
    let n = new_notes.len();
    *len = length;
    apply_placement_changes(&mut track.placements, &follow);
    *looping = new_loop;
    *notes = new_notes;
    Ok(edited(
        Output::SetContent { notes: count(n) },
        inverse,
        changed(clip_id, ClipPart::Content),
    ))
}

fn notes_add(state: &mut State, clip_id: &ClipId, batch: &[Note]) -> Result<Out, CommandError> {
    let in_project = project_notes(&state.project);
    let clip = clip_mut(&mut state.project, clip_id)?;
    let (length, _, notes) = midi_mut(clip)?;
    check_notes(batch, *length, "notes")?;
    let fresh: Vec<Note> = batch
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|n| !notes.contains(n))
        .collect();
    if fresh.is_empty() {
        return Ok(unchanged(Output::NotesAdd { added: 0 }));
    }
    check_note_limits(
        notes.len().saturating_add(fresh.len()),
        in_project.saturating_add(fresh.len()),
    )?;
    notes.extend(fresh.iter().copied());
    Ok(edited(
        Output::NotesAdd {
            added: count(fresh.len()),
        },
        Command::NotesRemove {
            clip: clip_id.clone(),
            notes: fresh,
        },
        changed(clip_id, ClipPart::Notes),
    ))
}

fn notes_remove(state: &mut State, clip_id: &ClipId, batch: &[Note]) -> Result<Out, CommandError> {
    let clip = clip_mut(&mut state.project, clip_id)?;
    let (_, _, notes) = midi_mut(clip)?;
    let present: Vec<Note> = batch
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|n| notes.contains(n))
        .collect();
    if present.is_empty() {
        return Ok(unchanged(Output::NotesRemove { removed: 0 }));
    }
    for n in &present {
        notes.remove(n);
    }
    Ok(edited(
        Output::NotesRemove {
            removed: count(present.len()),
        },
        Command::NotesAdd {
            clip: clip_id.clone(),
            notes: present,
        },
        changed(clip_id, ClipPart::Notes),
    ))
}

fn notes_update(
    state: &mut State,
    clip_id: &ClipId,
    changes: &[NoteChange],
) -> Result<Out, CommandError> {
    let clip = clip_mut(&mut state.project, clip_id)?;
    let (length, _, notes) = midi_mut(clip)?;
    let mut froms = HashSet::new();
    let mut tos = HashSet::new();
    for (i, change) in changes.iter().enumerate() {
        check_note(&change.to, *length, &format!("changes[{i}].to"))?;
        if !notes.contains(&change.from) {
            return Err(not_found(
                "note",
                format!("changes[{i}].from {:?}", change.from),
            ));
        }
        if !froms.insert(change.from) {
            return Err(invalid(
                format!("changes[{i}].from"),
                "two changes address the same note",
            ));
        }
        if !tos.insert(change.to) {
            return Err(invalid(
                format!("changes[{i}].to"),
                "two changes give the same note",
            ));
        }
    }
    // All the `from` notes leave, all the `to` notes arrive: a `to` may equal a `from`
    // (chain, swap, identity) but not a note that stays.
    for (i, change) in changes.iter().enumerate() {
        if notes.contains(&change.to) && !froms.contains(&change.to) {
            return Err(invalid(
                format!("changes[{i}].to"),
                "the note already is in the clip",
            ));
        }
    }
    // The net change: what leaves and what arrives once the overlap is cancelled. A
    // swap or an identity changes nothing and records nothing.
    let leaving: Vec<Note> = froms.difference(&tos).copied().collect();
    let arriving: Vec<Note> = tos.difference(&froms).copied().collect();
    if leaving.is_empty() && arriving.is_empty() {
        return Ok(unchanged(Output::NotesUpdate { updated: 0 }));
    }
    for n in &leaving {
        notes.remove(n);
    }
    notes.extend(arriving.iter().copied());
    let moved = changes.iter().filter(|c| c.from != c.to);
    Ok(edited(
        Output::NotesUpdate {
            updated: count(arriving.len()),
        },
        Command::NotesUpdate {
            clip: clip_id.clone(),
            changes: moved
                .map(|c| NoteChange {
                    from: c.to,
                    to: c.from,
                })
                .collect(),
        },
        changed(clip_id, ClipPart::Notes),
    ))
}

/// Where `start` goes when quantized to `grid` ticks at `strength` percent: toward the
/// nearest grid line (halfway goes to the later one), by that share of the distance
/// (halfway rounds away from zero). Exact integer maths, also for a negative `start`.
pub(crate) fn quantized_start(start: i64, grid: i64, strength: u8) -> i64 {
    let grid = i128::from(grid.max(1));
    let from = i128::from(start);
    let nearest = (from + grid / 2).div_euclid(grid) * grid;
    let scaled = (nearest - from) * i128::from(strength.min(100));
    let share = (scaled.abs() + 50) / 100;
    let moved = if scaled < 0 { -share } else { share };
    i64::try_from(from + moved).unwrap_or(start)
}

/// The notes of `wanted` as they are in the clip, each once: all of them, or a refusal naming the first that is not.
fn present_notes(
    notes: &BTreeSet<Note>,
    wanted: &[Note],
    field: &str,
) -> Result<Vec<Note>, CommandError> {
    let mut seen = HashSet::new();
    let mut found = Vec::with_capacity(wanted.len());
    for (i, note) in wanted.iter().enumerate() {
        if !notes.contains(note) {
            return Err(not_found("note", format!("{field}[{i}] {note:?}")));
        }
        if seen.insert(*note) {
            found.push(*note);
        }
    }
    Ok(found)
}

/// Applies moves of notes (each `from` is in the clip, each `to` is valid) and returns how
/// many moved with the inverse, or `None` when nothing changed. Notes that become equal
/// merge; then the inverse is the whole previous content, otherwise the opposite moves.
fn rewrite_notes(
    clip_id: &ClipId,
    length: Tick,
    looping: Option<TickLoop>,
    notes: &mut BTreeSet<Note>,
    moves: Vec<NoteChange>,
) -> Option<(usize, Command)> {
    let moves: Vec<NoteChange> = moves.into_iter().filter(|m| m.from != m.to).collect();
    let froms: HashSet<Note> = moves.iter().map(|m| m.from).collect();
    let tos: HashSet<Note> = moves.iter().map(|m| m.to).collect();
    if froms == tos {
        return None;
    }
    let merged =
        tos.len() < moves.len() || tos.iter().any(|t| notes.contains(t) && !froms.contains(t));
    let inverse = if merged {
        Command::SetContent {
            clip: clip_id.clone(),
            length,
            looping,
            notes: notes.iter().copied().collect(),
            placements: None,
        }
    } else {
        Command::NotesUpdate {
            clip: clip_id.clone(),
            changes: moves
                .iter()
                .map(|m| NoteChange {
                    from: m.to,
                    to: m.from,
                })
                .collect(),
        }
    };
    for from in &froms {
        notes.remove(from);
    }
    notes.extend(tos.iter().copied());
    Some((moves.len(), inverse))
}

fn notes_quantize(
    state: &mut State,
    clip_id: &ClipId,
    chosen: Option<&[Note]>,
    grid: Tick,
    strength: u8,
) -> Result<Out, CommandError> {
    if !(1..=MAX_TICK).contains(&grid.get()) {
        return Err(invalid(
            "grid",
            format!("a grid step is 1 to {MAX_TICK} ticks, not {}", grid.get()),
        ));
    }
    if strength > 100 {
        return Err(invalid("strength", "a strength is 0 to 100 percent"));
    }
    let clip = clip_mut(&mut state.project, clip_id)?;
    let (length, looping, notes) = midi_mut(clip)?;
    let targets: Vec<Note> = match chosen {
        Some(list) => present_notes(notes, list, "notes")?,
        None => notes.iter().copied().collect(),
    };
    let moves = targets
        .iter()
        .map(|&from| {
            let room = length.get().saturating_sub(from.length.get()).max(0);
            let start = quantized_start(from.start.get(), grid.get(), strength).clamp(0, room);
            NoteChange {
                from,
                to: Note {
                    start: Tick(start),
                    ..from
                },
            }
        })
        .collect();
    let (length, looping) = (*length, *looping);
    Ok(
        match rewrite_notes(clip_id, length, looping, notes, moves) {
            None => unchanged(Output::NotesQuantize { quantized: 0 }),
            Some((n, inverse)) => edited(
                Output::NotesQuantize {
                    quantized: count(n),
                },
                inverse,
                changed(clip_id, ClipPart::Notes),
            ),
        },
    )
}

fn notes_transpose(
    state: &mut State,
    clip_id: &ClipId,
    chosen: &[Note],
    semitones: i32,
) -> Result<Out, CommandError> {
    let clip = clip_mut(&mut state.project, clip_id)?;
    let (length, looping, notes) = midi_mut(clip)?;
    let targets = present_notes(notes, chosen, "notes")?;
    let mut moves = Vec::with_capacity(targets.len());
    for (i, from) in targets.iter().enumerate() {
        let pitch = i64::from(from.pitch) + i64::from(semitones);
        let Some(pitch) = u8::try_from(pitch).ok().filter(|p| *p <= 127) else {
            return Err(invalid(
                format!("notes[{i}]"),
                format!(
                    "transposing pitch {} by {semitones} leaves 0 to 127; nothing is clamped",
                    from.pitch
                ),
            ));
        };
        moves.push(NoteChange {
            from: *from,
            to: Note { pitch, ..*from },
        });
    }
    let (length, looping) = (*length, *looping);
    Ok(
        match rewrite_notes(clip_id, length, looping, notes, moves) {
            None => unchanged(Output::NotesTranspose { transposed: 0 }),
            Some((n, inverse)) => edited(
                Output::NotesTranspose {
                    transposed: count(n),
                },
                inverse,
                changed(clip_id, ClipPart::Notes),
            ),
        },
    )
}

fn notes_duplicate(
    state: &mut State,
    clip_id: &ClipId,
    chosen: &[Note],
    offset: Option<Tick>,
) -> Result<Out, CommandError> {
    let in_project = project_notes(&state.project);
    let in_bar = state.project.time_signature().ticks_per_bar();
    let clip = clip_mut(&mut state.project, clip_id)?;
    let (length, _, notes) = midi_mut(clip)?;
    let sources = present_notes(notes, chosen, "notes")?;
    if sources.is_empty() {
        return Ok(unchanged(Output::NotesDuplicate { added: 0 }));
    }
    let first = sources.iter().map(|n| n.start.get()).min().unwrap_or(0);
    let last = sources
        .iter()
        .map(|n| n.start.get().saturating_add(n.length.get()))
        .max()
        .unwrap_or(0);
    let bar = in_bar.get().max(1);
    let shift = offset.map_or_else(
        || {
            let from = first.saturating_sub(first.rem_euclid(bar));
            let span = last.saturating_sub(from).max(1);
            span.saturating_add(bar - 1)
                .div_euclid(bar)
                .saturating_mul(bar)
        },
        Tick::get,
    );
    if shift == 0 {
        return Err(invalid(
            "offset",
            "an offset of 0 would copy the notes onto themselves",
        ));
    }
    let copies: Vec<Note> = sources
        .iter()
        .map(|n| Note {
            start: Tick(n.start.get().saturating_add(shift)),
            ..*n
        })
        .collect();
    check_notes(&copies, *length, "notes")?;
    let fresh: Vec<Note> = copies
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|n| !notes.contains(n))
        .collect();
    if fresh.is_empty() {
        return Ok(unchanged(Output::NotesDuplicate { added: 0 }));
    }
    check_note_limits(
        notes.len().saturating_add(fresh.len()),
        in_project.saturating_add(fresh.len()),
    )?;
    notes.extend(fresh.iter().copied());
    Ok(edited(
        Output::NotesDuplicate {
            added: count(fresh.len()),
        },
        Command::NotesRemove {
            clip: clip_id.clone(),
            notes: fresh,
        },
        changed(clip_id, ClipPart::Notes),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizing_a_start_moves_a_share_of_the_way_to_the_nearest_line() {
        // Full strength: the nearest line, halfway to the later one.
        assert_eq!(quantized_start(150, 240, 100), 240);
        assert_eq!(quantized_start(119, 240, 100), 0);
        assert_eq!(quantized_start(120, 240, 100), 240);
        assert_eq!(quantized_start(240, 240, 100), 240);
        // Strength 0 moves nothing, 50 goes half of the way, 1 almost nothing.
        assert_eq!(quantized_start(150, 240, 0), 150);
        assert_eq!(quantized_start(150, 240, 50), 195);
        assert_eq!(quantized_start(150, 240, 1), 151);
        assert_eq!(quantized_start(20, 240, 1), 20);
        // The share rounds away from zero when halfway: 25 % of -30 is -7.5, so -8.
        assert_eq!(quantized_start(30, 240, 25), 22);
        assert_eq!(quantized_start(210, 240, 25), 218);
        // A grid of one tick, and one wider than the position, are fine.
        assert_eq!(quantized_start(77, 1, 100), 77);
        assert_eq!(quantized_start(77, 100_000, 100), 0);
        // Negative positions (no note has one, but the maths is total) snap the same way.
        assert_eq!(quantized_start(-150, 240, 100), -240);
        assert_eq!(quantized_start(-120, 240, 100), 0);
        assert_eq!(quantized_start(-150, 240, 50), -195);
        // Extremes saturate instead of panicking.
        let _ = quantized_start(i64::MAX, 960, 100);
        let _ = quantized_start(i64::MIN, i64::MAX, 100);
        let _ = quantized_start(5, i64::MIN, 100);
    }

    #[test]
    fn a_count_past_its_limit_is_a_typed_refusal() {
        assert_eq!(check_total("clips", MAX_CLIPS - 1, 1, MAX_CLIPS), Ok(()));
        assert_eq!(
            check_total("clips", MAX_CLIPS, 1, MAX_CLIPS),
            Err(CommandError::Limit {
                entity: "clips".into(),
                max: MAX_CLIPS as u64
            })
        );
        assert!(check_total("placements", MAX_PLACEMENTS - 3, 4, MAX_PLACEMENTS).is_err());
        assert!(check_total("placements", usize::MAX, 1, MAX_PLACEMENTS).is_err());
    }
}
