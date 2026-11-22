//! Swing, `faulk.swing`, displayed "Swing" (#168): a MIDI effect that delays the notes that
//! fall on the off-beats of a grid, to give a straight pattern the lilt of jazz, funk or
//! shuffle, and optionally humanizes every note's timing and velocity. A [`Device`] of role
//! MIDI effect (CT-5).
//!
//! # The swing
//!
//! `grid` is a cell of `G` ticks: 1/8 (480), 1/16 (240) or 1/32 (120) of a whole note (960
//! ticks a beat). Cells go in pairs of `2G` ticks counted from tick 0. In a straight pair the
//! second cell starts at `G`; `amount` (50 to 75 %) is where it starts, as a share of the pair:
//! at 50 % it stays, at 66 % it falls on the triplet (`2G × 2/3`), at 75 % on the dotted
//! position. An **off-beat note** is one whose song tick `t` (the place the draws of the other
//! MIDI effects use, [`super::draw::song_tick`]: the clip's own tick for a note of a clip) has
//! `|t mod 2G − G| ≤ window / 100 × G`; it is delayed by `2G × (amount − 50) / 100` ticks, that
//! is `round(ticks × sample_rate / ticks_per_second)` frames at the tempo of the block it
//! starts in, fixed then (a tempo change never moves a note already started). The grid is the
//! **song's**, from tick 0, not the clip's: a note of a clip launched on a boundary that is not
//! a multiple of `2G`, a clip looped at a length that is not one, or the bar after the first in a
//! signature whose bar is not (7/8 on the 1/8 grid) is judged against the song's phase. Every other note
//! (the on-beats, and any note far from an off-beat) is not swung. With `window` at 50 % every
//! note of the second half of a pair moves; at 0 only a note exactly on the off-beat.
//!
//! Notes only move **forward**: the device adds no latency, and a note that is not swung comes
//! out at its frame. A note played live while the transport runs takes the playhead's tick, so
//! only the notes that land near an off-beat are delayed. While the transport is **stopped**
//! the playhead does not move, there is no grid, and a note is not swung.
//!
//! # Humanize
//!
//! Applied to every note, swung or not, drawn from a hash of `seed`, the note's place and key
//! ([`super::draw`]): `humanize_time` delays it by `u × humanize_time` (`u` in `[0, 1)`, 0 to 20
//! ms: forward only), and `humanize_velocity` adds `(2 v − 1) × humanize_velocity / 127` to its
//! velocity (clamped to 0 to 1). The same seed gives the same notes at the same place on every
//! play, from any start, in the export, natively and in WebAssembly. Both are 0 by default.
//! While the transport is stopped the place is the count of live note-ons since the reset.
//!
//! # Note lengths
//!
//! A note's end is delayed by exactly its start's delay (kept with the note), so every note
//! keeps its length to the sample.
//!
//! # Notes held, none stuck
//!
//! Two fixed tables, built in the factory: the received notes that sound ([`HELD`]), each with
//! its output voice and delay, and the delayed events ([`PENDING`]) waiting for their frame. The
//! device writes its notes under its own voice ids (the contract lets a MIDI effect have its
//! namespace, as Trim does): a host that reuses an input id after its note-off never cuts a
//! note whose delayed end is still pending. A note-on for a voice already held ends the held
//! note first; a note-on when the table is full ends the oldest. A note-off for a voice the
//! table does not hold is dropped (the device never started it, or the table ended it). When the
//! pending table is full the earliest pending event comes out at once (earlier than planned,
//! never lost, in order). An `AllNotesOff` is passed on at its frame and empties both tables:
//! the delayed note-ons never sound. [`Device::reset`] empties them too.
//!
//! # Stops
//!
//! The first block with the clock stopped after one with it running (a **stop**) drops the
//! delayed note-ons that have not come out (and their notes), brings the delayed note-offs due
//! at the first frame and sets the delay of the notes sounding to 0: nothing rings on after a
//! stop. A seek or a loop wrap is not a stop: the notes started keep their delay and their
//! ends come out as the schedule sends them; a note-on still delayed at the jump comes out
//! after it (by at most its delay), and a note flushed early by a full pending table (a storm
//! of over [`PENDING`] delayed events) keeps its order but not its exact length (a note-on
//! flushed while its end is still pending lasts longer than its input).
//!
//! # Output bounds
//!
//! A received note-on writes at most four events (the end of a note it replaces and its own
//! start, each possibly with the early flush of a full table), a note-off, a flush and an
//! `AllNotesOff` fewer: `per_input` 4. The events of earlier blocks that come out in this one
//! are at most [`PENDING`]: `per_block`.
//!
//! Everything here runs on the audio thread: no allocation, lock or panic, plain arithmetic.

use super::Builtin;
use super::draw::{Draw, Place};
use super::held::HeldTable;
use super::params::{choice, stepped};
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MidiOutput,
    ParamDescriptor, Unit, VoiceId, clamp_tune_cents, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

/// Ids of the parameters.
pub const AMOUNT: u32 = 0;
pub const GRID: u32 = 1;
pub const WINDOW: u32 = 2;
pub const HUMANIZE_TIME: u32 = 3;
pub const HUMANIZE_VELOCITY: u32 = 4;
pub const SEED: u32 = 5;

/// The values of `grid`.
pub const EIGHTH: u32 = 0;
pub const SIXTEENTH: u32 = 1;
pub const THIRTY_SECOND: u32 = 2;

/// Received notes held at once.
pub const HELD: usize = 128;
/// Delayed events waiting at once.
pub const PENDING: usize = 512;
/// Longest humanize delay, in seconds.
pub const MAX_HUMANIZE_SECONDS: f64 = 0.02;
/// Longest delay of a note, in frames (a bound against overflow, never reached: a pair of the
/// 1/8 grid at 75 % lasts under 10 s down to 1 BPM).
const MAX_DELAY_FRAMES: f64 = (1u64 << 32) as f64;

const PARAMS: [ParamDescriptor; 6] = [
    stepped(
        AMOUNT,
        "amount",
        "Where the second note of each pair of the grid falls, from 50 % (straight) to 75 %: 58 % is a light swing, 66 % a triplet feel, 75 % a dotted feel. Only forward: a note is never played early.",
        Unit::Percent,
        50.0,
        75.0,
        58.0,
        0.0,
    ),
    choice(
        GRID,
        "grid",
        "The note value the pairs are made of: an eighth, a sixteenth or a thirty-second note. The second note of each pair is the one that is delayed.",
        SIXTEENTH as f64,
        &["eighth", "sixteenth", "thirty_second"],
    ),
    stepped(
        WINDOW,
        "window",
        "How close to an off-beat a note must be to be swung, from 0 % (exactly on it) to 50 % of a grid cell either side: wider catches looser notes, narrower leaves them alone.",
        Unit::Percent,
        0.0,
        50.0,
        25.0,
        0.0,
    ),
    stepped(
        HUMANIZE_TIME,
        "humanize_time",
        "Random delay added to every note, from 0 (none) to 20 ms: a repeatable draw, set by the seed. Only forward.",
        Unit::Seconds,
        0.0,
        MAX_HUMANIZE_SECONDS,
        0.0,
        0.0,
    ),
    stepped(
        HUMANIZE_VELOCITY,
        "humanize_velocity",
        "Random change of every note's velocity, up or down, from 0 (none) to 20 steps of 127: a repeatable draw, set by the seed.",
        Unit::None,
        0.0,
        20.0,
        0.0,
        1.0,
    ),
    stepped(
        SEED,
        "seed",
        "Seed of the humanize draws, from 0 to 1000: a note draws from the seed, its position in the song and its pitch, so the same seed gives the same notes at the same place on every play, from any start; another seed, another draw.",
        Unit::None,
        0.0,
        1000.0,
        0.0,
        1.0,
    ),
];

/// The descriptor of Swing (CT-5): `faulk.swing`, displayed "Swing" (one English name, never
/// translated). The defaults are a light 58 % swing of sixteenth notes without humanize.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    sidechain: false,
    kind: "faulk.swing",
    kind_id: DeviceKind(16),
    version: 1,
    name: "Swing",
    role: DeviceRole::MidiEffect,
    // The device, its two tables, and the most output the host gives it.
    memory_bytes: (size_of::<Swing>()
        + HeldTable::<Held, HELD>::BYTES
        + PENDING * size_of::<Pending>()
        + crate::device::MAX_MIDI_OUTPUT_EVENTS * size_of::<Event>()) as u64,
    midi_output: Some(MidiOutput {
        per_input: 4,
        per_block: PENDING as u32,
    }),
    named_keys: &[],
    params: &PARAMS,
};

/// The registry's line.
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory: Swing::build,
};

/// A received note that sounds (or will, when its delay is over).
#[derive(Clone, Copy, Debug, Default)]
struct Held {
    out_voice: u32,
    key: u8,
    /// The delay of its start and of its end, in frames.
    delay: u64,
    /// Order of its start: the oldest has the smallest.
    seq: u64,
}

/// A delayed event.
#[derive(Clone, Copy, Debug, Default)]
struct Pending {
    on: bool,
    /// Absolute frame it comes out at.
    due: u64,
    voice: u32,
    key: u8,
    velocity: f32,
    tune_cents: f32,
    seq: u64,
    /// Marked for removal at a stop.
    dead: bool,
}

/// The device (module doc).
pub struct Swing {
    sample_rate: f32,
    amount: f64,
    grid_ticks: i64,
    window: f64,
    humanize_seconds: f64,
    humanize_velocity: f64,
    seed: u64,
    held: HeldTable<Held, HELD>,
    pending: Box<[Pending; PENDING]>,
    pending_len: usize,
    /// The earliest due frame among the pending events (`u64::MAX` with none).
    next_due: u64,
    /// Absolute frame of the block's first frame.
    now: u64,
    was_playing: bool,
    /// Note-ons received while stopped, since the last reset: their place.
    stopped: u64,
    seq: u64,
    next_voice: u32,
    /// Whether the voice counter went round once: until then every id it gives is new.
    wrapped: bool,
}

impl Swing {
    pub fn new(setup: &DeviceSetup) -> Result<Self, MsgError> {
        let mut swing = Self {
            sample_rate: sane_rate(setup.sample_rate),
            amount: 50.0,
            grid_ticks: 240,
            window: 0.0,
            humanize_seconds: 0.0,
            humanize_velocity: 0.0,
            seed: 0,
            held: HeldTable::new()?,
            pending: try_box([Pending::default(); PENDING])?,
            pending_len: 0,
            next_due: u64::MAX,
            now: 0,
            was_playing: false,
            stopped: 0,
            seq: 0,
            next_voice: 0,
            wrapped: false,
        };
        for p in DESCRIPTOR.params {
            swing.set_param(p.id, p.default as f32);
        }
        Ok(swing)
    }

    pub fn build(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
        Ok(try_box(Self::new(setup)?)?)
    }

    /// Delayed events waiting (test hook).
    pub fn pending(&self) -> usize {
        self.pending_len
    }

    /// Notes held (test hook).
    pub fn held(&self) -> usize {
        self.held.iter().count()
    }

    /// Puts the voice counter at `next` (test build only).
    #[cfg(feature = "testing")]
    pub fn set_next_voice(&mut self, next: u32) {
        self.next_voice = next.min(VoiceId::MAX);
    }

    /// The delay in frames of a note-on at `place` of key `key` in the block of `ctx`, and its
    /// velocity change (module doc).
    fn delay_of(&self, ctx: &BlockCtx, place: Place, key: u8) -> (u64, f32) {
        let rate = rate_of(ctx, self.sample_rate);
        let mut seconds = 0.0;
        if let Place::Tick(tick) = place
            && self.amount > 50.0
            && self.is_off_beat(tick)
        {
            let ticks = (2 * self.grid_ticks) as f64 * (self.amount - 50.0) / 100.0;
            seconds += ticks / ctx.transport.tempo.ticks_per_second();
        }
        let mut velocity = 0.0;
        if self.humanize_seconds > 0.0 || self.humanize_velocity > 0.0 {
            let draw = Draw::new(self.seed, place, key);
            seconds += draw.unit(0) * self.humanize_seconds;
            velocity = ((draw.unit(1) * 2.0 - 1.0) * self.humanize_velocity / 127.0) as f32;
        }
        let frames = (seconds * rate).round();
        let frames = if frames.is_finite() {
            frames.clamp(0.0, MAX_DELAY_FRAMES) as u64
        } else {
            0
        };
        (frames, velocity)
    }

    /// Whether a note at song `tick` is near an off-beat of the grid (module doc).
    pub fn is_off_beat(&self, tick: i64) -> bool {
        let pair = 2 * self.grid_ticks;
        let r = tick.rem_euclid(pair);
        ((r - self.grid_ticks).abs() as f64) <= self.window / 100.0 * self.grid_ticks as f64
    }

    /// A voice id no held note and no pending event has.
    fn fresh_voice(&mut self) -> u32 {
        loop {
            let id = self.next_voice;
            if id >= VoiceId::MAX {
                self.next_voice = 0;
                self.wrapped = true;
            } else {
                self.next_voice = id + 1;
            }
            if !self.wrapped
                || (!self.held.iter().any(|(_, h)| h.out_voice == id)
                    && !self.pending[..self.pending_len]
                        .iter()
                        .any(|p| p.voice == id))
            {
                return id;
            }
        }
    }

    /// Writes `event` at frame `at`, now (`delay` 0) or at its delayed frame.
    #[allow(clippy::too_many_arguments)]
    fn put(
        &mut self,
        mut p: Pending,
        at: usize,
        delay: u64,
        cursor: &mut usize,
        frames: usize,
        out: &mut AudioIo<'_>,
    ) {
        if delay == 0 {
            self.write(&p, at, cursor, frames, out);
            return;
        }
        if self.pending_len >= PENDING {
            // Full: the earliest comes out now, never lost.
            if let Some(i) = self.earliest(u64::MAX, true) {
                let e = self.take(i);
                self.write(&e, at, cursor, frames, out);
            }
        }
        p.due = (self.now + at as u64).saturating_add(delay);
        self.seq += 1;
        p.seq = self.seq;
        self.pending[self.pending_len] = p;
        self.pending_len += 1;
        self.next_due = self.next_due.min(p.due);
    }

    /// Writes an event at frame `at` (never before the cursor, inside the block).
    fn write(
        &mut self,
        p: &Pending,
        at: usize,
        cursor: &mut usize,
        frames: usize,
        out: &mut AudioIo<'_>,
    ) {
        let at = event_frame(at.min(u32::MAX as usize) as u32, *cursor, frames.max(1));
        *cursor = at;
        let Some(voice) = VoiceId::new(p.voice) else {
            return;
        };
        out.events_out.push(if p.on {
            Event::NoteOn {
                offset: at as u32,
                voice,
                key: p.key,
                velocity: p.velocity,
                tune_cents: p.tune_cents,
            }
        } else {
            Event::NoteOff {
                offset: at as u32,
                voice,
                key: p.key,
            }
        });
    }

    /// The index of the earliest pending event due before `limit` (at or before, `inclusive`).
    fn earliest(&self, limit: u64, inclusive: bool) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, p) in self.pending[..self.pending_len].iter().enumerate() {
            let due = if inclusive {
                p.due <= limit
            } else {
                p.due < limit
            };
            if due
                && best.is_none_or(|b| {
                    let o = &self.pending[b];
                    (p.due, p.seq) < (o.due, o.seq)
                })
            {
                best = Some(i);
            }
        }
        best
    }

    /// Takes the pending event `i` out of the table.
    fn take(&mut self, i: usize) -> Pending {
        let p = self.pending[i];
        self.pending_len -= 1;
        self.pending[i] = self.pending[self.pending_len];
        if p.due <= self.next_due {
            self.next_due = self.pending[..self.pending_len]
                .iter()
                .map(|e| e.due)
                .min()
                .unwrap_or(u64::MAX);
        }
        p
    }

    /// Writes, in order, the pending events due at or before the absolute frame `limit`
    /// (`inclusive`) or before it, each at its own frame in the block.
    fn emit_due(
        &mut self,
        limit: u64,
        inclusive: bool,
        cursor: &mut usize,
        frames: usize,
        out: &mut AudioIo<'_>,
    ) {
        while if inclusive {
            self.next_due <= limit
        } else {
            self.next_due < limit
        } {
            let Some(i) = self.earliest(limit, inclusive) else {
                return;
            };
            let p = self.take(i);
            let at = p.due.saturating_sub(self.now).min(frames as u64) as usize;
            self.write(&p, at, cursor, frames, out);
        }
    }

    /// Ends the held note at table entry `entry`, at its delay.
    fn end_held(
        &mut self,
        entry: usize,
        at: usize,
        cursor: &mut usize,
        frames: usize,
        out: &mut AudioIo<'_>,
    ) {
        let Some(h) = self.held.get_mut(entry).map(|h| *h) else {
            return;
        };
        self.held.release(entry);
        let off = Pending {
            on: false,
            voice: h.out_voice,
            key: h.key,
            ..Pending::default()
        };
        self.put(off, at, h.delay, cursor, frames, out);
    }

    /// The oldest held note's entry.
    fn oldest(&self) -> Option<usize> {
        self.held.iter().min_by_key(|(_, h)| h.seq).map(|(i, _)| i)
    }

    #[allow(clippy::too_many_arguments)]
    fn note_on(
        &mut self,
        ctx: &BlockCtx,
        at: usize,
        voice: VoiceId,
        key: u8,
        velocity: f32,
        tune_cents: f32,
        cursor: &mut usize,
        frames: usize,
        out: &mut AudioIo<'_>,
    ) {
        if !ctx.transport.playing {
            self.stopped = self.stopped.wrapping_add(1);
        }
        let key = key.min(127);
        let place = Place::of(ctx, at, self.stopped);
        let (delay, dv) = self.delay_of(ctx, place, key);
        let (mut free, same) = self.held.free_and_find(voice.get());
        if let Some(entry) = same {
            // A voice already held (a broken guarantee): its note ends first.
            self.end_held(entry, at, cursor, frames, out);
            free = Some(entry);
        } else if free.is_none()
            && let Some(entry) = self.oldest()
        {
            self.end_held(entry, at, cursor, frames, out);
            free = Some(entry);
        }
        let Some(entry) = free else { return };
        let out_voice = self.fresh_voice();
        self.seq += 1;
        self.held.insert(
            entry,
            voice.get(),
            Held {
                out_voice,
                key,
                delay,
                seq: self.seq,
            },
        );
        let velocity = if velocity.is_finite() {
            (velocity + dv).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let on = Pending {
            on: true,
            voice: out_voice,
            key,
            velocity,
            tune_cents: clamp_tune_cents(tune_cents),
            ..Pending::default()
        };
        self.put(on, at, delay, cursor, frames, out);
    }

    /// A stop: the delayed note-ons that did not come out are dropped with their notes (and
    /// the delayed note-offs of those notes, whose input note-off came before), the other
    /// delayed note-offs come out at once, the notes sounding end undelayed. Bounded work of
    /// [`PENDING`]² comparisons at most, once per stop.
    fn stop(&mut self) {
        let len = self.pending_len;
        for i in 0..len {
            if self.pending[i].on {
                let voice = self.pending[i].voice;
                for entry in 0..HELD {
                    if self
                        .held
                        .get_mut(entry)
                        .is_some_and(|h| h.out_voice == voice)
                    {
                        self.held.release(entry);
                    }
                }
                self.pending[i].dead = true;
            }
        }
        for i in 0..len {
            if !self.pending[i].on {
                let voice = self.pending[i].voice;
                self.pending[i].dead = self.pending[..len]
                    .iter()
                    .any(|p| p.dead && p.voice == voice);
            }
        }
        let mut kept = 0;
        for i in 0..len {
            if !self.pending[i].dead {
                let mut p = self.pending[i];
                p.due = 0;
                self.pending[kept] = p;
                kept += 1;
            }
        }
        self.pending_len = kept;
        self.next_due = if kept > 0 { 0 } else { u64::MAX };
        for entry in 0..HELD {
            if let Some(h) = self.held.get_mut(entry) {
                h.delay = 0;
            }
        }
    }
}

impl Device for Swing {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = ctx.frames as usize;
        if self.was_playing && !ctx.transport.playing {
            self.stop();
        }
        self.was_playing = ctx.transport.playing;
        let mut cursor = 0;
        for event in events {
            let at = event_frame(event.offset(), cursor, frames.max(1));
            self.emit_due(self.now + at as u64, true, &mut cursor, frames, io);
            let at = at.max(cursor);
            match *event {
                Event::NoteOn {
                    voice,
                    key,
                    velocity,
                    tune_cents,
                    ..
                } => self.note_on(
                    ctx,
                    at,
                    voice,
                    key,
                    velocity,
                    tune_cents,
                    &mut cursor,
                    frames,
                    io,
                ),
                Event::NoteOff { voice, .. } => {
                    if let Some(entry) = self.held.find(voice.get()) {
                        self.end_held(entry, at, &mut cursor, frames, io);
                    }
                }
                Event::AllNotesOff { .. } => {
                    let p = at.min(u32::MAX as usize) as u32;
                    io.events_out.push(Event::AllNotesOff { offset: p });
                    cursor = at;
                    self.held.clear();
                    self.pending_len = 0;
                    self.next_due = u64::MAX;
                }
                Event::ParamValue { id, value, .. } => self.set_param(id, value),
            }
        }
        if frames > 0 {
            self.emit_due(self.now + frames as u64, false, &mut cursor, frames, io);
        }
        self.now = self.now.saturating_add(frames as u64);
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        let value = f64::from(value);
        match id.0 {
            AMOUNT => self.amount = value,
            GRID => {
                self.grid_ticks = match value as u32 {
                    EIGHTH => 480,
                    THIRTY_SECOND => 120,
                    _ => 240,
                }
            }
            WINDOW => self.window = value,
            HUMANIZE_TIME => self.humanize_seconds = value,
            HUMANIZE_VELOCITY => self.humanize_velocity = value,
            _ => self.seed = value as u64,
        }
    }

    fn reset(&mut self) {
        self.held.clear();
        self.pending_len = 0;
        self.next_due = u64::MAX;
        self.was_playing = false;
        self.stopped = 0;
    }
}

fn sane_rate(rate: f32) -> f32 {
    if rate.is_finite() && rate >= 8_000.0 {
        rate.min(crate::device::MAX_SAMPLE_RATE)
    } else {
        48_000.0
    }
}

/// The rate of a block: the context's when it is sane, else the one the device was built for.
fn rate_of(ctx: &BlockCtx, built: f32) -> f64 {
    let rate = if ctx.sample_rate.is_finite() && ctx.sample_rate >= 8_000.0 {
        ctx.sample_rate.min(crate::device::MAX_SAMPLE_RATE)
    } else {
        built
    };
    f64::from(rate)
}
