//! `arrangement.*`: placements in a track's timeline: place, move, resize, split, delete,
//! duplicate (independent copy by default, linked as an option) and the restore that
//! undoes them all (epic 7, AR; AR-1).
//!
//! The contract of the arrangement commands, written once here and repeated in their
//! descriptions for agents:
//!
//! - A placement plays a MIDI clip of its track from the clip's tick `offset`, for
//!   `length` ticks, from the timeline tick `start` (all in ticks, 960 per quarter note,
//!   inside `[0, 2^40]`). Two placements of the same clip are linked: editing the clip
//!   changes both. A placement has no loop of its own: when it is longer than its clip, the
//!   clip repeats inside it, through the clip's loop when it has one, the whole clip when
//!   it has none (#473).
//! - **An audio placement (AU-6)** plays `length` frames of an audio clip of its track from
//!   the clip's frame `offset`, from the timeline tick `start`; its end is `start` plus those
//!   frames at the project's tempo (at least one tick). It stays inside its clip (`offset +
//!   length` at most the clip's frames), looped or not: a looped clip's placement plays the
//!   loop for that length. Every command below takes audio placements under the same rules,
//!   cut in whole frames: a cut at tick `t` keeps the frames that play before `t`
//!   (`floor(t × frames per tick)`, the player's rule) on the left, the rest from there on the
//!   right (trimmed at the front so it never ends past where it did, through the clip's
//!   loop), so a cut never makes an overlap or plays a frame twice; a part where not even one
//!   frame fits goes. An edge resized past the clip stops at its first or last frame. A clip
//!   only moves to a track of its kind (MIDI to MIDI, audio to audio).
//! - **The overlap rule: the last one put down wins.** The placement a command places,
//!   moves, resizes or copies keeps its span; the other placements of the track that it
//!   covers are carved, so a track never holds two placements at the same tick after one
//!   of these commands:
//!   - a placement fully under it is removed;
//!   - one that starts before it is shortened to end where it starts;
//!   - one that starts under it and ends after it is trimmed at the front: it starts
//!     where the winner ends, and its offset moves on by what was cut, followed through
//!     the clip's own loop, or round the whole clip when it has none;
//!   - one that contains it is split in two: the left part keeps the identifier, the
//!     right part is a new placement.
//!
//!   The engine still cuts any table at the next start (AR-2), so a project that holds an
//!   overlap from elsewhere plays as "the last started wins". `clip.create` and
//!   `clip.import_audio` into the arrangement refuse an overlap, and `clip.set_length`
//!   stops a growing placement at the next one: a note edit never carves.
//! - Moving, deleting and copying take a list, so a selection is one command, one
//!   history entry, one undo. A group is lifted first and put down in the order of its
//!   list: a placement may move onto the old place of another one of the group.
//! - **One pool of clips per track (#473).** A clip stays in its track when it loses its
//!   last placement, even with no launch-grid slot: the interface lists it (the Clips
//!   panel), places it again or deletes it (`clip.delete`). `arrangement.place` places a
//!   clip of the arrangement or of the pool as a linked placement of that clip (an audio
//!   clip is placed whole); it refuses
//!   a clip of the grid: copying or linking it is `arrangement.place_from_slot` (AR-5,
//!   founder's decision 11).
//! - **Inserting and deleting time (#396).** `arrangement.insert_time` and
//!   `arrangement.delete_time` shift the whole timeline of the project, every track and the
//!   master together, in one history entry:
//!   - a placement that starts at or after the cut moves; one that straddles the insertion
//!     point is split there (the right part is a new placement of the same clip, it resumes
//!     the clip where the left part stops); a deletion cuts the placements at both edges and
//!     removes what lies between them (a clip of the arrangement only that loses its last
//!     placement goes with it, as for `arrangement.delete`);
//!   - the points of every automation lane (the tracks' and the master's) follow: a point at
//!     or after the insertion point moves; a deletion removes the points inside the deleted
//!     range and moves the later ones back. A lane stays continuous: the last point before
//!     the cut links straight to the first one after it (a ramp that straddles an insertion
//!     stretches over the inserted time);
//!   - the loop region follows: a boundary at or after the cut moves (an end exactly at the
//!     insertion point stays), a boundary inside a deleted range falls to its start; a loop
//!     that is left empty is cleared and switched off;
//!   - the tempo and the time-signature maps hold one point at tick 0 (the project format
//!     refuses others), so there is nothing of them to move; the playhead and the launch grid
//!     are not on the timeline and stay.
//!
//!   Audio placements move, split and are cut in frames like the others (AU-6). The
//!   playhead stays at its tick.
//!   Everything is checked first (a placement or a point that would pass 2^40, a limit): a
//!   refusal changes nothing. The inverse is `arrangement.restore_time`.
//! - Every command is atomic: it is checked entirely before anything changes. A command
//!   that would change nothing changes nothing and leaves no history entry.
//! - The inverse of every command is `arrangement.restore`, computed from what actually
//!   changed (the placements, and the clips that were created or removed), so an undo
//!   gives back the carved neighbours exactly, with their identifiers.

use crate::automation::{self, Point};
use crate::clip::{
    AudioClipSnapshot, ClipSnapshot, check_loop, check_name, check_note_limits, check_slot,
    check_total, invalid, locate, midi_ref, midi_track, not_found, project_notes, snapshot,
};
use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use project::limits::{
    MAX_CLIPS_PER_TRACK, MAX_PLACEMENTS, MAX_PLACEMENTS_PER_TRACK, MAX_POINTS, MAX_TICK,
};
use project::{
    AutomationPoint, Clip, ClipContent, ClipId, IdGenerator, LaneId, LoopRegion, MarkerId, Note,
    Placement, PlacementId, Project, SampleRate, SceneId, Span, Tempo, Tick, TickLoop, Track,
    TrackId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// The edge of a placement that `arrangement.resize` moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    /// The left edge: the end stays where it is, the placement shows the clip from
    /// further on (or earlier).
    Start,
    /// The right edge: the start and the offset stay.
    End,
}

/// A placement of a clip as `arrangement.restore` writes it, with its identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacementRecord {
    pub id: PlacementId,
    /// A clip of the track: a MIDI clip in a MIDI track, an audio clip in an audio track.
    pub clip: ClipId,
    /// Start in the arrangement, in ticks.
    pub start: Tick,
    /// Length on the timeline, in ticks (a MIDI placement; 0 and ignored when `frames` is
    /// given).
    #[serde(default)]
    pub length: Tick,
    /// Where in the clip the placement starts, in ticks, before the clip's length (a MIDI
    /// placement; 0 and ignored when `frames` is given).
    #[serde(default)]
    pub offset: Tick,
    /// An audio placement's span, in frames of its clip's file: given for a placement of
    /// an audio track and only for those.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<FrameSpan>,
}

/// The span of an audio placement, in frames of the clip's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameSpan {
    /// How many frames of the clip the placement plays, at least 1.
    pub length: u64,
    /// The frame of the clip where it starts, before the clip's frame count.
    pub offset: u64,
}

/// One placement and the MIDI track it moves to (`arrangement.move_to_track`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackMove {
    pub placement: PlacementId,
    /// The track the placement ends in, of its kind (MIDI or audio); its own track leaves it where it is.
    pub track: TrackId,
}

/// A MIDI clip that `arrangement.restore` brings back, with its identifier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipRecord {
    pub id: ClipId,
    pub snapshot: ClipSnapshot,
}

/// What `arrangement.restore` changes in one track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackEdit {
    pub track: TrackId,
    /// Placements to write exactly as given: created, or replaced when the identifier
    /// is already a placement of the track.
    pub put: Vec<PlacementRecord>,
    /// Placements of the track to remove.
    pub remove: Vec<PlacementId>,
    /// MIDI clips to bring back (their identifiers must be unused); a MIDI track only.
    pub clips: Vec<ClipRecord>,
    /// Audio clips to bring back (their identifiers must be unused), as
    /// `clip.restore_audio` takes them; an audio track only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_clips: Vec<AudioClipRecord>,
    /// Clips of the track to remove (MIDI or audio); no placement may be left on them.
    pub remove_clips: Vec<ClipId>,
}

/// An audio clip that `arrangement.restore` brings back, with its identifier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioClipRecord {
    pub id: ClipId,
    pub snapshot: AudioClipSnapshot,
}

/// The points of one automation lane, written whole by `arrangement.restore_time`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LanePoints {
    /// A lane of a track or of the master.
    pub lane: LaneId,
    /// All the points the lane holds afterwards, at most one per tick.
    pub points: Vec<Point>,
}

/// The loop settings as `arrangement.restore_time` writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopState {
    /// The loop region, `null` for none.
    pub loop_region: Option<TickLoop>,
    /// Whether the transport loops; only with a region.
    pub loop_enabled: bool,
}

domain! {
    domain = "arrangement", schema = ("ArrangementCommand", "ArrangementOutput");

    /// Put a clip of the track in the arrangement, as a new placement of the same clip
    /// (a linked clip: editing the clip changes every placement of it); a clip of the
    /// track's pool, with no placement and no slot, comes back this way (#473). A clip that
    /// sits in the launch grid is refused: copying or linking it is
    /// `arrangement.place_from_slot` (founder's decision 11). `start` is in
    /// ticks; `offset` (default 0, before the clip's length) is where in the clip the
    /// placement starts; `length` (default: what is left of the clip after `offset`) is
    /// its length on the timeline, which may exceed the clip (the clip repeats: its loop
    /// when it has one, the whole clip when it has none). The new placement wins on overlap: the
    /// other placements of the track that it covers are removed, shortened, trimmed at
    /// the front or split (`overwritten` counts them). An audio clip of an audio track is
    /// placed whole (from its first frame, its end computed from its frames at the project's
    /// tempo, so `length` and `offset` are refused; `arrangement.resize` shortens it) and
    /// wins on overlap the same way, the placements it covers cut in whole frames; a span
    /// outside `[0, 2^40]` ticks is refused. Undo: `arrangement.restore`.
    "arrangement.place" (kind = "edit", undo = "inverse", destructive = false)
    Place {
        clip: ClipId,
        start: Tick,
        length: Option<Tick>,
        offset: Option<Tick>,
    } => {
        /// The new placement.
        placement: PlacementId,
        /// How many other placements were carved.
        overwritten: u32,
    };

    /// Move placements in time, all by the same number of ticks (`by`, negative moves
    /// them earlier), in one command: a drag of a selection is one history entry. Each
    /// placement stays in its own track and keeps its length and offset (an audio one its
    /// frames, so its length in ticks at the project's tempo). The moved
    /// placements win on overlap (the placements of the track that they cover are
    /// carved, `overwritten` counts them); they may move onto each other's old place.
    /// Refused when one would start before tick 0 or end past 2^40, or when the list is
    /// empty, names a placement twice or an unknown one. A move by 0 changes nothing.
    /// Undo: `arrangement.restore`.
    "arrangement.move" (kind = "edit", undo = "inverse", destructive = false)
    Move {
        placements: Vec<PlacementId>,
        by: Tick,
    } => {
        /// How many other placements were carved.
        overwritten: u32,
    };

    /// Move placements to other tracks of their kind (a MIDI placement to a MIDI track, an
    /// audio one to an audio track), in one command and one history entry, and
    /// optionally in time too (`by` ticks, default 0, negative is earlier). `moves` pairs
    /// each placement with the track it ends in (its own track keeps it there): a selection
    /// of several tracks can move one track down together, each clip to its own target.
    /// The clip of a moved placement goes with it: the target track gets a copy of the
    /// clip (one copy per source clip and target track, so placements that were linked and
    /// move together stay linked; same name, colour, length, loop and notes, or for an
    /// audio clip the same file, range, gain and loop; not in the
    /// launch grid), and a source clip left with no placement and no grid slot is removed
    /// from its track (a clip in the grid, or one still placed elsewhere, stays). The
    /// moved placements get new identifiers in another track (`placements`, in the order of
    /// `moves`) and keep their length and offset; they win on overlap in the target (the
    /// placements they cover are carved, `overwritten` counts them). Refused when `moves`
    /// is empty, names a placement twice or an unknown one, names an unknown track or one
    /// of the other kind, when one would start before tick 0 or end past 2^40, or on a
    /// limit.
    /// Nothing moves (and no history entry is made) when every placement stays in its
    /// track and `by` is 0. Undo: `arrangement.restore`, which puts the placements and the
    /// removed clips back with their identifiers and removes the copies.
    "arrangement.move_to_track" (kind = "edit", undo = "inverse", destructive = false)
    MoveToTrack {
        moves: Vec<TrackMove>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Tick>,
    } => {
        /// The placements after the move, in the order of `moves`.
        placements: Vec<PlacementId>,
        /// How many other placements were carved.
        overwritten: u32,
    };

    /// Move one edge of a placement to the timeline tick `to`. `edge: "end"` changes the
    /// length (the end must stay after the start); `edge: "start"` moves the start with
    /// the end fixed, and the offset follows so the content does not slide: shortening
    /// from the left cuts the beginning of the clip (through the clip's loop), growing to
    /// the left shows earlier content and is refused beyond the clip's start (offset 0).
    /// An audio placement is resized in whole frames and stays inside its clip: its end
    /// stops at the clip's last frame and its start at the clip's first (offset 0) instead
    /// of being refused; its end is kept when its start moves (at most a tick earlier, the
    /// frames being whole). The resized placement wins on overlap (the others it now covers are carved,
    /// `overwritten` counts them). Moving the edge where it is changes nothing. Undo:
    /// `arrangement.restore`.
    "arrangement.resize" (kind = "edit", undo = "inverse", destructive = false)
    Resize {
        placement: PlacementId,
        edge: Edge,
        to: Tick,
    } => {
        /// How many other placements were carved.
        overwritten: u32,
    };

    /// Split a placement in two at the timeline tick `at`, strictly inside it: the left
    /// part keeps the identifier, the right part is a new placement (the same clip, so
    /// linked) that plays on from where the left one stops (round the clip's loop, or round
    /// the whole clip when it has none). An audio placement is split in whole frames (the
    /// frames that play before `at` on the left); refused when a part would hold not even
    /// one frame. Undo: `arrangement.restore`.
    "arrangement.split" (kind = "edit", undo = "inverse", destructive = false)
    Split { placement: PlacementId, at: Tick } => {
        /// The right part.
        placement: PlacementId,
    };

    /// Delete placements, in one command, MIDI or audio ones. Their clips stay in their
    /// tracks, even one that loses its last placement and sits in no grid slot: it stays in
    /// the track's pool (an audio clip keeps its file; `arrangement.place` puts a clip back,
    /// MIDI or audio; `clip.delete` deletes a clip). Refused when the
    /// list is empty, names a placement twice or an unknown one. Undo:
    /// `arrangement.restore`.
    "arrangement.delete" (kind = "edit", undo = "inverse", destructive = false)
    Delete { placements: Vec<PlacementId> } => {
        /// How many placements were deleted.
        deleted: u32,
    };

    /// Copy placements, in one command. The copies start `by` ticks after the originals
    /// (default: right after the selection, one selection length later; negative copies
    /// earlier). By default each copy is independent: a new clip, a copy of the source
    /// (same name, colour, length, loop and notes, or for an audio clip the same file,
    /// range, gain and loop: the file is not copied; not in the launch grid), one per
    /// distinct source clip, so placements that were linked stay linked among the copies
    /// and editing a copy never changes the original (founder's decision 11). With
    /// `linked: true` the copies are new placements of the same clips. The copies win
    /// on overlap (`overwritten` counts the placements they carved). Refused when the
    /// list is empty, names a placement twice or an unknown one, or when a copy would
    /// leave the timeline. Undo: `arrangement.restore`, which removes the copies and
    /// the clips created.
    "arrangement.duplicate" (kind = "edit", undo = "inverse", destructive = false)
    Duplicate {
        placements: Vec<PlacementId>,
        #[serde(default)]
        linked: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Tick>,
    } => {
        /// The new placements, in the order of the list.
        placements: Vec<PlacementId>,
        /// The clips created (none when `linked`).
        clips: Vec<ClipId>,
        /// How many other placements were carved.
        overwritten: u32,
    };

    /// Repeat one placement end to end up to the timeline tick `until` (#555): linked
    /// copies, new placements of the same clip, each as long as the placement and with
    /// its offset, the first starting where the placement ends, the next where the last
    /// ends; the one that would pass `until` is shortened to stop exactly there (an audio
    /// one at the last whole frame before it). To fill
    /// up to the end of bar N, give the tick where bar N ends. The copies win on overlap
    /// (`overwritten` counts the placements they carved). Refused when the placement is
    /// unknown, when `until` is not after the end of the placement or
    /// passes 2^40, or when the copies would pass the limit of placements. One history
    /// entry; undo: `arrangement.restore`, which removes the copies and brings back what
    /// they carved.
    "arrangement.repeat" (kind = "edit", undo = "inverse", destructive = false)
    Repeat {
        placement: PlacementId,
        until: Tick,
    } => {
        /// The new placements, in time order.
        placements: Vec<PlacementId>,
        /// How many other placements were carved.
        overwritten: u32,
    };

    /// Put a clip of the launch grid in the arrangement, on its own track, as a
    /// placement at the timeline tick `start` that plays the whole clip once (offset 0,
    /// the clip's length; an audio clip's frames from its first). By default the placement plays an independent copy: a new
    /// clip, a copy of the grid clip (same name, colour, length, loop and notes, or the same
    /// file, range, gain and loop; not in
    /// the launch grid), so editing one never changes the other (founder's decision 11).
    /// With `linked: true` the placement references the grid clip itself: editing the
    /// clip in the grid changes what the arrangement plays. The placement wins on overlap
    /// (`overwritten` counts the placements it carved). Refused for a clip that is not in
    /// the launch grid (use `arrangement.place` or `arrangement.duplicate`), or when the
    /// clip would leave the timeline. Undo: `arrangement.restore`, which removes the
    /// placement and the copy.
    "arrangement.place_from_slot" (kind = "edit", undo = "inverse", destructive = false)
    PlaceFromSlot {
        clip: ClipId,
        start: Tick,
        #[serde(default)]
        linked: bool,
    } => {
        /// The new placement.
        placement: PlacementId,
        /// The independent copy created (none when `linked`).
        clip: Option<ClipId>,
        /// How many other placements were carved.
        overwritten: u32,
    };

    /// `arrangement.place_from_slot` for a whole scene of the launch grid, in one command
    /// and one history entry: every clip of the scene (MIDI and audio) is placed on its own
    /// track at the same tick `start`, as a copy (default) or linked to the grid clip. Empty
    /// slots place nothing. Refused when the scene has no clip.
    /// The placements win on overlap. Undo: `arrangement.restore`.
    "arrangement.place_scene" (kind = "edit", undo = "inverse", destructive = false)
    PlaceScene {
        scene: SceneId,
        start: Tick,
        #[serde(default)]
        linked: bool,
    } => {
        /// The new placements, in the order of the tracks.
        placements: Vec<PlacementId>,
        /// The independent copies created (none when `linked`).
        clips: Vec<ClipId>,
        /// How many other placements were carved.
        overwritten: u32,
    };

    /// Insert `length` ticks of empty time at the timeline tick `at`, in every track and in
    /// the master, in one history entry (bar-level editing: a bar is 3840 ticks in 4/4).
    /// A placement that starts at or after `at` moves `length` ticks later; one that
    /// straddles `at` is split there, the right part (a new placement of the same clip, so
    /// linked) moving later and resuming the clip where the left part stops, round the clip.
    /// The points of every automation lane at or after `at` move too (a ramp that straddles
    /// `at` stretches), so do the section markers at or after `at` (#473), and so do the
    /// loop region's boundaries after `at` (a loop that contains `at` grows). Tempo and time
    /// signature hold one point at tick 0 and do not move; the playhead and the launch grid
    /// do not move. Refused with `invalid` when `at` is not in [0, 2^40], `length` is less
    /// than 1 or the cut would pass 2^40, or when a placement or a point would pass 2^40,
    /// and with `limit` when the splits pass the placement limits. An audio placement moves
    /// and splits like the others, in whole frames (a part where not even one frame fits
    /// goes). Changes nothing
    /// (no history entry) when nothing lies at or after `at`. Undo: `arrangement.restore_time`.
    "arrangement.insert_time" (kind = "edit", undo = "inverse", destructive = false)
    InsertTime { at: Tick, length: Tick } => {
        /// How many placements were split at `at`.
        split: u32,
        /// How many placements moved (the right parts of the splits included).
        shifted: u32,
        /// How many automation points moved.
        points_shifted: u32,
        /// Whether the loop region changed.
        loop_changed: bool,
    };

    /// Delete the time `[from, to)` ticks from every track and from the master,
    /// and close the gap, in one history entry. Placements are cut at both edges: what lies
    /// inside is removed (a placement that straddles the whole range is split in two, the
    /// right part a new placement resuming the clip where the range ends), what starts at or
    /// after `to` moves `to - from` ticks earlier; a clip that loses its last placement stays
    /// in its track's pool. The points of every automation lane inside the range are removed and the
    /// later ones move back, so the last point before the cut links straight to the first
    /// one after it (a `step` point keeps holding its value up to the cut, then the next
    /// value applies: the lane is not smoothed). The section markers after the range move
    /// back, one inside it goes to `from` (#473). The loop region follows (a boundary inside
    /// the range falls to `from`;
    /// a loop left empty is cleared and switched off). Tempo and time signature hold one
    /// point at tick 0 and do not move; the playhead and the launch grid do not move.
    /// Refused with `invalid` when `from` is not in [0, 2^40], `to` is not after `from` or
    /// passes 2^40, and with `limit` when the cuts pass the placement limits. An audio
    /// placement is cut and moves like the others, in whole frames. Changes nothing (no
    /// history entry) when nothing lies at or after `from`. Undo:
    /// `arrangement.restore_time`.
    "arrangement.delete_time" (kind = "edit", undo = "inverse", destructive = false)
    DeleteTime { from: Tick, to: Tick } => {
        /// How many placements were removed.
        removed: u32,
        /// How many placements were shortened, trimmed at the front or split.
        cut: u32,
        /// How many placements moved earlier.
        shifted: u32,
        /// How many automation points were removed.
        points_removed: u32,
        /// How many automation points moved earlier.
        points_shifted: u32,
        /// Whether the loop region changed.
        loop_changed: bool,
    };

    /// The inverse of `arrangement.insert_time` and `arrangement.delete_time`: write the
    /// placements as `arrangement.restore` does (`edits`, may be empty here), replace the
    /// points of the named automation lanes by exactly those given (`lanes`, a value kept as
    /// it is, within the parameter's range), put the loop settings back (`transport`,
    /// absent: untouched) and the section markers at their ticks (`markers`), in one history
    /// entry and all or nothing. Refused when nothing is given, when a lane or a marker is
    /// unknown or listed twice, when a point or a marker is out of range, when the loop is
    /// on without a region, or for any reason `arrangement.restore` refuses.
    /// Undo: the restore of what it changed.
    "arrangement.restore_time" (kind = "edit", undo = "inverse", destructive = false)
    RestoreTime {
        edits: Vec<TrackEdit>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lanes: Vec<LanePoints>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transport: Option<LoopState>,
        /// The section markers to put back at their ticks (#473).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        markers: Vec<MarkerTick>,
    } => {};

    /// Write the placements of one or more tracks exactly as given, without the overlap
    /// rule: the inverse of every arrangement command, which brings back what it carved,
    /// with the identifiers. Per track: `put` creates or replaces placements by
    /// identifier, `remove` removes placements, `clips` brings back MIDI clips (new
    /// identifiers, a snapshot as `clip.restore` takes) and `remove_clips` removes clips.
    /// Refused when a placement is outside the timeline or its offset outside its clip,
    /// when a placement is left on a removed clip or names an unknown clip, when an
    /// identifier to create is in use, or when a rule of the clip commands is broken (a
    /// note outside the length, a limit). In an audio track the placements are audio ones
    /// (a record with `frames`) and the clips it brings back are audio ones (`audio_clips`,
    /// a snapshot as `clip.restore_audio` takes; `clips` is refused there, and `audio_clips`
    /// in a MIDI track); an audio placement must end inside its clip (`offset + length` at
    /// most the clip's frames) and never overlap another placement of the track.
    /// Undo: the restore of what it changed.
    "arrangement.restore" (kind = "edit", undo = "inverse", destructive = false)
    Restore { edits: Vec<TrackEdit> } => {};

    /// Add a named section marker (a named point of the arrangement, not the transport's
    /// cue) at the timeline tick `tick` (960 per quarter note, 0 to
    /// 2^40). `name` is optional (trimmed, at most 200 characters): left out, empty or only
    /// spaces, the marker is called "Section n", n the first number no marker has (a blank
    /// name means "no name", unlike `arrangement.marker.rename`, which refuses it because it
    /// has no default to fall back on). Refused with `limit` past 1 024 markers. Undo: `arrangement.marker.delete`.
    "arrangement.marker.add" (kind = "edit", undo = "inverse", destructive = false)
    AddMarker {
        tick: Tick,
        #[serde(default)]
        name: Option<String>,
    } => {
        /// The marker added.
        marker: MarkerId,
    };

    /// Rename a marker. The name is trimmed, not empty, at most 200 characters. Undo:
    /// rename it back.
    "arrangement.marker.rename" (kind = "edit", undo = "inverse", destructive = false)
    RenameMarker { marker: MarkerId, name: String } => {
        /// The name before the command.
        previous: String,
    };

    /// Move a marker to the timeline tick `tick` (0 to 2^40). Undo: move it back.
    "arrangement.marker.move" (kind = "edit", undo = "inverse", destructive = false)
    MoveMarker { marker: MarkerId, tick: Tick } => {
        /// The tick before the command.
        previous: Tick,
    };

    /// Delete a marker. Not destructive: undo (`arrangement.marker.restore`) brings it back with its
    /// identifier.
    "arrangement.marker.delete" (kind = "edit", undo = "inverse", destructive = false)
    DeleteMarker { marker: MarkerId } => {
        /// Where it stood.
        tick: Tick,
        /// Its name.
        name: String,
    };

    /// Bring back a deleted marker with its identifier: the inverse of `arrangement.marker.delete`.
    /// Refused when the identifier is already in use in the project, or for a tick or a name
    /// `arrangement.marker.add` would refuse. Undo: `arrangement.marker.delete`.
    "arrangement.marker.restore" (kind = "edit", undo = "inverse", destructive = false)
    RestoreMarker { marker: MarkerId, tick: Tick, name: String } => {};

    /// List the section markers of the arrangement in timeline order (tick, then
    /// identifier): each with its identifier, its tick (960 per quarter note) and its name.
    /// A query: changes nothing and never enters the history.
    "arrangement.marker.list" (kind = "query", undo = "none", destructive = false)
    ListMarkers {} => {
        /// The markers, in timeline order.
        markers: Vec<crate::marker::MarkerInfo>,
    };
}

type Out = Outcome<Command, Output>;
type Placements = BTreeMap<PlacementId, Placement>;

/// A clip's length and loop, in ticks: what the offset arithmetic needs.
#[derive(Clone, Copy)]
struct Geo {
    length: i64,
    looping: Option<(i64, i64)>,
}

impl Geo {
    /// The clip position reached after `e` ticks of playing from its position 0, the way
    /// the engine reads it (`engine::clip_player`): straight up to the loop's end, then
    /// round the loop; a clip without a loop goes round the whole clip (#473). `None` only
    /// for a clip of no length.
    fn position(self, e: i64) -> Option<i64> {
        match self.looping {
            None => (self.length > 0).then(|| e.rem_euclid(self.length)),
            Some((ls, le)) if e >= le => Some(ls + (e - le).rem_euclid(le - ls)),
            Some(_) => Some(e),
        }
    }
}

/// An audio clip's geometry, in frames of its file: what cutting its placements needs.
#[derive(Clone, Copy)]
struct AudioGeo {
    /// The clip's length in frames.
    frames: i64,
    rate: SampleRate,
    /// Its loop, clip-relative, when it has a valid one.
    looping: Option<(i64, i64)>,
}

impl AudioGeo {
    fn of(content: &ClipContent) -> Option<Self> {
        let ClipContent::Audio {
            media,
            frames,
            looping,
            ..
        } = content
        else {
            return None;
        };
        let frames = frame_i64(*frames);
        Some(Self {
            frames,
            rate: media.sample_rate,
            looping: looping
                .map(|l| (frame_i64(l.start), frame_i64(l.end)))
                .filter(|&(s, e)| s < e && e <= frames),
        })
    }

    /// The frame of the clip reached `f` frames into a read from frame 0: straight up to the
    /// loop's end, then round the loop, the way the audio clip player reads it
    /// (`engine::devices::clip_player`).
    fn position(self, f: i64) -> i64 {
        match self.looping {
            Some((ls, le)) if f >= le => ls + (f - le).rem_euclid(le - ls),
            _ => f,
        }
    }
}

fn frame_i64(f: project::Frame) -> i64 {
    i64::try_from(f.get()).unwrap_or(i64::MAX)
}

fn frame_of(f: i64) -> project::Frame {
    project::Frame::new(u64::try_from(f.max(0)).unwrap_or(0))
}

/// The ticks `frames` frames at `rate` take at `tempo`, at least one: the rule of
/// [`crate::clip::placement_ticks`].
fn audio_ticks(tempo: Tempo, rate: SampleRate, frames: i64) -> i64 {
    tempo.frames_to_ticks(frame_of(frames), rate).get().max(1)
}

/// The frames that play in the first `ticks` ticks of a read: `floor(ticks × frames per
/// tick)` (the player starts a read at the floor of its sample, TR-1), at most `cap`, and
/// never so many that their ticks pass `ticks`.
fn frames_before(tempo: Tempo, rate: SampleRate, ticks: i64, cap: i64) -> i64 {
    if ticks < 1 || cap < 1 {
        return 0;
    }
    let exact = tempo.tick_to_sample(Tick(ticks), rate).floor();
    // `as` saturates and sends NaN to 0.
    let mut n = (exact as i64).clamp(0, cap);
    // The float product is off by a frame at most: the ticks of the result never pass.
    while n > 0 && audio_ticks(tempo, rate, n) > ticks {
        n -= 1;
    }
    n
}

/// The most frames, at most `cap`, whose ticks fit in `ticks` (0 when not even one does).
/// The ticks of a frame count never shrink as it grows, so this is a binary search.
fn frames_within(tempo: Tempo, rate: SampleRate, ticks: i64, cap: i64) -> i64 {
    if ticks < 1 || cap < 1 {
        return 0;
    }
    let (mut low, mut high) = (0i64, cap);
    while low < high {
        let mid = low + (high - low + 1) / 2;
        if audio_ticks(tempo, rate, mid) <= ticks {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    low
}

/// One track while a command works on it: the placements as they will be, the clips it
/// creates or removes, and the geometry of its clips (a MIDI clip's in ticks, an audio
/// clip's in frames of its file, read at the project's tempo).
struct TrackWork {
    track: TrackId,
    placements: Placements,
    geo: BTreeMap<ClipId, Geo>,
    audio: BTreeMap<ClipId, AudioGeo>,
    tempo: Tempo,
    new_clips: BTreeMap<ClipId, Clip>,
    gone_clips: Vec<ClipId>,
}

type Works = BTreeMap<TrackId, TrackWork>;

impl TrackWork {
    /// The work on a track of either kind: MIDI placements in ticks, audio ones in frames.
    fn new(project: &Project, id: &TrackId) -> Result<Self, CommandError> {
        let track = project
            .tracks
            .get(id)
            .ok_or_else(|| not_found("track", id))?;
        Ok(Self::of(track, id, project.tempo()))
    }

    fn of(track: &Track, id: &TrackId, tempo: Tempo) -> Self {
        let mut geo = BTreeMap::new();
        let mut audio = BTreeMap::new();
        for (clip_id, clip) in &track.clips {
            if let ClipContent::Midi {
                length, looping, ..
            } = &clip.content
            {
                geo.insert(
                    clip_id.clone(),
                    Geo {
                        length: length.get(),
                        looping: looping.map(|l| (l.start.get(), l.end.get())),
                    },
                );
            } else if let Some(g) = AudioGeo::of(&clip.content) {
                audio.insert(clip_id.clone(), g);
            }
        }
        Self {
            track: id.clone(),
            placements: track.placements.clone(),
            geo,
            audio,
            tempo,
            new_clips: BTreeMap::new(),
            gone_clips: Vec::new(),
        }
    }

    /// The ticks `[start, end)` a placement covers: its length for a MIDI one, its frames
    /// at the project's tempo for an audio one (at least one tick).
    fn span(&self, p: &Placement) -> Result<(i64, i64), CommandError> {
        let start = p.start.get();
        match p.span {
            Span::Ticks { length, .. } => Ok((start, start.saturating_add(length.get()))),
            Span::Frames { length, .. } => {
                let g = self
                    .audio
                    .get(&p.clip)
                    .ok_or_else(|| not_found("clip", &p.clip))?;
                Ok((
                    start,
                    start.saturating_add(audio_ticks(self.tempo, g.rate, frame_i64(length))),
                ))
            }
        }
    }

    /// The span of a placement a command puts down, checked to lie on the timeline.
    fn checked_span(&self, p: &Placement, field: &str) -> Result<(i64, i64), CommandError> {
        let (s, e) = self.span(p)?;
        check_extent(s, e - s, field)?;
        Ok((s, e))
    }

    /// The part of `p` that plays in the ticks `[a, b)`, as a placement that starts at
    /// `a` (clamped to `p`'s span); `None` when nothing of the clip is left there.
    ///
    /// A MIDI part resumes the clip where the cut falls, round its loop or round the whole
    /// clip. An audio part keeps whole frames: it skips the frames that play before `a`
    /// ([`frames_before`]), stops at those that play before `b` (or at the placement's last
    /// frame when `b` is its end), and is trimmed at the front while it would end after
    /// `b`. So the two parts of one cut never overlap and never play a frame twice, and a
    /// part never ends after the placement it comes from; a part where not even one frame
    /// fits is `None`. Its offset goes round the clip's loop, as the player reads it.
    fn part(&self, p: &Placement, a: i64, b: i64) -> Option<Placement> {
        let (s, e) = self.span(p).ok()?;
        let (a, b) = (a.max(s), b.min(e));
        if a >= b {
            return None;
        }
        match p.span {
            Span::Ticks { offset, .. } => {
                let o = offset.get();
                let offset = if a == s {
                    o
                } else {
                    self.geo.get(&p.clip)?.position(o.saturating_add(a - s))?
                };
                Some(shaped(&p.clip, a, b - a, offset))
            }
            Span::Frames { length, offset } => {
                let g = *self.audio.get(&p.clip)?;
                let (len, o) = (frame_i64(length), frame_i64(offset));
                let skip = if a == s {
                    0
                } else {
                    frames_before(self.tempo, g.rate, a - s, len)
                };
                let until = if b == e {
                    len
                } else {
                    frames_before(self.tempo, g.rate, b - s, len)
                };
                if until <= skip {
                    return None;
                }
                let kept = frames_within(self.tempo, g.rate, b - a, until - skip);
                if kept < 1 {
                    return None;
                }
                let from = until - kept;
                let offset = if from == 0 {
                    o
                } else {
                    g.position(o.saturating_add(from))
                };
                Some(Placement {
                    clip: p.clip.clone(),
                    start: Tick(a),
                    span: Span::Frames {
                        length: frame_of(kept),
                        offset: frame_of(offset),
                    },
                })
            }
        }
    }

    /// Carves the placements that overlap `[start, end)`; returns how many it touched.
    fn carve(&mut self, ids: &mut IdGenerator, start: i64, end: i64) -> Result<u32, CommandError> {
        let mut hit = Vec::new();
        for (id, p) in &self.placements {
            let (s, e) = self.span(p)?;
            if s < end && start < e {
                hit.push(id.clone());
            }
        }
        for id in &hit {
            let Some(p) = self.placements.remove(id) else {
                continue;
            };
            let (s, e) = self.span(&p)?;
            let left = if s < start {
                self.part(&p, s, start)
            } else {
                None
            };
            let right = if e > end { self.part(&p, end, e) } else { None };
            let had_left = left.is_some();
            if let Some(left) = left {
                self.placements.insert(id.clone(), left);
            }
            if let Some(right) = right {
                let at = if had_left {
                    PlacementId::generate(ids)
                } else {
                    id.clone()
                };
                self.placements.insert(at, right);
            }
        }
        Ok(count(hit.len()))
    }

    /// A clip the command makes in this track, with its geometry.
    fn add_clip(&mut self, id: ClipId, clip: Clip) {
        if let ClipContent::Midi {
            length, looping, ..
        } = &clip.content
        {
            self.geo.insert(
                id.clone(),
                Geo {
                    length: length.get(),
                    looping: looping.map(|l| (l.start.get(), l.end.get())),
                },
            );
        } else if let Some(g) = AudioGeo::of(&clip.content) {
            self.audio.insert(id.clone(), g);
        }
        self.new_clips.insert(id, clip);
    }

    /// Whether the work knows the geometry of `clip` (it is a clip of the track, or one the
    /// command makes).
    fn knows(&self, clip: &ClipId) -> bool {
        self.geo.contains_key(clip) || self.audio.contains_key(clip)
    }

    /// `p` moved `by` ticks; refused when its start leaves the tick range.
    fn shifted(p: &Placement, by: i64, field: &str) -> Result<Placement, CommandError> {
        let start = p
            .start
            .get()
            .checked_add(by)
            .ok_or_else(|| invalid(field, "the placement leaves the timeline"))?;
        Ok(Placement {
            start: Tick(start),
            ..p.clone()
        })
    }

    /// Opens `length` ticks at `at`: a placement that straddles `at` is split there (the
    /// right part is new and resumes the clip where the left part stops, round the clip),
    /// the ones from `at` on move later.
    /// Returns `(split, shifted)`.
    fn open_gap(
        &mut self,
        ids: &mut IdGenerator,
        at: i64,
        length: i64,
    ) -> Result<(u32, u32), CommandError> {
        let (mut split, mut shifted) = (0usize, 0usize);
        let mut out = Placements::new();
        for (id, p) in std::mem::take(&mut self.placements) {
            let (s, e) = self.span(&p)?;
            if e <= at {
                out.insert(id, p);
            } else if s >= at {
                let moved = Self::shifted(&p, length, "length")?;
                self.checked_span(&moved, "length")?;
                out.insert(id, moved);
                shifted += 1;
            } else {
                // The left part keeps the identifier (an audio one where not even a frame
                // fits before `at` is the right part only).
                let left = self.part(&p, s, at);
                let right = self.part(&p, at, e);
                let had_left = left.is_some();
                if let Some(left) = left {
                    out.insert(id.clone(), left);
                }
                split += 1;
                if let Some(right) = right {
                    let moved = Self::shifted(&right, length, "length")?;
                    self.checked_span(&moved, "length")?;
                    let at = if had_left {
                        PlacementId::generate(ids)
                    } else {
                        id.clone()
                    };
                    out.insert(at, moved);
                    shifted += 1;
                }
            }
        }
        self.placements = out;
        Ok((count(split), count(shifted)))
    }

    /// Removes `[start, end)` and closes the gap: the placements are cut at both edges (the
    /// rule of [`TrackWork::carve`]), the ones from `end` on move earlier. Returns
    /// `(removed, cut, shifted)`.
    fn close_gap(
        &mut self,
        ids: &mut IdGenerator,
        start: i64,
        end: i64,
    ) -> Result<(u32, u32, u32), CommandError> {
        let before: Vec<PlacementId> = self.placements.keys().cloned().collect();
        let touched = self.carve(ids, start, end)?;
        let removed = before
            .iter()
            .filter(|id| !self.placements.contains_key(*id))
            .count();
        let mut shifted = 0usize;
        for p in self.placements.values_mut() {
            if p.start.get() >= end {
                p.start = Tick(p.start.get() - (end - start));
                shifted += 1;
            }
        }
        Ok((
            count(removed),
            touched.saturating_sub(count(removed)),
            count(shifted),
        ))
    }

    fn put(&mut self, id: PlacementId, clip: &ClipId, start: i64, length: i64, offset: i64) {
        self.placements
            .insert(id, shaped(clip, start, length, offset));
    }
}

fn shaped(clip: &ClipId, start: i64, length: i64, offset: i64) -> Placement {
    Placement {
        clip: clip.clone(),
        start: Tick(start),
        span: Span::Ticks {
            length: Tick(length),
            offset: Tick(offset),
        },
    }
}

/// `(start, length, offset)` of a placement in ticks.
fn extent(p: &Placement) -> Result<(i64, i64, i64), CommandError> {
    match p.span {
        Span::Ticks { length, offset } => Ok((p.start.get(), length.get(), offset.get())),
        Span::Frames { .. } => Err(invalid(
            "placement",
            "an audio placement is measured in frames, not in ticks",
        )),
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A placement lies in `[0, MAX_TICK]` with a length of at least 1.
fn check_extent(start: i64, length: i64, field: &str) -> Result<(), CommandError> {
    match start.checked_add(length) {
        Some(end) if start >= 0 && length >= 1 && end <= MAX_TICK => Ok(()),
        _ => Err(invalid(
            field,
            format!("a placement lies in [0, {MAX_TICK}] ticks, with a length of at least 1"),
        )),
    }
}

/// The track of each placement, grouped by track in list order. Refuses an empty list, a
/// repeated placement and an unknown one.
fn group_by_track(
    project: &Project,
    ids: &[PlacementId],
    field: &str,
) -> Result<BTreeMap<TrackId, Vec<PlacementId>>, CommandError> {
    if ids.is_empty() {
        return Err(invalid(field, "name at least one placement"));
    }
    let mut owner: BTreeMap<&PlacementId, &TrackId> = BTreeMap::new();
    for (track_id, track) in &project.tracks {
        for id in track.placements.keys() {
            owner.insert(id, track_id);
        }
    }
    let mut seen = HashSet::new();
    let mut out: BTreeMap<TrackId, Vec<PlacementId>> = BTreeMap::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(invalid(field, format!("placement {id} is listed twice")));
        }
        let track = owner.get(id).ok_or_else(|| not_found("placement", id))?;
        out.entry((*track).clone()).or_default().push(id.clone());
    }
    Ok(out)
}

fn track_of(project: &Project, id: &PlacementId) -> Result<TrackId, CommandError> {
    project
        .tracks
        .iter()
        .find(|(_, t)| t.placements.contains_key(id))
        .map(|(track, _)| track.clone())
        .ok_or_else(|| not_found("placement", id))
}

fn placement_of<'a>(track: &'a Track, id: &PlacementId) -> Result<&'a Placement, CommandError> {
    track
        .placements
        .get(id)
        .ok_or_else(|| not_found("placement", id))
}

/// Writes the work to the project and builds the answer: the inverse is the restore of
/// what actually changed, the events are the clips created and removed and the tracks
/// whose arrangement changed.
fn commit(
    state: &mut State,
    works: Works,
    output: impl FnOnce() -> Output,
) -> Result<Out, CommandError> {
    commit_time(state, works, Extra::default(), output)
}

/// What a time edit (`arrangement.insert_time`, `delete_time`, `restore_time`) changes
/// besides the placements: the whole point tables of automation lanes, and the loop.
#[derive(Default)]
struct Extra {
    lanes: BTreeMap<LaneId, BTreeMap<Tick, AutomationPoint>>,
    /// The loop region and whether the loop is on, when they change.
    transport: Option<(Option<LoopRegion>, bool)>,
    /// The section markers' new ticks (#473), for those a time edit moves.
    markers: BTreeMap<MarkerId, Tick>,
}

/// A section marker at a tick: what `arrangement.restore_time` puts back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkerTick {
    pub marker: MarkerId,
    pub tick: Tick,
}

/// [`commit`] for an edit that also changes automation lanes and the loop: all of it
/// is checked before anything is written, and the inverse is one `arrangement.restore_time`
/// (a plain `arrangement.restore` when only placements changed).
fn commit_time(
    state: &mut State,
    works: Works,
    extra: Extra,
    output: impl FnOnce() -> Output,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let output = output();
    // Limits first, so a refusal leaves nothing changed.
    let mut total = project.counts().placements;
    for work in works.values() {
        let old = project
            .tracks
            .get(&work.track)
            .ok_or_else(|| not_found("track", &work.track))?
            .placements
            .len();
        check_total(
            "placements per track",
            work.placements.len(),
            0,
            MAX_PLACEMENTS_PER_TRACK,
        )?;
        total = total
            .saturating_sub(old)
            .saturating_add(work.placements.len());
    }
    check_total("placements", total, 0, MAX_PLACEMENTS)?;

    let mut inverse = Vec::new();
    let mut events = Vec::new();
    let mut changed_tracks = Vec::new();
    let mut created = Vec::new();
    let mut deleted = Vec::new();
    for work in works.values() {
        let track = project
            .tracks
            .get(&work.track)
            .ok_or_else(|| not_found("track", &work.track))?;
        let put: Vec<PlacementRecord> = track
            .placements
            .iter()
            .filter(|(id, p)| work.placements.get(*id) != Some(*p))
            .map(|(id, p)| record(id, p))
            .collect::<Result<_, _>>()?;
        let remove: Vec<PlacementId> = work
            .placements
            .keys()
            .filter(|id| !track.placements.contains_key(*id))
            .cloned()
            .collect();
        let mut clips = Vec::new();
        let mut audio_clips = Vec::new();
        for id in &work.gone_clips {
            let clip = track.clips.get(id).ok_or_else(|| not_found("clip", id))?;
            if let Some(snapshot) = crate::clip::audio_snapshot(clip) {
                audio_clips.push(AudioClipRecord {
                    id: id.clone(),
                    snapshot,
                });
            } else {
                clips.push(ClipRecord {
                    id: id.clone(),
                    snapshot: snapshot(clip)?,
                });
            }
            deleted.push(ProjectEvent::ClipDeleted {
                track: work.track.clone(),
                clip: id.clone(),
                placements: track
                    .placements
                    .iter()
                    .filter(|(_, p)| p.clip == *id)
                    .map(|(pid, _)| pid.clone())
                    .collect(),
            });
        }
        for id in work.new_clips.keys() {
            created.push(ProjectEvent::ClipCreated {
                track: work.track.clone(),
                clip: id.clone(),
                placements: work
                    .placements
                    .iter()
                    .filter(|(_, p)| p.clip == *id)
                    .map(|(pid, _)| pid.clone())
                    .collect(),
            });
        }
        let edit = TrackEdit {
            track: work.track.clone(),
            put,
            remove,
            clips,
            audio_clips,
            remove_clips: work.new_clips.keys().cloned().collect(),
        };
        if edit.put.is_empty()
            && edit.remove.is_empty()
            && edit.clips.is_empty()
            && edit.audio_clips.is_empty()
            && edit.remove_clips.is_empty()
        {
            continue;
        }
        changed_tracks.push(work.track.clone());
        inverse.push(edit);
    }
    // The lanes: the points as they are now go in the inverse; the limits are checked on
    // the tables as they will be.
    let mut lane_inverse = Vec::new();
    let mut lane_writes = Vec::new();
    let mut points = automation::points_in(state);
    for (id, table) in &extra.lanes {
        let owner = project
            .lane_owner(id)
            .ok_or_else(|| not_found("lane", id))?;
        let held = project
            .lanes(owner.as_ref())
            .and_then(|lanes| lanes.get(id))
            .ok_or_else(|| not_found("lane", id))?;
        if held.points == *table {
            continue;
        }
        check_total(
            "automation points",
            table.len(),
            0,
            project::limits::MAX_POINTS_PER_LANE,
        )?;
        points = points
            .saturating_sub(held.points.len())
            .saturating_add(table.len());
        lane_inverse.push(LanePoints {
            lane: id.clone(),
            points: automation::points_of(&held.points),
        });
        lane_writes.push((owner, id.clone(), table.clone()));
    }
    check_total("automation points", points, 0, MAX_POINTS)?;
    let mut marker_inverse = Vec::new();
    for (id, tick) in &extra.markers {
        let marker = project
            .markers
            .get(id)
            .ok_or_else(|| not_found("marker", id))?;
        if !(0..=MAX_TICK).contains(&tick.get()) {
            return Err(invalid(
                "markers",
                format!("a marker is within 0 to {MAX_TICK} ticks"),
            ));
        }
        if marker.tick != *tick {
            marker_inverse.push(MarkerTick {
                marker: id.clone(),
                tick: marker.tick,
            });
        }
    }
    let current = &project.transport;
    let loop_inverse = extra
        .transport
        .filter(|new| *new != (current.loop_region, current.loop_enabled))
        .map(|_| LoopState {
            loop_region: as_tick_loop(current.loop_region),
            loop_enabled: current.loop_enabled,
        });
    if inverse.is_empty()
        && lane_inverse.is_empty()
        && loop_inverse.is_none()
        && marker_inverse.is_empty()
    {
        return Ok(Outcome {
            output,
            undo: Undo::None,
            events: vec![],
            control: vec![],
        });
    }
    for work in works.into_values() {
        if !changed_tracks.contains(&work.track) {
            continue;
        }
        let track = state
            .project
            .tracks
            .get_mut(&work.track)
            .ok_or_else(|| not_found("track", &work.track))?;
        for id in &work.gone_clips {
            track.clips.remove(id);
        }
        track.clips.extend(work.new_clips);
        track.placements = work.placements;
    }
    events.extend(deleted);
    events.extend(created);
    events.extend(
        changed_tracks
            .into_iter()
            .map(|track| ProjectEvent::ArrangementChanged { track }),
    );
    for (owner, id, table) in lane_writes {
        if let Some(lane) = state
            .project
            .lanes_mut(owner.as_ref())
            .and_then(|lanes| lanes.get_mut(&id))
        {
            lane.points = table;
        }
        events.push(ProjectEvent::AutomationPointsChanged {
            track: owner,
            lane: id,
        });
    }
    if let (Some((region, enabled)), Some(_)) = (extra.transport, &loop_inverse) {
        state.project.transport.loop_region = region;
        state.project.transport.loop_enabled = enabled;
        events.push(ProjectEvent::LoopChanged {
            region: as_tick_loop(region),
            enabled,
        });
    }
    if !marker_inverse.is_empty() {
        for (id, tick) in extra.markers {
            if let Some(marker) = state.project.markers.get_mut(&id) {
                marker.tick = tick;
            }
        }
        events.push(ProjectEvent::MarkersChanged {});
    }
    let undo = if lane_inverse.is_empty() && loop_inverse.is_none() && marker_inverse.is_empty() {
        Command::Restore { edits: inverse }
    } else {
        Command::RestoreTime {
            edits: inverse,
            lanes: lane_inverse,
            transport: loop_inverse,
            markers: marker_inverse,
        }
    };
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(undo)),
        events,
        control: vec![],
    })
}

fn as_tick_loop(region: Option<LoopRegion>) -> Option<TickLoop> {
    region.map(|r| TickLoop {
        start: r.start(),
        end: r.end(),
    })
}

fn record(id: &PlacementId, p: &Placement) -> Result<PlacementRecord, CommandError> {
    if let Span::Frames { length, offset } = p.span {
        return Ok(PlacementRecord {
            id: id.clone(),
            clip: p.clip.clone(),
            start: p.start,
            length: Tick(0),
            offset: Tick(0),
            frames: Some(FrameSpan {
                length: length.get(),
                offset: offset.get(),
            }),
        });
    }
    let (start, length, offset) = extent(p)?;
    Ok(PlacementRecord {
        id: id.clone(),
        clip: p.clip.clone(),
        start: Tick(start),
        length: Tick(length),
        offset: Tick(offset),
        frames: None,
    })
}

/// Applies a command of this domain.
pub fn apply(command: &Command, state: &mut State, ctx: &mut Context) -> Result<Out, CommandError> {
    match command {
        Command::Place {
            clip,
            start,
            length,
            offset,
        } => place(state, ctx, clip, *start, *length, *offset),
        Command::PlaceFromSlot {
            clip,
            start,
            linked,
        } => {
            let sent = send_from_grid(state, ctx, std::slice::from_ref(clip), *start, *linked)?;
            let (placement, copy) = sent
                .placed
                .into_iter()
                .next()
                .ok_or_else(|| invalid("clip", "nothing to place"))?;
            let overwritten = sent.overwritten;
            commit(state, sent.works, || Output::PlaceFromSlot {
                placement,
                clip: copy,
                overwritten,
            })
        }
        Command::PlaceScene {
            scene,
            start,
            linked,
        } => {
            let clips = scene_clips(&state.project, scene)?;
            let sent = send_from_grid(state, ctx, &clips, *start, *linked)?;
            let overwritten = sent.overwritten;
            let (placements, copies): (Vec<_>, Vec<_>) = sent.placed.into_iter().unzip();
            commit(state, sent.works, || Output::PlaceScene {
                placements,
                clips: copies.into_iter().flatten().collect(),
                overwritten,
            })
        }
        Command::Move { placements, by } => move_placements(state, ctx, placements, *by),
        Command::MoveToTrack { moves, by } => {
            move_to_track(state, ctx, moves, by.map_or(0, Tick::get))
        }
        Command::Resize {
            placement,
            edge,
            to,
        } => resize(state, ctx, placement, *edge, *to),
        Command::Split { placement, at } => split(state, ctx, placement, *at),
        Command::Delete { placements } => delete(state, placements),
        Command::Duplicate {
            placements,
            linked,
            by,
        } => duplicate(state, ctx, placements, *linked, *by),
        Command::Repeat { placement, until } => repeat(state, ctx, placement, until.get()),
        Command::Restore { edits } => restore(state, edits),
        Command::InsertTime { at, length } => insert_time(state, ctx, at.get(), length.get()),
        Command::DeleteTime { from, to } => delete_time(state, ctx, from.get(), to.get()),
        Command::RestoreTime {
            edits,
            lanes,
            transport,
            markers,
        } => restore_time(state, edits, lanes, *transport, markers),
        // The section markers (#473): their own module.
        _ => crate::marker::apply(command, state, ctx).unwrap_or_else(|| {
            Err(CommandError::Unsupported {
                command: command.name().into(),
            })
        }),
    }
}

fn place(
    state: &mut State,
    ctx: &mut Context,
    clip: &ClipId,
    start: Tick,
    length: Option<Tick>,
    offset: Option<Tick>,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let track_id = locate(project, clip)?;
    let any_track = project
        .tracks
        .get(&track_id)
        .ok_or_else(|| not_found("track", &track_id))?;
    if any_track.kind == project::TrackKind::Audio {
        return place_audio(state, ctx, &track_id, clip, start, length, offset);
    }
    let track = midi_track(project, &track_id)?;
    let source = track
        .clips
        .get(clip)
        .ok_or_else(|| not_found("clip", clip))?;
    let (clip_length, _, _) = midi_ref(source)?;
    if source.slot.is_some() {
        return Err(invalid(
            "clip",
            "the clip sits in the launch grid: copying or linking it into the arrangement is arrangement.place_from_slot (AR-5)",
        ));
    }
    let offset = offset.map_or(0, Tick::get);
    if !(0..clip_length.get()).contains(&offset) {
        return Err(invalid("offset", "the offset is inside the clip"));
    }
    let length = length.map_or(clip_length.get() - offset, Tick::get);
    check_extent(start.get(), length, "start")?;
    let mut work = TrackWork::new(project, &track_id)?;
    let overwritten = work.carve(&mut ctx.ids, start.get(), start.get() + length)?;
    let id = PlacementId::generate(&mut ctx.ids);
    work.put(id.clone(), clip, start.get(), length, offset);
    commit(state, BTreeMap::from([(track_id, work)]), || {
        Output::Place {
            placement: id,
            overwritten,
        }
    })
}

/// `arrangement.place` for an audio clip of an audio track: a placement of the whole clip
/// (offset 0, its frames), starting at the tick `start`, as `clip.import_audio` with
/// `at: arrangement` puts one. Its end is the clip's frames at the project's tempo. It wins
/// on overlap like a MIDI placement (AU-6): the placements it covers are carved in frames.
fn place_audio(
    state: &mut State,
    ctx: &mut Context,
    track_id: &TrackId,
    clip: &ClipId,
    start: Tick,
    length: Option<Tick>,
    offset: Option<Tick>,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let track = project
        .tracks
        .get(track_id)
        .ok_or_else(|| not_found("track", track_id))?;
    let source = track
        .clips
        .get(clip)
        .ok_or_else(|| not_found("clip", clip))?;
    let ClipContent::Audio { frames, .. } = &source.content else {
        return Err(not_found("clip", clip));
    };
    if length.is_some() || offset.is_some() {
        return Err(invalid(
            if length.is_some() { "length" } else { "offset" },
            "an audio clip is placed whole, from its first frame: `length` and `offset` are ticks, for a MIDI clip; the range an audio clip plays is `clip.audio.set_range`, and `arrangement.resize` shortens a placement",
        ));
    }
    if source.slot.is_some() {
        return Err(invalid(
            "clip",
            "the clip sits in the launch grid: copying or linking it into the arrangement is arrangement.place_from_slot (AR-5)",
        ));
    }
    let mut work = TrackWork::new(project, track_id)?;
    let placed = Placement {
        clip: clip.clone(),
        start,
        span: Span::Frames {
            length: *frames,
            offset: project::Frame::new(0),
        },
    };
    let (s, e) = work.checked_span(&placed, "start")?;
    // The last one put down wins, as for a MIDI placement (AU-6).
    let overwritten = work.carve(&mut ctx.ids, s, e)?;
    let id = PlacementId::generate(&mut ctx.ids);
    work.placements.insert(id.clone(), placed);
    commit(state, BTreeMap::from([(track_id.clone(), work)]), || {
        Output::Place {
            placement: id,
            overwritten,
        }
    })
}

/// What sending clips of the grid to the arrangement works out, before it is committed.
struct Sent {
    works: Works,
    /// Per clip, in order: the placement and the independent copy (none when linked).
    placed: Vec<(PlacementId, Option<ClipId>)>,
    overwritten: u32,
}

/// The clips of a scene (MIDI and audio ones), one per track, in the order of the tracks.
fn scene_clips(project: &Project, scene: &SceneId) -> Result<Vec<ClipId>, CommandError> {
    if !project.scenes.contains_key(scene) {
        return Err(not_found("scene", scene));
    }
    let mut out = Vec::new();
    // In the order the tracks are shown.
    let mut ordered: Vec<_> = project.tracks.iter().collect();
    ordered.sort_by(|a, b| a.1.order.cmp(&b.1.order).then_with(|| a.0.cmp(b.0)));
    for (_, track) in ordered {
        for (id, clip) in &track.clips {
            if clip.slot.as_ref() == Some(scene) && fits(track, clip) {
                out.push(id.clone());
            }
        }
    }
    if out.is_empty() {
        return Err(invalid("scene", "the scene holds no clip to place"));
    }
    Ok(out)
}

/// Whether a clip is of its track's kind (a MIDI clip in a MIDI track, an audio clip in an
/// audio track): the only clips the arrangement places.
fn fits(track: &Track, clip: &Clip) -> bool {
    matches!(
        (&track.kind, &clip.content),
        (project::TrackKind::Midi, ClipContent::Midi { .. })
            | (project::TrackKind::Audio, ClipContent::Audio { .. })
    )
}

/// The notes a copy of `clip` adds (none for an audio clip).
fn notes_of(clip: &Clip) -> usize {
    match &clip.content {
        ClipContent::Midi { notes, .. } => notes.len(),
        ClipContent::Audio { .. } => 0,
    }
}

/// A placement of the whole clip from `start`: its length from offset 0 for a MIDI clip,
/// its frames from frame 0 for an audio one.
fn whole(clip_id: &ClipId, clip: &Clip, start: Tick) -> Placement {
    let span = match &clip.content {
        ClipContent::Midi { length, .. } => Span::Ticks {
            length: *length,
            offset: Tick(0),
        },
        ClipContent::Audio { frames, .. } => Span::Frames {
            length: *frames,
            offset: project::Frame::new(0),
        },
    };
    Placement {
        clip: clip_id.clone(),
        start,
        span,
    }
}

/// Puts each clip of the grid in the arrangement of its own track at `start`, whole, as
/// an independent copy or linked. The clips are checked and placed all or none.
fn send_from_grid(
    state: &State,
    ctx: &mut Context,
    ids: &[ClipId],
    start: Tick,
    linked: bool,
) -> Result<Sent, CommandError> {
    let project = &state.project;
    let mut works = Works::new();
    let mut placed = Vec::new();
    let mut overwritten = 0;
    let mut seen = HashSet::new();
    let mut new_clips = 0usize;
    // The names of the copies made so far: the project does not hold them yet.
    let mut notes_added = 0usize;
    let notes_in_project = project_notes(project);
    for id in ids {
        if !seen.insert(id) {
            return Err(invalid("clip", format!("clip {id} is listed twice")));
        }
        let track_id = locate(project, id)?;
        let track = project
            .tracks
            .get(&track_id)
            .ok_or_else(|| not_found("track", &track_id))?;
        let clip = track.clips.get(id).ok_or_else(|| not_found("clip", id))?;
        if !fits(track, clip) {
            return Err(invalid(
                "clip",
                "a MIDI clip is placed in a MIDI track, an audio clip in an audio track",
            ));
        }
        let notes = notes_of(clip);
        if clip.slot.is_none() {
            return Err(invalid(
                "clip",
                "the clip does not sit in the launch grid: use arrangement.place or arrangement.duplicate",
            ));
        }
        if !works.contains_key(&track_id) {
            works.insert(track_id.clone(), TrackWork::new(project, &track_id)?);
        }
        let work = works
            .get_mut(&track_id)
            .ok_or_else(|| not_found("track", &track_id))?;
        let (s, e) = work.checked_span(&whole(id, clip, start), "start")?;
        let mut copy = None;
        let target = if linked {
            id.clone()
        } else {
            notes_added = notes_added.saturating_add(notes);
            check_total(
                "clips per track",
                track.clips.len(),
                work.new_clips.len() + 1,
                MAX_CLIPS_PER_TRACK,
            )?;
            check_total(
                "clips",
                project.counts().clips,
                new_clips + 1,
                project::limits::MAX_CLIPS,
            )?;
            check_note_limits(notes, notes_in_project.saturating_add(notes_added))?;
            new_clips += 1;
            let copy_id = ClipId::generate(&mut ctx.ids);
            let mut copied = clip.clone();
            copied.slot = None;
            work.add_clip(copy_id.clone(), copied);
            copy = Some(copy_id.clone());
            copy_id
        };
        overwritten += work.carve(&mut ctx.ids, s, e)?;
        let placement = PlacementId::generate(&mut ctx.ids);
        work.placements
            .insert(placement.clone(), whole(&target, clip, start));
        placed.push((placement, copy));
    }
    Ok(Sent {
        works,
        placed,
        overwritten,
    })
}

fn move_placements(
    state: &mut State,
    ctx: &mut Context,
    ids: &[PlacementId],
    by: Tick,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let groups = group_by_track(project, ids, "placements")?;
    let mut works = Works::new();
    let mut overwritten = 0;
    for (track_id, moved) in groups {
        let mut work = TrackWork::new(project, &track_id)?;
        let lifted: Vec<(PlacementId, Placement)> = moved
            .iter()
            .filter_map(|id| work.placements.remove(id).map(|p| (id.clone(), p)))
            .collect();
        for (id, p) in lifted {
            // A placement keeps its span (an audio one its frames): only its start moves.
            let moved = TrackWork::shifted(&p, by.get(), "by")?;
            let (s, e) = work.checked_span(&moved, "by")?;
            overwritten += work.carve(&mut ctx.ids, s, e)?;
            work.placements.insert(id, moved);
        }
        works.insert(track_id, work);
    }
    commit(state, works, || Output::Move { overwritten })
}

/// `arrangement.move_to_track`: the placements are lifted, then put down in their target
/// track (carving what they cover), with a copy of their clip when the track is another.
fn move_to_track(
    state: &mut State,
    ctx: &mut Context,
    moves: &[TrackMove],
    by: i64,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let ids: Vec<PlacementId> = moves.iter().map(|m| m.placement.clone()).collect();
    let groups = group_by_track(project, &ids, "moves")?;
    let mut works = Works::new();
    for track_id in groups.keys() {
        works.insert(track_id.clone(), TrackWork::new(project, track_id)?);
    }
    for m in moves {
        let target = project
            .tracks
            .get(&m.track)
            .ok_or_else(|| not_found("track", &m.track))?;
        let source = track_of(project, &m.placement)?;
        let kind = project
            .tracks
            .get(&source)
            .ok_or_else(|| not_found("track", &source))?
            .kind;
        if target.kind != kind {
            return Err(invalid(
                "moves",
                format!(
                    "track {} is not of the clip's kind: a MIDI clip only moves to a MIDI track, an audio clip to an audio track",
                    m.track
                ),
            ));
        }
        if !works.contains_key(&m.track) {
            works.insert(m.track.clone(), TrackWork::new(project, &m.track)?);
        }
    }
    // Lift them all first: they may land on each other's old place.
    let mut lifted = Vec::new();
    for m in moves {
        let source = track_of(project, &m.placement)?;
        let p = works
            .get_mut(&source)
            .and_then(|w| w.placements.remove(&m.placement))
            .ok_or_else(|| not_found("placement", &m.placement))?;
        lifted.push((m, source, p));
    }
    let mut copies: BTreeMap<(TrackId, ClipId), ClipId> = BTreeMap::new();
    let mut left: Vec<(TrackId, ClipId)> = Vec::new();
    let mut moved_ids = Vec::new();
    let mut overwritten = 0;
    let mut new_clips = 0usize;
    let mut notes_added = 0usize;
    let notes_in_project = project_notes(project);
    for (m, source, p) in lifted {
        let moved = TrackWork::shifted(&p, by, "by")?;
        let stays = m.track == source;
        let clip = if stays {
            p.clip.clone()
        } else if let Some(copy) = copies.get(&(m.track.clone(), p.clip.clone())) {
            copy.clone()
        } else {
            let from = project
                .tracks
                .get(&source)
                .and_then(|t| t.clips.get(&p.clip))
                .ok_or_else(|| not_found("clip", &p.clip))?;
            let notes = notes_of(from);
            notes_added = notes_added.saturating_add(notes);
            check_note_limits(notes, notes_in_project.saturating_add(notes_added))?;
            let target = project
                .tracks
                .get(&m.track)
                .ok_or_else(|| not_found("track", &m.track))?;
            let work = works
                .get_mut(&m.track)
                .ok_or_else(|| not_found("track", &m.track))?;
            check_total(
                "clips per track",
                target.clips.len(),
                work.new_clips.len() + 1,
                MAX_CLIPS_PER_TRACK,
            )?;
            check_total(
                "clips",
                project.counts().clips,
                new_clips + 1,
                project::limits::MAX_CLIPS,
            )?;
            new_clips += 1;
            let copy = ClipId::generate(&mut ctx.ids);
            let mut copied = from.clone();
            copied.slot = None;
            work.add_clip(copy.clone(), copied);
            copies.insert((m.track.clone(), p.clip.clone()), copy.clone());
            left.push((source.clone(), p.clip.clone()));
            copy
        };
        let work = works
            .get_mut(&m.track)
            .ok_or_else(|| not_found("track", &m.track))?;
        let moved = Placement { clip, ..moved };
        let (s, e) = work.checked_span(&moved, "by")?;
        overwritten += work.carve(&mut ctx.ids, s, e)?;
        let id = if stays {
            m.placement.clone()
        } else {
            PlacementId::generate(&mut ctx.ids)
        };
        work.placements.insert(id.clone(), moved);
        moved_ids.push(id);
    }
    // A clip that no placement uses any more and that no slot holds goes with its placements.
    for (source, clip) in left {
        let held = project
            .tracks
            .get(&source)
            .and_then(|t| t.clips.get(&clip))
            .is_some_and(|c| c.slot.is_some());
        let Some(work) = works.get_mut(&source) else {
            continue;
        };
        if held
            || work.gone_clips.contains(&clip)
            || work.placements.values().any(|p| p.clip == clip)
        {
            continue;
        }
        work.geo.remove(&clip);
        work.audio.remove(&clip);
        work.gone_clips.push(clip);
    }
    commit(state, works, || Output::MoveToTrack {
        placements: moved_ids,
        overwritten,
    })
}

fn resize(
    state: &mut State,
    ctx: &mut Context,
    id: &PlacementId,
    edge: Edge,
    to: Tick,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let track_id = track_of(project, id)?;
    let mut work = TrackWork::new(project, &track_id)?;
    let p = work
        .placements
        .remove(id)
        .ok_or_else(|| not_found("placement", id))?;
    if let Span::Frames { .. } = p.span {
        let resized = resize_audio(&work, &p, edge, to.get())?;
        let (s, e) = work.checked_span(&resized, "to")?;
        let overwritten = work.carve(&mut ctx.ids, s, e)?;
        work.placements.insert(id.clone(), resized);
        return commit(state, BTreeMap::from([(track_id, work)]), || {
            Output::Resize { overwritten }
        });
    }
    let (start, length, offset) = extent(&p)?;
    let end = start + length;
    let to = to.get();
    let (new_start, new_length, new_offset) = match edge {
        Edge::End => {
            if to <= start {
                return Err(invalid("to", "the end must stay after the start"));
            }
            (start, to - start, offset)
        }
        Edge::Start => {
            if to >= end {
                return Err(invalid("to", "the start must stay before the end"));
            }
            let delta = to
                .checked_sub(start)
                .filter(|_| to >= 0)
                .ok_or_else(|| invalid("to", "the start lies inside the timeline"))?;
            let geo = work
                .geo
                .get(&p.clip)
                .copied()
                .ok_or_else(|| not_found("clip", &p.clip))?;
            let moved = if delta >= 0 {
                geo.position(offset + delta)
            } else {
                (offset + delta >= 0).then_some(offset + delta)
            };
            let moved = moved.ok_or_else(|| {
                invalid(
                    "to",
                    "there is no content of the clip there: the start cannot move before the \
                     clip's start or past its end",
                )
            })?;
            (to, end - to, moved)
        }
    };
    check_extent(new_start, new_length, "to")?;
    let overwritten = work.carve(&mut ctx.ids, new_start, new_start + new_length)?;
    work.put(id.clone(), &p.clip, new_start, new_length, new_offset);
    commit(state, BTreeMap::from([(track_id, work)]), || {
        Output::Resize { overwritten }
    })
}

/// An audio placement with one edge moved to the tick `to`, in frames of its clip and kept
/// inside the clip: the end edge stops at the clip's last frame, the start edge at its first
/// (offset 0). The start edge keeps the end where it is: shortening skips the frames whose
/// ticks fit before `to`, growing shows earlier frames (as many as end by the old end).
fn resize_audio(
    work: &TrackWork,
    p: &Placement,
    edge: Edge,
    to: i64,
) -> Result<Placement, CommandError> {
    let Span::Frames { length, offset } = p.span else {
        return Err(invalid("placement", "not an audio placement"));
    };
    let g = *work
        .audio
        .get(&p.clip)
        .ok_or_else(|| not_found("clip", &p.clip))?;
    let (s, e) = work.span(p)?;
    let (len, o) = (frame_i64(length), frame_i64(offset));
    match edge {
        Edge::End => {
            if to <= s {
                return Err(invalid("to", "the end must stay after the start"));
            }
            let kept = frames_before(work.tempo, g.rate, to - s, g.frames - o);
            if kept < 1 {
                return Err(invalid(
                    "to",
                    "not even one frame of the clip fits before that tick",
                ));
            }
            Ok(Placement {
                span: Span::Frames {
                    length: frame_of(kept),
                    offset,
                },
                ..p.clone()
            })
        }
        Edge::Start => {
            if to >= e {
                return Err(invalid("to", "the start must stay before the end"));
            }
            if to < 0 {
                return Err(invalid("to", "the start lies inside the timeline"));
            }
            if to >= s {
                return work.part(p, to, e).ok_or_else(|| {
                    invalid(
                        "to",
                        "not even one frame of the clip fits between that tick and the end",
                    )
                });
            }
            // Growing to the left: the frames that play between `to` and the old start, at
            // most the ones before the offset, and never so many that the end moves on.
            let mut grow = frames_before(work.tempo, g.rate, s - to, o);
            while grow > 0 && to + audio_ticks(work.tempo, g.rate, len + grow) > e {
                grow -= 1;
            }
            let start = if grow == o && o > 0 {
                // Back to the clip's first frame: it starts where that frame falls, the end
                // kept.
                (e - audio_ticks(work.tempo, g.rate, len + o)).max(to)
            } else {
                to
            };
            Ok(Placement {
                clip: p.clip.clone(),
                start: Tick(start),
                span: Span::Frames {
                    length: frame_of(len + grow),
                    offset: frame_of(o - grow),
                },
            })
        }
    }
}

fn split(
    state: &mut State,
    ctx: &mut Context,
    id: &PlacementId,
    at: Tick,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let track_id = track_of(project, id)?;
    let mut work = TrackWork::new(project, &track_id)?;
    let p = placement_of(
        project
            .tracks
            .get(&track_id)
            .ok_or_else(|| not_found("track", &track_id))?,
        id,
    )?
    .clone();
    if let Span::Frames { .. } = p.span {
        let (s, e) = work.span(&p)?;
        let at = at.get();
        if !(s + 1..e).contains(&at) {
            return Err(invalid("at", "split strictly inside the placement"));
        }
        let (Some(left), Some(right)) = (work.part(&p, s, at), work.part(&p, at, e)) else {
            return Err(invalid(
                "at",
                "a part would hold not even one frame of the clip: split further from the edges",
            ));
        };
        let right_id = PlacementId::generate(&mut ctx.ids);
        work.placements.insert(id.clone(), left);
        work.placements.insert(right_id.clone(), right);
        return commit(state, BTreeMap::from([(track_id, work)]), || {
            Output::Split {
                placement: right_id,
            }
        });
    }
    let (start, length, offset) = extent(&p)?;
    let at = at.get();
    if !(start + 1..start + length).contains(&at) {
        return Err(invalid("at", "split strictly inside the placement"));
    }
    let geo = work
        .geo
        .get(&p.clip)
        .copied()
        .ok_or_else(|| not_found("clip", &p.clip))?;
    let right_offset = geo
        .position(offset + (at - start))
        .ok_or_else(|| invalid("at", "the clip has nothing left to play at that tick"))?;
    let right = PlacementId::generate(&mut ctx.ids);
    work.put(id.clone(), &p.clip, start, at - start, offset);
    work.put(
        right.clone(),
        &p.clip,
        at,
        start + length - at,
        right_offset,
    );
    commit(state, BTreeMap::from([(track_id, work)]), || {
        Output::Split { placement: right }
    })
}

fn delete(state: &mut State, ids: &[PlacementId]) -> Result<Out, CommandError> {
    let project = &state.project;
    let groups = group_by_track(project, ids, "placements")?;
    let mut works = Works::new();
    for (track_id, gone) in groups {
        // An audio placement goes like a MIDI one: its clip and its blob stay.
        let mut work = TrackWork::new(project, &track_id)?;
        for id in &gone {
            work.placements.remove(id);
        }
        works.insert(track_id, work);
    }
    commit(state, works, || Output::Delete {
        deleted: count(ids.len()),
    })
}

fn duplicate(
    state: &mut State,
    ctx: &mut Context,
    ids: &[PlacementId],
    linked: bool,
    by: Option<Tick>,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let groups = group_by_track(project, ids, "placements")?;
    // The selection's extent, for the default shift.
    let (mut first, mut last) = (i64::MAX, i64::MIN);
    for (track_id, list) in &groups {
        let track = project
            .tracks
            .get(track_id)
            .ok_or_else(|| not_found("track", track_id))?;
        let work = TrackWork::of(track, track_id, project.tempo());
        for id in list {
            let (s, e) = work.span(placement_of(track, id)?)?;
            first = first.min(s);
            last = last.max(e);
        }
    }
    let by = by.map_or(last - first, Tick::get);
    let mut works = Works::new();
    let mut overwritten = 0;
    let mut new_placements: BTreeMap<PlacementId, PlacementId> = BTreeMap::new();
    let mut new_clip_ids = Vec::new();
    // The names of the copies made so far: the project does not hold them yet.
    let mut notes_added = 0usize;
    let notes_in_project = project_notes(project);
    for (track_id, list) in groups {
        let track = project
            .tracks
            .get(&track_id)
            .ok_or_else(|| not_found("track", &track_id))?;
        let mut work = TrackWork::new(project, &track_id)?;
        // One copy per distinct source clip of the track, in order of first use.
        let mut copies: BTreeMap<ClipId, ClipId> = BTreeMap::new();
        if !linked {
            for id in &list {
                let source = &placement_of(track, id)?.clip;
                if copies.contains_key(source) {
                    continue;
                }
                let clip = track
                    .clips
                    .get(source)
                    .ok_or_else(|| not_found("clip", source))?;
                let notes = notes_of(clip);
                notes_added = notes_added.saturating_add(notes);
                check_total(
                    "clips per track",
                    track.clips.len(),
                    copies.len() + 1,
                    MAX_CLIPS_PER_TRACK,
                )?;
                check_total(
                    "clips",
                    project.counts().clips,
                    new_clip_ids.len() + 1,
                    project::limits::MAX_CLIPS,
                )?;
                check_note_limits(notes, notes_in_project.saturating_add(notes_added))?;
                let copy_id = ClipId::generate(&mut ctx.ids);
                let mut copy = clip.clone();
                copy.slot = None;
                work.add_clip(copy_id.clone(), copy);
                new_clip_ids.push(copy_id.clone());
                copies.insert(source.clone(), copy_id);
            }
        }
        for id in &list {
            let p = placement_of(track, id)?;
            let copy = Placement {
                clip: copies.get(&p.clip).unwrap_or(&p.clip).clone(),
                ..TrackWork::shifted(p, by, "by")?
            };
            let (s, e) = work.checked_span(&copy, "by")?;
            overwritten += work.carve(&mut ctx.ids, s, e)?;
            let new_id = PlacementId::generate(&mut ctx.ids);
            work.placements.insert(new_id.clone(), copy);
            new_placements.insert(id.clone(), new_id);
        }
        works.insert(track_id, work);
    }
    commit(state, works, || Output::Duplicate {
        // In the order of the list.
        placements: ids
            .iter()
            .filter_map(|id| new_placements.get(id).cloned())
            .collect(),
        clips: new_clip_ids,
        overwritten,
    })
}

/// `arrangement.repeat`: linked copies of one placement, end to end, up to `until`.
fn repeat(
    state: &mut State,
    ctx: &mut Context,
    id: &PlacementId,
    until: i64,
) -> Result<Out, CommandError> {
    let project = &state.project;
    let track_id = track_of(project, id)?;
    let track = project
        .tracks
        .get(&track_id)
        .ok_or_else(|| not_found("track", &track_id))?;
    let mut work = TrackWork::new(project, &track_id)?;
    let source = placement_of(track, id)?.clone();
    let (start, end) = work.span(&source)?;
    let length = end - start;
    if until <= end || until > MAX_TICK {
        return Err(invalid(
            "until",
            format!("repeat up to a tick after the placement's end ({end}) and within {MAX_TICK}"),
        ));
    }
    // The count first, so a far `until` is refused before any copy is made.
    let copies = (until - end + length - 1) / length;
    check_total(
        "placements per track",
        track.placements.len(),
        usize::try_from(copies).unwrap_or(usize::MAX),
        MAX_PLACEMENTS_PER_TRACK,
    )?;
    let mut made = Vec::new();
    // The copies tile `[end, until)`: one carve for the whole range (a carve per copy would
    // scan the track once for each of up to a hundred thousand copies).
    let overwritten = work.carve(&mut ctx.ids, end, until)?;
    let mut at = end;
    while at < until {
        let copy = Placement {
            start: Tick(at),
            ..source.clone()
        };
        // The last one stops at `until` (an audio one at the last whole frame before it).
        let last = at + length > until;
        let copy = if last {
            match work.part(&copy, at, until) {
                Some(part) => part,
                None => break,
            }
        } else {
            copy
        };
        let (_, e) = work.checked_span(&copy, "until")?;
        let new_id = PlacementId::generate(&mut ctx.ids);
        work.placements.insert(new_id.clone(), copy);
        made.push(new_id);
        if last {
            break;
        }
        at = e;
    }
    let mut works = Works::new();
    works.insert(track_id, work);
    commit(state, works, || Output::Repeat {
        placements: made,
        overwritten,
    })
}

fn restore(state: &mut State, edits: &[TrackEdit]) -> Result<Out, CommandError> {
    if edits.is_empty() {
        return Err(invalid("edits", "name at least one track"));
    }
    let works = restore_works(&state.project, edits)?;
    commit(state, works, || Output::Restore {})
}

/// The tracks of a restore as they will be, checked entirely.
fn restore_works(project: &Project, edits: &[TrackEdit]) -> Result<Works, CommandError> {
    let mut tracks = HashSet::new();
    let mut fresh: HashSet<project::Id> = HashSet::new();
    let mut works = Works::new();
    let notes_in_project = project_notes(project);
    for (i, edit) in edits.iter().enumerate() {
        let at = format!("edits[{i}]");
        if !tracks.insert(&edit.track) {
            return Err(invalid(format!("{at}.track"), "a track is listed twice"));
        }
        let track = project
            .tracks
            .get(&edit.track)
            .ok_or_else(|| not_found("track", &edit.track))?;
        // A MIDI track takes back MIDI clips (`clips`), an audio track audio ones
        // (`audio_clips`, AU-6: the copies the arrangement commands make).
        let audio = track.kind == project::TrackKind::Audio;
        if audio && !edit.clips.is_empty() {
            return Err(invalid(
                format!("{at}.clips"),
                "an audio track takes back audio clips: give them in `audio_clips`",
            ));
        }
        if !audio && !edit.audio_clips.is_empty() {
            return Err(invalid(
                format!("{at}.audio_clips"),
                "a MIDI track holds no audio clip",
            ));
        }
        let mut work = TrackWork::new(project, &edit.track)?;
        let mut removed = HashSet::new();
        for id in &edit.remove {
            if !removed.insert(id) || work.placements.remove(id).is_none() {
                return Err(invalid(
                    format!("{at}.remove"),
                    format!("placement {id} is not a placement of the track, or is listed twice"),
                ));
            }
        }
        let mut gone = HashSet::new();
        for id in &edit.remove_clips {
            if !gone.insert(id) || !track.clips.contains_key(id) {
                return Err(invalid(
                    format!("{at}.remove_clips"),
                    format!("clip {id} is not a clip of the track, or is listed twice"),
                ));
            }
            work.geo.remove(id);
            work.audio.remove(id);
            work.gone_clips.push(id.clone());
        }
        let mut slots: BTreeSet<&SceneId> = BTreeSet::new();
        for (k, rec) in edit.audio_clips.iter().enumerate() {
            let field = format!("{at}.audio_clips[{k}]");
            let snap = &rec.snapshot;
            if project.contains_id(&rec.id.0) || !fresh.insert(rec.id.0.clone()) {
                return Err(invalid(
                    format!("{field}.id"),
                    format!("the identifier {} is already in use", rec.id),
                ));
            }
            check_name(&snap.name, &format!("{field}.snapshot.name"))?;
            crate::clip::check_audio(snap, &format!("{field}.snapshot"))?;
            check_total(
                "clips per track",
                track.clips.len() - gone.len() + k,
                1,
                MAX_CLIPS_PER_TRACK,
            )?;
            check_total(
                "clips",
                project.counts().clips + k,
                1,
                project::limits::MAX_CLIPS,
            )?;
            if let Some(scene) = &snap.slot {
                check_slot(project, track, scene, &format!("{field}.snapshot.slot"))?;
                if !slots.insert(scene) {
                    return Err(invalid(
                        format!("{field}.snapshot.slot"),
                        "two clips sit in the same slot",
                    ));
                }
            }
            work.add_clip(rec.id.clone(), crate::clip::audio_clip_of(snap));
        }
        let mut notes_added = 0usize;
        for (k, rec) in edit.clips.iter().enumerate() {
            let field = format!("{at}.clips[{k}]");
            let snap = &rec.snapshot;
            if project.contains_id(&rec.id.0) || !fresh.insert(rec.id.0.clone()) {
                return Err(invalid(
                    format!("{field}.id"),
                    format!("the identifier {} is already in use", rec.id),
                ));
            }
            check_name(&snap.name, &format!("{field}.snapshot.name"))?;
            crate::clip::check_length(snap.length, &format!("{field}.snapshot.length"))?;
            check_loop(
                &snap.looping,
                snap.length,
                &format!("{field}.snapshot.looping"),
            )?;
            crate::clip::check_notes(&snap.notes, snap.length, &format!("{field}.snapshot.notes"))?;
            let notes: BTreeSet<Note> = snap.notes.iter().copied().collect();
            if notes.len() != snap.notes.len() {
                return Err(invalid(
                    format!("{field}.snapshot.notes"),
                    "a note appears twice",
                ));
            }
            notes_added = notes_added.saturating_add(notes.len());
            check_note_limits(notes.len(), notes_in_project.saturating_add(notes_added))?;
            check_total(
                "clips per track",
                track.clips.len() - gone.len() + k,
                1,
                MAX_CLIPS_PER_TRACK,
            )?;
            check_total(
                "clips",
                project.counts().clips + k,
                1,
                project::limits::MAX_CLIPS,
            )?;
            if let Some(scene) = &snap.slot {
                check_slot(project, track, scene, &format!("{field}.snapshot.slot"))?;
                if !slots.insert(scene) {
                    return Err(invalid(
                        format!("{field}.snapshot.slot"),
                        "two clips sit in the same slot",
                    ));
                }
            }
            work.geo.insert(
                rec.id.clone(),
                Geo {
                    length: snap.length.get(),
                    looping: snap.looping.map(|l| (l.start.get(), l.end.get())),
                },
            );
            work.new_clips.insert(
                rec.id.clone(),
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
        }
        let mut put = HashSet::new();
        let mut audio_put: Vec<(PlacementId, String)> = Vec::new();
        for (k, rec) in edit.put.iter().enumerate() {
            let field = format!("{at}.put[{k}]");
            if !put.insert(&rec.id) {
                return Err(invalid(&field, "a placement is listed twice"));
            }
            let existed = track.placements.contains_key(&rec.id);
            if !existed && (project.contains_id(&rec.id.0) || !fresh.insert(rec.id.0.clone())) {
                return Err(invalid(
                    format!("{field}.id"),
                    format!("the identifier {} is already in use", rec.id),
                ));
            }
            if audio {
                // A clip of the track that stays, or one this restore brings back.
                let clip = work.new_clips.get(&rec.clip).or_else(|| {
                    track
                        .clips
                        .get(&rec.clip)
                        .filter(|_| !gone.contains(&rec.clip))
                });
                let placement = audio_placement(project, clip, rec, &field)?;
                work.placements.insert(rec.id.clone(), placement);
                audio_put.push((rec.id.clone(), field));
                continue;
            }
            if rec.frames.is_some() {
                return Err(invalid(
                    format!("{field}.frames"),
                    "a MIDI track holds no audio placement",
                ));
            }
            let geo = work
                .geo
                .get(&rec.clip)
                .ok_or_else(|| not_found("clip", &rec.clip))?;
            check_extent(rec.start.get(), rec.length.get(), &field)?;
            if !(0..geo.length).contains(&rec.offset.get()) {
                return Err(invalid(&field, "a placement's offset is inside the clip"));
            }
            work.put(
                rec.id.clone(),
                &rec.clip,
                rec.start.get(),
                rec.length.get(),
                rec.offset.get(),
            );
        }
        // A restored audio placement may not overlap another one of the track, restored or
        // already there: the arrangement commands never leave an overlap, so an undo never
        // brings one.
        for (id, field) in &audio_put {
            let Some((s, e)) = work.placements.get(id).and_then(|p| work.span(p).ok()) else {
                continue;
            };
            let clash = work.placements.iter().any(|(other, p)| {
                other != id && work.span(p).is_ok_and(|(os, oe)| os < e && s < oe)
            });
            if clash {
                return Err(invalid(
                    field,
                    "the placement would overlap another placement of the track",
                ));
            }
        }
        if let Some(p) = work.placements.values().find(|p| !work.knows(&p.clip)) {
            return Err(invalid(
                format!("{at}.remove_clips"),
                format!("a placement is left on the removed clip {}", p.clip),
            ));
        }
        works.insert(edit.track.clone(), work);
    }
    Ok(works)
}

/// The audio placement a restore record describes, checked like `clip.restore_audio`
/// checks one: an audio clip of the track (`clip`, one it keeps or one the restore brings
/// back), a span of at least one frame that ends on the timeline, inside the clip.
fn audio_placement(
    project: &Project,
    clip: Option<&Clip>,
    rec: &PlacementRecord,
    field: &str,
) -> Result<Placement, CommandError> {
    let Some(span) = rec.frames else {
        return Err(invalid(
            format!("{field}.frames"),
            "an audio track holds audio placements: give `frames`",
        ));
    };
    let Some(ClipContent::Audio { media, frames, .. }) = clip.map(|c| &c.content) else {
        return Err(invalid(
            format!("{field}.clip"),
            format!("clip {} is not an audio clip of the track", rec.clip),
        ));
    };
    if rec.length.get() != 0 || rec.offset.get() != 0 {
        return Err(invalid(
            field,
            "an audio placement is given in `frames`: `length` and `offset` stay 0",
        ));
    }
    let length = project::Frame::new(span.length);
    let end = rec
        .start
        .saturating_add(project.tempo().frames_to_ticks(length, media.sample_rate));
    if !(0..=MAX_TICK).contains(&rec.start.get())
        || end.get() > MAX_TICK
        || span.length == 0
        || span.length > project::limits::MAX_FRAME
    {
        return Err(invalid(field, "a placement lies on the timeline"));
    }
    if span.offset >= frames.get() {
        return Err(invalid(field, "a placement's offset is inside the clip"));
    }
    if span.offset.saturating_add(span.length) > frames.get() {
        return Err(invalid(
            field,
            "a placement ends inside the clip: `offset + length` is at most the clip's frames",
        ));
    }
    Ok(Placement {
        clip: rec.clip.clone(),
        start: rec.start,
        span: Span::Frames {
            length,
            offset: project::Frame::new(span.offset),
        },
    })
}

/// `[start, start + length)` lies in `[0, 2^40]`, with a length of at least 1.
fn check_cut(start: i64, length: i64, field: &str) -> Result<(), CommandError> {
    match start.checked_add(length) {
        Some(end) if start >= 0 && length >= 1 && end <= MAX_TICK => Ok(()),
        _ => Err(invalid(
            field,
            format!("the cut lies in [0, {MAX_TICK}] ticks, with a length of at least 1"),
        )),
    }
}

/// A track-by-track plan of a time edit: the tracks that hold placements are worked on, the
/// others are left alone.
fn time_works(
    project: &Project,
    mut edit: impl FnMut(&mut TrackWork) -> Result<(), CommandError>,
) -> Result<Works, CommandError> {
    let mut works = Works::new();
    for (id, track) in &project.tracks {
        if track.placements.is_empty() {
            continue;
        }
        let mut work = TrackWork::new(project, id)?;
        edit(&mut work)?;
        works.insert(id.clone(), work);
    }
    Ok(works)
}

type Table = BTreeMap<Tick, AutomationPoint>;

/// Every automation lane of the project, the master's first.
fn all_lanes(project: &Project) -> impl Iterator<Item = (&LaneId, &project::AutomationLane)> + '_ {
    project
        .master_automation
        .iter()
        .chain(project.tracks.values().flat_map(|t| t.automation.iter()))
}

/// The lanes whose points `map` changes, with their new tables, and the sums of the
/// `(removed, moved)` counts that `map` returns.
fn time_lanes(
    project: &Project,
    map: impl Fn(&Table) -> Result<(Table, u32, u32), CommandError>,
) -> Result<(BTreeMap<LaneId, Table>, u32, u32), CommandError> {
    let (mut removed, mut moved) = (0u32, 0u32);
    let mut lanes = BTreeMap::new();
    for (id, lane) in all_lanes(project) {
        let (table, r, m) = map(&lane.points)?;
        if table != lane.points {
            lanes.insert(id.clone(), table);
        }
        removed = removed.saturating_add(r);
        moved = moved.saturating_add(m);
    }
    Ok((lanes, removed, moved))
}

fn insert_time(
    state: &mut State,
    ctx: &mut Context,
    at: i64,
    length: i64,
) -> Result<Out, CommandError> {
    check_cut(at, length, "length")?;
    let project = &state.project;
    let (mut split, mut shifted) = (0u32, 0u32);
    let works = time_works(project, |work| {
        let (s, m) = work.open_gap(&mut ctx.ids, at, length)?;
        split = split.saturating_add(s);
        shifted = shifted.saturating_add(m);
        Ok(())
    })?;
    let (lanes, _, points_shifted) = time_lanes(project, |points| {
        let mut table = Table::new();
        let mut moved = 0u32;
        for (tick, point) in points {
            let t = tick.get();
            if t >= at {
                let to = t
                    .checked_add(length)
                    .filter(|n| *n <= MAX_TICK)
                    .ok_or_else(|| {
                        invalid(
                            "length",
                            format!("an automation point would pass {MAX_TICK} ticks"),
                        )
                    })?;
                table.insert(Tick(to), *point);
                moved += 1;
            } else {
                table.insert(*tick, *point);
            }
        }
        Ok((table, 0, moved))
    })?;
    let transport = match project.transport.loop_region {
        None => None,
        Some(region) => {
            let (s, e) = (region.start().get(), region.end().get());
            let s = if s >= at { s + length } else { s };
            let e = if e > at { e + length } else { e };
            if e > MAX_TICK {
                return Err(invalid(
                    "length",
                    format!("the loop region would pass {MAX_TICK} ticks"),
                ));
            }
            Some((
                LoopRegion::new(Tick(s), Tick(e)),
                project.transport.loop_enabled,
            ))
        }
    };
    let loop_changed = transport.is_some_and(|t| t.0 != project.transport.loop_region);
    // The section markers from the point on move with the time (#473).
    let mut markers = BTreeMap::new();
    for (id, marker) in &project.markers {
        let t = marker.tick.get();
        if t >= at {
            let to = t
                .checked_add(length)
                .filter(|n| *n <= MAX_TICK)
                .ok_or_else(|| {
                    invalid(
                        "length",
                        format!("a section marker would pass {MAX_TICK} ticks"),
                    )
                })?;
            markers.insert(id.clone(), Tick(to));
        }
    }
    commit_time(
        state,
        works,
        Extra {
            lanes,
            transport,
            markers,
        },
        || Output::InsertTime {
            split,
            shifted,
            points_shifted,
            loop_changed,
        },
    )
}

fn delete_time(
    state: &mut State,
    ctx: &mut Context,
    start: i64,
    end: i64,
) -> Result<Out, CommandError> {
    if end <= start {
        return Err(invalid("to", "`to` must be after `from`"));
    }
    // Checked: the two ticks come from the caller and may be extreme.
    let length = end
        .checked_sub(start)
        .ok_or_else(|| invalid("to", "the cut is too long"))?;
    check_cut(start, length, "to")?;
    let project = &state.project;
    let (mut removed, mut cut, mut shifted) = (0u32, 0u32, 0u32);
    let works = time_works(project, |work| {
        let (r, c, m) = work.close_gap(&mut ctx.ids, start, end)?;
        removed = removed.saturating_add(r);
        cut = cut.saturating_add(c);
        shifted = shifted.saturating_add(m);
        Ok(())
    })?;
    let (lanes, points_removed, points_shifted) = time_lanes(project, |points| {
        let mut table = Table::new();
        let (mut gone, mut moved) = (0u32, 0u32);
        for (tick, point) in points {
            let t = tick.get();
            if t < start {
                table.insert(*tick, *point);
            } else if t < end {
                gone += 1;
            } else {
                table.insert(Tick(t - length), *point);
                moved += 1;
            }
        }
        Ok((table, gone, moved))
    })?;
    // A boundary inside the range falls to its start; a loop left empty is cleared and
    // switched off (a loop needs a region).
    let transport = project.transport.loop_region.map(|region| {
        let fall = |x: i64| {
            if x <= start {
                x
            } else if x >= end {
                x - length
            } else {
                start
            }
        };
        let moved = LoopRegion::new(
            Tick(fall(region.start().get())),
            Tick(fall(region.end().get())),
        );
        (moved, project.transport.loop_enabled && moved.is_some())
    });
    let loop_changed = transport.is_some_and(|t| {
        t.0 != project.transport.loop_region || t.1 != project.transport.loop_enabled
    });
    // The section markers after the cut move back; one inside it goes to its start (#473).
    let markers = project
        .markers
        .iter()
        .filter(|(_, m)| m.tick.get() > start)
        .map(|(id, m)| {
            let t = m.tick.get();
            (id.clone(), Tick(if t >= end { t - length } else { start }))
        })
        .collect();
    commit_time(
        state,
        works,
        Extra {
            lanes,
            transport,
            markers,
        },
        || Output::DeleteTime {
            removed,
            cut,
            shifted,
            points_removed,
            points_shifted,
            loop_changed,
        },
    )
}

fn restore_time(
    state: &mut State,
    edits: &[TrackEdit],
    lanes: &[LanePoints],
    transport: Option<LoopState>,
    markers: &[MarkerTick],
) -> Result<Out, CommandError> {
    if edits.is_empty() && lanes.is_empty() && transport.is_none() && markers.is_empty() {
        return Err(invalid(
            "edits",
            "give placements, lanes, the loop or markers to restore",
        ));
    }
    let works = if edits.is_empty() {
        Works::new()
    } else {
        restore_works(&state.project, edits)?
    };
    let mut tables = BTreeMap::new();
    for l in lanes {
        let table = automation::verbatim_table(state, &l.lane, &l.points)?;
        if tables.insert(l.lane.clone(), table).is_some() {
            return Err(invalid("lanes", format!("lane {} is listed twice", l.lane)));
        }
    }
    let transport = match transport {
        None => None,
        Some(LoopState {
            loop_region,
            loop_enabled,
        }) => {
            let region = match loop_region {
                None => None,
                Some(l) => {
                    if l.start.get() < 0 || l.end.get() > MAX_TICK {
                        return Err(invalid(
                            "transport",
                            format!("a loop is within 0 to {MAX_TICK} ticks"),
                        ));
                    }
                    Some(
                        LoopRegion::new(l.start, l.end)
                            .ok_or_else(|| invalid("transport", "a loop needs start < end"))?,
                    )
                }
            };
            if loop_enabled && region.is_none() {
                return Err(invalid(
                    "transport",
                    "the loop cannot be on without a region",
                ));
            }
            Some((region, loop_enabled))
        }
    };
    let mut moves = BTreeMap::new();
    for m in markers {
        if moves.insert(m.marker.clone(), m.tick).is_some() {
            return Err(invalid(
                "markers",
                format!("marker {} is listed twice", m.marker),
            ));
        }
    }
    commit_time(
        state,
        works,
        Extra {
            lanes: tables,
            transport,
            markers: moves,
        },
        || Output::RestoreTime {},
    )
}
