//! Sample-accurate timed events (MO-4, `docs/specs/02-decoupage-mvp.md` § 2.2, § 2.3 and
//! § 2.5): the tick-dated tables a track plays, how the graph reads them into the events
//! of a block, and the sink that routes them to the devices of the track.
//!
//! # A track's events in a block
//!
//! The graph ([`crate::graph`]) renders a track in **sub-blocks**: one per segment of the
//! clock ([`Segment`], cut by TR-1 at every loop wrap), or the whole block when the clock
//! is stopped, and a segment is cut again when the track's [`Sink`] is full (below). A
//! sub-block never crosses a loop wrap or a seek, so the transport position moves linearly
//! within it: the device sees it in [`crate::device::BlockCtx`].
//!
//! The events of a sub-block are gathered in the [`Sink`], kept in two runs: the **note
//! events** (the release of a changed chain, the live notes, the release after a live
//! overflow, then the notes of the schedule) and the **parameter values**, each with the
//! device it is addressed to. Each device then receives the merge, by offset, of its own
//! parameter values (first at an equal offset) and, for an instrument or a MIDI effect, of
//! the note stream ([`Sink::merge_for`]); an audio effect receives its parameter values
//! only. That is CT-5's guarantee: sorted by offset, parameter values first, notes in the
//! order they were issued, offsets below the sub-block's length.
//!
//! # The schedule: a table, no queue
//!
//! A [`Schedule`] is a content table of tick-dated events ([`Timed`]), sorted by tick,
//! built off the audio path ([`Schedule::try_new`], fallible allocations only) and read by
//! the audio side with a cursor. It is what a clip player (CL-2), the arrangement (AR-2)
//! and the automation (AM-2) compile their tables into; until they land, nothing in the
//! production path fills it (no message carries one), and the tests set it through
//! `Engine::state_mut`.
//!
//! [`Schedule::read`] delivers the events of a segment's ticks `[from, to)`: each at
//! [`Segment::offset_of`] its tick (the floor of its fractional sample, TR-1's rule),
//! relative to the sub-block. Nothing goes through a bounded queue: the cursor is an index
//! into the table. The events are taken by **groups of equal offset**; when a group does
//! not fit in the sink, the read stops before it and says where the sub-block must end
//! ([`Read::Full`]): the graph renders up to there, empties the sink and reads again from
//! the same index. A group too large for an empty sink, at the first frame of a
//! sub-block, is delivered in part, and the sub-block is one frame long: the rest lands
//! one frame later (only beyond [`SCHEDULE_CHUNK`] note events or [`PARAM_CHUNK`]
//! parameter values on one sample). Every event of the table is delivered exactly once.
//!
//! # Jumps and stops
//!
//! A segment whose `from` is not the tick the cursor expects (the first read, a play, a
//! seek, a loop wrap) is a **jump**: the notes the schedule holds are released (one
//! `AllNotesOff` at the segment's first frame, only when it holds some), then the cursor
//! moves by binary search to the first event at or after `from`. A note-off whose note-on
//! was not delivered since the jump (its note started before `from`) is dropped. A block
//! with the clock stopped releases the held notes the same way and forgets the cursor.
//! That `AllNotesOff` also ends the live notes of the track held at that moment: releasing
//! each scheduled note on its own (a note that crosses the loop end cut cleanly) is CL-2's.
//!
//! # Voices
//!
//! A schedule's notes have the voice ids `first_voice + index`; the live notes have 1 to
//! 128 (their key, plus one): a schedule built with [`SCHEDULE_FIRST_VOICE`] never collides
//! with the live input of its track. The graph never merges two other note streams (the
//! output of a MIDI effect replaces the stream), so no other remapping is needed.
//!
//! Nothing here allocates, locks or panics on the audio path.

use crate::device::{Event, EventBuffer, VoiceId, clamp_tune_cents};
use crate::msg::{DeviceSlot, MsgError, ParamId, Table, limits};
use crate::time::Tick;
use crate::transport::Segment;

/// First voice id of a schedule that shares its track with the live input (whose voices
/// are 1 to 128).
pub const SCHEDULE_FIRST_VOICE: u32 = 129;

/// Note events of a sub-block beyond those of the live block: the room the schedule reads
/// into before the sub-block is cut.
pub const SCHEDULE_CHUNK: usize = 1024;

/// Parameter values of a sub-block before it is cut.
pub const PARAM_CHUNK: usize = 1024;

/// Note events the live block can bring to a track: every live event, the release of a
/// changed chain and the release after a live overflow.
pub const LIVE_EVENTS: usize = limits::LIVE_QUEUE_CAPACITY + 2;

/// Note events of a sub-block: the live block, the release of a jump, then the schedule.
pub const NOTE_ROOM: usize = LIVE_EVENTS + 1 + SCHEDULE_CHUNK;

// ---------------------------------------------------------------- the table

/// What a [`Timed`] entry does at its tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimedKind {
    /// A parameter value for the device in `device` of the track.
    Param {
        device: DeviceSlot,
        id: ParamId,
        value: f32,
    },
    /// The end of a note; `start` is the tick of its note-on.
    NoteOff {
        voice: VoiceId,
        key: u8,
        start: Tick,
    },
    /// The start of a note.
    NoteOn {
        voice: VoiceId,
        key: u8,
        velocity: f32,
        tune_cents: f32,
    },
}

/// An entry of a [`Schedule`]: an event at a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timed {
    pub tick: Tick,
    pub kind: TimedKind,
}

impl Timed {
    /// Order at an equal tick: parameter values, then note-offs, then note-ons.
    fn class(&self) -> u8 {
        match self.kind {
            TimedKind::Param { .. } => 0,
            TimedKind::NoteOff { .. } => 1,
            TimedKind::NoteOn { .. } => 2,
        }
    }

    /// The device event at `offset`.
    fn event(&self, offset: u32) -> Event {
        match self.kind {
            TimedKind::Param { id, value, .. } => Event::ParamValue { offset, id, value },
            TimedKind::NoteOff { voice, key, .. } => Event::NoteOff { offset, voice, key },
            TimedKind::NoteOn {
                voice,
                key,
                velocity,
                tune_cents,
            } => Event::NoteOn {
                offset,
                voice,
                key,
                velocity,
                tune_cents,
            },
        }
    }
}

/// A note to schedule: from `start` for `length` ticks (at least one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduledNote {
    pub start: Tick,
    pub length: Tick,
    pub key: u8,
    /// 0 to 1 (clamped; non-finite: 0).
    pub velocity: f32,
    /// Fine tuning in cents (clamped to the device contract's range; non-finite: 0).
    pub tune_cents: f32,
}

/// A parameter value to schedule for the device in `device` of the track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduledParam {
    pub tick: Tick,
    pub device: DeviceSlot,
    pub id: ParamId,
    pub value: f32,
}

/// Where a read of a segment stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// Every event of the segment is in the sink.
    Done,
    /// The sink is full: the sub-block must end this many frames after its start (at
    /// least 1), and the segment is read again from there.
    Full(usize),
}

/// A tick-sorted table of timed events and its read cursor (module doc).
#[derive(Debug)]
pub struct Schedule {
    table: Table<Timed>,
    /// Index of the next entry to deliver.
    next: usize,
    /// End of the group of equal offset delivered in part: entries `next..group_end` are
    /// delivered first, at the first frame of the next sub-block.
    group_end: usize,
    /// The tick the next segment starts at when the playhead moves on without a jump.
    expect: Option<Tick>,
    first_voice: u32,
    /// The key of each note, by index (a note-off of a release names it).
    keys: Box<[u8]>,
    /// One bit per note: delivered and not ended yet.
    held_bits: Box<[u64]>,
    held: u32,
    /// A release of the held notes is in progress (a jump or a stop).
    releasing: bool,
    /// Word of `held_bits` the release goes on from.
    release_word: usize,
}

fn try_boxed<T: Copy>(n: usize, value: T) -> Result<Box<[T]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| MsgError::OutOfMemory {
        bytes: n.saturating_mul(size_of::<T>()),
    })?;
    v.resize(n, value);
    Ok(v.into_boxed_slice())
}

impl Schedule {
    /// Builds the table off the audio path: each note gives a note-on at `start` and a
    /// note-off at `start + max(length, 1)`, with the voice id `first_voice + index`; at
    /// an equal tick, parameter values come first, then note-offs, then note-ons, each in
    /// the order given. Every allocation is fallible; a voice id of 0 or beyond 24 bits is
    /// [`MsgError::LimitExceeded`].
    pub fn try_new(
        notes: &[ScheduledNote],
        params: &[ScheduledParam],
        first_voice: u32,
    ) -> Result<Self, MsgError> {
        let count = notes
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(params.len()))
            .ok_or(MsgError::OutOfMemory { bytes: usize::MAX })?;
        let last_voice = u64::from(first_voice).saturating_add(notes.len() as u64);
        if first_voice == 0 || last_voice > u64::from(VoiceId::MAX) + 1 {
            return Err(MsgError::LimitExceeded {
                what: "schedule voice ids",
                limit: u64::from(VoiceId::MAX),
                requested: last_voice.saturating_sub(1),
            });
        }
        let mut entries: Vec<(u32, Timed)> = Vec::new();
        entries
            .try_reserve_exact(count)
            .map_err(|_| MsgError::OutOfMemory {
                bytes: count.saturating_mul(size_of::<(u32, Timed)>()),
            })?;
        let mut keys = try_boxed(notes.len(), 0u8)?;
        let held_bits = try_boxed(notes.len().div_ceil(64), 0u64)?;
        let mut seq = 0u32;
        for p in params {
            entries.push((
                seq,
                Timed {
                    tick: p.tick,
                    kind: TimedKind::Param {
                        device: p.device,
                        id: p.id,
                        value: p.value,
                    },
                },
            ));
            seq = seq.wrapping_add(1);
        }
        for (i, n) in notes.iter().enumerate() {
            // Checked above: every id is in 1..=VoiceId::MAX.
            let Some(voice) = VoiceId::new(first_voice + i as u32) else {
                continue;
            };
            let key = n.key.min(127);
            keys[i] = key;
            let velocity = if n.velocity.is_finite() {
                n.velocity.clamp(0.0, 1.0)
            } else {
                0.0
            };
            entries.push((
                seq,
                Timed {
                    tick: n.start,
                    kind: TimedKind::NoteOn {
                        voice,
                        key,
                        velocity,
                        tune_cents: clamp_tune_cents(n.tune_cents),
                    },
                },
            ));
            entries.push((
                seq,
                Timed {
                    tick: n.start.saturating_add(Tick(n.length.0.max(1))),
                    kind: TimedKind::NoteOff {
                        voice,
                        key,
                        start: n.start,
                    },
                },
            ));
            seq = seq.wrapping_add(1);
        }
        // In place (no allocation): the sequence number makes the order total.
        entries.sort_unstable_by(|(sa, a), (sb, b)| {
            (a.tick, a.class(), *sa).cmp(&(b.tick, b.class(), *sb))
        });
        let mut sorted = Vec::new();
        sorted
            .try_reserve_exact(count)
            .map_err(|_| MsgError::OutOfMemory {
                bytes: count.saturating_mul(size_of::<Timed>()),
            })?;
        sorted.extend(entries.into_iter().map(|(_, timed)| timed));
        let table = Table::from_vec(sorted);
        Ok(Schedule {
            table,
            next: 0,
            group_end: 0,
            expect: None,
            first_voice,
            keys,
            held_bits,
            held: 0,
            releasing: false,
            release_word: 0,
        })
    }

    /// The entries, in delivery order.
    pub fn entries(&self) -> &[Timed] {
        self.table.as_slice()
    }

    /// Notes delivered and not ended yet.
    pub fn held(&self) -> u32 {
        self.held
    }

    /// The index of a voice of this schedule.
    fn index(&self, voice: VoiceId) -> usize {
        voice.get().wrapping_sub(self.first_voice) as usize
    }

    fn is_held(&self, voice: VoiceId) -> bool {
        let i = self.index(voice);
        self.held_bits
            .get(i / 64)
            .is_some_and(|w| w & (1 << (i % 64)) != 0)
    }

    fn set_held(&mut self, voice: VoiceId, held: bool) {
        let i = self.index(voice);
        if let Some(w) = self.held_bits.get_mut(i / 64) {
            let bit = 1u64 << (i % 64);
            let was = *w & bit != 0;
            if held && !was {
                *w |= bit;
                self.held = self.held.saturating_add(1);
            } else if !held && was {
                *w &= !bit;
                self.held = self.held.saturating_sub(1);
            }
        }
    }

    /// Reads the events of `segment` into `sink` for the sub-block that starts at frame
    /// `start` of the block (module doc). No allocation, no panic.
    pub fn read(&mut self, segment: &Segment, start: usize, sink: &mut Sink) -> Read {
        let rel = |tick: Tick| (segment.offset_of(tick).max(start) - start) as u32;
        // The rest of a group delivered in part, at the first frame.
        if !self.deliver_pending(sink) {
            return Read::Full(1);
        }
        if self.expect != Some(segment.from) {
            let from = segment.from;
            self.releasing = self.held > 0;
            self.release_word = 0;
            self.next = self.table.as_slice().partition_point(|e| e.tick < from);
            self.group_end = 0;
            self.expect = Some(from);
        }
        if self.releasing {
            let at = (segment.offset.max(start) - start) as u32;
            if !self.release(at, sink) {
                return Read::Full((at as usize).max(1));
            }
        }
        while self.next < self.table.len() && self.table.as_slice()[self.next].tick < segment.to {
            let table = self.table.as_slice();
            let at = rel(table[self.next].tick);
            // The group of entries at this offset, and the room it needs.
            let mut end = self.next;
            let (mut notes, mut params) = (0usize, 0usize);
            while end < table.len() && table[end].tick < segment.to && rel(table[end].tick) == at {
                match table[end].kind {
                    TimedKind::Param { .. } => params += 1,
                    TimedKind::NoteOff { voice, .. } if !self.is_held(voice) => {}
                    _ => notes += 1,
                }
                end += 1;
            }
            if notes <= sink.note_room() && params <= sink.param_room() {
                while self.next < end {
                    let timed = self.table.as_slice()[self.next];
                    self.deliver(&timed, at, sink);
                    self.next += 1;
                }
                continue;
            }
            if at > 0 {
                return Read::Full(at as usize);
            }
            // Larger than the room at the first frame: as much as fits now, the rest at
            // the first frame of the next sub-block, even past a jump.
            self.group_end = end;
            self.deliver_pending(sink);
            return Read::Full(1);
        }
        self.expect = Some(segment.to);
        Read::Done
    }

    /// Delivers the rest of a group delivered in part, at offset 0; `false` when some is
    /// still left.
    fn deliver_pending(&mut self, sink: &mut Sink) -> bool {
        while self.next < self.group_end {
            let timed = self.table.as_slice()[self.next];
            if !self.deliver(&timed, 0, sink) {
                return false;
            }
            self.next += 1;
        }
        self.group_end = 0;
        true
    }

    /// Ends every held note with its own note-off at `at` (the live notes of the track are
    /// not touched); `false` when the sink is full before the end.
    fn release(&mut self, at: u32, sink: &mut Sink) -> bool {
        while self.release_word < self.held_bits.len() {
            let w = self.release_word;
            while self.held_bits[w] != 0 {
                let bit = self.held_bits[w].trailing_zeros() as usize;
                let i = w * 64 + bit;
                let Some(voice) = VoiceId::new(self.first_voice.wrapping_add(i as u32)) else {
                    self.held_bits[w] &= !(1 << bit);
                    continue;
                };
                let key = self.keys.get(i).copied().unwrap_or(0);
                if !sink.push_note(Event::NoteOff {
                    offset: at,
                    voice,
                    key,
                }) {
                    return false;
                }
                self.set_held(voice, false);
            }
            self.release_word += 1;
        }
        self.releasing = false;
        true
    }

    /// Pushes one entry at `at`; `false` (nothing pushed) when the sink has no room for
    /// it. A note-off of a note that is not held (released at a jump, or started before
    /// it) is dropped (`true`).
    fn deliver(&mut self, timed: &Timed, at: u32, sink: &mut Sink) -> bool {
        match timed.kind {
            TimedKind::Param { device, .. } => sink.push_param(device, timed.event(at)),
            TimedKind::NoteOff { voice, .. } => {
                if !self.is_held(voice) {
                    return true;
                }
                let pushed = sink.push_note(timed.event(at));
                if pushed {
                    self.set_held(voice, false);
                }
                pushed
            }
            TimedKind::NoteOn { voice, .. } => {
                let pushed = sink.push_note(timed.event(at));
                if pushed {
                    self.set_held(voice, true);
                }
                pushed
            }
        }
    }

    /// The clock is stopped for this block: delivers the rest of a group delivered in
    /// part, then releases the held notes at offset 0, each with its note-off, and forgets
    /// the cursor (the next read is a jump). What does not fit goes on at the next block.
    pub fn stop(&mut self, sink: &mut Sink) {
        if !self.deliver_pending(sink) {
            return;
        }
        if self.expect.is_some() {
            self.expect = None;
            self.releasing = self.held > 0;
            self.release_word = 0;
        }
        if self.releasing {
            self.release(0, sink);
        }
    }
}

// ---------------------------------------------------------------- the sink

/// The events of a track's sub-block, before they reach its devices (module doc): the note
/// events in one run, the parameter values with their device in another, both sorted by
/// offset. Fixed capacities ([`NOTE_ROOM`], [`PARAM_CHUNK`]), allocated off the audio path.
#[derive(Debug)]
pub struct Sink {
    notes: Box<[Event]>,
    notes_len: usize,
    params: Box<[(DeviceSlot, Event)]>,
    params_len: usize,
}

fn filled<T: Copy>(n: usize, value: T) -> Result<Box<[T]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| MsgError::OutOfMemory {
        bytes: n.saturating_mul(size_of::<T>()),
    })?;
    v.resize(n, value);
    Ok(v.into_boxed_slice())
}

impl Sink {
    /// A sink of [`NOTE_ROOM`] note events and [`PARAM_CHUNK`] parameter values.
    pub fn try_new() -> Result<Self, MsgError> {
        Self::try_with_capacity(NOTE_ROOM, PARAM_CHUNK)
    }

    /// A sink of `notes` note events and `params` parameter values.
    pub fn try_with_capacity(notes: usize, params: usize) -> Result<Self, MsgError> {
        let none = Event::AllNotesOff { offset: 0 };
        Ok(Sink {
            notes: filled(notes, none)?,
            notes_len: 0,
            params: filled(params, (DeviceSlot(0), none))?,
            params_len: 0,
        })
    }

    /// Empties the sink, before a sub-block.
    pub fn clear(&mut self) {
        self.notes_len = 0;
        self.params_len = 0;
    }

    /// Note events that still fit.
    pub fn note_room(&self) -> usize {
        self.notes.len() - self.notes_len
    }

    /// Parameter values that still fit.
    pub fn param_room(&self) -> usize {
        self.params.len() - self.params_len
    }

    /// Appends a note event (`false` when full). An offset earlier than the last one's is
    /// raised to it: the run stays sorted.
    pub fn push_note(&mut self, event: Event) -> bool {
        if self.notes_len >= self.notes.len() {
            return false;
        }
        let last = match self.notes_len {
            0 => 0,
            n => self.notes[n - 1].offset(),
        };
        self.notes[self.notes_len] = if event.offset() < last {
            event.at(last)
        } else {
            event
        };
        self.notes_len += 1;
        true
    }

    /// Appends a parameter value for `device` (`false` when full); kept sorted like the
    /// notes.
    pub fn push_param(&mut self, device: DeviceSlot, event: Event) -> bool {
        if self.params_len >= self.params.len() {
            return false;
        }
        let last = match self.params_len {
            0 => 0,
            n => self.params[n - 1].1.offset(),
        };
        let event = if event.offset() < last {
            event.at(last)
        } else {
            event
        };
        self.params[self.params_len] = (device, event);
        self.params_len += 1;
        true
    }

    /// The note events, sorted by offset, in the order they were pushed.
    pub fn notes(&self) -> &[Event] {
        &self.notes[..self.notes_len]
    }

    /// The parameter values and their devices, sorted by offset.
    pub fn params(&self) -> &[(DeviceSlot, Event)] {
        &self.params[..self.params_len]
    }

    /// Whether `device` has a parameter value in the sub-block.
    pub fn has_params_for(&self, device: DeviceSlot) -> bool {
        self.params().iter().any(|(d, _)| *d == device)
    }

    /// Writes into `out` the events of `device`: its parameter values merged with
    /// `stream` by offset, the parameter values first at an equal offset (CT-5). `out`
    /// is cleared first and closed ([`EventBuffer::finish`]) after.
    pub fn merge_for(&self, device: DeviceSlot, stream: &[Event], out: &mut EventBuffer) {
        out.clear();
        let mut params = self
            .params()
            .iter()
            .filter(|(d, _)| *d == device)
            .map(|(_, e)| *e)
            .peekable();
        let mut notes = stream.iter().copied().peekable();
        loop {
            let take_param = match (params.peek(), notes.peek()) {
                (Some(p), Some(n)) => p.offset() <= n.offset(),
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let next = if take_param {
                params.next()
            } else {
                notes.next()
            };
            if let Some(event) = next {
                out.push(event);
            }
        }
        out.finish();
    }
}

/// A scenario for the native/WASM parity test (`--features testing`): the test
/// instrument and effect of the device contract on one track, a schedule of notes (fine
/// tuning, overlapping, on block boundaries) and parameter values for both devices, a
/// loop, a seek, a stop, odd block sizes, at 44.1 kHz.
#[cfg(feature = "testing")]
pub mod testing {
    use super::*;
    use crate::device::testing::{GAIN, TONE, gain, tone};
    use crate::engine::cycle::Cycle;
    use crate::msg::{ControlMsg, TrackKind, TrackSlot, encode_stream};
    use crate::time::{LoopRegion, Quantize, Tempo, TimeSignature};

    /// Sample rate of the scenario.
    pub const SCENARIO_RATE: f32 = 44_100.0;
    /// Block sizes, in turn.
    pub const SCENARIO_SIZES: [usize; 7] = [64, 127, 480, 512, 1021, 33, 2048];
    /// Blocks rendered.
    pub const SCENARIO_BLOCKS: usize = 160;

    /// The scenario's schedule: overlapping notes with fine tuning, and parameter values
    /// for the instrument (slot 0) and the effect (slot 1).
    pub fn schedule() -> Result<Schedule, MsgError> {
        let mut notes = Vec::new();
        for i in 0..48i64 {
            notes.push(ScheduledNote {
                start: Tick(960 + i * 173),
                length: Tick(90 + (i % 5) * 211),
                key: 48 + (i % 24) as u8,
                velocity: 0.3 + (i % 7) as f32 * 0.1,
                tune_cents: (i % 9) as f32 * 25.0 - 100.0,
            });
        }
        let mut params = Vec::new();
        for i in 0..24i64 {
            let (device, id) = if i % 2 == 0 {
                (DeviceSlot(0), tone::GAIN)
            } else {
                (DeviceSlot(1), gain::GAIN)
            };
            params.push(ScheduledParam {
                tick: Tick(1000 + i * 337),
                device,
                id,
                value: 0.2 + (i % 4) as f32 * 0.2,
            });
        }
        Schedule::try_new(&notes, &params, SCHEDULE_FIRST_VOICE)
    }

    struct Rig {
        cycle: Cycle,
        seq: u32,
    }

    impl Rig {
        fn send(&mut self, msgs: &[ControlMsg]) -> Result<(), MsgError> {
            let bytes = encode_stream(1, self.seq, msgs)?;
            self.seq = self.seq.wrapping_add(msgs.len() as u32);
            self.cycle.submit(&bytes)
        }
    }

    /// Renders the scenario: left then right of every block, in order.
    pub fn scenario() -> Result<Vec<f32>, MsgError> {
        let mut rig = Rig {
            cycle: Cycle::new(SCENARIO_RATE)?,
            seq: 0,
        };
        let track = TrackSlot(0);
        rig.send(&[
            ControlMsg::Reset { epoch: 1 },
            ControlMsg::SetTransport {
                tempo: Tempo::clamped(131.7),
                signature: TimeSignature::COMMON,
                launch_quantize: Quantize::default(),
            },
            ControlMsg::TrackCreate {
                track,
                kind: TrackKind::Midi,
            },
            ControlMsg::DeviceInsert {
                track,
                device: DeviceSlot(0),
                kind: TONE.kind_id,
                state: Vec::new(),
            },
            ControlMsg::DeviceInsert {
                track,
                device: DeviceSlot(1),
                kind: GAIN.kind_id,
                state: Vec::new(),
            },
            ControlMsg::SetLoop {
                region: LoopRegion::new(Tick(960), Tick(960 + 4 * 960 + 77)),
            },
            ControlMsg::Play { from: Tick(0) },
        ])?;
        let mut schedule = Some(Box::new(schedule()?));
        let mut out = Vec::new();
        for block in 0..SCENARIO_BLOCKS {
            match block {
                60 => rig.send(&[ControlMsg::Seek { tick: Tick(2500) }])?,
                100 => rig.send(&[ControlMsg::Stop])?,
                110 => rig.send(&[ControlMsg::Play { from: Tick(1500) }])?,
                _ => {}
            }
            let frames = rig
                .cycle
                .render(SCENARIO_SIZES[block % SCENARIO_SIZES.len()]);
            out.extend_from_slice(&rig.cycle.left()[..frames]);
            out.extend_from_slice(&rig.cycle.right()[..frames]);
            // Off the audio path, between two blocks: the track exists after the first.
            if let Some(t) = rig.cycle.engine.state_mut().track_mut(track)
                && let Some(s) = schedule.take()
            {
                drop(t.graph.set_schedule(Some(s)));
            }
        }
        Ok(out)
    }
}
