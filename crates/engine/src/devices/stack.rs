//! Stack, `faulk.stack`, displayed "Stack" (#690): a MIDI effect that plays a chord from one
//! note. Each note it receives comes out with up to six added notes, at fixed intervals or in
//! the project's scale, all at once or strummed over up to 400 ms.
//!
//! # The chord
//!
//! The received note always plays: it is the chord's root. Each of the six **voices** that is
//! on adds one note at its `interval` from the root, with its `velocity` (a share of the
//! received velocity, 1 to 200 %, at most 1) and its `chance` of playing. A note that would fall
//! outside 0 to 127 is not played (never clamped: no doubled key). Every note of the chord
//! carries the received note's fine tuning (`tune_cents`), so a tuned note gives a tuned chord.
//!
//! With `mode` at `scale` and a project scale set ([`Transport::scale`]), an interval counts
//! **degrees** of the scale: the root's degree plus `interval` degrees, the octave wrapping (in A
//! minor, +2 and +4 degrees give A4 → C5 and E5, B4 → D5 and F5). A root outside the scale counts
//! from the scale's note below it and keeps its distance to it, so the chord moves with it.
//! Without a project scale, `scale` reads semitones, as `semitones` does.
//!
//! # The strum
//!
//! The chord's notes that play (after the chance draw) are ordered by pitch, rising (`up`) or
//! falling (`down`). Of `n` notes, the `k`-th starts `round(strum × sample_rate × f(x))` frames
//! after the received note-on, with `x = k / (n − 1)` (0 when `n` is 1) and the tension curve
//! `f(x) = x / (x + (1 − x) · w)`: `w` is 1 at tension 0 (`f(x) = x`, even gaps: 200 ms on three
//! notes start them at 0, 100 and 200 ms), `1 / (1 + 3t)` at tension `t` above 0 (the gaps
//! tighten toward the last note) and `1 − 3t` below 0 (they widen). Only additions, products and
//! quotients of `f64`: the onsets are the same frame natively and in WebAssembly. `crescendo` `c`
//! scales the `k`-th velocity by `1 − c (1 − x)` above 0 (the last at full) and `1 + c x` below
//! (the first at full); a lone note keeps its velocity. A velocity never goes below 1/127.
//!
//! # The draw
//!
//! `chance` draws per voice and per received note: a hash (splitmix64) of the `seed`, a count of
//! the notes received and the voice, against `chance / 100`. The count restarts at
//! [`Device::reset`] and when the transport starts playing, so a render from the start draws the
//! same chords at equal seed, on both targets.
//!
//! # Notes held, none stuck
//!
//! A fixed table, built in the factory, holds the received notes that sound ([`HELD`] of them),
//! each with the notes it started and the ones its strum has still to start. A received note-off
//! ends exactly the notes its note-on started and drops the ones still to start, whatever the
//! settings are now: a setting applies to the notes received after it. An `AllNotesOff` (a stop,
//! a change of the chain) empties the table and goes on. A note received while the table is full
//! is not played (and its note-off ends nothing). [`Device::reset`] empties the table: the engine
//! resets a MIDI effect when it is switched on or off (#219), and the host ends the instrument's
//! notes when Stack is switched, removed or replaced. An output voice id is `entry × 8 + note`:
//! unique among the notes Stack holds, under 1024.
//!
//! # Output bounds
//!
//! A received note-on writes at most its 7 note-ons, a note-off its 7 note-offs, an
//! `AllNotesOff` one event: `per_input` 7. The strum notes that start in a block are at most
//! the table's: `per_block` `HELD × 7`. Pending notes are written in time order between the
//! events, so the output stays sorted.

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MidiOutput,
    ParamDescriptor, PitchSet, Scale, Unit, VoiceId, clamp_tune_cents, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

/// Ids of the parameters that are not a voice's.
pub const MODE: u32 = 0;
pub const STRUM: u32 = 1;
pub const TENSION: u32 = 2;
pub const CRESCENDO: u32 = 3;
pub const DIRECTION: u32 = 4;
pub const SEED: u32 = 5;
/// The id of voice `v` (0 to 5)'s parameter `p`: `VOICE_BASE + 4 v + p`, `p` one of
/// [`VOICE_ON`], [`VOICE_INTERVAL`], [`VOICE_VELOCITY`], [`VOICE_CHANCE`].
pub const VOICE_BASE: u32 = 10;
pub const VOICE_ON: u32 = 0;
pub const VOICE_INTERVAL: u32 = 1;
pub const VOICE_VELOCITY: u32 = 2;
pub const VOICE_CHANCE: u32 = 3;

/// The id of voice `voice`'s parameter `which`.
pub const fn voice_param(voice: u32, which: u32) -> u32 {
    VOICE_BASE + 4 * voice + which
}

/// The values of `mode`.
pub const SEMITONES: u32 = 0;
pub const IN_SCALE: u32 = 1;
/// The values of `direction`.
pub const UP: u32 = 0;
pub const DOWN: u32 = 1;

/// Added voices.
pub const VOICES: usize = 6;
/// Notes of a chord: the root and the voices.
const CHORD: usize = VOICES + 1;
/// Received notes held at once.
pub const HELD: usize = 128;
/// Longest strum, in seconds.
pub const STRUM_MAX: f64 = 0.4;
/// The lowest velocity a note of the chord gets.
const MIN_VELOCITY: f32 = 1.0 / 127.0;

const fn choice(
    id: u32,
    key: &'static str,
    description: &'static str,
    default: f64,
    values: &'static [&'static str],
) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit: Unit::None,
        min: 0.0,
        max: (values.len() - 1) as f64,
        default,
        scale: Scale::Linear,
        step: 1.0,
        smoothing_seconds: 0.0,
        values,
    }
}

#[allow(clippy::too_many_arguments)]
const fn stepped(
    id: u32,
    key: &'static str,
    description: &'static str,
    unit: Unit,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit,
        min,
        max,
        default,
        scale: Scale::Linear,
        step,
        smoothing_seconds: 0.0,
        values: &[],
    }
}

/// The four parameters of one voice.
macro_rules! voice {
    ($v:expr, $n:literal, $on:expr, $interval:expr) => {
        [
            choice(
                voice_param($v, VOICE_ON),
                concat!("voice", $n),
                concat!("Whether added voice ", $n, " plays."),
                $on,
                &["off", "on"],
            ),
            stepped(
                voice_param($v, VOICE_INTERVAL),
                concat!("voice", $n, "_interval"),
                concat!(
                    "Interval of voice ",
                    $n,
                    " from the received note, from -36 to +36: semitones, or degrees of the project's scale in scale mode."
                ),
                Unit::None,
                -36.0,
                36.0,
                $interval,
                1.0,
            ),
            stepped(
                voice_param($v, VOICE_VELOCITY),
                concat!("voice", $n, "_velocity"),
                concat!(
                    "Velocity of voice ",
                    $n,
                    ", from 1 to 200 % of the received note's."
                ),
                Unit::Percent,
                1.0,
                200.0,
                100.0,
                0.0,
            ),
            stepped(
                voice_param($v, VOICE_CHANCE),
                concat!("voice", $n, "_chance"),
                concat!(
                    "Chance that voice ",
                    $n,
                    " plays on each received note, from 0 to 100 %: a repeatable draw, set by the seed."
                ),
                Unit::Percent,
                0.0,
                100.0,
                100.0,
                1.0,
            ),
        ]
    };
}

const V1: [ParamDescriptor; 4] = voice!(0, "1", 1.0, 4.0);
const V2: [ParamDescriptor; 4] = voice!(1, "2", 1.0, 7.0);
const V3: [ParamDescriptor; 4] = voice!(2, "3", 0.0, 12.0);
const V4: [ParamDescriptor; 4] = voice!(3, "4", 0.0, -12.0);
const V5: [ParamDescriptor; 4] = voice!(4, "5", 0.0, 19.0);
const V6: [ParamDescriptor; 4] = voice!(5, "6", 0.0, 24.0);

const PARAMS: [ParamDescriptor; 30] = [
    choice(
        MODE,
        "mode",
        "How the voices' intervals are read: in semitones, or in degrees of the project's scale (a third is two degrees) so the chord stays in key. Without a project scale, in semitones.",
        SEMITONES as f64,
        &["semitones", "scale"],
    ),
    stepped(
        STRUM,
        "strum",
        "Time from the first to the last note of the chord, from 0 (all at once) to 0.4 s: the notes are spread like a strummed guitar.",
        Unit::Seconds,
        0.0,
        STRUM_MAX,
        0.0,
        0.001,
    ),
    stepped(
        TENSION,
        "tension",
        "How the strum's gaps change, from -100 % (they widen toward the last note) to +100 % (they tighten); 0 spaces the notes evenly.",
        Unit::Percent,
        -100.0,
        100.0,
        0.0,
        1.0,
    ),
    stepped(
        CRESCENDO,
        "crescendo",
        "Velocity across the strum: above 0 the notes grow louder toward the last, below 0 softer; at 100 % the first (or last) note is at the lowest velocity.",
        Unit::Percent,
        -100.0,
        100.0,
        0.0,
        1.0,
    ),
    choice(
        DIRECTION,
        "direction",
        "Order of the strum: up plays from the lowest note to the highest, down from the highest to the lowest.",
        UP as f64,
        &["up", "down"],
    ),
    stepped(
        SEED,
        "seed",
        "Seed of the chance draws, from 0 to 1000: the same seed gives the same chords on every play from the start; another seed, another draw.",
        Unit::None,
        0.0,
        1000.0,
        0.0,
        1.0,
    ),
    V1[0],
    V1[1],
    V1[2],
    V1[3],
    V2[0],
    V2[1],
    V2[2],
    V2[3],
    V3[0],
    V3[1],
    V3[2],
    V3[3],
    V4[0],
    V4[1],
    V4[2],
    V4[3],
    V5[0],
    V5[1],
    V5[2],
    V5[3],
    V6[0],
    V6[1],
    V6[2],
    V6[3],
];

/// The descriptor of Stack (CT-5): `faulk.stack`, displayed "Stack" (one English name, never
/// translated, founder's decision of 2026-11-06). The defaults are a major triad in semitones,
/// played at once.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.stack",
    kind_id: DeviceKind(9),
    version: 1,
    name: "Stack",
    role: DeviceRole::MidiEffect,
    memory_bytes: (size_of::<Stack>() + size_of::<[Held; HELD]>()) as u64,
    midi_output: Some(MidiOutput {
        per_input: CHORD as u32,
        per_block: (HELD * CHORD) as u32,
    }),
    named_keys: &[],
    params: &PARAMS,
};

/// The registry's line.
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory: Stack::build,
};

/// A note of a held chord.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ChordNote {
    state: NoteState,
    key: u8,
    velocity: f32,
    /// The frame it starts at, counted from the last reset.
    due: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NoteState {
    #[default]
    None,
    /// Its strum has still to start it.
    Pending,
    Sounding,
}

/// A received note that sounds, with its chord.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Held {
    used: bool,
    /// The received note's voice id.
    voice: u32,
    tune_cents: f32,
    notes: [ChordNote; CHORD],
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct VoiceSettings {
    on: bool,
    interval: i32,
    /// A share of the received velocity (1 = 100 %).
    velocity: f32,
    /// 0 to 1.
    chance: f64,
}

/// The device (module doc).
pub struct Stack {
    in_scale: bool,
    strum_seconds: f64,
    /// -1 to 1.
    tension: f64,
    /// -1 to 1.
    crescendo: f32,
    down: bool,
    seed: u64,
    voices: [VoiceSettings; VOICES],
    held: Box<[Held; HELD]>,
    /// Frames rendered since the last reset.
    now: u64,
    /// Notes received since the last reset or the last start of the transport (the draw).
    received: u64,
    playing: bool,
    /// Notes the table holds still to start (0: nothing to look for).
    pending: u32,
    sample_rate: f64,
}

impl Stack {
    pub fn new(setup: &DeviceSetup) -> Result<Self, MsgError> {
        let held = try_box([Held::default(); HELD])?;
        let mut stack = Self {
            in_scale: false,
            strum_seconds: 0.0,
            tension: 0.0,
            crescendo: 0.0,
            down: false,
            seed: 0,
            voices: [VoiceSettings {
                on: false,
                interval: 0,
                velocity: 1.0,
                chance: 1.0,
            }; VOICES],
            held,
            now: 0,
            received: 0,
            playing: false,
            pending: 0,
            sample_rate: sane_rate(setup.sample_rate),
        };
        for p in DESCRIPTOR.params {
            stack.set_param(p.id, p.default as f32);
        }
        Ok(stack)
    }

    pub fn build(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
        Ok(try_box(Self::new(setup)?)?)
    }

    /// Starts the pending notes due before `limit` (an offset in the block), in time order.
    fn start_due(&mut self, limit: usize, out: &mut crate::device::EventBuffer) {
        let end = self.now.saturating_add(limit as u64);
        while self.pending > 0 {
            let mut first: Option<(usize, usize, u64)> = None;
            for (e, held) in self.held.iter().enumerate() {
                if !held.used {
                    continue;
                }
                for (j, note) in held.notes.iter().enumerate() {
                    if note.state == NoteState::Pending
                        && note.due < end
                        && first.is_none_or(|(_, _, due)| note.due < due)
                    {
                        first = Some((e, j, note.due));
                    }
                }
            }
            let Some((e, j, due)) = first else {
                return;
            };
            let held = &mut self.held[e];
            let note = &mut held.notes[j];
            note.state = NoteState::Sounding;
            self.pending = self.pending.saturating_sub(1);
            out.push(Event::NoteOn {
                offset: due.saturating_sub(self.now) as u32,
                voice: out_voice(e, j),
                key: note.key,
                velocity: note.velocity,
                tune_cents: held.tune_cents,
            });
        }
    }

    fn note_on(
        &mut self,
        at: usize,
        voice: VoiceId,
        key: u8,
        velocity: f32,
        tune_cents: f32,
        scale: PitchSet,
        out: &mut crate::device::EventBuffer,
    ) {
        // A note-on for a voice already held (a broken guarantee) ends that one first.
        self.note_off(at, voice, out);
        let Some(entry) = self.held.iter().position(|h| !h.used) else {
            return;
        };
        let draw = self.received;
        self.received = self.received.wrapping_add(1);
        let key = key.min(127);
        let velocity = if velocity.is_finite() {
            velocity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        // The chord: the root, then each voice that is on, falls in 0..=127 and wins its draw.
        let mut chord = [(0u8, 0f32); CHORD];
        chord[0] = (key, velocity);
        let mut n = 1;
        let in_scale = self.in_scale && !scale.is_empty();
        for (v, settings) in self.voices.iter().enumerate() {
            if !settings.on || !wins(self.seed, draw, v as u64, settings.chance) {
                continue;
            }
            let target = if in_scale {
                degree_shift(key, settings.interval, scale)
            } else {
                i32::from(key) + settings.interval
            };
            if !(0..=127).contains(&target) {
                continue;
            }
            chord[n] = (target as u8, velocity * settings.velocity);
            n += 1;
        }
        // The strum's order: by pitch, rising or falling (insertion sort: stable, no allocation).
        for i in 1..n {
            let mut k = i;
            while k > 0 && before(chord[k].0, chord[k - 1].0, self.down) {
                chord.swap(k, k - 1);
                k -= 1;
            }
        }
        let tune_cents = clamp_tune_cents(tune_cents);
        let start = self.now.saturating_add(at as u64);
        let held = &mut self.held[entry];
        *held = Held {
            used: true,
            voice: voice.get(),
            tune_cents,
            notes: [ChordNote::default(); CHORD],
        };
        let span = self.strum_seconds * self.sample_rate;
        let w = tension_weight(self.tension);
        for (k, &(note_key, note_velocity)) in chord[..n].iter().enumerate() {
            let x = if n > 1 {
                k as f64 / (n - 1) as f64
            } else {
                0.0
            };
            let onset = (span * curve(x, w)).round();
            let onset = if onset.is_finite() && onset > 0.0 {
                onset as u64
            } else {
                0
            };
            let factor = if n > 1 {
                crescendo_factor(self.crescendo, x as f32)
            } else {
                1.0
            };
            let velocity = (note_velocity * factor).clamp(MIN_VELOCITY, 1.0);
            let pending = onset > 0;
            held.notes[k] = ChordNote {
                state: if pending {
                    NoteState::Pending
                } else {
                    NoteState::Sounding
                },
                key: note_key,
                velocity,
                due: start.saturating_add(onset),
            };
            if pending {
                self.pending += 1;
            } else {
                out.push(Event::NoteOn {
                    offset: at as u32,
                    voice: out_voice(entry, k),
                    key: note_key,
                    velocity,
                    tune_cents,
                });
            }
        }
    }

    fn note_off(&mut self, at: usize, voice: VoiceId, out: &mut crate::device::EventBuffer) {
        let Some(entry) = self
            .held
            .iter()
            .position(|h| h.used && h.voice == voice.get())
        else {
            return;
        };
        let held = &mut self.held[entry];
        for (j, note) in held.notes.iter_mut().enumerate() {
            match note.state {
                NoteState::Sounding => {
                    out.push(Event::NoteOff {
                        offset: at as u32,
                        voice: out_voice(entry, j),
                        key: note.key,
                    });
                }
                NoteState::Pending => self.pending = self.pending.saturating_sub(1),
                NoteState::None => {}
            }
            note.state = NoteState::None;
        }
        held.used = false;
    }

    fn clear(&mut self) {
        for held in self.held.iter_mut() {
            held.used = false;
        }
        self.pending = 0;
    }
}

impl Device for Stack {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).max(1);
        if ctx.transport.playing && !self.playing {
            self.received = 0;
        }
        self.playing = ctx.transport.playing;
        let scale = ctx.transport.scale;
        let mut cursor = 0;
        for event in events {
            let at = event_frame(event.offset(), cursor, frames);
            cursor = at;
            match *event {
                Event::NoteOn {
                    voice,
                    key,
                    velocity,
                    tune_cents,
                    ..
                } => {
                    self.start_due(at, io.events_out);
                    self.note_on(at, voice, key, velocity, tune_cents, scale, io.events_out);
                }
                Event::NoteOff { voice, .. } => {
                    // A strum note due at the note-off's frame would end at once: it is dropped.
                    self.start_due(at, io.events_out);
                    self.note_off(at, voice, io.events_out);
                }
                Event::AllNotesOff { .. } => {
                    self.clear();
                    io.events_out.push(Event::AllNotesOff { offset: at as u32 });
                }
                Event::ParamValue { id, value, .. } => self.set_param(id, value),
            }
        }
        self.start_due(ctx.frames as usize, io.events_out);
        self.now = self.now.saturating_add(u64::from(ctx.frames));
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        match id.0 {
            MODE => self.in_scale = value as u32 == IN_SCALE,
            STRUM => self.strum_seconds = f64::from(value),
            TENSION => self.tension = f64::from(value) / 100.0,
            CRESCENDO => self.crescendo = value / 100.0,
            DIRECTION => self.down = value as u32 == DOWN,
            SEED => self.seed = value as u64,
            id => {
                let v = ((id - VOICE_BASE) / 4) as usize;
                let Some(voice) = self.voices.get_mut(v) else {
                    return;
                };
                match (id - VOICE_BASE) % 4 {
                    VOICE_ON => voice.on = value >= 0.5,
                    VOICE_INTERVAL => voice.interval = value as i32,
                    VOICE_VELOCITY => voice.velocity = value / 100.0,
                    _ => voice.chance = f64::from(value) / 100.0,
                }
            }
        }
    }

    fn reset(&mut self) {
        self.clear();
        self.now = 0;
        self.received = 0;
        self.playing = false;
    }
}

/// The output voice id of note `note` of entry `entry`.
fn out_voice(entry: usize, note: usize) -> VoiceId {
    VoiceId::new((entry * 8 + note) as u32).unwrap_or_default()
}

/// Whether `a` comes before `b` in the strum.
fn before(a: u8, b: u8, down: bool) -> bool {
    if down { a > b } else { a < b }
}

/// The weight of the tension curve (module doc).
fn tension_weight(tension: f64) -> f64 {
    let t = tension.clamp(-1.0, 1.0);
    if t >= 0.0 {
        1.0 / (1.0 + 3.0 * t)
    } else {
        1.0 - 3.0 * t
    }
}

/// The tension curve: `x / (x + (1 − x) w)`, exactly `x` at `w = 1`.
fn curve(x: f64, w: f64) -> f64 {
    if w == 1.0 {
        return x;
    }
    let d = x + (1.0 - x) * w;
    if d > 0.0 {
        (x / d).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The velocity factor of the note at `x` of the strum (module doc).
fn crescendo_factor(c: f32, x: f32) -> f32 {
    if c >= 0.0 {
        1.0 - c * (1.0 - x)
    } else {
        1.0 + c * x
    }
}

/// Whether a voice with `chance` plays on the `draw`-th received note.
fn wins(seed: u64, draw: u64, voice: u64, chance: f64) -> bool {
    if chance >= 1.0 {
        return true;
    }
    if chance <= 0.0 {
        return false;
    }
    let h = splitmix64(seed.wrapping_mul(0x1_0000_0001) ^ draw.wrapping_mul(8) ^ voice);
    let u = (h >> 11) as f64 / (1u64 << 53) as f64;
    u < chance
}

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `key` moved by `degrees` degrees of `scale` (not empty), as a MIDI key (may fall outside
/// 0..=127). A key outside the scale counts from the scale's note below it.
pub fn degree_shift(key: u8, degrees: i32, scale: PitchSet) -> i32 {
    let mut classes = [0u8; 12];
    let mut m = 0usize;
    for pc in 0..12u8 {
        if scale.contains(pc) {
            classes[m] = pc;
            m += 1;
        }
    }
    if m == 0 {
        return i32::from(key) + degrees;
    }
    let key = i32::from(key);
    let mut octave = key.div_euclid(12);
    let pc = key.rem_euclid(12);
    let index = match classes[..m].iter().rposition(|&c| i32::from(c) <= pc) {
        Some(i) => i,
        None => {
            octave -= 1;
            m - 1
        }
    };
    let base = octave * 12 + i32::from(classes[index]);
    let offset = key - base;
    let total = index as i32 + degrees;
    let m = m as i32;
    let target_octave = octave + total.div_euclid(m);
    let target = classes[total.rem_euclid(m) as usize];
    target_octave * 12 + i32::from(target) + offset
}

fn sane_rate(rate: f32) -> f64 {
    if rate.is_finite() && rate >= 8_000.0 {
        f64::from(rate.min(crate::device::MAX_SAMPLE_RATE))
    } else {
        48_000.0
    }
}
