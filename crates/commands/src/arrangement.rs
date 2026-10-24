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
//!   changes both. A placement has no loop of its own: when it is longer than a clip that
//!   has a loop, the clip's loop repeats inside it; a clip without a loop is silent after
//!   its end. Audio placements come with the audio import (AU) and are refused here.
//! - **The overlap rule: the last one put down wins.** The placement a command places,
//!   moves, resizes or copies keeps its span; the other placements of the track that it
//!   covers are carved, so a track never holds two placements at the same tick after one
//!   of these commands:
//!   - a placement fully under it is removed;
//!   - one that starts before it is shortened to end where it starts;
//!   - one that starts under it and ends after it is trimmed at the front: it starts
//!     where the winner ends, and its offset moves on by what was cut, followed through
//!     the clip's own loop (nothing is left of a clip without a loop that ended in the
//!     cut part: the placement is then removed);
//!   - one that contains it is split in two: the left part keeps the identifier, the
//!     right part is a new placement.
//!
//!   The engine still cuts any table at the next start (AR-2), so a project that holds an
//!   overlap from elsewhere plays as "the last started wins". `clip.create` into the
//!   arrangement refuses an overlap, and `clip.set_length` stops a growing placement at
//!   the next one: a note edit never carves.
//! - Moving, deleting and copying take a list, so a selection is one command, one
//!   history entry, one undo. A group is lifted first and put down in the order of its
//!   list: a placement may move onto the old place of another one of the group.
//! - A clip that lives in the arrangement only (no launch-grid slot) goes with its last
//!   placement, in the same command: nothing in the interface would reach it, and it would
//!   weigh on the limits and the engine. The undo brings it back with its notes. A clip of
//!   the launch grid stays. `arrangement.place` refuses a clip of the grid: copying or
//!   linking it is `arrangement.place_from_slot` (AR-5, founder's decision 11).
//! - Every command is atomic: it is checked entirely before anything changes. A command
//!   that would change nothing changes nothing and leaves no history entry.
//! - The inverse of every command is `arrangement.restore`, computed from what actually
//!   changed (the placements, and the clips that were created or removed), so an undo
//!   gives back the carved neighbours exactly, with their identifiers.

use crate::clip::{
    ClipSnapshot, check_loop, check_name, check_note_limits, check_slot, check_total, invalid,
    locate, midi_ref, midi_track, not_found, project_notes, snapshot,
};
use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use project::limits::{MAX_CLIPS_PER_TRACK, MAX_PLACEMENTS, MAX_PLACEMENTS_PER_TRACK, MAX_TICK};
use project::{
    Clip, ClipContent, ClipId, IdGenerator, Note, Placement, PlacementId, Project, SceneId, Span,
    Tick, Track, TrackId,
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
    /// A MIDI clip of the track.
    pub clip: ClipId,
    /// Start in the arrangement, in ticks.
    pub start: Tick,
    /// Length on the timeline, in ticks.
    pub length: Tick,
    /// Where in the clip the placement starts, in ticks, before the clip's length.
    pub offset: Tick,
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
    /// Clips to bring back (their identifiers must be unused).
    pub clips: Vec<ClipRecord>,
    /// Clips of the track to remove; no placement may be left on them.
    pub remove_clips: Vec<ClipId>,
}

domain! {
    domain = "arrangement", schema = ("ArrangementCommand", "ArrangementOutput");

    /// Put a MIDI clip of the arrangement again in the arrangement, as a new placement of
    /// the same clip (a linked clip: editing the clip changes every placement of it). A
    /// clip that sits in the launch grid is refused: copying or linking it is
    /// `arrangement.place_from_slot`. `start` is in
    /// ticks; `offset` (default 0, before the clip's length) is where in the clip the
    /// placement starts; `length` (default: what is left of the clip after `offset`) is
    /// its length on the timeline, which may exceed the clip (a clip with a loop repeats
    /// it, one without is silent after its end). The new placement wins on overlap: the
    /// other placements of the track that it covers are removed, shortened, trimmed at
    /// the front or split (`overwritten` counts them). Undo: `arrangement.restore`.
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
        /// How many clips of the arrangement lost their last placement and were deleted.
        clips_removed: u32,
    };

    /// Move placements in time, all by the same number of ticks (`by`, negative moves
    /// them earlier), in one command: a drag of a selection is one history entry. Each
    /// placement stays in its own track and keeps its length and offset. The moved
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
        /// How many clips of the arrangement lost their last placement and were deleted.
        clips_removed: u32,
    };

    /// Move one edge of a placement to the timeline tick `to`. `edge: "end"` changes the
    /// length (the end must stay after the start); `edge: "start"` moves the start with
    /// the end fixed, and the offset follows so the content does not slide: shortening
    /// from the left cuts the beginning of the clip (through the clip's loop), growing to
    /// the left shows earlier content and is refused beyond the clip's start (offset 0).
    /// The resized placement wins on overlap (the others it now covers are carved,
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
        /// How many clips of the arrangement lost their last placement and were deleted.
        clips_removed: u32,
    };

    /// Split a placement in two at the timeline tick `at`, strictly inside it: the left
    /// part keeps the identifier, the right part is a new placement (the same clip, so
    /// linked) that plays on from where the left one stops. Refused when the clip has
    /// nothing left to play at `at` (a clip without a loop that ended before). Undo:
    /// `arrangement.restore`.
    "arrangement.split" (kind = "edit", undo = "inverse", destructive = false)
    Split { placement: PlacementId, at: Tick } => {
        /// The right part.
        placement: PlacementId,
    };

    /// Delete placements, in one command. A clip that sits in the launch grid or has
    /// other placements stays; a clip of the arrangement only that loses its last
    /// placement is deleted with it (the undo brings it back with its notes). Refused when the
    /// list is empty, names a placement twice or an unknown one. Undo:
    /// `arrangement.restore`.
    "arrangement.delete" (kind = "edit", undo = "inverse", destructive = false)
    Delete { placements: Vec<PlacementId> } => {
        /// How many placements were deleted.
        deleted: u32,
        /// How many clips of the arrangement lost their last placement and were deleted.
        clips_removed: u32,
    };

    /// Copy placements, in one command. The copies start `by` ticks after the originals
    /// (default: right after the selection, one selection length later; negative copies
    /// earlier). By default each copy is independent: a new clip, a copy of the source
    /// (the next name, "Clip 2" gives "Clip 3"; same colour, length, loop and notes; not
    /// in the launch grid), one per
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
        /// How many clips of the arrangement lost their last placement and were deleted.
        clips_removed: u32,
    };

    /// Write the placements of one or more tracks exactly as given, without the overlap
    /// rule: the inverse of every arrangement command, which brings back what it carved,
    /// with the identifiers. Per track: `put` creates or replaces placements by
    /// identifier, `remove` removes placements, `clips` brings back MIDI clips (new
    /// identifiers, a snapshot as `clip.restore` takes) and `remove_clips` removes clips.
    /// Refused when a placement is outside the timeline or its offset outside its clip,
    /// when a placement is left on a removed clip or names an unknown clip, when an
    /// identifier to create is in use, or when a rule of the clip commands is broken (a
    /// note outside the length, a limit). Undo: the restore of what it changed.
    "arrangement.restore" (kind = "edit", undo = "inverse", destructive = false)
    Restore { edits: Vec<TrackEdit> } => {};
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
    /// round the loop. `None` when a clip without a loop has ended.
    fn position(self, e: i64) -> Option<i64> {
        match self.looping {
            None => (e < self.length).then_some(e),
            Some((ls, le)) if e >= le => Some(ls + (e - le).rem_euclid(le - ls)),
            Some(_) => Some(e),
        }
    }
}

/// One track while a command works on it: the placements as they will be, the clips it
/// creates or removes, and the geometry of its clips.
struct TrackWork {
    track: TrackId,
    placements: Placements,
    geo: BTreeMap<ClipId, Geo>,
    new_clips: BTreeMap<ClipId, Clip>,
    gone_clips: Vec<ClipId>,
}

type Works = BTreeMap<TrackId, TrackWork>;

impl TrackWork {
    fn new(project: &Project, id: &TrackId) -> Result<Self, CommandError> {
        let track = midi_track(project, id)?;
        let mut geo = BTreeMap::new();
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
            }
        }
        Ok(Self {
            track: id.clone(),
            placements: track.placements.clone(),
            geo,
            new_clips: BTreeMap::new(),
            gone_clips: Vec::new(),
        })
    }

    /// Carves the placements that overlap `[start, end)`; returns how many it touched.
    fn carve(&mut self, ids: &mut IdGenerator, start: i64, end: i64) -> Result<u32, CommandError> {
        let hit: Vec<PlacementId> = self
            .placements
            .iter()
            .filter_map(|(id, p)| {
                let (s, l, _) = extent(p).ok()?;
                (s < end && start < s.saturating_add(l)).then(|| id.clone())
            })
            .collect();
        for id in &hit {
            let Some(p) = self.placements.remove(id) else {
                continue;
            };
            let (s, l, o) = extent(&p)?;
            let e = s + l;
            if s < start {
                self.placements
                    .insert(id.clone(), shaped(&p.clip, s, start - s, o));
            }
            if e > end {
                let offset = self
                    .geo
                    .get(&p.clip)
                    .and_then(|g| g.position(o.saturating_add(end - s)));
                if let Some(offset) = offset {
                    let right = if s < start {
                        PlacementId::generate(ids)
                    } else {
                        id.clone()
                    };
                    self.placements
                        .insert(right, shaped(&p.clip, end, e - end, offset));
                }
            }
        }
        Ok(count(hit.len()))
    }

    /// A clip that sat in the arrangement only (no launch-grid slot) and has lost its last
    /// placement is nowhere any more: no screen reaches it, yet it would weigh on the
    /// limits, the engine and the file. It goes with the placement; the restore brings it
    /// back with its notes.
    fn drop_unused(&mut self, track: &Track) {
        let now: HashSet<&ClipId> = self.placements.values().map(|p| &p.clip).collect();
        let before: HashSet<&ClipId> = track.placements.values().map(|p| &p.clip).collect();
        for id in before {
            let slotless = track.clips.get(id).is_some_and(|c| c.slot.is_none());
            if slotless && !now.contains(id) && !self.gone_clips.contains(id) {
                self.gone_clips.push(id.clone());
                self.geo.remove(id);
            }
        }
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
            "an audio placement is not edited in ticks; audio arrangement comes with the import",
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
    output: impl FnOnce(u32) -> Output,
) -> Result<Out, CommandError> {
    commit_with(state, works, output, true)
}

/// [`commit`], with or without the removal of the clips a command leaves unused (the
/// restore gives back exactly what it is told).
fn commit_with(
    state: &mut State,
    mut works: Works,
    output: impl FnOnce(u32) -> Output,
    drop_unused: bool,
) -> Result<Out, CommandError> {
    let project = &state.project;
    if drop_unused {
        for work in works.values_mut() {
            let track = project
                .tracks
                .get(&work.track)
                .ok_or_else(|| not_found("track", &work.track))?;
            work.drop_unused(track);
        }
    }
    let clips_removed = count(works.values().map(|w| w.gone_clips.len()).sum());
    let output = output(clips_removed);
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
        for id in &work.gone_clips {
            let clip = track.clips.get(id).ok_or_else(|| not_found("clip", id))?;
            clips.push(ClipRecord {
                id: id.clone(),
                snapshot: snapshot(clip)?,
            });
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
            remove_clips: work.new_clips.keys().cloned().collect(),
        };
        if edit.put.is_empty()
            && edit.remove.is_empty()
            && edit.clips.is_empty()
            && edit.remove_clips.is_empty()
        {
            continue;
        }
        changed_tracks.push(work.track.clone());
        inverse.push(edit);
    }
    if inverse.is_empty() {
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
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::Restore { edits: inverse })),
        events,
        control: vec![],
    })
}

fn record(id: &PlacementId, p: &Placement) -> Result<PlacementRecord, CommandError> {
    let (start, length, offset) = extent(p)?;
    Ok(PlacementRecord {
        id: id.clone(),
        clip: p.clip.clone(),
        start: Tick(start),
        length: Tick(length),
        offset: Tick(offset),
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
        Command::Move { placements, by } => move_placements(state, ctx, placements, *by),
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
        Command::Restore { edits } => restore(state, edits),
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
    commit(state, BTreeMap::from([(track_id, work)]), |clips_removed| {
        Output::Place {
            placement: id,
            overwritten,
            clips_removed,
        }
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
            let (start, length, offset) = extent(&p)?;
            let start = start
                .checked_add(by.get())
                .ok_or_else(|| invalid("by", "the placement leaves the timeline"))?;
            check_extent(start, length, "by")?;
            overwritten += work.carve(&mut ctx.ids, start, start + length)?;
            work.put(id, &p.clip, start, length, offset);
        }
        works.insert(track_id, work);
    }
    commit(state, works, |clips_removed| Output::Move {
        overwritten,
        clips_removed,
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
    commit(state, BTreeMap::from([(track_id, work)]), |clips_removed| {
        Output::Resize {
            overwritten,
            clips_removed,
        }
    })
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
    commit(state, BTreeMap::from([(track_id, work)]), |_| {
        Output::Split { placement: right }
    })
}

fn delete(state: &mut State, ids: &[PlacementId]) -> Result<Out, CommandError> {
    let project = &state.project;
    let groups = group_by_track(project, ids, "placements")?;
    let mut works = Works::new();
    for (track_id, gone) in groups {
        let mut work = TrackWork::new(project, &track_id)?;
        for id in &gone {
            work.placements.remove(id);
        }
        works.insert(track_id, work);
    }
    commit(state, works, |clips_removed| Output::Delete {
        deleted: count(ids.len()),
        clips_removed,
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
        let track = midi_track(project, track_id)?;
        for id in list {
            let (s, l, _) = extent(placement_of(track, id)?)?;
            first = first.min(s);
            last = last.max(s + l);
        }
    }
    let by = by.map_or(last - first, Tick::get);
    let mut works = Works::new();
    let mut overwritten = 0;
    let mut new_placements: BTreeMap<PlacementId, PlacementId> = BTreeMap::new();
    let mut new_clip_ids = Vec::new();
    // The names of the copies made so far: the project does not hold them yet.
    let mut new_names: Vec<String> = Vec::new();
    let mut notes_added = 0usize;
    let notes_in_project = project_notes(project);
    for (track_id, list) in groups {
        let track = midi_track(project, &track_id)?;
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
                let (_, _, notes) = midi_ref(clip)?;
                notes_added = notes_added.saturating_add(notes.len());
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
                check_note_limits(notes.len(), notes_in_project.saturating_add(notes_added))?;
                let copy_id = ClipId::generate(&mut ctx.ids);
                let mut copy = clip.clone();
                copy.name = project.copy_clip_name(&clip.name, &new_names);
                new_names.push(copy.name.clone());
                copy.slot = None;
                work.new_clips.insert(copy_id.clone(), copy);
                if let Some(geo) = work.geo.get(source).copied() {
                    work.geo.insert(copy_id.clone(), geo);
                }
                new_clip_ids.push(copy_id.clone());
                copies.insert(source.clone(), copy_id);
            }
        }
        for id in &list {
            let p = placement_of(track, id)?;
            let (start, length, offset) = extent(p)?;
            let start = start
                .checked_add(by)
                .ok_or_else(|| invalid("by", "the copy leaves the timeline"))?;
            check_extent(start, length, "by")?;
            let clip = copies.get(&p.clip).unwrap_or(&p.clip).clone();
            overwritten += work.carve(&mut ctx.ids, start, start + length)?;
            let new_id = PlacementId::generate(&mut ctx.ids);
            work.put(new_id.clone(), &clip, start, length, offset);
            new_placements.insert(id.clone(), new_id);
        }
        works.insert(track_id, work);
    }
    commit(state, works, |clips_removed| Output::Duplicate {
        clips_removed,
        // In the order of the list.
        placements: ids
            .iter()
            .filter_map(|id| new_placements.get(id).cloned())
            .collect(),
        clips: new_clip_ids,
        overwritten,
    })
}

fn restore(state: &mut State, edits: &[TrackEdit]) -> Result<Out, CommandError> {
    let project = &state.project;
    if edits.is_empty() {
        return Err(invalid("edits", "name at least one track"));
    }
    let mut tracks = HashSet::new();
    let mut fresh: HashSet<project::Id> = HashSet::new();
    let mut works = Works::new();
    let notes_in_project = project_notes(project);
    for (i, edit) in edits.iter().enumerate() {
        let at = format!("edits[{i}]");
        if !tracks.insert(&edit.track) {
            return Err(invalid(format!("{at}.track"), "a track is listed twice"));
        }
        let track = midi_track(project, &edit.track)?;
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
            work.gone_clips.push(id.clone());
        }
        let mut slots: BTreeSet<&SceneId> = BTreeSet::new();
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
        if let Some(p) = work
            .placements
            .values()
            .find(|p| !work.geo.contains_key(&p.clip))
        {
            return Err(invalid(
                format!("{at}.remove_clips"),
                format!("a placement is left on the removed clip {}", p.clip),
            ));
        }
        works.insert(edit.track.clone(), work);
    }
    commit_with(state, works, |_| Output::Restore {}, false)
}
