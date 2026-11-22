//! Cascade, `faulk.cascade`, displayed "Cascade" (#166): a MIDI effect that turns the notes held
//! into a pattern, one note at a time, on the tempo's grid (an arpeggiator). A [`Device`] of
//! role MIDI effect (CT-5).
//!
//! # The notes held
//!
//! A table of [`SOURCES`] received notes, each with its key, velocity and fine tuning, in the
//! order they were played. A note already held at the same key is the same entry (it takes the
//! new voice and velocity). A note-on received while the table is full replaces the oldest. A
//! note-off ends its entry, unless `latch` is on: then the entry stays (released, not held) and
//! the pattern goes on, until the **next chord** (a note-on while no note is down) which empties
//! the table first. Switching `latch` off drops the released entries at once. An `AllNotesOff` (a
//! stop, a jump, a change of the chain) empties the table, the latched notes included.
//!
//! # The pool
//!
//! At each step the held notes are repeated over `octaves` (1 to 4: the notes and the same notes
//! 12, 24, 36 semitones above, keys over 127 left out). `order` then walks it:
//!
//! - `up`, `down`: the pool sorted by pitch, duplicates (a note 12 above another) merged, the
//!   held note (with its velocity and tuning) kept over the copy of the note below it;
//! - `up_down`, `down_up`: the same, turning at the ends without playing an end twice;
//! - `played`: in the order the notes were played, the octaves one after the other;
//! - `random`: a note of the sorted pool drawn from the seed and the place of the step's group
//!   of repeats in the song's grid ([`super::draw`]), nothing else: with the same notes held, the
//!   same seed gives the same notes at the same step on every play, from any start and with
//!   either `restart`, in the export, natively and in WebAssembly;
//! - `chord`: the whole sorted pool at each step.
//!
//! Each note is played `repeats` times (1 to 4) before the next one. A random draw is made once
//! per group of repeats, the groups being cut on the song's grid (steps `0..R`, `R..2R`, ...): a
//! pattern that starts over at a new chord in the middle of a group plays that group's note for
//! the rest of the group only.
//!
//! # The clock
//!
//! A step is a cell of `S` ticks: `rate` (1/1 to 1/64 of a whole note, 3840 ticks) as is,
//! `dotted` (× 3/2) or `triplet` (× 2/3), always a whole number of ticks. Step `k` is at song
//! tick `k × S` from tick 0, the grid of the song's clock: an arpeggio is the same whatever the
//! clip or the place the transport started from. With the transport **stopped** the device keeps
//! its own clock, from the playhead and at the tempo, so a chord played on a keyboard sounds its
//! pattern with nothing playing. A seek, a loop wrap or a start finds the next step of the grid
//! at or after the new position and moves no note sounding: each ends at its own frame. A **stop**
//! (the first block with the transport stopped after one playing) ends the notes sounding at its
//! first frame and the latched notes with them (a note still held by a key stays, and goes on
//! on the device's own clock). A step is at the frame its tick falls in (the floor, as the
//! engine places a clip's note, with a margin of 1e-4 frame against rounding of the position),
//! so the notes are placed to the sample whatever the rate (at 44.1 kHz and 120 BPM a sixteenth
//! is 5512.5 frames: its notes fall on frames 0, 5512, 11025, 16537, ...); the length of a step
//! is fixed in frames at the tempo of the block it starts in. Two blocks are one clock when the
//! second starts within half a tick of where the first ended; a tempo change at a block's start
//! keeps the position, so the grid goes on.
//!
//! `restart` says where the pattern starts. `new_chord`: at the first step after a chord is
//! started (a note-on while no note is down), the pattern starts over from its first note.
//! `free`: the pattern is a function of the step's place in the song only (step `k` of the grid
//! plays the same note whenever the arpeggio starts).
//!
//! # The notes
//!
//! A step plays its note (`chord`: its notes) at the step's frame, for `gate` % of the step
//! (1 to 200 %: above 100 the notes overlap, a key still sounding when it is struck again is
//! ended at that frame first). The velocity is the played one (`velocity_source` `input`) or
//! `velocity` (1 to 127, `fixed`); `accent` (0 to 60, of 127; 0 is off) is added to the first
//! step of each pass of the pattern. The fine tuning of the played note is kept. The device
//! writes its notes under voice ids of its own (the contract lets a MIDI effect have its
//! namespace, as Trim does).
//!
//! # None stuck
//!
//! Every note started is in a fixed table of [`SOUNDING`] entries with the frame it ends at, so
//! a setting changed in the middle of a note (the gate, the rate, the order, the latch, the
//! octaves), a note-off, a seek or the end of the held notes never leaves one on: it ends when it
//! was meant to, and a stop ends them all at once. A table that is full ends the earliest first.
//! An `AllNotesOff` is passed on and empties both tables; [`Device::reset`] empties them too
//! (the host ends the instrument's notes when the effect is switched, removed or replaced,
//! graph).
//!
//! # Output bounds
//!
//! A received event writes at most one event (an `AllNotesOff`). At most [`MAX_STEPS`] steps are
//! made per block, the later ones skipped: that is more than a block of
//! [`crate::msg::limits::MAX_BLOCK_FRAMES`] holds at 44.1 kHz or more, at 999 BPM with the
//! shortest step (a triplet sixty-fourth, 18 steps), so no step is lost within the musical
//! range. A step writes at most [`POOL`] note-ons, each with at most one note-off (the key it
//! strikes again, or the entry it evicts from a full table); the notes of the table due in the
//! block are the other writes: `per_block` is that bound, kept low so that a chain of effects
//! after Cascade (each sized from the output before it) stays within the host's cap.
//!
//! # The load
//!
//! The worst case (a chord of the whole pool at the shortest step, with a full table) scans the
//! table of [`SOUNDING`] entries once per note: [`MAX_STEPS`] × [`POOL`] × [`SOUNDING`]
//! comparisons a block, about 200 000, far from the usual few notes a step.
//!
//! Everything here runs on the audio thread: no allocation, lock or panic, plain arithmetic.

use super::Builtin;
use super::draw::{Draw, Place};
use super::params::{choice, stepped};
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MidiOutput,
    ParamDescriptor, Unit, VoiceId, clamp_tune_cents, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

/// Ids of the parameters.
pub const ORDER: u32 = 0;
pub const RATE: u32 = 1;
pub const FEEL: u32 = 2;
pub const OCTAVES: u32 = 3;
pub const REPEATS: u32 = 4;
pub const GATE: u32 = 5;
pub const VELOCITY_SOURCE: u32 = 6;
pub const VELOCITY: u32 = 7;
pub const ACCENT: u32 = 8;
pub const LATCH: u32 = 9;
pub const RESTART: u32 = 10;
pub const SEED: u32 = 11;

/// The values of `order`.
pub const UP: u32 = 0;
pub const DOWN: u32 = 1;
pub const UP_DOWN: u32 = 2;
pub const DOWN_UP: u32 = 3;
pub const PLAYED: u32 = 4;
pub const RANDOM: u32 = 5;
pub const CHORD: u32 = 6;
/// The values of `rate`, from the whole note to the sixty-fourth.
pub const WHOLE: u32 = 0;
pub const SIXTY_FOURTH: u32 = 6;
pub const SIXTEENTH: u32 = 4;
/// The values of `feel`.
pub const STRAIGHT: u32 = 0;
pub const DOTTED: u32 = 1;
pub const TRIPLET: u32 = 2;
/// The values of `velocity_source`.
pub const INPUT: u32 = 0;
pub const FIXED: u32 = 1;
/// The values of `restart`.
pub const NEW_CHORD: u32 = 0;
pub const FREE: u32 = 1;

/// Received notes held at once.
pub const SOURCES: usize = 16;
/// The most notes of the pool: the held notes over four octaves.
pub const POOL: usize = SOURCES * 4;
/// Notes sounding at once.
pub const SOUNDING: usize = 128;
/// The most steps made in one block.
pub const MAX_STEPS: usize = 24;
/// The most events one block writes (module doc).
pub const PER_BLOCK: u32 = (MAX_STEPS * 2 * POOL + SOUNDING + 1) as u32;
/// A margin, in frames, against the rounding of a position (module doc).
const FRAME_MARGIN: f64 = 1e-4;
/// A margin, in grid steps, against the rounding of a position.
const STEP_MARGIN: f64 = 1e-6;

const PARAMS: [ParamDescriptor; 12] = [
    choice(
        ORDER,
        "order",
        "How the held notes are walked: up, down, up then down, down then up, in the order they were played, at random (a repeatable draw, set by the seed) or all together as a chord at each step.",
        UP as f64,
        &[
            "up", "down", "up_down", "down_up", "played", "random", "chord",
        ],
    ),
    choice(
        RATE,
        "rate",
        "The note value of a step, from a whole note to a sixty-fourth note, on the grid of the transport. Combine it with the feel for dotted and triplet steps.",
        SIXTEENTH as f64,
        &[
            "whole",
            "half",
            "quarter",
            "eighth",
            "sixteenth",
            "thirty_second",
            "sixty_fourth",
        ],
    ),
    choice(
        FEEL,
        "feel",
        "Whether a step is of its plain note value, dotted (one and a half times longer) or a triplet (two thirds as long).",
        STRAIGHT as f64,
        &["straight", "dotted", "triplet"],
    ),
    stepped(
        OCTAVES,
        "octaves",
        "How many octaves the held notes are spread over, from 1 (as played) to 4: the pattern climbs through the notes and the same notes an octave higher.",
        Unit::None,
        1.0,
        4.0,
        1.0,
        1.0,
    ),
    stepped(
        REPEATS,
        "repeats",
        "How many times each note is played in a row before the next one, from 1 to 4.",
        Unit::None,
        1.0,
        4.0,
        1.0,
        1.0,
    ),
    stepped(
        GATE,
        "gate",
        "How long each note lasts, as a share of the step, from 1 % (a click) to 200 % (two steps, the notes overlap).",
        Unit::Percent,
        1.0,
        200.0,
        80.0,
        1.0,
    ),
    choice(
        VELOCITY_SOURCE,
        "velocity_source",
        "Whether the notes keep the velocity they were played with, or all play at the fixed velocity below.",
        INPUT as f64,
        &["input", "fixed"],
    ),
    stepped(
        VELOCITY,
        "velocity",
        "The velocity of every note when the velocity source is fixed, from 1 to 127.",
        Unit::None,
        1.0,
        127.0,
        100.0,
        1.0,
    ),
    stepped(
        ACCENT,
        "accent",
        "Extra velocity of the first step of each pass of the pattern, from 0 (none) to 60 steps of 127.",
        Unit::None,
        0.0,
        60.0,
        0.0,
        1.0,
    ),
    choice(
        LATCH,
        "latch",
        "Whether the pattern goes on after the keys are released, until the next chord is played.",
        0.0,
        &["off", "on"],
    ),
    choice(
        RESTART,
        "restart",
        "Where the pattern starts: over from its first note at each new chord, or free, a note of the pattern for each step of the grid, whenever the notes are played.",
        NEW_CHORD as f64,
        &["new_chord", "free"],
    ),
    stepped(
        SEED,
        "seed",
        "Seed of the random order, from 0 to 1000: a step draws from the seed and its position in the song, so the same seed gives the same notes at the same place on every play, from any start; another seed, another draw.",
        Unit::None,
        0.0,
        1000.0,
        0.0,
        1.0,
    ),
];

/// The descriptor of Cascade (CT-5): `faulk.cascade`, displayed "Cascade" (one English name,
/// never translated). The defaults are an upward pattern of sixteenth notes.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    sidechain: false,
    kind: "faulk.cascade",
    kind_id: DeviceKind(17),
    version: 1,
    name: "Cascade",
    role: DeviceRole::MidiEffect,
    // The device and the most output the host gives it.
    memory_bytes: (size_of::<Cascade>()
        + crate::device::MAX_MIDI_OUTPUT_EVENTS * size_of::<Event>()) as u64,
    midi_output: Some(MidiOutput {
        per_input: 1,
        per_block: PER_BLOCK,
    }),
    named_keys: &[],
    params: &PARAMS,
};

/// The registry's line.
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory: Cascade::build,
};

/// A received note held (or latched).
#[derive(Clone, Copy, Debug, Default)]
struct Source {
    used: bool,
    /// Whether the key is down (false: released, kept by `latch`).
    down: bool,
    voice: u32,
    key: u8,
    velocity: f32,
    tune_cents: f32,
    /// Order of its start: the oldest has the smallest.
    seq: u64,
}

/// A note of the pool.
#[derive(Clone, Copy, Debug, Default)]
struct PoolNote {
    key: u8,
    /// The octaves above the held note it is a copy at (0: the held note itself).
    octave: u8,
    velocity: f32,
    tune_cents: f32,
}

/// A note started and not yet ended.
#[derive(Clone, Copy, Debug, Default)]
struct Sounding {
    used: bool,
    voice: u32,
    key: u8,
    /// Absolute frame it ends at.
    due: u64,
}

/// The device (module doc).
pub struct Cascade {
    sample_rate: f32,
    order: u32,
    /// The `rate` and `feel` values, and the ticks of a step they make.
    rate_index: u32,
    feel_index: u32,
    step_ticks: i64,
    octaves: usize,
    repeats: i64,
    gate: f64,
    fixed_velocity: bool,
    velocity: f32,
    accent: f32,
    latch: bool,
    free: bool,
    seed: u64,
    sources: [Source; SOURCES],
    sounding: [Sounding; SOUNDING],
    seq: u64,
    next_voice: u32,
    /// Whether the voice counter went round once: until then every id it gives is new.
    wrapped: bool,
    /// Absolute frame of the block's first frame.
    now: u64,
    /// The next step to make, on the grid.
    next_step: i64,
    /// Where the previous block ended, in ticks, and whether it was valid and playing.
    clock_end: f64,
    clock_valid: bool,
    clock_playing: bool,
    /// The rate or the feel changed: the next step is found again.
    resync: bool,
    /// A chord started: the next step plays the first note of the pattern.
    restart_pending: bool,
    /// The steps made since the pattern started (`new_chord`).
    counter: i64,
}

impl Cascade {
    pub fn new(setup: &DeviceSetup) -> Result<Self, MsgError> {
        let mut cascade = Self {
            sample_rate: sane_rate(setup.sample_rate),
            order: UP,
            rate_index: SIXTEENTH,
            feel_index: STRAIGHT,
            step_ticks: 240,
            octaves: 1,
            repeats: 1,
            gate: 0.8,
            fixed_velocity: false,
            velocity: 100.0 / 127.0,
            accent: 0.0,
            latch: false,
            free: false,
            seed: 0,
            sources: [Source::default(); SOURCES],
            sounding: [Sounding::default(); SOUNDING],
            seq: 0,
            next_voice: 0,
            wrapped: false,
            now: 0,
            next_step: 0,
            clock_end: 0.0,
            clock_valid: false,
            clock_playing: false,
            resync: true,
            restart_pending: false,
            counter: 0,
        };
        for p in DESCRIPTOR.params {
            cascade.set_param(p.id, p.default as f32);
        }
        Ok(cascade)
    }

    pub fn build(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
        Ok(try_box(Self::new(setup)?)?)
    }

    /// Notes held or latched (test hook).
    pub fn held(&self) -> usize {
        self.sources.iter().filter(|s| s.used).count()
    }

    /// Notes started and not ended (test hook).
    pub fn sounding(&self) -> usize {
        self.sounding.iter().filter(|s| s.used).count()
    }

    /// Puts the voice counter at `next` (test build only).
    #[cfg(feature = "testing")]
    pub fn set_next_voice(&mut self, next: u32) {
        self.next_voice = next.min(VoiceId::MAX);
    }

    /// The first step of the grid at or after song tick `tick`.
    fn first_step_at(&self, tick: f64) -> i64 {
        let k = (tick / self.step_ticks as f64 - STEP_MARGIN).ceil();
        if k.is_finite() {
            // Saturating, never negative: the grid starts at tick 0.
            (k as i64).max(0)
        } else {
            0
        }
    }

    /// A voice id no sounding note has.
    fn fresh_voice(&mut self) -> u32 {
        loop {
            let id = self.next_voice;
            if id >= VoiceId::MAX {
                self.next_voice = 0;
                self.wrapped = true;
            } else {
                self.next_voice = id + 1;
            }
            if !self.wrapped || !self.sounding.iter().any(|s| s.used && s.voice == id) {
                return id;
            }
        }
    }

    /// Writes a note-off of `voice` at frame `at` (never before the cursor).
    fn write_off(
        &mut self,
        voice: u32,
        key: u8,
        at: usize,
        cursor: &mut usize,
        frames: usize,
        out: &mut AudioIo<'_>,
    ) {
        let at = event_frame(at.min(u32::MAX as usize) as u32, *cursor, frames.max(1));
        *cursor = at;
        if let Some(voice) = VoiceId::new(voice) {
            out.events_out.push(Event::NoteOff {
                offset: at as u32,
                voice,
                key,
            });
        }
    }

    /// The index of the sounding note that ends first, among those ending before `limit` (at
    /// or before it when `inclusive`).
    fn earliest(&self, limit: u64, inclusive: bool) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, s) in self.sounding.iter().enumerate() {
            if s.used
                && (if inclusive {
                    s.due <= limit
                } else {
                    s.due < limit
                })
                && best.is_none_or(|b| s.due < self.sounding[b].due)
            {
                best = Some(i);
            }
        }
        best
    }

    /// Ends, in order, the notes due before `limit` (at or before it when `inclusive`), each at
    /// its own frame in the block.
    fn emit_due(
        &mut self,
        limit: u64,
        inclusive: bool,
        cursor: &mut usize,
        frames: usize,
        out: &mut AudioIo<'_>,
    ) {
        while let Some(i) = self.earliest(limit, inclusive) {
            let s = self.sounding[i];
            self.sounding[i].used = false;
            let at = s.due.saturating_sub(self.now).min(frames as u64) as usize;
            self.write_off(s.voice, s.key, at, cursor, frames, out);
        }
    }

    /// Starts a note of the pool at frame `at`, to end at the absolute frame `due`.
    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        note: PoolNote,
        velocity: f32,
        at: usize,
        due: u64,
        cursor: &mut usize,
        frames: usize,
        out: &mut AudioIo<'_>,
    ) {
        // A key still sounding is struck again: it ends first.
        let mut slot = None;
        for i in 0..SOUNDING {
            let s = self.sounding[i];
            if s.used && s.key == note.key {
                self.sounding[i].used = false;
                self.write_off(s.voice, s.key, at, cursor, frames, out);
            }
            if !self.sounding[i].used && slot.is_none() {
                slot = Some(i);
            }
        }
        let slot = match slot {
            Some(i) => i,
            None => {
                // Full: the earliest to end ends now.
                let Some(i) = self.earliest(u64::MAX, true) else {
                    return;
                };
                let s = self.sounding[i];
                self.write_off(s.voice, s.key, at, cursor, frames, out);
                i
            }
        };
        let voice = self.fresh_voice();
        let Some(id) = VoiceId::new(voice) else {
            return;
        };
        let at = event_frame(at.min(u32::MAX as usize) as u32, *cursor, frames.max(1));
        *cursor = at;
        out.events_out.push(Event::NoteOn {
            offset: at as u32,
            voice: id,
            key: note.key,
            velocity,
            tune_cents: note.tune_cents,
        });
        self.sounding[slot] = Sounding {
            used: true,
            voice,
            key: note.key,
            due,
        };
    }

    /// The pool of the held notes over the octaves, in the order `order` walks it. Returns the
    /// number of notes.
    fn pool(&self, pool: &mut [PoolNote; POOL]) -> usize {
        let mut held = [Source::default(); SOURCES];
        let mut n = 0;
        for s in self.sources.iter().filter(|s| s.used) {
            held[n] = *s;
            n += 1;
        }
        let held = &mut held[..n];
        let sorted = self.order != PLAYED;
        held.sort_unstable_by_key(|s| s.seq);
        let mut len = 0;
        for octave in 0..self.octaves {
            for s in held.iter() {
                let key = s.key as usize + 12 * octave;
                if key > 127 || len >= POOL {
                    continue;
                }
                pool[len] = PoolNote {
                    key: key as u8,
                    octave: octave as u8,
                    velocity: s.velocity,
                    tune_cents: s.tune_cents,
                };
                len += 1;
            }
        }
        if sorted {
            // Sorted on the key, then the octave: of two notes on the same key (a held note and
            // the copy of one an octave below) the held one, the lowest octave, is kept. The
            // order is total (the held keys are all different), so the result never depends on
            // the sort.
            pool[..len].sort_unstable_by_key(|p| (p.key, p.octave));
            let mut kept = 0;
            for i in 0..len {
                if kept == 0 || pool[kept - 1].key != pool[i].key {
                    pool[kept] = pool[i];
                    kept += 1;
                }
            }
            len = kept;
        }
        len
    }

    /// The index in a pool of `n` notes of the `j`-th note of the walk (`order` up, down, up
    /// then down, down then up or as played).
    fn walk(&self, j: i64, n: usize) -> usize {
        let n = n.max(1) as i64;
        let up_down = |j: i64| {
            if n < 2 {
                0
            } else {
                let period = 2 * n - 2;
                let m = j.rem_euclid(period);
                if m < n { m } else { period - m }
            }
        };
        let i = match self.order {
            DOWN => n - 1 - j.rem_euclid(n),
            UP_DOWN => up_down(j),
            DOWN_UP => n - 1 - up_down(j),
            _ => j.rem_euclid(n),
        };
        i.clamp(0, n - 1) as usize
    }

    /// The length of a pass of the walk, in notes.
    fn period(&self, n: usize) -> i64 {
        let n = n.max(1) as i64;
        match self.order {
            UP_DOWN | DOWN_UP if n >= 2 => 2 * n - 2,
            CHORD => 1,
            _ => n,
        }
    }

    /// Makes step `k` (at song tick `tick`) at frame `at` of the block: `p0` is the position
    /// of the block's first frame, `tpf` the ticks of a frame.
    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        k: i64,
        tick: f64,
        p0: f64,
        tpf: f64,
        at: usize,
        cursor: &mut usize,
        frames: usize,
        out: &mut AudioIo<'_>,
    ) {
        let mut pool = [PoolNote::default(); POOL];
        let n = self.pool(&mut pool);
        if n == 0 {
            return;
        }
        let c = if self.free {
            k
        } else {
            if self.restart_pending {
                self.restart_pending = false;
                self.counter = 0;
            }
            let c = self.counter;
            self.counter = self.counter.saturating_add(1);
            c
        };
        let repeats = self.repeats.max(1);
        let j = c.div_euclid(repeats);
        let period = self.period(n);
        let first = c.rem_euclid(period.saturating_mul(repeats).max(1)) == 0;
        // The frame the notes end at: the gate's share of the step, on the same rule as their start.
        let end = ((tick + self.step_ticks as f64 * self.gate - p0) / tpf + FRAME_MARGIN).floor();
        let end = if end.is_finite() {
            end.clamp(0.0, u32::MAX as f64) as u64
        } else {
            0
        };
        let due = (self.now + end).max(self.now + at as u64 + 1);
        let accent = if first { self.accent } else { 0.0 };
        if self.order == CHORD {
            for note in &pool[..n] {
                let v = self.velocity_of(note, accent);
                self.start(*note, v, at, due, cursor, frames, out);
            }
        } else {
            let i = if self.order == RANDOM {
                // The group of repeats is cut on the grid, not on the pattern's start.
                let group = (k - k.rem_euclid(repeats))
                    .max(0)
                    .saturating_mul(self.step_ticks);
                Draw::new(self.seed, Place::Tick(group), 0).pick(0, n as u32) as usize
            } else {
                self.walk(j, n)
            };
            let note = pool[i.min(n - 1)];
            let v = self.velocity_of(&note, accent);
            self.start(note, v, at, due, cursor, frames, out);
        }
    }

    /// The velocity a note of the pool plays at.
    fn velocity_of(&self, note: &PoolNote, accent: f32) -> f32 {
        let base = if self.fixed_velocity {
            self.velocity
        } else if note.velocity.is_finite() {
            note.velocity
        } else {
            0.0
        };
        (base + accent / 127.0).clamp(0.0, 1.0)
    }

    /// A received note-on.
    fn note_on(&mut self, voice: VoiceId, key: u8, velocity: f32, tune_cents: f32) {
        let key = key.min(127);
        if !self.sources.iter().any(|s| s.used && s.down) {
            // A new chord: what is latched goes, the pattern starts over.
            self.sources = [Source::default(); SOURCES];
            self.restart_pending = true;
        }
        self.seq += 1;
        let entry = Source {
            used: true,
            down: true,
            voice: voice.get(),
            key,
            velocity: if velocity.is_finite() {
                velocity.clamp(0.0, 1.0)
            } else {
                0.0
            },
            tune_cents: clamp_tune_cents(tune_cents),
            seq: self.seq,
        };
        let slot = self
            .sources
            .iter()
            // The same key is the same entry; the same voice only while its key is down (a
            // released, latched note keeps an old voice id the host may give to another key).
            .position(|s| s.used && (s.key == key || (s.down && s.voice == voice.get())))
            .or_else(|| self.sources.iter().position(|s| !s.used))
            .or_else(|| {
                self.sources
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, s)| s.seq)
                    .map(|(i, _)| i)
            });
        if let Some(i) = slot {
            self.sources[i] = entry;
        }
    }

    /// A received note-off.
    fn note_off(&mut self, voice: VoiceId) {
        let latch = self.latch;
        for s in self.sources.iter_mut() {
            if s.used && s.down && s.voice == voice.get() {
                if latch {
                    s.down = false;
                } else {
                    s.used = false;
                }
                return;
            }
        }
    }
}

impl Device for Cascade {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = ctx.frames as usize;
        let rate = rate_of(ctx, self.sample_rate);
        let tpf = ctx.transport.tempo.ticks_per_second() / rate;
        let steps_possible = frames > 0 && tpf.is_finite() && tpf > 0.0;
        let playing = ctx.transport.playing;
        let position = if ctx.transport.position_ticks.is_finite() {
            ctx.transport.position_ticks
        } else {
            0.0
        };
        // Stopped, the device's own clock goes on from where the last block ended.
        let p0 = if !playing && self.clock_valid && !self.clock_playing {
            self.clock_end
        } else {
            position
        };
        let continuous = self.clock_valid
            && self.clock_playing == playing
            && !self.resync
            && (p0 - self.clock_end).abs() <= 0.5;
        if !continuous {
            self.next_step = self.first_step_at(p0);
            self.resync = false;
        }
        let mut cursor = 0;
        if self.clock_valid && self.clock_playing && !playing {
            // A stop: the notes sounding end now, the latched notes go.
            for i in 0..SOUNDING {
                let s = self.sounding[i];
                if s.used {
                    self.sounding[i].used = false;
                    self.write_off(s.voice, s.key, 0, &mut cursor, frames, io);
                }
            }
            for s in self.sources.iter_mut() {
                if !s.down {
                    s.used = false;
                }
            }
        }
        let mut event = 0;
        let mut made = 0;
        loop {
            let e_at = events
                .get(event)
                .map(|e| event_frame(e.offset(), cursor, frames.max(1)));
            let s_at = if steps_possible && made < MAX_STEPS {
                let tick = self.next_step.saturating_mul(self.step_ticks) as f64;
                let off = ((tick - p0) / tpf + FRAME_MARGIN).floor().max(0.0);
                (off < frames as f64).then_some(off as usize)
            } else {
                None
            };
            match (e_at, s_at) {
                (None, None) => break,
                (Some(at), s) if s.is_none_or(|s| at <= s) => {
                    self.emit_due(self.now + at as u64, true, &mut cursor, frames, io);
                    match events[event] {
                        Event::NoteOn {
                            voice,
                            key,
                            velocity,
                            tune_cents,
                            ..
                        } => self.note_on(voice, key, velocity, tune_cents),
                        Event::NoteOff { voice, .. } => self.note_off(voice),
                        Event::AllNotesOff { .. } => {
                            io.events_out.push(Event::AllNotesOff { offset: at as u32 });
                            cursor = at;
                            self.sources = [Source::default(); SOURCES];
                            self.sounding = [Sounding::default(); SOUNDING];
                        }
                        Event::ParamValue { id, value, .. } => {
                            self.set_param(id, value);
                            if self.resync {
                                self.resync = false;
                                self.next_step = self.first_step_at(p0 + at as f64 * tpf);
                            }
                        }
                    }
                    event += 1;
                }
                (_, Some(at)) => {
                    let at = at.max(cursor);
                    self.emit_due(self.now + at as u64, true, &mut cursor, frames, io);
                    let k = self.next_step;
                    let tick = k.saturating_mul(self.step_ticks) as f64;
                    self.step(k, tick, p0, tpf, at, &mut cursor, frames, io);
                    self.next_step = k.saturating_add(1);
                    made += 1;
                }
                _ => break,
            }
        }
        if frames > 0 {
            self.emit_due(self.now + frames as u64, false, &mut cursor, frames, io);
        }
        let end = p0 + frames as f64 * tpf;
        if steps_possible && made >= MAX_STEPS {
            // A storm of steps: the ones left are skipped.
            self.next_step = self.first_step_at(end);
        }
        self.clock_end = if end.is_finite() { end } else { 0.0 };
        self.clock_valid = true;
        self.clock_playing = playing;
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
            ORDER => self.order = value as u32,
            RATE | FEEL => {
                // The cells are `base × feel`, whole ticks: 3840 >> rate, then 3/2 or 2/3.
                if id.0 == RATE {
                    self.rate_index = value as u32;
                } else {
                    self.feel_index = value as u32;
                }
                let base = 3840_i64 >> self.rate_index.min(SIXTY_FOURTH);
                self.step_ticks = match self.feel_index {
                    DOTTED => base * 3 / 2,
                    TRIPLET => base * 2 / 3,
                    _ => base,
                }
                .max(1);
                self.resync = true;
            }
            OCTAVES => self.octaves = (value as usize).clamp(1, 4),
            REPEATS => self.repeats = (value as i64).clamp(1, 4),
            GATE => self.gate = value / 100.0,
            VELOCITY_SOURCE => self.fixed_velocity = value as u32 == FIXED,
            VELOCITY => self.velocity = (value / 127.0) as f32,
            ACCENT => self.accent = value as f32,
            LATCH => {
                self.latch = value >= 0.5;
                if !self.latch {
                    for s in self.sources.iter_mut() {
                        if !s.down {
                            s.used = false;
                        }
                    }
                }
            }
            RESTART => self.free = value as u32 == FREE,
            _ => self.seed = value as u64,
        }
    }

    fn reset(&mut self) {
        self.sources = [Source::default(); SOURCES];
        self.sounding = [Sounding::default(); SOUNDING];
        self.clock_valid = false;
        self.restart_pending = false;
        self.counter = 0;
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
