//! Trim, `faulk.trim`, displayed "Trim" (issue #693): a MIDI effect that gives every note a
//! set length before the instrument, whatever the way it was played. A [`Device`] of role
//! MIDI effect (CT-5).
//!
//! # Length
//!
//! - **Free** (`mode` = free): `time`, 1 ms to 10 s.
//! - **Synced** (`mode` = synced, the default): a duration of the transport's grid, in ticks,
//!   `L = division × feel × gate / 100`. `division` is 1/64 to 1/2 of a whole note (`3840 /
//!   n` ticks for 1/n) or 1, 2 or 4 bars (`k × ticks_per_bar` of the signature the block
//!   sees); `feel` multiplies it by 1 (straight), 2/3 (triplet) or 3/2 (dotted); `gate` is 1
//!   to 200 %. The ticks become frames at the tempo of the block the note starts in: `frames =
//!   L × sample_rate / ticks_per_second`. 1/8 at 120 BPM, gate 100 %: 480 ticks, 0.25 s.
//!
//! Then **key tracking** (`key_track`, −100 to +100 %) scales the length with the pitch
//! (`key + tune_cents / 100`): `factor = 2^(key_track / 100 × (60 − pitch) / 12)`. At +100 %
//! the length doubles from one octave to the octave below (C3 lasts twice as long as C4, C5
//! half as long: long basses, short highs); −100 % reverses it; 0 leaves the length as is.
//! The length in frames is `round(seconds × factor × sample_rate)` (half away from zero),
//! kept between 1 frame and [`MAX_LENGTH_FRAMES`].
//!
//! A note's length is fixed when it starts: a tempo or parameter change while it sounds does
//! not move its end (nothing jumps, nothing is left stuck). Every parameter declares a
//! smoothing of 0: a value applies to the notes that start at or after its offset.
//!
//! # Trigger
//!
//! - **Start** (default): a received `NoteOn` comes out at its frame and its `NoteOff` comes
//!   out `length` frames later, at the sample; the received `NoteOff` is dropped, so a
//!   shorter note is lengthened and a longer one cut.
//! - **Release**: a received `NoteOn` is held back; its `NoteOn` comes out at the frame of
//!   the received `NoteOff`, at the velocity of the start (the contract's `NoteOff` carries
//!   no release velocity), and its `NoteOff` `length` frames later. A key pressed in this
//!   mode comes out at its release even if the mode changed meanwhile.
//!
//! # Notes held
//!
//! The device writes its notes under its own voice ids (the contract lets a MIDI effect have
//! its namespace): a host that reuses an input id after its note-off never cuts a note Trim
//! still holds, so notes played faster than their length overlap, each one its full length.
//! Two fixed tables of [`CAPACITY`] entries, allocated when the device is built, hold the
//! notes it sounds (with their end frame) and, in release mode, the keys pressed and not yet
//! released. A note when the first is full ends the oldest note at once (its `NoteOff` comes
//! just before the new `NoteOn`); a key when the second is full forgets the oldest key held
//! (nothing was sent for it). Within a block, the ends due and the received events come out
//! in time order, an end due at the frame of an event first (so an end and a start at the
//! same frame are a retrigger, never a stuck note).
//!
//! A received `AllNotesOff` (a change of the chain, lost live events: the host sends it at
//! the head of the chain) is passed on at its frame and empties both tables;
//! [`Device::reset`] empties them too.
//!
//! # Stops and jumps
//!
//! The schedule ends its notes by their own note-offs when the clock stops or jumps (a play,
//! a seek, a loop wrap: `crate::events`), not by an `AllNotesOff`. Trim reads the transport of
//! each block it receives:
//!
//! - the first block with the clock stopped after a block with it running (a **stop**) ends
//!   every note Trim sounds at its first frame and forgets the keys held back, before its
//!   events: nothing rings on after a stop, and the note-offs of the stop fire no note in
//!   release mode;
//! - a block with the clock running whose position is not where the last one ended (a seek
//!   or a loop wrap while it plays: more than [`JUMP_TICKS`] away) forgets the keys held back
//!   before its events, so a jump fires no note either; the notes sounding keep their length.
//!
//! A key played live and held back in release mode across a stop or a jump is forgotten
//! too (nothing sounds at its release). Nothing here allocates, locks or panics.

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MidiOutput,
    ParamDescriptor, Scale, Unit, VoiceId, clamp_tune_cents, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};
use crate::time::{PPQ, TimeSignature};

/// Ids of the parameters.
pub const MODE: u32 = 0;
pub const TIME: u32 = 1;
pub const DIVISION: u32 = 2;
pub const FEEL: u32 = 3;
pub const GATE: u32 = 4;
pub const TRIGGER: u32 = 5;
pub const KEY_TRACK: u32 = 6;

/// The values of `mode`.
pub const FREE: u32 = 0;
pub const SYNCED: u32 = 1;

/// The values of `division`, from 1/64 to 4 bars.
pub const DIV_1_64: u32 = 0;
pub const DIV_1_32: u32 = 1;
pub const DIV_1_16: u32 = 2;
pub const DIV_1_8: u32 = 3;
pub const DIV_1_4: u32 = 4;
pub const DIV_1_2: u32 = 5;
pub const DIV_1_BAR: u32 = 6;
pub const DIV_2_BARS: u32 = 7;
pub const DIV_4_BARS: u32 = 8;

/// The values of `feel`.
pub const STRAIGHT: u32 = 0;
pub const TRIPLET: u32 = 1;
pub const DOTTED: u32 = 2;

/// The values of `trigger`.
pub const START: u32 = 0;
pub const RELEASE: u32 = 1;

/// Notes the device sounds at once, and keys it holds back in release mode.
pub const CAPACITY: usize = 128;

/// Longest note, in frames (about 260 days at 48 kHz: a bound against overflow, never a
/// musical limit).
pub const MAX_LENGTH_FRAMES: u64 = 1 << 40;

/// A block whose position is further than this from where the last one ended, in ticks, is
/// a jump (a seek, a loop wrap).
pub const JUMP_TICKS: f64 = 0.5;

/// The pitch at which key tracking leaves the length as is (C4).
pub const KEY_TRACK_CENTER: f64 = 60.0;

/// The descriptor of Trim (CT-5): `faulk.trim`, displayed "Trim" (one English name, never
/// translated). The defaults are the values of [`Trim::new`]: a sixteenth note at the tempo,
/// gate 100 %, from the start of the note, no key tracking.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.trim",
    kind_id: DeviceKind(10),
    version: 1,
    name: "Trim",
    role: DeviceRole::MidiEffect,
    memory_bytes: size_of::<Trim>() as u64,
    midi_output: Some(MidiOutput {
        // A note-on and its own note-off in the same block, or the end of the oldest note on
        // an overflow and the note-on; an all-notes-off is one.
        per_input: 2,
        // The ends of the notes started in earlier blocks: each ends once.
        per_block: CAPACITY as u32,
    }),
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(MODE),
            key: "mode",
            description: "How the length is set: a time in seconds (free) or a note value that follows the project tempo (synced).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: SYNCED as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["free", "synced"],
        },
        ParamDescriptor {
            id: ParamId(TIME),
            key: "time",
            description: "Length of every note in free mode, in seconds, from 1 ms to 10 s.",
            unit: Unit::Seconds,
            min: 0.001,
            max: 10.0,
            default: 0.1,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(DIVISION),
            key: "division",
            description: "Length of every note in synced mode, as a note value of the project tempo, from a 64th note to 4 bars of the time signature.",
            unit: Unit::None,
            min: 0.0,
            max: 8.0,
            default: DIV_1_16 as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &[
                "sixty_fourth",
                "thirty_second",
                "sixteenth",
                "eighth",
                "quarter",
                "half",
                "bar",
                "two_bars",
                "four_bars",
            ],
        },
        ParamDescriptor {
            id: ParamId(FEEL),
            key: "feel",
            description: "Straight, triplet (two thirds of the division) or dotted (one and a half times the division), in synced mode.",
            unit: Unit::None,
            min: 0.0,
            max: 2.0,
            default: STRAIGHT as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["straight", "triplet", "dotted"],
        },
        ParamDescriptor {
            id: ParamId(GATE),
            key: "gate",
            description: "Share of the division a note lasts in synced mode, from 1 % to 200 % (100 %: exactly the division).",
            unit: Unit::Percent,
            min: 1.0,
            max: 200.0,
            default: 100.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(TRIGGER),
            key: "trigger",
            description: "When a note comes out: when the key is pressed (start), or when it is let go (release), at the velocity it was pressed with.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: START as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["start", "release"],
        },
        ParamDescriptor {
            id: ParamId(KEY_TRACK),
            key: "key_track",
            description: "How the length follows the pitch, from -100 % to +100 %: at +100 % it doubles one octave below C4 and halves one octave above.",
            unit: Unit::Percent,
            min: -100.0,
            max: 100.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
    ],
};

/// Trim's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a Trim for `setup`, off the audio path.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Trim::new(setup.sample_rate))?)
}

/// The length of a `division` in ticks under `signature`.
pub fn division_ticks(division: u32, signature: TimeSignature) -> i64 {
    let whole = PPQ * 4;
    match division {
        DIV_1_64 => whole / 64,
        DIV_1_32 => whole / 32,
        DIV_1_16 => whole / 16,
        DIV_1_8 => whole / 8,
        DIV_1_4 => whole / 4,
        DIV_1_2 => whole / 2,
        DIV_1_BAR => signature.ticks_per_bar().get(),
        DIV_2_BARS => 2 * signature.ticks_per_bar().get(),
        _ => 4 * signature.ticks_per_bar().get(),
    }
}

/// The factor of `feel` on a division.
pub fn feel_factor(feel: u32) -> f64 {
    match feel {
        TRIPLET => 2.0 / 3.0,
        DOTTED => 1.5,
        _ => 1.0,
    }
}

/// The factor key tracking applies to the length of a note of `pitch` (semitones, 60 = C4).
pub fn key_factor(key_track_percent: f64, pitch: f64) -> f64 {
    if key_track_percent == 0.0 {
        1.0
    } else {
        (key_track_percent / 100.0 * (KEY_TRACK_CENTER - pitch) / 12.0).exp2()
    }
}

/// A note the device sounds.
#[derive(Clone, Copy, Debug, Default)]
struct Sounding {
    voice: u32,
    key: u8,
    /// Absolute frame of its end.
    end: u64,
    /// Order of its start: the oldest has the smallest.
    seq: u64,
}

/// A key pressed in release mode, not yet released.
#[derive(Clone, Copy, Debug, Default)]
struct Held {
    voice: u32,
    key: u8,
    velocity: f32,
    tune_cents: f32,
    seq: u64,
}

/// The device (module doc).
#[derive(Clone, Debug)]
pub struct Trim {
    sample_rate: f32,
    mode: u32,
    time: f64,
    division: u32,
    feel: u32,
    gate: f64,
    trigger: u32,
    key_track: f64,
    /// Absolute frame of the block's first frame.
    now: u64,
    /// Whether the clock ran in the last block, and the position it would be at now.
    was_playing: bool,
    expect_ticks: f64,
    seq: u64,
    next_voice: u32,
    sounding: [Sounding; CAPACITY],
    sounding_len: usize,
    held: [Held; CAPACITY],
    held_len: usize,
}

impl Trim {
    /// A Trim at its defaults for `sample_rate` (a rate that is not finite or below 8 kHz is
    /// taken as 48 kHz).
    pub fn new(sample_rate: f32) -> Self {
        let mut t = Self {
            sample_rate: sane_rate(sample_rate),
            mode: SYNCED,
            time: 0.1,
            division: DIV_1_16,
            feel: STRAIGHT,
            gate: 100.0,
            trigger: START,
            key_track: 0.0,
            now: 0,
            was_playing: false,
            expect_ticks: 0.0,
            seq: 0,
            next_voice: 0,
            sounding: [Sounding::default(); CAPACITY],
            sounding_len: 0,
            held: [Held::default(); CAPACITY],
            held_len: 0,
        };
        for p in DESCRIPTOR.params {
            t.set_param(p.id, p.default as f32);
        }
        t
    }

    /// Notes sounding (emitted and not yet ended).
    pub fn sounding(&self) -> usize {
        self.sounding_len
    }

    /// Keys held back in release mode.
    pub fn held(&self) -> usize {
        self.held_len
    }

    /// The length, in frames, of a note of `key` and `tune_cents` starting in a block of
    /// `ctx`.
    pub fn length_frames(&self, ctx: &BlockCtx, key: u8, tune_cents: f32) -> u64 {
        let rate = rate_of(ctx, self.sample_rate);
        let seconds = if self.mode == FREE {
            self.time
        } else {
            let ticks = division_ticks(self.division, ctx.transport.signature) as f64
                * feel_factor(self.feel)
                * self.gate
                / 100.0;
            ticks / ctx.transport.tempo.ticks_per_second()
        };
        let pitch = f64::from(key.min(127)) + f64::from(tune_cents) / 100.0;
        let frames = (seconds * key_factor(self.key_track, pitch) * rate).round();
        if frames.is_finite() {
            (frames.max(1.0) as u64).min(MAX_LENGTH_FRAMES)
        } else {
            1
        }
    }

    /// Emits, in order, the end of every note due at or before the absolute frame `limit`
    /// (`inclusive`) or before it, at its offset in the block, never before `cursor`.
    fn end_due(&mut self, limit: u64, inclusive: bool, cursor: usize, out: &mut AudioIo<'_>) {
        loop {
            let mut best: Option<usize> = None;
            for i in 0..self.sounding_len {
                let s = &self.sounding[i];
                let due = if inclusive {
                    s.end <= limit
                } else {
                    s.end < limit
                };
                if due
                    && best.is_none_or(|b| {
                        let o = &self.sounding[b];
                        (s.end, s.seq) < (o.end, o.seq)
                    })
                {
                    best = Some(i);
                }
            }
            let Some(i) = best else {
                return;
            };
            let end = self.sounding[i].end;
            let at = (end.saturating_sub(self.now) as usize).max(cursor);
            self.end_at(i, at, out);
        }
    }

    /// Emits the end of the sounding note `i` at frame `at` and frees its entry.
    fn end_at(&mut self, i: usize, at: usize, out: &mut AudioIo<'_>) {
        let s = self.sounding[i];
        if let Some(voice) = VoiceId::new(s.voice) {
            out.events_out.push(Event::NoteOff {
                offset: offset(at),
                voice,
                key: s.key,
            });
        }
        self.sounding_len -= 1;
        self.sounding[i] = self.sounding[self.sounding_len];
    }

    /// A voice id no sounding note has.
    fn fresh_voice(&mut self) -> u32 {
        loop {
            let id = self.next_voice;
            self.next_voice = if id >= VoiceId::MAX { 0 } else { id + 1 };
            if !self.sounding[..self.sounding_len]
                .iter()
                .any(|s| s.voice == id)
            {
                return id;
            }
        }
    }

    /// Emits a note at frame `at` and schedules its end.
    fn start(
        &mut self,
        ctx: &BlockCtx,
        at: usize,
        key: u8,
        velocity: f32,
        tune_cents: f32,
        out: &mut AudioIo<'_>,
    ) {
        if self.sounding_len >= CAPACITY {
            let oldest = oldest(self.sounding[..self.sounding_len].iter().map(|s| s.seq));
            self.end_at(oldest, at, out);
        }
        let voice = self.fresh_voice();
        let length = self.length_frames(ctx, key, tune_cents);
        if let Some(v) = VoiceId::new(voice) {
            out.events_out.push(Event::NoteOn {
                offset: offset(at),
                voice: v,
                key,
                velocity,
                tune_cents,
            });
        }
        self.seq += 1;
        self.sounding[self.sounding_len] = Sounding {
            voice,
            key,
            end: (self.now + at as u64).saturating_add(length),
            seq: self.seq,
        };
        self.sounding_len += 1;
    }

    /// The held key of input `voice`, taken out of the table.
    fn take_held(&mut self, voice: u32) -> Option<Held> {
        let i = self.held[..self.held_len]
            .iter()
            .position(|h| h.voice == voice)?;
        let h = self.held[i];
        self.held_len -= 1;
        self.held[i] = self.held[self.held_len];
        Some(h)
    }

    fn hold(&mut self, held: Held) {
        if self.held_len >= CAPACITY {
            let oldest = oldest(self.held[..self.held_len].iter().map(|h| h.seq));
            self.held_len -= 1;
            self.held[oldest] = self.held[self.held_len];
        }
        self.seq += 1;
        self.held[self.held_len] = Held {
            seq: self.seq,
            ..held
        };
        self.held_len += 1;
    }
}

impl Device for Trim {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = ctx.frames as usize;
        let t = ctx.transport;
        let position = if t.position_ticks.is_finite() {
            t.position_ticks
        } else {
            self.expect_ticks
        };
        if self.was_playing && !t.playing {
            // A stop: every note ends at the first frame, the keys held back are forgotten.
            while self.sounding_len > 0 {
                self.end_at(self.sounding_len - 1, 0, io);
            }
            self.held_len = 0;
        } else if self.was_playing && (position - self.expect_ticks).abs() > JUMP_TICKS {
            self.held_len = 0;
        }
        self.was_playing = t.playing;
        self.expect_ticks =
            position + frames as f64 * t.tempo.ticks_per_second() / rate_of(ctx, self.sample_rate);
        let mut cursor = 0;
        for event in events {
            let at = event_frame(event.offset(), cursor, frames);
            // The ends due up to the event, each at its own frame (never before the last
            // event's), then the event.
            self.end_due(self.now + at as u64, true, cursor, io);
            cursor = at;
            match *event {
                Event::NoteOn {
                    voice,
                    key,
                    velocity,
                    tune_cents,
                    ..
                } => {
                    let key = key.min(127);
                    let velocity = sane_velocity(velocity);
                    let tune_cents = clamp_tune_cents(tune_cents);
                    // A voice reused while a key of it is still held back: the old one is
                    // forgotten.
                    let _ = self.take_held(voice.get());
                    if self.trigger == RELEASE {
                        self.hold(Held {
                            voice: voice.get(),
                            key,
                            velocity,
                            tune_cents,
                            seq: 0,
                        });
                    } else {
                        self.start(ctx, at, key, velocity, tune_cents, io);
                    }
                }
                Event::NoteOff { voice, .. } => {
                    if let Some(h) = self.take_held(voice.get()) {
                        self.start(ctx, at, h.key, h.velocity, h.tune_cents, io);
                    }
                }
                Event::AllNotesOff { .. } => {
                    io.events_out
                        .push(Event::AllNotesOff { offset: offset(at) });
                    self.sounding_len = 0;
                    self.held_len = 0;
                }
                Event::ParamValue { id, value, .. } => self.set_param(id, value),
            }
        }
        if frames > 0 {
            self.end_due(self.now + frames as u64, false, cursor, io);
        }
        self.now += frames as u64;
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize_f64(f64::from(value)) else {
            return;
        };
        match id.0 {
            MODE => self.mode = value as u32,
            TIME => self.time = value,
            DIVISION => self.division = value as u32,
            FEEL => self.feel = value as u32,
            GATE => self.gate = value,
            TRIGGER => self.trigger = value as u32,
            KEY_TRACK => self.key_track = value,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.sounding_len = 0;
        self.held_len = 0;
    }
}

/// The index of the smallest of `seqs` (0 when empty: the callers check the table is full).
fn oldest(seqs: impl Iterator<Item = u64>) -> usize {
    let mut best = (u64::MAX, 0);
    for (i, s) in seqs.enumerate() {
        if s < best.0 {
            best = (s, i);
        }
    }
    best.1
}

/// An offset in the block: frames are below `u32::MAX` (`BlockCtx::frames` is a `u32`).
fn offset(at: usize) -> u32 {
    at.min(u32::MAX as usize) as u32
}

fn sane_velocity(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
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

/// The native/WASM parity scenario of Trim (test build only, `--features testing`): a
/// seeded run of notes, parameter changes, stops, jumps and an overflow through blocks of
/// varied sizes and tempos, and the event stream it writes.
#[cfg(feature = "testing")]
pub mod testing {
    use super::{CAPACITY, DESCRIPTOR, Trim};
    use crate::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport, VoiceId};
    use crate::msg::ParamId;
    use crate::time::{Tempo, TimeSignature};

    /// Sample rate of the scenario.
    pub const RATE: f32 = 48_000.0;

    /// Values per event of [`scenario`]: absolute frame, kind (0 note-on, 1 note-off, 2
    /// all-notes-off), voice, key, velocity bits, tune bits.
    pub const STRIDE: usize = 6;

    /// Runs the scenario and returns its event stream, [`STRIDE`] values per event.
    pub fn scenario() -> Vec<i64> {
        let mut rng = 0x5EED_0693_u64;
        let mut next = move || {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        let mut trim = Trim::new(RATE);
        let Ok(mut out) = EventBuffer::try_with_capacity(4 * CAPACITY + 64) else {
            return Vec::new();
        };
        let mut left = vec![0.0f32; 2048];
        let mut right = vec![0.0f32; 2048];
        let mut stream = Vec::new();
        let sizes = [1usize, 37, 128, 500, 2048, 64, 333];
        let tempos = [120.0, 174.0, 61.3, 999.0, 20.0];
        let signatures = [(4, 4), (3, 4), (6, 8), (7, 8)];
        let mut frame = 0u64;
        let mut position = 0.0f64;
        let mut voice = 0u32;
        let mut held: Vec<(u32, u8)> = Vec::new();
        for block in 0..1200usize {
            let frames = sizes[block % sizes.len()];
            let tempo = Tempo::clamped(tempos[(block / 150) % tempos.len()]);
            let (n, d) = signatures[(block / 90) % signatures.len()];
            // The clock stops for a while every 200 blocks, and jumps every 170.
            let playing = block % 200 < 170;
            if block % 170 == 169 {
                position = 0.0;
            }
            let mut events = Vec::new();
            // Parameter changes (every parameter in turn, to its bounds and between).
            if block % 7 == 0 {
                let p = &DESCRIPTOR.params[(next() % DESCRIPTOR.params.len() as u64) as usize];
                let value = match next() % 4 {
                    0 => p.min,
                    1 => p.max,
                    _ => p.min + (p.max - p.min) * (next() % 1000) as f64 / 1000.0,
                };
                events.push(Event::ParamValue {
                    offset: (next() % frames as u64) as u32,
                    id: ParamId(p.id.0),
                    value: value as f32,
                });
            }
            // Notes, with the overflow (a burst of more than CAPACITY) once.
            let count = if block == 600 {
                CAPACITY + 8
            } else {
                (next() % 4) as usize
            };
            for _ in 0..count {
                let offset = (next() % frames as u64) as u32;
                if !held.is_empty() && next() % 2 == 0 {
                    let (v, key) = held.swap_remove((next() % held.len() as u64) as usize);
                    if let Some(voice) = VoiceId::new(v) {
                        events.push(Event::NoteOff { offset, voice, key });
                    }
                } else {
                    voice = (voice + 1) % 300;
                    let key = (next() % 128) as u8;
                    held.retain(|(v, _)| *v != voice);
                    held.push((voice, key));
                    if let Some(v) = VoiceId::new(voice) {
                        events.push(Event::NoteOn {
                            offset,
                            voice: v,
                            key,
                            velocity: (next() % 101) as f32 / 100.0,
                            tune_cents: (next() % 2401) as f32 - 1200.0,
                        });
                    }
                }
            }
            if block % 333 == 332 {
                events.push(Event::AllNotesOff {
                    offset: (next() % frames as u64) as u32,
                });
                held.clear();
            }
            events.sort_by_key(|e| (e.offset(), e.rank()));
            let ctx = BlockCtx {
                sample_rate: RATE,
                frames: frames as u32,
                transport: Transport {
                    playing,
                    tempo,
                    signature: TimeSignature::new(n, d).unwrap_or_default(),
                    position_ticks: position,
                },
            };
            out.clear();
            let mut io = AudioIo {
                left: &mut left[..frames],
                right: &mut right[..frames],
                events_out: &mut out,
            };
            trim.process(&ctx, &events, &mut io);
            out.finish();
            for e in out.events() {
                let (kind, voice, key, velocity, tune) = match *e {
                    Event::NoteOn {
                        voice,
                        key,
                        velocity,
                        tune_cents,
                        ..
                    } => (
                        0,
                        voice.get(),
                        key,
                        velocity.to_bits(),
                        tune_cents.to_bits(),
                    ),
                    Event::NoteOff { voice, key, .. } => (1, voice.get(), key, 0, 0),
                    Event::AllNotesOff { .. } => (2, 0, 0, 0, 0),
                    Event::ParamValue { .. } => continue,
                };
                stream.extend([
                    (frame + u64::from(e.offset())) as i64,
                    kind,
                    i64::from(voice),
                    i64::from(key),
                    i64::from(velocity),
                    i64::from(tune),
                ]);
            }
            frame += frames as u64;
            if playing {
                position += frames as f64 * tempo.ticks_per_second() / f64::from(RATE);
            }
        }
        stream
    }
}
