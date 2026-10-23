//! The arrangement in the engine (AR-2, `docs/specs/02-decoupage-mvp.md` § 2.2 and the AR-2
//! entry): each track plays its own timeline, the placements of its clips, in step with the
//! transport, until the launch grid takes the track over ([`crate::clip_player`]).
//!
//! # What a track reads
//!
//! A track's arrangement is a content table of [`Placement`]s (`TrackPlacementsUpload`),
//! sorted by start by `prepare` (which also drops the placements of no length and the audio
//! ones, whose reading is AU-4's). A placement does not copy any note: it reads the notes
//! of its clip, in the clip's own table, through the MO-4 cursor, exactly as the clip player
//! reads a launched clip ([`crate::clip_player::read_pieces`]); two placements of the same
//! clip (linked clips) cost one table.
//!
//! The placement `{ clip, start, span: Ticks { offset, length } }` plays the clip from its
//! tick `offset` for `length` ticks, from the transport tick `start`: the clip's position
//! at the transport tick `t` is `offset + (t − start)`, followed through the clip's own
//! loop region when it has one (a placement longer than a looped clip repeats the loop; a
//! clip without a loop is silent after its end). A note is delivered when its start is in
//! `[offset, offset + length)`, and the notes still held at the end of the placement are
//! released **at the sample of the end tick**, each by its own note-off. Placements do not
//! overlap (the arrangement commands, AR-1, carve the placements a new one covers); the
//! engine does not trust that:
//! the effective end of a placement is the least of its own end and the start of the next
//! one, so the later start wins and the earlier is cut, whatever the table holds.
//!
//! # The rule of a start in the middle of a note
//!
//! **A note is started only at its own tick.** A read that begins inside a note (a play
//! from there, a seek, the wrap of the transport's region loop, a placement entered at an
//! `offset` that falls inside the note, the return from the grid by `BackToArrangement`,
//! see [`crate::launcher`]) does not start that note: it stays silent until its end, and its
//! note-off is dropped (the cursor's jump rule, MO-4). A note that starts exactly on the
//! first tick of the read starts on the first sample. So a note is never started late
//! (after its tick), never twice (a read that goes on from the previous one does not start
//! it again, and a jump never replays a note it did not deliver), and never left held: the
//! notes the previous read held are released by their own note-offs on the first sample of
//! the jump. A note that crosses the end of the transport's region loop is cut at the wrap
//! and starts again, at its own tick, only if the next pass delivers it.
//!
//! # The grid takes the track
//!
//! The arrangement plays while the grid does not hold the track. A launch (or a stop)
//! given from the grid runs at its quantization boundary: the arrangement plays up to the
//! boundary, its notes are released at the sample of the boundary, then the clip starts
//! there (a stop leaves the track silent). From then on the grid holds the track, also
//! after the clip has ended and after the transport has stopped, until
//! `BackToArrangement`: the arrangement then resumes at the transport position of that
//! block, by the rule above. A scene launch and `StopAll` leave a track that follows its
//! arrangement alone (a track with nothing to stop).
//!
//! # State and audio path
//!
//! [`ArrangementPlayer`] keeps the cursor and the placement being read (its slot and its
//! clip). The placement is derived from the transport tick at each read: a table swapped
//! while it plays needs no message, a placement that moved or vanished ends like a jump.
//! While it reads, the player holds the clip like the clip player does (`Clip::on_track`):
//! a clip another track holds is not played. When the sink fills, the read is cut and
//! resumes from the same tick ([`Cut`]). Nothing here allocates, locks or panics on the
//! audio path: every lookup is a binary search over a table of any content, and each step
//! of a read moves the tick forward.

use crate::clip_player::{Cut, Env, read_pieces};
use crate::engine::ClipContent;
use crate::events::{Cursor, Sink};
use crate::msg::{ClipSlot, Placement, PlacementSlot, Span};
use crate::time::Tick;
use crate::transport::Segment;

/// The placement being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Active {
    placement: PlacementSlot,
    clip: ClipSlot,
}

/// The arrangement player of a track (module doc).
#[derive(Debug, Default)]
pub struct ArrangementPlayer {
    cursor: Cursor,
    active: Option<Active>,
}

/// What the table holds at a transport tick.
enum Located {
    /// A placement plays here until its effective end.
    In {
        placement: Placement,
        offset: Tick,
        end: Tick,
    },
    /// Nothing plays here until `until` (`None`: ever).
    Gap { until: Option<Tick> },
}

/// The effective end of placement `i` of `table` (module doc); its start when it has no
/// tick length.
fn effective_end(table: &[Placement], i: usize) -> Tick {
    let Some(p) = table.get(i) else {
        return Tick(i64::MIN);
    };
    let Span::Ticks { length, .. } = p.span else {
        return p.start;
    };
    let own = p.start.saturating_add(length);
    match table.get(i + 1) {
        Some(next) => own.min(next.start),
        None => own,
    }
}

/// What plays at tick `t`: the last placement that started at or before it, if it has not
/// ended. A binary search: an unsorted table gives some answer, never a panic.
fn locate(table: &[Placement], t: Tick) -> Located {
    let started = table.partition_point(|p| p.start <= t);
    if let Some(i) = started.checked_sub(1)
        && let Some(&placement) = table.get(i)
        && let Span::Ticks { offset, .. } = placement.span
    {
        let end = effective_end(table, i);
        if t < end {
            return Located::In {
                placement,
                offset,
                end,
            };
        }
    }
    // The next placement starts after `t` (a table out of order may say otherwise: the gap
    // is at least one tick, so that a read always moves on).
    let until = table
        .get(started)
        .map(|p| p.start.max(Tick(t.0.saturating_add(1))));
    Located::Gap { until }
}

/// Sorts the placements by start (then slot) and drops those the engine does not play: no
/// tick length, or a length in source frames (AU-4). A negative offset into the clip, which
/// no project writes, counts as 0. Off the audio path (`prepare`); no allocation.
pub fn prepare_table(placements: &mut Vec<Placement>) {
    placements.retain(|p| matches!(p.span, Span::Ticks { length, .. } if length.0 > 0));
    for p in placements.iter_mut() {
        if let Span::Ticks { offset, .. } = &mut p.span {
            offset.0 = offset.0.max(0);
        }
    }
    placements.sort_unstable_by_key(|p| (p.start.0, p.placement.0));
}

impl ArrangementPlayer {
    /// The clip the player reads, if any.
    pub fn clip(&self) -> Option<ClipSlot> {
        self.active.map(|a| a.clip)
    }

    /// Whether a placement is being read (or released): the player must run to end it.
    pub fn busy(&self) -> bool {
        self.active.is_some()
    }

    /// Notes of the placement being read delivered and not ended yet.
    pub fn held(&self) -> u32 {
        self.cursor.held()
    }

    /// `apply` replaced (`removed == false`) or removed the clip in `slot`. When the
    /// player reads it, its cursor starts over (the next read is a jump into the new table)
    /// and, when removed, the placement stops. Returns whether notes of the old table were
    /// held: their note-offs are gone with it, the caller releases the track.
    pub fn clip_changed(&mut self, slot: ClipSlot, removed: bool) -> bool {
        if self.active.is_none_or(|a| a.clip != slot) {
            return false;
        }
        let held = self.cursor.held() > 0;
        self.cursor = Cursor::default();
        if removed {
            self.active = None;
        }
        held
    }

    /// Ends the placement being read, its notes released at `at`; `false` when the sink
    /// filled first (call again).
    pub(crate) fn halt(&mut self, env: &mut Env<'_>, at: u32, sink: &mut Sink) -> bool {
        let Some(a) = self.active else {
            return true;
        };
        if let Some(clip) = env.clips.get_mut(a.clip.0) {
            let ours = clip.on_track == Some(env.track);
            if ours
                && let ClipContent::Notes(notes) = &mut clip.content
                && !self.cursor.stop_at(notes, at, sink)
            {
                return false;
            }
            if ours {
                clip.on_track = None;
            }
        }
        self.active = None;
        self.cursor = Cursor::default();
        true
    }

    /// Takes the clip of `placement`: a MIDI clip no other track holds. `false`: the
    /// placement stays silent.
    fn claim(&mut self, env: &mut Env<'_>, placement: &Placement) -> bool {
        let Some(clip) = env.clips.get_mut(placement.clip.0) else {
            return false;
        };
        if clip.on_track.is_some_and(|t| t != env.track) {
            return false;
        }
        let ClipContent::Notes(notes) = &mut clip.content else {
            return false;
        };
        // A clip left with bits set (it was removed from a track mid-note) starts clean.
        notes.clear_held();
        clip.on_track = Some(env.track);
        self.active = Some(Active {
            placement: placement.placement,
            clip: placement.clip,
        });
        self.cursor = Cursor::default();
        true
    }

    /// Reads the transport ticks `[p, end)` of `segment` for the sub-block that starts at
    /// frame `start` (module doc). `Err` when the sink filled: the sub-block keeps
    /// `frames` frames and the read goes on from `from`.
    pub(crate) fn read(
        &mut self,
        env: &mut Env<'_>,
        p: Tick,
        end: Tick,
        segment: &Segment,
        start: usize,
        sink: &mut Sink,
    ) -> Result<(), Cut> {
        let rel = |tick: Tick| (segment.offset_of(tick).max(start) - start) as u32;
        let mut t = p;
        while t < end {
            let (placement, offset, pend) = match locate(env.placements, t) {
                Located::In {
                    placement,
                    offset,
                    end: pend,
                } => (placement, offset, pend),
                Located::Gap { until } => {
                    // Nothing plays here: what was read ends at this tick.
                    let at = rel(t);
                    if !self.halt(env, at, sink) {
                        return Err(Cut {
                            frames: at as usize + 1,
                            from: t,
                        });
                    }
                    t = until.map_or(end, |u| u.min(end));
                    continue;
                }
            };
            let current = Active {
                placement: placement.placement,
                clip: placement.clip,
            };
            // Another placement than the one read (a jump, a table swapped, a clip that is
            // not ours any more): the one read ends here.
            let ours = self.active.is_none_or(|a| {
                a == current
                    && env
                        .clips
                        .get(a.clip.0)
                        .is_some_and(|c| c.on_track == Some(env.track))
            });
            if !ours {
                let at = rel(t);
                if !self.halt(env, at, sink) {
                    return Err(Cut {
                        frames: at as usize + 1,
                        from: t,
                    });
                }
            }
            let stop = pend.min(end);
            if self.active.is_none() && !self.claim(env, &placement) {
                t = stop;
                continue;
            }
            if let Some(clip) = env.clips.get_mut(placement.clip.0) {
                let (length, looping) = (clip.length, clip.looping);
                if let ClipContent::Notes(notes) = &mut clip.content {
                    let anchor = Tick(placement.start.0.saturating_sub(offset.0));
                    read_pieces(
                        &mut self.cursor,
                        notes,
                        (anchor, length, looping),
                        (t, stop),
                        segment,
                        start,
                        sink,
                    )?;
                    // A clip without a loop, shorter than its placement, ends with its last
                    // tick: a note that runs past it is released there, not at the end of
                    // the placement (the grid's rule for a clip played once).
                    let clip_end = anchor.saturating_add(length);
                    if looping.is_none() && clip_end < stop && self.cursor.held() > 0 {
                        let at = rel(clip_end.max(t));
                        if !self.cursor.stop_at(notes, at, sink) {
                            return Err(Cut {
                                frames: at as usize + 1,
                                from: clip_end.max(t),
                            });
                        }
                    }
                }
            }
            // A placement that ends here is ended by the next step: the gap or the next
            // placement at tick `stop` releases its notes on that tick's sample (or, when
            // `stop` is the end of the read, the next read does, on its first sample).
            t = stop;
        }
        Ok(())
    }
}
