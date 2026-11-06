//! `clip.*`: MIDI clips and their notes: create, delete, restore, duplicate, rename,
//! length, loop, content and the note batches (CL-1; epics 5 and 8, CL and PN). Notes are
//! addressed by value: there is no `NoteId` (CT-4).
//!
//! The contract of the clip commands, written once here and repeated in their
//! descriptions for agents:
//!
//! - A clip lives in a track and is addressed by its `ClipId`; only `clip.create`,
//!   `clip.restore`, `clip.import_audio` and `clip.restore_audio` name the track. A MIDI clip
//!   lives in a MIDI track; an audio clip lives in an audio track and comes from an imported
//!   file (`clip.import_audio`, AU-1, into the pool, a slot or the arrangement since AU-5):
//!   the commands that read or write notes, lengths or loops in ticks, and
//!   `clip.duplicate`, refuse it; its range, gain and loop in frames are the
//!   `clip.audio.*` commands (AU-5), which refuse a MIDI clip; `clip.rename`, `clip.move`
//!   and `clip.delete` take both (the undo of deleting an audio clip is
//!   `clip.restore_audio`).
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
    MAX_CHANNELS, MAX_CLIPS, MAX_CLIPS_PER_TRACK, MAX_FRAME, MAX_NAME_CHARS, MAX_NOTES,
    MAX_NOTES_PER_CLIP, MAX_PLACEMENTS, MAX_PLACEMENTS_PER_TRACK, MAX_TICK, SAMPLE_RATES,
};
use project::{
    AudioMedia, Clip, ClipContent, ClipId, Frame, FrameLoop, Hash, Note, Placement, PlacementId,
    Project, Quantize, Rgb, SampleRate, SceneId, Span, Tempo, Tick, TickLoop, Track, TrackId,
    TrackKind,
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

/// The length of one placement of an audio clip, in frames of its file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacementFrames {
    pub placement: PlacementId,
    #[schemars(with = "u64")]
    pub length: Frame,
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

/// An audio clip as `clip.restore_audio` brings it back (everything but its identifier).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioClipSnapshot {
    pub name: String,
    pub color: Option<Rgb>,
    /// The scene of its launch-grid slot, when it sits in one.
    pub slot: Option<SceneId>,
    /// A per-clip launch quantization, when one was set (the MVP never sets one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_quantize: Option<Quantize>,
    /// The audio file it plays, with its metadata.
    pub media: AudioMedia,
    /// First frame of the file the clip plays.
    #[schemars(with = "u64")]
    pub start: Frame,
    /// How many frames it plays.
    #[schemars(with = "u64")]
    pub frames: Frame,
    /// Clip gain in dB; `null` for silence.
    #[schemars(with = "Option<f64>")]
    #[serde(with = "project::serde_util::db")]
    pub gain_db: f64,
    /// Loop in frames, relative to `start`.
    pub looping: Option<FrameLoop>,
}

/// A placement of an audio clip, with its identifier, as `clip.restore_audio` brings it
/// back: it starts at a tick of the arrangement and plays `length` frames of the clip from
/// `offset`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioPlacementSnapshot {
    pub id: PlacementId,
    /// Start in the arrangement, in ticks.
    pub start: Tick,
    /// Length played, in frames of the file.
    #[schemars(with = "u64")]
    pub length: Frame,
    /// Where in the clip the placement starts, in frames.
    #[schemars(with = "u64")]
    pub offset: Frame,
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

    /// Delete a clip (MIDI or audio) and its placements in the arrangement. Undo:
    /// `clip.restore` brings back a MIDI clip, its notes and its placements with their
    /// identifiers; `clip.restore_audio` an audio clip and its placements. The file of an
    /// audio clip stays in the project's `blobs/` (blobs are removed on request only).
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

    /// Make an audio clip in an audio track from an imported audio file (AU-1). `media` is
    /// the SHA-256 of the file (64 lower-case hexadecimal characters, the name of its blob):
    /// either a file the page has just imported (the page reads the file the user chose,
    /// stores its bytes in the project's `blobs/` and stages its metadata before running
    /// this command), or a file an audio clip of the project already uses (`clip.get` shows
    /// its `media.hash`), so another clip of the same file needs no new import. The clip
    /// plays the whole file, at 0 dB, without a loop. `at` is optional: without it (or
    /// `null`) the clip sits in the track's pool (no launch-grid slot, no placement); with
    /// `{"at": "slot", "scene": <scene id>}` it sits in that launch-grid slot (the scene must
    /// exist and the slot be free in the track); with `{"at": "arrangement", "start": <tick>}`
    /// it is also placed whole in the arrangement from that tick (its end is computed from
    /// the file's duration at the project's tempo), refused when that placement would
    /// overlap another placement of the track or end past 2^40. `name` is optional (trimmed,
    /// at most 200 characters): without one the clip takes the file's name without its
    /// extension. Refused when the track is not an audio track or the media is unknown.
    /// Undo: `clip.delete` of the clip (and of its placement); the file stays in the
    /// project's `blobs/` (blobs are removed on request only).
    "clip.import_audio" (kind = "edit", undo = "inverse", destructive = false)
    ImportAudio {
        track: TrackId,
        media: Hash,
        name: Option<String>,
        #[serde(default)]
        at: Option<ClipTarget>,
    } => {
        /// The clip created.
        clip: ClipId,
        /// Its placement, when it was imported into the arrangement.
        placement: Option<PlacementId>,
    };

    /// Set the range of the file an audio clip plays: the frames `[start, start + frames)`
    /// of its file (frames of the file, at its own sample rate). Refused when the range is
    /// empty or leaves the file, when the clip's loop would end past the new length (clear
    /// or move the loop first with `clip.audio.set_loop`: nothing is cut silently), and when
    /// a placement of the clip starts inside it at or past the new length. Every placement
    /// that plays the clip whole (offset 0, length equal to its old length in frames) takes
    /// the new length in the same command; a growing placement stops at the start of the
    /// next placement of its track; a placement that plays only part of the clip is left
    /// alone, and the command is refused when that would leave one ending past the new
    /// length (an audio placement always ends inside its clip) or overlapping another
    /// placement of the track: move or resize it first. Refused for a MIDI clip. Undo: set
    /// the range back, with the lengths of the placements.
    "clip.audio.set_range" (kind = "edit", undo = "inverse", destructive = false)
    AudioSetRange {
        clip: ClipId,
        /// First frame of the file played.
        #[schemars(with = "u64")]
        start: Frame,
        /// How many frames are played, at least 1.
        #[schemars(with = "u64")]
        frames: Frame,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        // Internal: the exact placement lengths an undo gives back, not in the schema.
        #[schemars(skip)]
        placements: Option<Vec<PlacementFrames>>,
    } => {
        /// The first frame before the command.
        #[schemars(with = "u64")]
        previous_start: Frame,
        /// The length in frames before the command.
        #[schemars(with = "u64")]
        previous_frames: Frame,
    };

    /// Set an audio clip's gain in dB, from -96 to +6; `null` is silence. A value outside
    /// the range or not a number is refused, and so is a MIDI clip. Undo: set the previous
    /// gain back.
    "clip.audio.set_gain" (kind = "edit", undo = "inverse", destructive = false)
    AudioSetGain {
        clip: ClipId,
        /// Required: `null` is silence, a missing field is an error.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(required, schema_with = "crate::mixer::nullable_gain")]
        gain_db: Option<f64>,
    } => {
        /// The gain before the command, in dB; `null` was silence.
        #[schemars(required, schema_with = "crate::mixer::nullable_number")]
        previous: Option<f64>,
    };

    /// Set an audio clip's loop: `{"start": <frame>, "end": <frame>}` in frames from the
    /// clip's start (not from the file's), with `0 <= start < end <= frames` (the clip's
    /// length in frames), or `null` for no loop. Refused for a MIDI clip (`clip.set_loop`
    /// sets its loop in ticks). Undo: set the previous loop back.
    "clip.audio.set_loop" (kind = "edit", undo = "inverse", destructive = false)
    AudioSetLoop {
        clip: ClipId,
        looping: Option<FrameLoop>,
    } => {
        /// The loop before the command.
        previous: Option<FrameLoop>,
    };

    /// Bring back an audio clip with its identifier, as `clip.delete` removed it: the
    /// inverse of deleting an audio clip. Its placements give their length and offset in
    /// frames of the file. Refused when the track is not an audio track, when the clip's
    /// identifier or a placement's already exists in the project, when the slot is taken
    /// or its scene is gone, or when the snapshot breaks a rule of the project (a range
    /// outside the file, a loop outside the range, a gain that is not a number, a
    /// placement ending past the clip: `offset + length` at most the clip's frames, or
    /// overlapping another placement of the track or of the snapshot). Undo:
    /// `clip.delete`.
    "clip.restore_audio" (kind = "edit", undo = "inverse", destructive = false)
    RestoreAudio {
        track: TrackId,
        clip: ClipId,
        snapshot: AudioClipSnapshot,
        placements: Vec<AudioPlacementSnapshot>,
    } => {};

    /// Ask the user for audio files to import into an audio track (AU-1): the page opens
    /// its file dialog, then imports each file chosen (its bytes are kept in the project's
    /// `blobs/`, its decoded audio checked) and makes a clip of it with `clip.import_audio`.
    /// A file dialog needs a click by the user, so an agent that is not driven by one gets a
    /// notice in the page instead. Formats: WAV, AIFF, CAF, FLAC, MP3, AAC (MP4, M4A, ADTS),
    /// ALAC, Ogg Vorbis, Opus (Ogg, WebM: mono or stereo). The answer only says the request was
    /// accepted; the clips arrive with `clip.import_audio`, one history entry each. Refused
    /// when the track is not an audio track. Not undoable itself.
    "clip.choose_audio" (kind = "live", undo = "none", destructive = false)
    ChooseAudio { track: TrackId } => {};

    /// Stop the audio import under way in the page (the files chosen by `clip.choose_audio`):
    /// the file being imported is dropped (nothing of it is kept) and the files after it are
    /// not imported; the clips of the files already imported stay. Does nothing when no import
    /// runs. Not undoable itself.
    "clip.cancel_audio_import" (kind = "live", undo = "none", destructive = false)
    CancelAudioImport {} => {};

    /// Copy a clip into a new, independent clip with the same name ("Bass 2" stays
    /// "Bass 2": a copy plays the same part), colour, length, loop and notes (an audio clip:
    /// the same file, range, gain and loop; its file is not copied); editing one never
    /// changes the other. The copy always sits in a launch-grid slot: of the source's
    /// track, or of `track` when given (another track of its kind: a MIDI clip to a MIDI
    /// track, an audio clip to an audio track). `slot` when given must be free in that track, otherwise the copy takes
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

    /// Move a clip to another launch-grid slot, of the same track or of `track` (another
    /// track of its kind: a MIDI clip to a MIDI track, an audio clip to an audio track),
    /// keeping its identifier, its name and its content. `slot`
    /// is the scene of the target slot, which must be free; `null` takes the clip out of
    /// the grid (with no placement, it stays in its track's pool, #473). A clip
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

    /// Repeat notes of a MIDI clip end to end up to the tick `until` (#710), in one batch
    /// (one history entry): an arpeggio, a hi-hat or a bass figure of a beat or two filled
    /// up to the end of a bar. Each source note is addressed by its whole value and must be
    /// in the clip. The period of the repetition (#770) is the extent of the notes, from the
    /// `grid` line at or before the first start to the last end, rounded up to a whole
    /// number of a unit that depends on that extent: the `grid` step (ticks, at least 1) for
    /// a figure of one beat or less, the beat for one of more than a beat up to half a bar,
    /// the bar for one of more than half a bar (a unit finer than the grid takes the grid
    /// step); beat and bar are the project's time signature's. Leave `grid` out for the
    /// exact span, first start to last end, with no rounding. Copy k of every note starts k
    /// periods later, for each copy of the figure that starts before `until`. A copied note that would
    /// start at or after `until` is left out, and one that would end after it is shortened
    /// to end there. To fill up to the end of bar N, give the tick where bar N ends. The
    /// copies never pass the end of the clip: when `until` is past it, a copied note that
    /// would end past the clip's end is shortened to end there and one that would start
    /// there or later is left out; `at_clip_end` says whether that happened (a copy shortened
    /// or left out because the clip ends before `until`). Refused when `grid` is
    /// below 1, when `until` is not after the start of the first copy, when the first copy
    /// would start at or after the end of the clip, or when the copies would pass the limit
    /// of notes. A copy equal to a note already in the clip changes nothing. Undo: remove
    /// the notes actually added.
    "clip.notes.repeat" (kind = "edit", undo = "inverse", destructive = false)
    NotesRepeat {
        clip: ClipId,
        notes: Vec<Note>,
        until: Tick,
        grid: Option<Tick>,
    } => {
        /// How many copies were new.
        added: u32,
        /// True when the clip's end, not `until`, shortened or left out a copy.
        at_clip_end: bool,
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

fn taken_spans(tempo: Tempo, track: &Track) -> Vec<(i64, i64)> {
    track
        .placements
        .values()
        .filter_map(|p| placement_ticks(tempo, track, p))
        .collect()
}

/// The ticks `[start, end)` a placement covers: its length for a MIDI placement; for an
/// audio one, its frames at the rate of its clip's file converted at `tempo` (no tick
/// length is stored), at least one tick. `None` when the clip is not in the track.
pub(crate) fn placement_ticks(tempo: Tempo, track: &Track, p: &Placement) -> Option<(i64, i64)> {
    match p.span {
        Span::Ticks { length, .. } => {
            Some((p.start.get(), p.start.get().saturating_add(length.get())))
        }
        Span::Frames { length, .. } => {
            let ClipContent::Audio { media, .. } = &track.clips.get(&p.clip)?.content else {
                return None;
            };
            let ticks = tempo
                .frames_to_ticks(length, media.sample_rate)
                .get()
                .max(1);
            Some((p.start.get(), p.start.get().saturating_add(ticks)))
        }
    }
}

/// The snapshot `clip.restore_audio` (and `arrangement.restore`) needs to bring an audio
/// clip back; `None` for a MIDI clip.
pub(crate) fn audio_snapshot(clip: &Clip) -> Option<AudioClipSnapshot> {
    let ClipContent::Audio {
        media,
        start,
        frames,
        gain_db,
        looping,
    } = &clip.content
    else {
        return None;
    };
    Some(AudioClipSnapshot {
        name: clip.name.clone(),
        color: clip.color,
        slot: clip.slot.clone(),
        launch_quantize: clip.launch_quantize,
        media: media.clone(),
        start: *start,
        frames: *frames,
        gain_db: *gain_db,
        looping: *looping,
    })
}

/// The audio clip an [`AudioClipSnapshot`] describes.
pub(crate) fn audio_clip_of(snap: &AudioClipSnapshot) -> Clip {
    Clip {
        name: snap.name.clone(),
        color: snap.color,
        slot: snap.slot.clone(),
        launch_quantize: snap.launch_quantize,
        content: ClipContent::Audio {
            media: snap.media.clone(),
            start: snap.start,
            frames: snap.frames,
            gain_db: snap.gain_db,
            looping: snap.looping,
        },
    }
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
            let d =
                crate::query::clip_get(&state.project, &ctx.missing_media, clip, *offset, *limit)?;
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
        Command::ImportAudio {
            track,
            media,
            name,
            at,
        } => import_audio(state, ctx, track, media, name.as_deref(), at.as_ref()),
        Command::AudioSetRange {
            clip,
            start,
            frames,
            placements,
        } => audio_set_range(state, clip, *start, *frames, placements.as_deref()),
        Command::AudioSetGain { clip, gain_db } => audio_set_gain(state, clip, *gain_db),
        Command::AudioSetLoop { clip, looping } => audio_set_loop(state, clip, *looping),
        Command::RestoreAudio {
            track,
            clip,
            snapshot,
            placements,
        } => restore_audio(state, track, clip, snapshot, placements),
        // The file dialog and the import belong to the page (a dialog needs a click): the
        // command only checks the track, so an agent hears at once of a wrong one.
        Command::ChooseAudio { track } => {
            audio_track(&state.project, track)?;
            Ok(unchanged(Output::ChooseAudio {}))
        }
        // The import runs in the page: the command only says it is to stop.
        Command::CancelAudioImport {} => Ok(unchanged(Output::CancelAudioImport {})),
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
        Command::NotesRepeat {
            clip,
            notes,
            until,
            grid,
        } => notes_repeat(state, clip, notes, *until, *grid),
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
            check_span(
                *start,
                length,
                &mut taken_spans(project.tempo(), track),
                "at.start",
            )?;
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
    if !clip.content.is_midi() {
        return delete_audio(state, &track_id, clip_id);
    }
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

pub(crate) fn audio_track<'a>(
    project: &'a Project,
    track: &TrackId,
) -> Result<&'a Track, CommandError> {
    let found = project
        .tracks
        .get(track)
        .ok_or_else(|| not_found("track", track))?;
    if found.kind != TrackKind::Audio {
        return Err(invalid(
            "track",
            "an audio clip lives in an audio track (`track.create` with `kind: audio`)",
        ));
    }
    Ok(found)
}

/// The clip name a file gives: its name without the extension, trimmed and cut to the
/// longest name a clip takes.
fn name_of_file(file: &str) -> String {
    let stem = match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.trim().is_empty() && !ext.is_empty() && !ext.contains(' ') => {
            stem
        }
        _ => file,
    };
    stem.trim()
        .chars()
        .take(MAX_NAME_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The metadata of a media: staged by the host after an import, or recorded by a clip of the
/// project. The staged one comes from decoding the file's bytes, so it wins.
fn known_media(project: &Project, ctx: &Context, hash: &Hash) -> Option<AudioMedia> {
    ctx.media.get(hash).cloned().or_else(|| {
        project
            .tracks
            .values()
            .flat_map(|t| t.clips.values())
            .find_map(|c| match &c.content {
                ClipContent::Audio { media, .. } if media.hash == *hash => Some(media.clone()),
                _ => None,
            })
    })
}

/// Refuses an audio file's metadata that the project's validation would refuse.
fn check_media(media: &AudioMedia, field: &str) -> Result<(), CommandError> {
    check_name(&media.name, &format!("{field}.name"))?;
    if !(1..=MAX_CHANNELS).contains(&media.channels) {
        return Err(invalid(format!("{field}.channels"), "1 to 64 channels"));
    }
    if !SAMPLE_RATES.contains(&media.sample_rate.hz()) {
        return Err(invalid(
            format!("{field}.sample_rate"),
            "a sample rate from 8 to 768 kHz",
        ));
    }
    if media.frames.get() == 0 || media.frames.get() > MAX_FRAME {
        return Err(invalid(format!("{field}.frames"), "1 to 2^40 frames"));
    }
    Ok(())
}

/// Refuses an audio clip's content that the project's validation would refuse.
pub(crate) fn check_audio(snap: &AudioClipSnapshot, field: &str) -> Result<(), CommandError> {
    check_media(&snap.media, &format!("{field}.media"))?;
    let media = &snap.media;
    let end = snap.start.get().checked_add(snap.frames.get());
    if snap.frames.get() == 0 || end.is_none_or(|end| end > media.frames.get()) {
        return Err(invalid(
            format!("{field}.frames"),
            "the range [start, start + frames) is inside the file and not empty",
        ));
    }
    // Any number, as `validate` takes: a restore brings back what a delete took, even a
    // gain beyond what `clip.audio.set_gain` sets.
    if !(snap.gain_db.is_finite() || snap.gain_db == f64::NEG_INFINITY) {
        return Err(invalid(
            format!("{field}.gain_db"),
            "a number of dB, or null",
        ));
    }
    if let Some(l) = snap.looping
        && !(l.start < l.end && l.end <= snap.frames)
    {
        return Err(invalid(
            format!("{field}.looping"),
            "0 <= start < end <= frames",
        ));
    }
    Ok(())
}

fn import_audio(
    state: &mut State,
    ctx: &mut Context,
    track_id: &TrackId,
    hash: &Hash,
    name: Option<&str>,
    at: Option<&ClipTarget>,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let track = audio_track(project, track_id)?;
    let media = known_media(project, ctx, hash).ok_or_else(|| {
        invalid(
            "media",
            format!("no imported file has the SHA-256 {hash}: import the file first (`clip.choose_audio`)"),
        )
    })?;
    check_media(&media, "media")?;
    let name = name.map(str::trim).unwrap_or_default();
    check_name(name, "name")?;
    let name = if name.is_empty() {
        name_of_file(&media.name)
    } else {
        name.to_owned()
    };
    check_clips(project, track)?;
    let (slot, start) = match at {
        None => (None, None),
        Some(ClipTarget::Slot { scene }) => {
            check_slot(project, track, scene, "at.scene")?;
            (Some(scene.clone()), None)
        }
        Some(ClipTarget::Arrangement { start }) => {
            check_placements(project, track, 1)?;
            let tempo = project.tempo();
            let ticks = tempo
                .frames_to_ticks(media.frames, media.sample_rate)
                .get()
                .max(1);
            check_span(
                *start,
                Tick(ticks),
                &mut taken_spans(tempo, track),
                "at.start",
            )?;
            (None, Some(*start))
        }
    };
    let snap = AudioClipSnapshot {
        name,
        color: None,
        slot,
        launch_quantize: None,
        frames: media.frames,
        start: Frame(0),
        gain_db: 0.0,
        looping: None,
        media,
    };
    let clip_id = ClipId::generate(&mut ctx.ids);
    let placements: Vec<AudioPlacementSnapshot> = start
        .map(|start| AudioPlacementSnapshot {
            id: PlacementId::generate(&mut ctx.ids),
            start,
            length: snap.frames,
            offset: Frame(0),
        })
        .into_iter()
        .collect();
    insert_audio(state, track_id, &clip_id, &snap, &placements)?;
    let placement = placements.first().map(|p| p.id.clone());
    Ok(edited(
        Output::ImportAudio {
            clip: clip_id.clone(),
            placement: placement.clone(),
        },
        Command::Delete {
            clip: clip_id.clone(),
        },
        vec![ProjectEvent::ClipCreated {
            track: track_id.clone(),
            clip: clip_id,
            placements: placement.into_iter().collect(),
        }],
    ))
}

/// The track of an audio clip, mutably; refuses a MIDI clip and an unknown one.
fn audio_mut<'a>(
    project: &'a mut Project,
    clip_id: &ClipId,
) -> Result<&'a mut Track, CommandError> {
    let track_id = locate(project, clip_id)?;
    let track = project
        .tracks
        .get_mut(&track_id)
        .ok_or_else(|| not_found("track", &track_id))?;
    match track.clips.get(clip_id).map(|c| &c.content) {
        Some(ClipContent::Audio { .. }) => Ok(track),
        Some(ClipContent::Midi { .. }) => Err(invalid(
            "clip",
            "a MIDI clip has no range, gain or loop in frames (`clip.set_length`, `clip.set_loop`)",
        )),
        None => Err(not_found("clip", clip_id)),
    }
}

/// What an audio clip holds, read from its track.
struct AudioParts {
    start: Frame,
    frames: Frame,
    gain_db: f64,
    looping: Option<FrameLoop>,
    file_frames: Frame,
    rate: SampleRate,
}

fn audio_parts(track: &Track, clip_id: &ClipId) -> Result<AudioParts, CommandError> {
    match track.clips.get(clip_id).map(|c| &c.content) {
        Some(ClipContent::Audio {
            media,
            start,
            frames,
            gain_db,
            looping,
        }) => Ok(AudioParts {
            start: *start,
            frames: *frames,
            gain_db: *gain_db,
            looping: *looping,
            file_frames: media.frames,
            rate: media.sample_rate,
        }),
        _ => Err(not_found("clip", clip_id)),
    }
}

/// The audio content of a clip, mutably.
fn audio_content<'a>(
    track: &'a mut Track,
    clip_id: &ClipId,
) -> Result<
    (
        &'a mut Frame,
        &'a mut Frame,
        &'a mut f64,
        &'a mut Option<FrameLoop>,
    ),
    CommandError,
> {
    match track.clips.get_mut(clip_id).map(|c| &mut c.content) {
        Some(ClipContent::Audio {
            start,
            frames,
            gain_db,
            looping,
            ..
        }) => Ok((start, frames, gain_db, looping)),
        _ => Err(not_found("clip", clip_id)),
    }
}

fn audio_set_range(
    state: &mut State,
    clip_id: &ClipId,
    start: Frame,
    frames: Frame,
    explicit: Option<&[PlacementFrames]>,
) -> Result<Out, CommandError> {
    let tempo = state.project.tempo();
    let track = audio_mut(&mut state.project, clip_id)?;
    let old = audio_parts(track, clip_id)?;
    let end = start.get().checked_add(frames.get());
    if frames.get() == 0 || end.is_none_or(|end| end > old.file_frames.get()) {
        return Err(invalid(
            "frames",
            format!(
                "the range [start, start + frames) is inside the file ({} frames) and not empty",
                old.file_frames.get()
            ),
        ));
    }
    if old.looping.is_some_and(|l| l.end > frames) {
        return Err(invalid(
            "frames",
            "the clip's loop ends past that length; clear or move it first (`clip.audio.set_loop`)",
        ));
    }
    if let Some((id, _)) = track.placements.iter().find(|(_, p)| {
        p.clip == *clip_id && matches!(p.span, Span::Frames { offset, .. } if offset >= frames)
    }) {
        return Err(invalid(
            "frames",
            format!(
                "placement {id} has its offset at or past that length; move or resize the \
                 placement first"
            ),
        ));
    }
    let follow = match explicit {
        Some(list) => explicit_frames(tempo, old.rate, track, clip_id, list)?,
        None => following_frames(tempo, old.rate, track, clip_id, old.frames, frames),
    };
    check_audio_placements(tempo, track, clip_id, old.frames, frames, &follow)?;
    let output = Output::AudioSetRange {
        previous_start: old.start,
        previous_frames: old.frames,
    };
    if old.start == start && old.frames == frames && follow.is_empty() {
        return Ok(unchanged(output));
    }
    let inverse = Command::AudioSetRange {
        clip: clip_id.clone(),
        start: old.start,
        frames: old.frames,
        placements: Some(
            follow
                .iter()
                .map(|(id, before, _)| PlacementFrames {
                    placement: id.clone(),
                    length: *before,
                })
                .collect(),
        ),
    };
    for (id, _, new) in &follow {
        if let Some(Span::Frames { length, .. }) = track.placements.get_mut(id).map(|p| &mut p.span)
        {
            *length = *new;
        }
    }
    let (s, f, _, _) = audio_content(track, clip_id)?;
    *s = start;
    *f = frames;
    Ok(edited(output, inverse, changed(clip_id, ClipPart::Length)))
}

/// What `clip.audio.set_range` must never leave behind: an audio placement of the clip
/// ending past the clip (`offset + length > frames`), or one of the placements it changes
/// overlapping another placement of the track. Only what the command newly breaks is
/// refused: a state the project already held (an older file) does not block the command.
fn check_audio_placements(
    tempo: Tempo,
    track: &Track,
    clip_id: &ClipId,
    old_frames: Frame,
    frames: Frame,
    follow: &[(PlacementId, Frame, Frame)],
) -> Result<(), CommandError> {
    let new_length = |id: &PlacementId, length: Frame| {
        follow
            .iter()
            .find(|(f, _, _)| f == id)
            .map_or(length, |(_, _, new)| *new)
    };
    for (id, p) in &track.placements {
        let Span::Frames { length, offset } = p.span else {
            continue;
        };
        if p.clip != *clip_id {
            continue;
        }
        let end = offset.get().saturating_add(new_length(id, length).get());
        if end > frames.get() && offset.get().saturating_add(length.get()) <= old_frames.get() {
            return Err(invalid(
                "frames",
                format!(
                    "placement {id} would end past the clip's {} frames; resize or move the \
                     placement first",
                    frames.get()
                ),
            ));
        }
    }
    let spans = |changed: bool| -> Vec<(&PlacementId, Option<(i64, i64)>)> {
        track
            .placements
            .iter()
            .map(|(id, p)| {
                let mut q = p.clone();
                if let (true, Span::Frames { offset, length }) = (changed, p.span) {
                    q.span = Span::Frames {
                        offset,
                        length: new_length(id, length),
                    };
                }
                (id, placement_ticks(tempo, track, &q))
            })
            .collect()
    };
    let (before, after) = (spans(false), spans(true));
    for (i, (id, new)) in after.iter().enumerate() {
        if !follow.iter().any(|(f, _, _)| f == *id) {
            continue;
        }
        for (j, (other, other_new)) in after.iter().enumerate() {
            let clash = |a: &Option<(i64, i64)>, b: &Option<(i64, i64)>| matches!((a, b), (Some((s, e)), Some((os, oe))) if os < e && s < oe);
            if i == j || !clash(new, other_new) || clash(&before[i].1, &before[j].1) {
                continue;
            }
            return Err(invalid(
                "frames",
                format!(
                    "placement {id} would overlap placement {other}: an audio placement never \
                     carves; move or resize one of them first"
                ),
            ));
        }
    }
    Ok(())
}

/// The placements of an audio clip that play it whole (offset 0, length `old` frames),
/// with their new length: `new`, a growing one stopping at the start of the next placement
/// of the track (the gap converted at `tempo` and the file's `rate`, rounded down) and
/// never ending past 2^40 ticks; one already blocked there is left as it is.
fn following_frames(
    tempo: Tempo,
    rate: SampleRate,
    track: &Track,
    clip_id: &ClipId,
    old: Frame,
    new: Frame,
) -> Vec<(PlacementId, Frame, Frame)> {
    let mut out = Vec::new();
    if old == new {
        return out;
    }
    let mut starts: Vec<i64> = track.placements.values().map(|q| q.start.get()).collect();
    starts.sort_unstable();
    // The most frames whose ticks fit in `ticks` at the tempo (frames_to_ticks rounds to
    // the nearest tick, so a frame count is checked by the same conversion).
    let fit = |ticks: i64| -> u64 {
        let samples = tempo.tick_to_sample(Tick(ticks), rate);
        if !(samples.is_finite() && samples > 0.0) {
            return 0;
        }
        let mut n = samples.floor().min(MAX_FRAME as f64) as u64;
        while n > 0 && tempo.frames_to_ticks(Frame(n), rate).get() > ticks {
            n -= 1;
        }
        n
    };
    for (id, p) in &track.placements {
        let Span::Frames { length, offset } = p.span else {
            continue;
        };
        if p.clip != *clip_id || offset.get() != 0 || length != old {
            continue;
        }
        let start = p.start.get();
        let mut target = new.get().min(fit(MAX_TICK.saturating_sub(start)));
        if new > old {
            let after = starts.partition_point(|s| *s <= start);
            if let Some(next) = starts.get(after) {
                target = target.min(fit(next - start));
            }
            if target <= length.get() {
                // Already blocked (or overlapping) there: it does not shrink on a grow.
                continue;
            }
        }
        if target >= 1 && target != length.get() {
            out.push((id.clone(), length, Frame(target)));
        }
    }
    out
}

/// The exact lengths of `list`, for undo: placements of the clip, each listed once, 1 to
/// 2^40 frames, ending by tick 2^40 at the project's tempo (what `validate` asks). No
/// overlap check: they are lengths a state of the project already held.
fn explicit_frames(
    tempo: Tempo,
    rate: SampleRate,
    track: &Track,
    clip_id: &ClipId,
    list: &[PlacementFrames],
) -> Result<Vec<(PlacementId, Frame, Frame)>, CommandError> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in list {
        if !seen.insert(&item.placement) {
            return Err(invalid("placements", "a placement is listed twice"));
        }
        let p = track
            .placements
            .get(&item.placement)
            .filter(|p| p.clip == *clip_id)
            .ok_or_else(|| not_found("placement", &item.placement))?;
        let Span::Frames { length, .. } = p.span else {
            return Err(invalid("placements", "a MIDI placement has no frames"));
        };
        let end = p
            .start
            .get()
            .saturating_add(tempo.frames_to_ticks(item.length, rate).get());
        if item.length.get() == 0 || item.length.get() > MAX_FRAME || end > MAX_TICK {
            return Err(invalid(
                "placements",
                format!("a placement plays 1 to 2^40 frames and ends by tick {MAX_TICK}"),
            ));
        }
        if length != item.length {
            out.push((item.placement.clone(), length, item.length));
        }
    }
    Ok(out)
}

fn audio_set_gain(
    state: &mut State,
    clip_id: &ClipId,
    gain: Option<f64>,
) -> Result<Out, CommandError> {
    let db = crate::mixer::to_db(gain)?;
    let track = audio_mut(&mut state.project, clip_id)?;
    let previous = audio_parts(track, clip_id)?.gain_db;
    let before = crate::mixer::from_db(previous);
    let output = Output::AudioSetGain { previous: before };
    if previous == db {
        return Ok(unchanged(output));
    }
    if !(previous == f64::NEG_INFINITY
        || (project::MIN_GAIN_DB..=project::MAX_GAIN_DB).contains(&previous))
    {
        // A file may hold such a gain (validation only asks for a number): this command
        // could not set it back on undo, so it does not change it.
        return Err(invalid(
            "clip",
            format!(
                "the clip's gain is {previous} dB, outside -96 to +6 dB: its undo could not                  set it back"
            ),
        ));
    }
    *audio_content(track, clip_id)?.2 = db;
    Ok(edited(
        output,
        Command::AudioSetGain {
            clip: clip_id.clone(),
            gain_db: before,
        },
        changed(clip_id, ClipPart::Gain),
    ))
}

fn audio_set_loop(
    state: &mut State,
    clip_id: &ClipId,
    new: Option<FrameLoop>,
) -> Result<Out, CommandError> {
    let track = audio_mut(&mut state.project, clip_id)?;
    let parts = audio_parts(track, clip_id)?;
    if let Some(l) = new
        && !(l.start < l.end && l.end <= parts.frames)
    {
        return Err(invalid(
            "looping",
            format!(
                "0 <= start < end <= {} (the clip's length in frames)",
                parts.frames.get()
            ),
        ));
    }
    let previous = parts.looping;
    let output = Output::AudioSetLoop { previous };
    if previous == new {
        return Ok(unchanged(output));
    }
    *audio_content(track, clip_id)?.3 = new;
    Ok(edited(
        output,
        Command::AudioSetLoop {
            clip: clip_id.clone(),
            looping: previous,
        },
        changed(clip_id, ClipPart::Loop),
    ))
}

/// Puts an audio clip and its placements into a track (checked before).
fn insert_audio(
    state: &mut State,
    track_id: &TrackId,
    clip_id: &ClipId,
    snap: &AudioClipSnapshot,
    placements: &[AudioPlacementSnapshot],
) -> Result<(), CommandError> {
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
            content: ClipContent::Audio {
                media: snap.media.clone(),
                start: snap.start,
                frames: snap.frames,
                gain_db: snap.gain_db,
                looping: snap.looping,
            },
        },
    );
    for p in placements {
        track.placements.insert(
            p.id.clone(),
            Placement {
                clip: clip_id.clone(),
                start: p.start,
                span: Span::Frames {
                    length: p.length,
                    offset: p.offset,
                },
            },
        );
    }
    Ok(())
}

fn delete_audio(
    state: &mut State,
    track_id: &TrackId,
    clip_id: &ClipId,
) -> Result<Out, CommandError> {
    let track = state
        .project
        .tracks
        .get_mut(track_id)
        .ok_or_else(|| not_found("track", track_id))?;
    let clip = track
        .clips
        .get(clip_id)
        .ok_or_else(|| not_found("clip", clip_id))?;
    let ClipContent::Audio {
        media,
        start,
        frames,
        gain_db,
        looping,
    } = &clip.content
    else {
        return Err(invalid("clip", "not an audio clip"));
    };
    let snap = AudioClipSnapshot {
        name: clip.name.clone(),
        color: clip.color,
        slot: clip.slot.clone(),
        launch_quantize: clip.launch_quantize,
        media: media.clone(),
        start: *start,
        frames: *frames,
        gain_db: *gain_db,
        looping: *looping,
    };
    let mut placements = Vec::new();
    for (id, p) in &track.placements {
        if p.clip != *clip_id {
            continue;
        }
        let Span::Frames { length, offset } = p.span else {
            return Err(invalid("clip", "an audio clip has a placement in ticks"));
        };
        placements.push(AudioPlacementSnapshot {
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
        Command::RestoreAudio {
            track: track_id.clone(),
            clip: clip_id.clone(),
            snapshot: snap,
            placements,
        },
        vec![ProjectEvent::ClipDeleted {
            track: track_id.clone(),
            clip: clip_id.clone(),
            placements: ids,
        }],
    ))
}

fn restore_audio(
    state: &mut State,
    track_id: &TrackId,
    clip_id: &ClipId,
    snap: &AudioClipSnapshot,
    placements: &[AudioPlacementSnapshot],
) -> Result<Out, CommandError> {
    let project = &state.project;
    let track = audio_track(project, track_id)?;
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
    check_audio(snap, "snapshot")?;
    check_clips(project, track)?;
    check_placements(project, track, placements.len())?;
    if let Some(scene) = &snap.slot {
        check_slot(project, track, scene, "snapshot.slot")?;
    }
    let tempo = project.tempo();
    for (i, p) in placements.iter().enumerate() {
        let field = format!("placements[{i}]");
        let end = p
            .start
            .saturating_add(tempo.frames_to_ticks(p.length, snap.media.sample_rate));
        if !(0..=MAX_TICK).contains(&p.start.get())
            || end.get() > MAX_TICK
            || p.length.get() == 0
            || p.length.get() > MAX_FRAME
        {
            return Err(invalid(field, "a placement lies on the timeline"));
        }
        if p.offset >= snap.frames {
            return Err(invalid(field, "a placement's offset is inside the clip"));
        }
        if p.offset.saturating_add(p.length) > snap.frames {
            return Err(invalid(
                field,
                "a placement ends inside the clip: `offset + length` is at most the clip's frames",
            ));
        }
    }
    // An audio placement never overlaps another one of the track, nor one of the others
    // restored with it (the same rule as `arrangement.restore`).
    let mut spans = taken_spans(tempo, track);
    for (i, p) in placements.iter().enumerate() {
        let start = p.start.get();
        let end = start.saturating_add(
            tempo
                .frames_to_ticks(p.length, snap.media.sample_rate)
                .get()
                .max(1),
        );
        if spans.iter().any(|(s, e)| *s < end && start < *e) {
            return Err(invalid(
                format!("placements[{i}]"),
                "the placement would overlap another placement of the track: an audio \
                 placement never carves",
            ));
        }
        spans.push((start, end));
    }
    insert_audio(state, track_id, clip_id, snap, placements)?;
    Ok(edited(
        Output::RestoreAudio {},
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
    let from = project
        .tracks
        .get(&from_id)
        .ok_or_else(|| not_found("track", &from_id))?;
    let source = from
        .clips
        .get(clip_id)
        .ok_or_else(|| not_found("clip", clip_id))?;
    // An audio clip is copied into an audio track (the same file, range, gain and loop),
    // a MIDI clip into a MIDI track (AU-6).
    let track = match &source.content {
        ClipContent::Midi { notes, .. } => {
            let track = midi_track(project, &track_id)?;
            check_note_limits(
                notes.len(),
                project_notes(project).saturating_add(notes.len()),
            )?;
            track
        }
        ClipContent::Audio { .. } => audio_track(project, &track_id)?,
    };
    check_clips(project, track)?;
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
    let from = project
        .tracks
        .get(&from_id)
        .ok_or_else(|| not_found("track", &from_id))?;
    let clip = from
        .clips
        .get(clip_id)
        .ok_or_else(|| not_found("clip", clip_id))?;
    // A MIDI clip goes to a MIDI track, an audio clip to an audio track.
    let target = if clip.content.is_midi() {
        midi_track(project, &to_id)?
    } else {
        audio_track(project, &to_id)?
    };
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
    // Out of the grid, a clip with no placement stays in its track's pool (#473).
    if let Some(scene) = slot {
        check_slot(project, target, scene, "slot")?;
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

/// The period of `clip.notes.repeat` (#770) for a figure from `first` to `last` (ticks): the
/// extent from the `grid` line at or before `first` to `last`, rounded up to whole grid steps
/// when it is one `beat` or less, to whole beats up to half a `bar`, to whole bars beyond (never
/// a unit finer than the grid). Without a grid, the exact span. At least 1, and at least
/// `last - first`, so every note of the figure starts within one period of `first`.
fn repeat_period(first: i64, last: i64, grid: Option<i64>, beat: i64, bar: i64) -> i64 {
    let Some(step) = grid else {
        return last.saturating_sub(first).max(1);
    };
    let step = step.max(1);
    let origin = first.div_euclid(step).saturating_mul(step);
    let extent = last.saturating_sub(origin).max(1);
    let unit = if extent <= beat {
        step
    } else if extent.saturating_mul(2) <= bar {
        beat.max(step)
    } else {
        bar.max(step)
    };
    extent
        .saturating_add(unit - 1)
        .div_euclid(unit)
        .saturating_mul(unit)
        .max(1)
}

/// `clip.notes.repeat` (#710): copies of the notes end to end, one period apart, up to
/// `until` and never past the end of the clip.
fn notes_repeat(
    state: &mut State,
    clip_id: &ClipId,
    chosen: &[Note],
    until: Tick,
    grid: Option<Tick>,
) -> Result<Out, CommandError> {
    let in_project = project_notes(&state.project);
    let signature = state.project.time_signature();
    let clip = clip_mut(&mut state.project, clip_id)?;
    let (length, _, notes) = midi_mut(clip)?;
    let step = grid.map_or(1, Tick::get);
    if step < 1 {
        return Err(invalid(
            "grid",
            format!("a grid step of at least 1 tick is required, got {step}"),
        ));
    }
    let sources = present_notes(notes, chosen, "notes")?;
    let (Some(first), Some(last)) = (
        sources.iter().map(|n| n.start.get()).min(),
        sources
            .iter()
            .map(|n| n.start.get().saturating_add(n.length.get()))
            .max(),
    ) else {
        return Ok(unchanged(Output::NotesRepeat {
            added: 0,
            at_clip_end: false,
        }));
    };
    // Every source starts in `[first, first + period)`, so the copies of one round never
    // meet those of another.
    let period = repeat_period(
        first,
        last,
        grid.map(|_| step),
        signature.ticks_per_beat().get(),
        signature.ticks_per_bar().get(),
    );
    let first_copy = first.saturating_add(period);
    let until = until.get();
    if until <= first_copy {
        return Err(invalid(
            "until",
            format!(
                "repeat up to a tick after the start of the first copy ({first_copy}), got {until}"
            ),
        ));
    }
    let clip_end = length.get();
    if first_copy >= clip_end {
        return Err(invalid(
            "notes",
            format!(
                "no room for a copy: the first one would start at {first_copy}, at or after \
                 the end of the clip ({clip_end})"
            ),
        ));
    }
    let end = until.min(clip_end);
    // The rounds whose start lies before `end`: k = 1 ..= rounds.
    let rounds = end
        .saturating_sub(first)
        .saturating_add(period - 1)
        .div_euclid(period)
        .saturating_sub(1)
        .max(1);
    // A bound before any allocation: rounds of `sources.len()` notes each, of which at
    // most the notes already in the clip and the last round's shortened notes can merge.
    let most = usize::try_from(rounds)
        .unwrap_or(usize::MAX)
        .saturating_mul(sources.len());
    if most > MAX_NOTES_PER_CLIP.saturating_add(notes.len().saturating_add(sources.len())) {
        return Err(CommandError::Limit {
            entity: "notes per clip".into(),
            max: MAX_NOTES_PER_CLIP as u64,
        });
    }
    let mut copies = BTreeSet::new();
    for k in 1..=rounds {
        let shift = k.saturating_mul(period);
        for n in &sources {
            let start = n.start.get().saturating_add(shift);
            if start >= end {
                continue;
            }
            let length = n.length.get().min(end - start);
            copies.insert(Note {
                start: Tick(start),
                length: Tick(length),
                ..*n
            });
        }
    }
    let fresh: Vec<Note> = copies.into_iter().filter(|n| !notes.contains(n)).collect();
    // The last guard of `clip.notes.add`: every copy inside the clip, at least one tick long.
    check_notes(&fresh, *length, "notes")?;
    // The clip's end, not `until`, stopped a copy: one is shortened there, or the next copy of a
    // note would start there or later but before `until`.
    let at_clip_end = until > clip_end
        && sources.iter().any(|n| {
            let start = n.start.get();
            let from_end = clip_end.saturating_sub(start).max(0);
            let k = from_end
                .saturating_add(period - 1)
                .div_euclid(period)
                .max(1);
            let next = start.saturating_add(k.saturating_mul(period));
            let before = next.saturating_sub(period);
            (before > start && before.saturating_add(n.length.get()) > clip_end)
                || (next >= clip_end && next < until)
        });
    if fresh.is_empty() {
        return Ok(unchanged(Output::NotesRepeat {
            added: 0,
            at_clip_end,
        }));
    }
    check_note_limits(
        notes.len().saturating_add(fresh.len()),
        in_project.saturating_add(fresh.len()),
    )?;
    notes.extend(fresh.iter().copied());
    Ok(edited(
        Output::NotesRepeat {
            added: count(fresh.len()),
            at_clip_end,
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
    fn the_repeat_period_follows_the_extent_of_the_figure() {
        let (beat, bar) = (960, 3840);
        // One beat or less: whole grid steps, measured from the grid line before the start.
        assert_eq!(repeat_period(0, 960, Some(240), beat, bar), 960);
        assert_eq!(repeat_period(100, 340, Some(240), beat, bar), 480);
        // More than a beat up to half a bar: whole beats; beyond: whole bars.
        assert_eq!(repeat_period(0, 961, Some(240), beat, bar), 1920);
        assert_eq!(repeat_period(0, 1920, Some(240), beat, bar), 1920);
        assert_eq!(repeat_period(0, 1921, Some(240), beat, bar), 3840);
        assert_eq!(repeat_period(0, 3841, Some(240), beat, bar), 7680);
        // Without a grid, the exact span; never below a tick; no overflow at the ends of the range.
        assert_eq!(repeat_period(10, 1210, None, beat, bar), 1200);
        assert_eq!(repeat_period(5, 5, Some(240), beat, bar), 240);
        assert_eq!(repeat_period(5, 5, None, beat, bar), 1);
        assert!(repeat_period(i64::MIN, i64::MAX, Some(240), beat, bar) > 0);
        assert!(repeat_period(i64::MAX - 1, i64::MAX, Some(i64::MAX), beat, bar) > 0);
    }

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
