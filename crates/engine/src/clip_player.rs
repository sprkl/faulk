//! The MIDI clip player (CL-2, `docs/specs/02-decoupage-mvp.md` § 2.2, § 2.3 and the
//! CL-2 entry): the note table of a clip, built off the audio path, and the per-track
//! player that launches a clip at the launch quantization boundary and plays it, looped or
//! once, in step with the transport.
//!
//! # A clip's notes
//!
//! [`ClipNotes`] is the content table of a MIDI clip, built by `prepare` from the last
//! chunk of a `ClipNotesUpload` ([`ClipNotes::try_new`], fallible allocations, sorted in
//! place) and swapped in by `apply` with the clip. It is a run of [`Events`] that MO-4's
//! [`Cursor`] reads exactly as it reads a [`crate::events::Schedule`]: groups of equal
//! offset, a cut sub-block when the sink is full, a jump that releases the held notes with
//! their own note-offs. It costs about what the project's notes cost on the wire: each note
//! is packed in 16 bytes ([`PackedNote`]), its two entries in the delivery order are one
//! `u32` each, and it has one held bit, so that the safety limits of CT-3 still fit the
//! engine's memory budget (test `limits_fit_under_the_wasm32_cap`). A note's length is
//! kept on 32 bits: beyond `u32::MAX` ticks (about 25 days at 120 BPM) it saturates.
//!
//! The note `i` of a clip has the voice id [`SCHEDULE_FIRST_VOICE`]` + i`: never one of the
//! live notes of its track (1 to 128). Its fine tuning (`cents`) reaches the device as the
//! note's `tune_cents`; `chance` and `repeat` are kept for #192, which plays them.
//!
//! # The player
//!
//! Each track has a [`ClipPlayer`]. It takes the track's launch order (`Track::launch`,
//! set by `apply` from `LaunchClip` and `StopTrack`) and runs it at the **boundary** of the
//! global launch quantization ([`crate::time::Quantize::next_boundary_at_or_after`] the
//! playhead, with the project's signature; `None` quantization runs it at once). The
//! boundary is computed at the first segment read after the order. When the playhead
//! jumps before reaching it (the transport loop wraps first, a seek), the order runs at
//! the jump: an order given before the end of the transport loop runs at its start, even
//! when the loop holds no boundary. While the transport is stopped the order waits; the
//! boundary then comes from the position the transport plays from.
//!
//! At the boundary `B`, the notes the current clip holds are released at the sample of `B`
//! (each with its note-off), and the new clip's position 0 is anchored at `B`. A stop
//! order, a grid slot without a clip, an audio clip, or a clip another track plays leave
//! the track silent from `B`.
//!
//! The position in the clip of the transport tick `t` is, with `e = t − B`:
//!
//! - a clip played once: `e`, for `0 ≤ e ≤ length` (a note-off at the clip's end is
//!   delivered), then the player stops;
//! - a looped clip, loop `[ls, le)`: `e` for `0 ≤ e < le` (the clip starts at its position
//!   0, an intro before the loop when `ls > 0`), then `ls + (e − le) mod (le − ls)`, the
//!   same formula for `e < 0`, so that a transport loop that wraps before `B` keeps the
//!   clip in phase.
//!
//! Each segment of the clock is cut at the clip's loop wraps into **pieces**; a piece is
//! read through the cursor with a segment shifted into the clip's time
//! ([`Segment::shifted`]: the same tick → sample map, so a note lands on the floor of its
//! fractional transport sample, TR-1's rule). The piece after a wrap starts at `ls` where
//! the cursor expected `le`: a jump, which releases the held notes (a note that crosses
//! the loop end) at the wrap's sample, each with its note-off, and moves the cursor to
//! `ls`.
//!
//! When the sink fills ([`Read::Full`]), the player keeps the transport tick of the piece
//! being read and the segment it was in: reading the same segment again, or the next
//! block's segment that continues it, goes on from that piece (the pieces already
//! delivered are not read twice); any other segment is a jump.
//!
//! `BackToArrangement` (GR-1, [`crate::launcher`]) is the one order that does not wait for a
//! boundary: the pending launch is dropped, even while the transport is stopped, and the
//! playing clip's notes are released at the first sample of the read, going on in the next
//! sub-blocks when the sink fills ([`ClipPlayer::engaged`] holds until it is done).
//! An order's boundary is never in the past of the block that takes it: it is computed from
//! the start of that block's segment at the earliest, also for a read that goes on from a
//! cut of the block before.
//!
//! A transport stop releases the held notes and stops the clip; a launch order not yet run
//! stays. A clip plays on one track at a time: a track whose launch names a clip another
//! track still plays stays silent. Tracks are read in slot order, so a clip released by a
//! track at a boundary can be taken at that boundary only by a track of a higher slot
//! (a clip sits in one track's grid slot: only raw messages, or linked clips later, meet
//! this). A clip removed or replaced while it plays is handled by `apply`
//! ([`ClipPlayer::clip_changed`]).
//!
//! # The grid and the arrangement (AR-2)
//!
//! A track plays its arrangement ([`crate::arrangement`]) until the grid takes it. A launch
//! order, or a stop, waits for its boundary like any order, and **the arrangement plays
//! until then**: at the boundary the arrangement's notes are released at the sample of the
//! boundary (and the clip's, if one plays), then the order runs and the grid **holds** the
//! track ([`ClipPlayer::taken`]): the clip plays, or, after a stop or when the clip ends or
//! is missing, the track is silent. The hold lasts across a stop of the transport, until
//! `BackToArrangement`, which releases the clip at once as before and gives the track to its
//! arrangement again, from the transport position of that read ([`crate::arrangement`]'s
//! rule for a read that starts inside a note). The release of the old content always
//! completes before the new clip starts: a sink that fills in between cuts the read, and the
//! boundary is retried from the cut. This matters when the clip is in the grid and in a
//! placement of the same track: both read the one table of notes and its held bits.
//!
//! Nothing here allocates, locks or panics on the audio path.

use crate::arrangement::ArrangementPlayer;
use crate::device::{VoiceId, clamp_tune_cents};
use crate::engine::{Clip, ClipContent, LaunchRequest, Paged};
use crate::events::{
    Cursor, Events, Read, SCHEDULE_FIRST_VOICE, Sink, Timed, TimedKind, check_voices, try_boxed,
};
use crate::msg::{ClipSlot, MsgError, Note, Placement, SceneSlot, Slots, TrackSlot};
use crate::time::{LoopRegion, Quantize, Tick, TimeSignature};
use crate::transport::Segment;

// ---------------------------------------------------------------- the note table

/// A note of a clip in 16 bytes: start on 64 bits (two halves), length on 32 bits
/// (saturated, at least 1), key, velocity, cents, chance and repeat packed in one word
/// (7 + 7 + 7 + 7 + 4 bits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackedNote {
    start_lo: u32,
    start_hi: u32,
    length: u32,
    bits: u32,
}

impl PackedNote {
    /// Packs `note`; every field is clamped to its range (the decoder already refused what
    /// is outside).
    pub fn pack(note: &Note) -> Self {
        let start = note.start.0 as u64;
        let length = note.length.0.clamp(1, i64::from(u32::MAX)) as u32;
        let pitch = u32::from(note.pitch.min(127));
        let velocity = u32::from(note.velocity.min(127));
        let cents = (i32::from(note.cents.clamp(-Note::MAX_CENTS, Note::MAX_CENTS)) + 64) as u32;
        let chance = u32::from(note.chance.min(Note::MAX_CHANCE));
        let repeat = u32::from(note.repeat.clamp(1, Note::MAX_REPEAT) - 1);
        PackedNote {
            start_lo: start as u32,
            start_hi: (start >> 32) as u32,
            length,
            bits: pitch | velocity << 7 | cents << 14 | chance << 21 | repeat << 28,
        }
    }

    /// The four words of the note, in its table.
    pub fn words(&self) -> [u32; 4] {
        [self.start_lo, self.start_hi, self.length, self.bits]
    }

    /// The note of four words of its table.
    pub fn from_words(w: [u32; 4]) -> Self {
        PackedNote {
            start_lo: w[0],
            start_hi: w[1],
            length: w[2].max(1),
            bits: w[3],
        }
    }

    pub fn start(&self) -> Tick {
        Tick((u64::from(self.start_hi) << 32 | u64::from(self.start_lo)) as i64)
    }

    /// At least 1 tick.
    pub fn length(&self) -> Tick {
        Tick(i64::from(self.length))
    }

    /// The tick of the note-off.
    pub fn end(&self) -> Tick {
        self.start().saturating_add(self.length())
    }

    pub fn pitch(&self) -> u8 {
        (self.bits & 0x7F) as u8
    }

    pub fn velocity(&self) -> u8 {
        (self.bits >> 7 & 0x7F) as u8
    }

    pub fn cents(&self) -> i8 {
        ((self.bits >> 14 & 0x7F) as i32 - 64) as i8
    }

    pub fn chance(&self) -> u8 {
        (self.bits >> 21 & 0x7F) as u8
    }

    pub fn repeat(&self) -> u8 {
        (self.bits >> 28 & 0xF) as u8 + 1
    }
}

/// The notes of a MIDI clip and their delivery order (module doc), in one allocation of
/// 32-bit words, so that a clip stays small in the clip table: the note count, then each
/// note's four words ([`PackedNote`]), then the order (two entries per note,
/// `index << 1 | 1` for its note-on and `index << 1` for its note-off, sorted by tick, then
/// note-offs before note-ons, then index: MO-4's order), then one held bit per note.
#[derive(Debug, Default)]
pub struct ClipNotes {
    data: Box<[u32]>,
}

/// Words of a packed note.
const NOTE_WORDS: usize = 4;

impl ClipNotes {
    /// Builds the table off the audio path ([`ClipNotes::try_with_capacity`], then
    /// [`ClipNotes::fill`]).
    pub fn try_new(notes: &[Note]) -> Result<Self, MsgError> {
        let mut table = Self::try_with_capacity(notes.len())?;
        table.fill(notes);
        Ok(table)
    }

    /// Allocates a table for `notes` notes, fallibly, so that `prepare` allocates before
    /// it changes anything. [`MsgError::LimitExceeded`] when the notes would need a voice
    /// id beyond 24 bits (never under `MAX_NOTES_PER_CLIP`).
    pub fn try_with_capacity(notes: usize) -> Result<Self, MsgError> {
        check_voices(SCHEDULE_FIRST_VOICE, notes)?;
        let words = notes
            .checked_mul(NOTE_WORDS + 2)
            .and_then(|w| w.checked_add(1 + notes.div_ceil(32)))
            .filter(|_| notes <= (u32::MAX / 2) as usize)
            .ok_or(MsgError::LimitExceeded {
                what: "notes per clip",
                limit: u64::from(u32::MAX / 2),
                requested: notes as u64,
            })?;
        let mut data = try_boxed(words, 0u32)?;
        data[0] = notes as u32;
        Ok(ClipNotes { data })
    }

    /// Packs `notes` (as many as the table holds) and sorts their entries in place: no
    /// allocation, no failure.
    pub fn fill(&mut self, notes: &[Note]) {
        let n = self.notes();
        let (packed, rest) = self.split_mut();
        for (words, note) in packed.as_chunks_mut::<NOTE_WORDS>().0.iter_mut().zip(notes) {
            *words = PackedNote::pack(note).words();
        }
        let (order, held) = rest.split_at_mut((2 * n).min(rest.len()));
        held.iter_mut().for_each(|w| *w = 0);
        for (i, o) in order.iter_mut().enumerate() {
            *o = i as u32;
        }
        let packed = &*packed;
        let key = |o: u32| {
            let (index, on) = ((o >> 1) as usize, o & 1);
            let tick = match note_at(packed, index) {
                Some(n) if on == 1 => n.start(),
                Some(n) => n.end(),
                None => Tick(i64::MAX),
            };
            (tick, on, o >> 1)
        };
        // In place: the index makes the order total.
        order.sort_unstable_by_key(|&o| key(o));
    }

    /// The packed notes, and the order with the held bits.
    fn split_mut(&mut self) -> (&mut [u32], &mut [u32]) {
        let n = self.notes();
        let (_, body) = self.data.split_at_mut(1.min(self.data.len()));
        body.split_at_mut((NOTE_WORDS * n).min(body.len()))
    }

    fn packed(&self) -> &[u32] {
        let n = self.notes();
        self.data.get(1..1 + NOTE_WORDS * n).unwrap_or(&[])
    }

    fn order(&self) -> &[u32] {
        let n = self.notes();
        let at = 1 + NOTE_WORDS * n;
        self.data.get(at..at + 2 * n).unwrap_or(&[])
    }

    fn held_range(&self) -> core::ops::Range<usize> {
        let n = self.notes();
        let at = 1 + (NOTE_WORDS + 2) * n;
        at..self.data.len().max(at)
    }

    /// Notes in the clip.
    pub fn notes(&self) -> usize {
        self.data.first().map_or(0, |&n| n as usize)
    }

    /// Note `index`, if any.
    pub fn note(&self, index: usize) -> Option<PackedNote> {
        note_at(self.packed(), index)
    }

    /// The entries in delivery order, as timed events (tests, tools).
    pub fn entries(&self) -> impl Iterator<Item = Timed> + '_ {
        (0..self.order().len()).filter_map(|i| self.timed(i))
    }

    /// Notes whose held bit is set.
    pub fn held_count(&self) -> u32 {
        self.held_bits().iter().map(|w| w.count_ones()).sum()
    }

    /// Clears every held bit (a clip whose last player left without releasing).
    pub fn clear_held(&mut self) {
        self.held_bits_mut().iter_mut().for_each(|w| *w = 0);
    }
}

/// Note `index` of a run of packed words.
fn note_at(packed: &[u32], index: usize) -> Option<PackedNote> {
    let at = index.checked_mul(NOTE_WORDS)?;
    let w = packed.get(at..at.checked_add(NOTE_WORDS)?)?;
    Some(PackedNote::from_words([w[0], w[1], w[2], w[3]]))
}

impl Events for ClipNotes {
    fn len(&self) -> usize {
        self.order().len()
    }

    fn tick(&self, i: usize) -> Tick {
        let Some(&o) = self.order().get(i) else {
            return Tick(i64::MAX);
        };
        match self.note((o >> 1) as usize) {
            Some(n) if o & 1 == 1 => n.start(),
            Some(n) => n.end(),
            None => Tick(i64::MAX),
        }
    }

    fn timed(&self, i: usize) -> Option<Timed> {
        let o = *self.order().get(i)?;
        let index = (o >> 1) as usize;
        let n = self.note(index)?;
        // Checked by `try_with_capacity`: every voice id fits.
        let voice = VoiceId::new(SCHEDULE_FIRST_VOICE.wrapping_add(index as u32))?;
        let key = n.pitch();
        Some(if o & 1 == 1 {
            Timed {
                tick: n.start(),
                kind: TimedKind::NoteOn {
                    voice,
                    key,
                    velocity: f32::from(n.velocity()) / 127.0,
                    tune_cents: clamp_tune_cents(f32::from(n.cents())),
                },
            }
        } else {
            Timed {
                tick: n.end(),
                kind: TimedKind::NoteOff {
                    voice,
                    key,
                    start: n.start(),
                },
            }
        })
    }

    fn key(&self, index: usize) -> u8 {
        self.note(index).map_or(0, |n| n.pitch())
    }

    fn first_voice(&self) -> u32 {
        SCHEDULE_FIRST_VOICE
    }

    fn held_bits(&self) -> &[u32] {
        let r = self.held_range();
        self.data.get(r).unwrap_or(&[])
    }

    fn held_bits_mut(&mut self) -> &mut [u32] {
        let r = self.held_range();
        self.data.get_mut(r).unwrap_or(&mut [])
    }
}

// ---------------------------------------------------------------- the player

/// The clip a player plays, the transport tick of its position 0 and the scene of the
/// grid cell it was launched from (what the `Feedback` reports; `apply` follows the clip
/// when it moves to another cell, [`ClipPlayer::moved`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Playing {
    pub clip: ClipSlot,
    pub anchor: Tick,
    pub scene: SceneSlot,
}

/// A launch order taken from the track, and its boundary once known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Queued {
    pub order: LaunchRequest,
    pub at: Option<Tick>,
}

/// What the player reads besides the track's own state.
pub struct Env<'a> {
    pub track: TrackSlot,
    pub clips: &'a mut Paged<Box<Clip>>,
    pub scenes: &'a Slots<ClipSlot>,
    /// The track's arrangement: sorted by start ([`crate::arrangement`]).
    pub placements: &'a [Placement],
    pub launch: &'a mut Option<LaunchRequest>,
    /// `BackToArrangement` asked for this track and not taken yet.
    pub back: &'a mut bool,
    pub quantize: Quantize,
    pub signature: TimeSignature,
}

/// A cut piece: the frames the sub-block keeps, and the transport tick to read again from.
pub(crate) struct Cut {
    pub(crate) frames: usize,
    pub(crate) from: Tick,
}

/// The clip player of a track (module doc).
#[derive(Debug, Default)]
pub struct ClipPlayer {
    playing: Option<Playing>,
    queued: Option<Queued>,
    /// `BackToArrangement`: the playing clip is released at once; set until the release
    /// is complete (a full sink finishes it at the next read).
    back_pending: bool,
    /// The grid holds the track: an order (a launch or a stop) ran at its boundary and
    /// `BackToArrangement` has not come since. The arrangement plays while this is false.
    taken: bool,
    /// The track's own timeline (AR-2).
    arrangement: ArrangementPlayer,
    cursor: Cursor,
    /// The segment `(from, to)` left unfinished by a `Read::Full`, and the transport tick
    /// of the piece to read again.
    cut: Option<(Tick, Tick, Tick)>,
    /// The `to` of the last segment read to its end: the next one continues it when it
    /// starts there.
    last: Option<Tick>,
}

impl ClipPlayer {
    /// The clip playing, if any.
    pub fn playing(&self) -> Option<Playing> {
        self.playing
    }

    /// The launch order waiting for its boundary, if any.
    pub fn queued(&self) -> Option<Queued> {
        self.queued
    }

    /// Notes of the playing clip delivered and not ended yet.
    pub fn held(&self) -> u32 {
        self.cursor.held()
    }

    /// Whether the player has the track: a clip plays or a launch waits (module doc).
    pub fn engaged(&self) -> bool {
        self.playing.is_some() || self.queued.is_some() || self.back_pending
    }

    /// Whether the grid holds the track (an order ran, `BackToArrangement` has not come
    /// since), whether or not a clip plays: the arrangement is silent.
    pub fn taken(&self) -> bool {
        self.taken
    }

    /// Whether the track's arrangement still has notes or a clip to give back: the player
    /// must run (at the next block) to release them.
    pub fn arrangement_busy(&self) -> bool {
        self.arrangement.busy()
    }

    /// The clip `clip` sits in cell `scene` of the grid now (`apply` of `SceneSetSlot`):
    /// when this player plays it, it is reported there.
    pub fn moved(&mut self, clip: ClipSlot, scene: SceneSlot) {
        if let Some(p) = &mut self.playing
            && p.clip == clip
        {
            p.scene = scene;
        }
    }

    /// `apply` replaced (`removed == false`) or removed the clip in `slot`. When this
    /// player plays it (from the grid or from the arrangement), its cursor starts over (the
    /// next read is a jump into the new table) and, when removed, the clip stops. Returns
    /// whether notes of the old table were held: their note-offs are gone with it, the
    /// caller releases the track.
    pub fn clip_changed(&mut self, slot: ClipSlot, removed: bool) -> bool {
        let mut held = false;
        if self.playing.is_some_and(|p| p.clip == slot) {
            held = self.cursor.held() > 0;
            self.cursor = Cursor::default();
            self.cut = None;
            if removed {
                self.playing = None;
            }
        }
        if self.arrangement.clip_changed(slot, removed) {
            held = true;
            self.cut = None;
        }
        held
    }

    /// The clip this player holds, as `on_track` must say: the one it plays from the grid,
    /// else the one its arrangement reads (a track that leaves its slot gives its clip
    /// back: `apply`).
    pub fn holding_clip(&self) -> Option<ClipSlot> {
        self.playing
            .map(|p| p.clip)
            .or_else(|| self.arrangement.clip())
    }

    /// Takes a new launch order from the track.
    fn take_launch(&mut self, env: &mut Env<'_>) {
        // `BackToArrangement` first, whatever the transport does: the order waiting for its
        // boundary is dropped (a launch given after it, in `launch`, is taken next).
        if core::mem::take(env.back) {
            self.back_pending = true;
            self.queued = None;
        }
        if let Some(order) = env.launch.take() {
            self.queued = Some(Queued { order, at: None });
        }
    }

    /// The clock is stopped for this block: releases the held notes at offset 0 and stops
    /// the clip (what does not fit goes on at the next block); a launch order waits.
    pub fn stop(&mut self, env: &mut Env<'_>, sink: &mut Sink) {
        self.take_launch(env);
        self.cut = None;
        self.last = None;
        if let Some(q) = &mut self.queued {
            q.at = None;
        }
        // What does not fit is released at the next block.
        if self.halt_all(env, 0, sink) && self.back_pending {
            self.back_pending = false;
            self.taken = false;
        }
    }

    /// Releases what the track plays, whichever side plays it, at `at`; `false` when the
    /// sink filled first (call again).
    fn halt_all(&mut self, env: &mut Env<'_>, at: u32, sink: &mut Sink) -> bool {
        self.arrangement.halt(env, at, sink) && self.halt(env, at, sink)
    }

    /// Stops the playing clip with its notes released at `at`; `false` when the sink
    /// filled first (call again).
    fn halt(&mut self, env: &mut Env<'_>, at: u32, sink: &mut Sink) -> bool {
        let Some(p) = self.playing else {
            return true;
        };
        if let Some(clip) = env.clips.get_mut(p.clip.0) {
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
        self.forget();
        true
    }

    /// No clip plays any more; the cursor starts over.
    fn forget(&mut self) {
        self.playing = None;
        self.cursor = Cursor::default();
    }

    /// Reads the events of `segment` into `sink` for the sub-block that starts at frame
    /// `start` (module doc), with [`crate::events::Schedule::read`]'s contract.
    pub fn read(
        &mut self,
        env: &mut Env<'_>,
        segment: &Segment,
        start: usize,
        sink: &mut Sink,
    ) -> Read {
        self.take_launch(env);
        let rel = |tick: Tick| (segment.offset_of(tick).max(start) - start) as u32;
        let cut = self.cut.take();
        let (from, jumped) = match cut {
            // The same segment read again, or the next block's that continues it.
            Some((f, t, piece)) if (segment.from == f && segment.to == t) || segment.from == t => {
                (piece, false)
            }
            // After a cut, any other segment is a jump.
            _ => (
                segment.from,
                cut.is_some() || self.last != Some(segment.from),
            ),
        };
        if self.back_pending {
            // Released at once, at the first sample of this read.
            if !self.halt(env, 0, sink) {
                self.cut = Some((segment.from, segment.to, from));
                return Read::Full(1);
            }
            // The grid lets go: the arrangement plays again from this read on.
            self.back_pending = false;
            self.taken = false;
        }
        if let Some(q) = &mut self.queued {
            q.at = match q.at {
                // From the start of this segment at the earliest: an order is never given
                // in the past of the block that takes it (a read that went on from a cut
                // of the block before starts earlier than its segment).
                None => env
                    .quantize
                    .next_boundary_at_or_after(from.max(segment.from), env.signature),
                // The playhead jumped before reaching the boundary (a transport loop that
                // wraps first, a seek): the order runs at the jump.
                Some(_) if jumped => Some(from),
                at => at,
            };
        }
        let mut p = from;
        loop {
            let switch = self.queued.and_then(|q| q.at).filter(|&b| b < segment.to);
            let end = switch.map_or(segment.to, |b| b.max(p));
            let read = if self.taken {
                self.read_clip(env, p, end, segment, start, sink)
            } else {
                self.arrangement.read(env, p, end, segment, start, sink)
            };
            if let Err(cut) = read {
                self.cut = Some((segment.from, segment.to, cut.from));
                return Read::Full(cut.frames);
            }
            p = end;
            if switch.is_none() {
                break;
            }
            // The boundary: the old clip's notes end here, the new clip starts here.
            let at = rel(p);
            if !self.halt_all(env, at, sink) {
                self.cut = Some((segment.from, segment.to, p));
                // The note-offs pushed sit at `at`: the sub-block keeps that frame.
                return Read::Full(at as usize + 1);
            }
            if let Some(q) = self.queued.take() {
                // The order takes the track from the arrangement, a stop included.
                self.taken = true;
                if let LaunchRequest::Clip(scene) = q.order {
                    self.start(env, scene, p);
                }
            }
        }
        self.last = Some(segment.to);
        // A clip played once ends after its last tick.
        if let Some(pl) = self.playing
            && let Some(clip) = env.clips.get(pl.clip.0)
            && clip.looping.is_none()
            && segment.to.0.saturating_sub(pl.anchor.0) > clip.length.0
        {
            let at = rel(Tick(pl.anchor.0.saturating_add(clip.length.0)));
            // What does not fit is released at the next read (the clip stays until then).
            self.halt(env, at, sink);
        }
        Read::Done
    }

    /// Starts the clip of `scene` at transport tick `at`, if it is a MIDI clip no other
    /// track plays.
    fn start(&mut self, env: &mut Env<'_>, scene: SceneSlot, at: Tick) {
        let Some(slot) = env
            .scenes
            .as_slice()
            .get(scene.0 as usize)
            .copied()
            .flatten()
        else {
            return;
        };
        let Some(clip) = env.clips.get_mut(slot.0) else {
            return;
        };
        if !matches!(clip.content, ClipContent::Notes(_))
            || clip.on_track.is_some_and(|t| t != env.track)
        {
            return;
        }
        clip.on_track = Some(env.track);
        if let ClipContent::Notes(notes) = &mut clip.content {
            // A clip left with bits set (it was removed from a track mid-note) starts clean.
            notes.clear_held();
        }
        self.playing = Some(Playing {
            clip: slot,
            anchor: at,
            scene,
        });
        self.cursor = Cursor::default();
    }

    /// Reads the playing clip over the transport ticks `[p, end)` of `segment`, piece by
    /// piece (module doc). `Err` when the sink filled.
    fn read_clip(
        &mut self,
        env: &mut Env<'_>,
        p: Tick,
        end: Tick,
        segment: &Segment,
        start: usize,
        sink: &mut Sink,
    ) -> Result<(), Cut> {
        let Some(pl) = self.playing else {
            return Ok(());
        };
        let track = env.track;
        let Some(clip) = env.clips.get_mut(pl.clip.0) else {
            // Removed: `apply` stopped it; nothing to read.
            self.forget();
            return Ok(());
        };
        if clip.on_track != Some(track) {
            self.forget();
            return Ok(());
        }
        let (length, looping) = (clip.length, clip.looping);
        let ClipContent::Notes(notes) = &mut clip.content else {
            return Ok(());
        };
        read_pieces(
            &mut self.cursor,
            notes,
            (pl.anchor, length, looping),
            (p, end),
            segment,
            start,
            sink,
        )
    }
}

/// Reads the transport ticks `[p, end)` of `segment` from the notes of a clip placed at
/// `anchor` (its position 0, the transport tick), piece by piece (module doc): the clip's
/// `length` and `looping` give the pieces. The shared reading of the grid's clip and of an
/// arrangement's placement ([`crate::arrangement`]). `Err` when the sink filled.
pub(crate) fn read_pieces(
    cursor: &mut Cursor,
    notes: &mut ClipNotes,
    (anchor, length, looping): (Tick, Tick, Option<LoopRegion>),
    (p, end): (Tick, Tick),
    segment: &Segment,
    start: usize,
    sink: &mut Sink,
) -> Result<(), Cut> {
    let mut t = p;
    while t < end {
        let Some((local, piece_end)) = piece(anchor, length, looping, t, end) else {
            break;
        };
        if let Some(local) = local {
            let base = t.0.saturating_sub(local.0);
            let shifted = segment.shifted(base, t, piece_end);
            if let Read::Full(frames) = cursor.read(notes, &shifted, start, sink) {
                return Err(Cut { frames, from: t });
            }
        }
        t = piece_end;
    }
    Ok(())
}

/// The piece of the clip that starts at transport tick `t` (module doc): the clip-local
/// tick of `t` (`None`: silent until the piece's end) and the transport tick where the
/// piece ends (at most `end`). `None` when the clip has nothing left to play.
fn piece(
    anchor: Tick,
    length: Tick,
    looping: Option<LoopRegion>,
    t: Tick,
    end: Tick,
) -> Option<(Option<Tick>, Tick)> {
    let e = t.0.saturating_sub(anchor.0);
    let at = |ticks: i64| Tick(t.0.saturating_add(ticks)).min(end);
    match looping {
        None => {
            if e < 0 {
                Some((None, at(e.saturating_neg())))
            } else if e <= length.0 {
                // Through the clip's last tick: a note-off at `length` is delivered.
                Some((
                    Some(Tick(e)),
                    at(length.0.saturating_sub(e).saturating_add(1)),
                ))
            } else {
                None
            }
        }
        Some(r) => {
            let (ls, le) = (r.start().0, r.end().0);
            if (0..le).contains(&e) {
                Some((Some(Tick(e)), at(le - e)))
            } else {
                let span = le - ls;
                let k = e.saturating_sub(le).rem_euclid(span);
                // Before the anchor, the piece also ends there: the clip's position 0.
                let rest = if e < 0 {
                    (span - k).min(e.saturating_neg())
                } else {
                    span - k
                };
                Some((Some(Tick(ls + k)), at(rest)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_packs_and_unpacks() {
        let n = Note {
            start: Tick(-5_000_000_000),
            length: Tick(1 << 40),
            pitch: 127,
            velocity: 1,
            cents: -50,
            chance: 37,
            repeat: 16,
        };
        let p = PackedNote::pack(&n);
        assert_eq!(p.start(), n.start);
        assert_eq!(p.length(), Tick(i64::from(u32::MAX)), "saturated");
        assert_eq!(
            (p.pitch(), p.velocity(), p.cents(), p.chance(), p.repeat()),
            (127, 1, -50, 37, 16)
        );
        let z = PackedNote::pack(&Note {
            length: Tick(0),
            cents: 50,
            ..Note::default()
        });
        assert_eq!(
            (z.length(), z.cents(), z.chance(), z.repeat()),
            (Tick(1), 50, 100, 1)
        );
    }

    #[test]
    fn the_order_is_mo4s() {
        let note = |start, length, pitch| Note {
            start: Tick(start),
            length: Tick(length),
            pitch,
            velocity: 100,
            ..Note::default()
        };
        // A note that ends where another starts: its note-off first.
        let c = ClipNotes::try_new(&[note(10, 5, 60), note(0, 10, 62), note(10, 1, 64)])
            .expect("built");
        let got: Vec<(i64, bool, u8)> = c
            .entries()
            .map(|t| match t.kind {
                TimedKind::NoteOn { key, .. } => (t.tick.0, true, key),
                TimedKind::NoteOff { key, .. } => (t.tick.0, false, key),
                TimedKind::Param { .. } => (t.tick.0, false, 0),
            })
            .collect();
        assert_eq!(
            got,
            [
                (0, true, 62),
                (10, false, 62),
                (10, true, 60),
                (10, true, 64),
                (11, false, 64),
                (15, false, 60)
            ]
        );
        for i in 1..c.len() {
            assert!(c.tick(i - 1) <= c.tick(i));
        }
        assert_eq!(c.tick(c.len()), Tick(i64::MAX));
        assert!(c.timed(c.len()).is_none());
    }

    #[test]
    fn pieces_follow_the_loop() {
        let r = LoopRegion::new(Tick(100), Tick(400));
        let a = Tick(1000);
        // Intro, then the loop.
        assert_eq!(
            piece(a, Tick(500), r, Tick(1000), Tick(5000)),
            Some((Some(Tick(0)), Tick(1400)))
        );
        assert_eq!(
            piece(a, Tick(500), r, Tick(1400), Tick(5000)),
            Some((Some(Tick(100)), Tick(1700)))
        );
        // Before the anchor: in phase with the loop.
        assert_eq!(
            piece(a, Tick(500), r, Tick(900), Tick(5000)),
            Some((Some(Tick(200)), Tick(1000)))
        );
        // Played once: silent before, through the last tick, then nothing.
        assert_eq!(
            piece(a, Tick(500), None, Tick(0), Tick(5000)),
            Some((None, Tick(1000)))
        );
        assert_eq!(
            piece(a, Tick(500), None, Tick(1200), Tick(5000)),
            Some((Some(Tick(200)), Tick(1501)))
        );
        assert_eq!(piece(a, Tick(500), None, Tick(1501), Tick(5000)), None);
    }

    #[test]
    fn pieces_saturate_and_always_move_on() {
        let (lo, hi) = (Tick(i64::MIN), Tick(i64::MAX));
        let r = LoopRegion::new(Tick(1), hi);
        for (anchor, length, looping, t) in [
            (hi, hi, None, lo),
            (lo, hi, None, Tick(0)),
            (lo, hi, r, Tick(5)),
            (hi, hi, r, lo),
            (Tick(0), Tick(-5), None, Tick(0)),
        ] {
            if let Some((_, end)) = piece(anchor, length, looping, t, hi) {
                assert!(end > t, "{anchor:?} {length:?} {t:?}");
            }
        }
    }
}
