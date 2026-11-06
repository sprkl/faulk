//! Lattice, `faulk.lattice`, displayed "Lattice" (#692): a MIDI effect that puts every note it
//! receives on a scale, so that whatever is played comes out in key.
//!
//! # Where a note goes
//!
//! `source` picks the notes allowed:
//!
//! - `project` (the default): the project's scale ([`Transport::scale`], `ControlMsg::SetScale`),
//!   read at each note, so the notes follow its changes. A project without a scale lets every
//!   note through.
//! - `device`: the scale set in the device, `root` (a pitch class, C to B) and `scale` (one of
//!   the twelve [`MODES`], the project's list).
//! - `matrix`: twelve cells, one per pitch class of the received note, each naming the pitch
//!   class it becomes or `blocked`. A note goes to the nearest key of its cell's class, six
//!   semitones down to five up (a tie, six either way, goes down): C mapped to B plays the B
//!   below. A blocked note plays nothing.
//!
//! With a scale (`project` or `device`), a note of the scale stays, and `rule` moves one outside
//! it: `nearest` to the scale's closest note (a tie goes down), `down` to the scale's note below
//! it, `up` to the one above it, `block` nowhere (it plays nothing). The matrix ignores `rule`:
//! its cells say where each note goes. `transpose` (−24 to +24 semitones) is added after. A note
//! that would land outside 0 to 127 is not played (never clamped). The received velocity and
//! fine tuning (`tune_cents`) are kept.
//!
//! # Notes held, none stuck
//!
//! A fixed table, built in the factory, holds the received notes that sound ([`HELD`] of them),
//! each with the key it plays. A received note-off ends the key its note-on played, whatever
//! the settings or the scale are now: a change applies to the notes received after it. Two
//! received notes that land on the same key share it: the first starts it, the others are
//! counted, and the key ends at the last one's note-off, once (a count per key). A note-on for
//! a voice Lattice already holds (a broken guarantee) ends that voice's note first. An
//! `AllNotesOff` (a stop, a jump, a change of the chain) empties the table and goes on. A note
//! received while the table is full is not played (and its note-off ends nothing).
//! [`Device::reset`] empties the table and the counts in O(1) (a generation counter: the
//! entries of an older generation hold nothing; the tables are wiped only when it wraps, once in
//! 2^32): the engine resets a MIDI effect when it is
//! switched on or off (#219), and the host ends the instrument's notes when Lattice is
//! switched, removed or replaced. An output voice id is the key it plays: one per key sounding.
//!
//! # Output bounds
//!
//! A received note-on writes at most a note-off (the repeated voice's) and its note-on, a
//! note-off at most one note-off, an `AllNotesOff` one event: `per_input` 2, `per_block` 0.

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, EventBuffer,
    MidiOutput, ParamDescriptor, PitchSet, Scale, Unit, VoiceId, clamp_tune_cents, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

/// Ids of the parameters.
pub const SOURCE: u32 = 0;
pub const RULE: u32 = 1;
pub const TRANSPOSE: u32 = 2;
pub const ROOT: u32 = 3;
pub const SCALE: u32 = 4;
/// The id of the matrix cell of pitch class `pc` (0 = C … 11 = B): `MAP_BASE + pc`.
pub const MAP_BASE: u32 = 10;

/// The id of the matrix cell of pitch class `pc`.
pub const fn map_param(pc: u32) -> u32 {
    MAP_BASE + pc
}

/// The values of `source`.
pub const FROM_PROJECT: u32 = 0;
pub const FROM_DEVICE: u32 = 1;
pub const FROM_MATRIX: u32 = 2;
/// The values of `rule`.
pub const NEAREST: u32 = 0;
pub const DOWN: u32 = 1;
pub const UP: u32 = 2;
pub const BLOCK: u32 = 3;
/// The value of a matrix cell that plays nothing (after the twelve pitch classes).
pub const BLOCKED: u32 = 12;

/// Received notes held at once.
pub const HELD: usize = 128;
/// Widest transposition, in semitones either way.
pub const TRANSPOSE_MAX: f64 = 24.0;

/// The pitch classes' value keys, C first, spelled as the project's key picker spells them.
const PITCH_CLASSES: [&str; 12] = [
    "c", "c_sharp", "d", "e_flat", "e", "f", "f_sharp", "g", "a_flat", "a", "b_flat", "b",
];
/// A matrix cell's values: a pitch class, or blocked.
const CELLS: [&str; 13] = [
    "c", "c_sharp", "d", "e_flat", "e", "f", "f_sharp", "g", "a_flat", "a", "b_flat", "b",
    "blocked",
];

/// The scales of the `device` source: the project's list, in its order and under its names
/// (`project::ScaleMode`; a test of `crates/commands` keeps the two the same), with their
/// degrees in semitones above the root.
pub const MODES: [(&str, &[u8]); 12] = [
    ("major", &[0, 2, 4, 5, 7, 9, 11]),
    ("natural_minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("harmonic_minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("melodic_minor", &[0, 2, 3, 5, 7, 9, 11]),
    ("dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    ("lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("locrian", &[0, 1, 3, 5, 6, 8, 10]),
    ("major_pentatonic", &[0, 2, 4, 7, 9]),
    ("minor_pentatonic", &[0, 3, 5, 7, 10]),
    ("blues", &[0, 3, 5, 6, 7, 10]),
];

/// The names of [`MODES`], the values of `scale`.
const MODE_NAMES: [&str; 12] = {
    let mut names = [""; 12];
    let mut i = 0;
    while i < MODES.len() {
        names[i] = MODES[i].0;
        i += 1;
    }
    names
};

/// The pitch classes of `MODES[mode]` on `root`.
pub fn mode_set(root: u8, mode: usize) -> PitchSet {
    let Some((_, degrees)) = MODES.get(mode) else {
        return PitchSet::EMPTY;
    };
    let bits = degrees
        .iter()
        .fold(0u16, |bits, d| bits | 1 << ((root % 12 + d) % 12));
    PitchSet::from_bits(bits)
}

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

/// The matrix cell of pitch class `$pc`, named `$name` ("C sharp").
macro_rules! cell {
    ($pc:expr, $key:literal, $name:literal) => {
        choice(
            map_param($pc),
            concat!("map_", $key),
            concat!(
                "In matrix mode, the note a received ",
                $name,
                " becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played)."
            ),
            $pc as f64,
            &CELLS,
        )
    };
}

const PARAMS: [ParamDescriptor; 17] = [
    choice(
        SOURCE,
        "source",
        "Which notes are allowed: the project's scale (it follows the project's changes; without one, every note), the scale set in the device by root and scale, or the matrix, which says where each of the twelve notes goes.",
        FROM_PROJECT as f64,
        &["project", "device", "matrix"],
    ),
    choice(
        RULE,
        "rule",
        "Where a note outside the scale goes: to the nearest note of the scale (a tie goes down), always down, always up, or nowhere (it is not played). The matrix ignores it.",
        NEAREST as f64,
        &["nearest", "down", "up", "block"],
    ),
    ParamDescriptor {
        id: ParamId(TRANSPOSE),
        key: "transpose",
        description: "Semitones added to every note after it is put on the scale, from -24 to +24. A note pushed outside 0 to 127 is not played.",
        unit: Unit::None,
        min: -TRANSPOSE_MAX,
        max: TRANSPOSE_MAX,
        default: 0.0,
        scale: Scale::Linear,
        step: 1.0,
        smoothing_seconds: 0.0,
        values: &[],
    },
    choice(
        ROOT,
        "root",
        "Root of the device's scale, C to B (used when the source is the device).",
        0.0,
        &PITCH_CLASSES,
    ),
    choice(
        SCALE,
        "scale",
        "The device's scale, built on its root (used when the source is the device): the project's list of scales.",
        0.0,
        &MODE_NAMES,
    ),
    cell!(0, "c", "C"),
    cell!(1, "c_sharp", "C sharp"),
    cell!(2, "d", "D"),
    cell!(3, "e_flat", "E flat"),
    cell!(4, "e", "E"),
    cell!(5, "f", "F"),
    cell!(6, "f_sharp", "F sharp"),
    cell!(7, "g", "G"),
    cell!(8, "a_flat", "A flat"),
    cell!(9, "a", "A"),
    cell!(10, "b_flat", "B flat"),
    cell!(11, "b", "B"),
];

/// The descriptor of Lattice (CT-5): `faulk.lattice`, displayed "Lattice" (one English name,
/// never translated, founder's decision of 2026-11-06). The defaults follow the project's
/// scale, to its nearest note: nothing to set for the common case.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.lattice",
    kind_id: DeviceKind(12),
    version: 1,
    name: "Lattice",
    role: DeviceRole::MidiEffect,
    // The device, its two tables, and the most output the host gives it
    // (`MidiOutput::capacity` is at most `MAX_MIDI_OUTPUT_EVENTS`).
    memory_bytes: (size_of::<Lattice>()
        + size_of::<[Held; HELD]>()
        + size_of::<[KeyCount; 128]>()
        + crate::device::MAX_MIDI_OUTPUT_EVENTS * size_of::<Event>()) as u64,
    midi_output: Some(MidiOutput {
        per_input: 2,
        per_block: 0,
    }),
    named_keys: &[],
    params: &PARAMS,
};

/// The registry's line.
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory: Lattice::build,
};

/// A received note that sounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Held {
    /// The entry holds a note when this is the table's generation ([`Lattice::generation`]).
    generation: u32,
    /// The received note's voice id.
    voice: u32,
    /// The key it plays.
    key: u8,
}

/// How many held notes play a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct KeyCount {
    /// The count is this generation's ([`Lattice::generation`]); an older one counts 0.
    generation: u32,
    count: u16,
}

/// The device (module doc).
pub struct Lattice {
    source: u32,
    rule: u32,
    transpose: i32,
    root: u8,
    mode: usize,
    /// The matrix: the pitch class each one becomes, or [`BLOCKED`].
    matrix: [u8; 12],
    held: Box<[Held; HELD]>,
    keys: Box<[KeyCount; 128]>,
    /// The generation of the entries and counts in use (never 0, the generation of an entry
    /// never used).
    generation: u32,
}

impl Lattice {
    pub fn new(_setup: &DeviceSetup) -> Result<Self, MsgError> {
        let held = try_box([Held::default(); HELD])?;
        let keys = try_box([KeyCount::default(); 128])?;
        let mut lattice = Self {
            source: FROM_PROJECT,
            rule: NEAREST,
            transpose: 0,
            root: 0,
            mode: 0,
            matrix: [0; 12],
            held,
            keys,
            generation: 1,
        };
        for p in DESCRIPTOR.params {
            lattice.set_param(p.id, p.default as f32);
        }
        Ok(lattice)
    }

    pub fn build(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
        Ok(try_box(Self::new(setup)?)?)
    }

    /// The key a received `key` plays now, with the project's `scale`; `None`: not played.
    pub fn target(&self, key: u8, scale: PitchSet) -> Option<u8> {
        let key = key.min(127);
        let mapped = match self.source {
            FROM_MATRIX => {
                let pc = key % 12;
                let to = *self.matrix.get(usize::from(pc))?;
                if u32::from(to) >= BLOCKED {
                    return None;
                }
                let offset = (i32::from(to) - i32::from(pc) + 6).rem_euclid(12) - 6;
                i32::from(key) + offset
            }
            FROM_DEVICE => quantize(key, mode_set(self.root, self.mode), self.rule)?,
            _ => quantize(key, scale, self.rule)?,
        };
        let out = mapped + self.transpose;
        u8::try_from(out).ok().filter(|k| *k <= 127)
    }

    #[allow(clippy::too_many_arguments)]
    fn note_on(
        &mut self,
        at: usize,
        voice: VoiceId,
        key: u8,
        velocity: f32,
        tune_cents: f32,
        scale: PitchSet,
        out: &mut EventBuffer,
    ) {
        // A note-on for a voice already held (a broken guarantee) ends that one first.
        self.note_off(at, voice, out);
        let Some(key) = self.target(key, scale) else {
            return;
        };
        let generation = self.generation;
        let Some(entry) = self.held.iter_mut().find(|h| h.generation != generation) else {
            return;
        };
        *entry = Held {
            generation,
            voice: voice.get(),
            key,
        };
        let Some(count) = self.keys.get_mut(usize::from(key)) else {
            return;
        };
        if count.generation != generation {
            *count = KeyCount {
                generation,
                count: 0,
            };
        }
        count.count = count.count.saturating_add(1);
        if count.count == 1 {
            let velocity = if velocity.is_finite() {
                velocity.clamp(0.0, 1.0)
            } else {
                0.0
            };
            out.push(Event::NoteOn {
                offset: at as u32,
                voice: out_voice(key),
                key,
                velocity,
                tune_cents: clamp_tune_cents(tune_cents),
            });
        }
    }

    fn note_off(&mut self, at: usize, voice: VoiceId, out: &mut EventBuffer) {
        let generation = self.generation;
        let Some(entry) = self
            .held
            .iter_mut()
            .find(|h| h.generation == generation && h.voice == voice.get())
        else {
            return;
        };
        entry.generation = 0;
        let key = entry.key;
        let Some(count) = self.keys.get_mut(usize::from(key)) else {
            return;
        };
        if count.generation != generation || count.count == 0 {
            return;
        }
        count.count -= 1;
        if count.count == 0 {
            out.push(Event::NoteOff {
                offset: at as u32,
                voice: out_voice(key),
                key,
            });
        }
    }

    /// Empties the table and the counts in O(1): the entries of the old generation hold
    /// nothing any more.
    fn clear(&mut self) {
        self.generation = match self.generation.wrapping_add(1) {
            // Once in 2^32 clears the counter wraps: the oldest entries would hold again, so
            // the tables are wiped then (256 small writes, never otherwise).
            0 => {
                self.held.iter_mut().for_each(|h| h.generation = 0);
                self.keys.iter_mut().for_each(|k| k.generation = 0);
                1
            }
            g => g,
        };
    }
}

impl Device for Lattice {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).max(1);
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
                } => self.note_on(at, voice, key, velocity, tune_cents, scale, io.events_out),
                Event::NoteOff { voice, .. } => self.note_off(at, voice, io.events_out),
                Event::AllNotesOff { .. } => {
                    self.clear();
                    io.events_out.push(Event::AllNotesOff { offset: at as u32 });
                }
                Event::ParamValue { id, value, .. } => self.set_param(id, value),
            }
        }
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        match id.0 {
            SOURCE => self.source = value as u32,
            RULE => self.rule = value as u32,
            TRANSPOSE => self.transpose = value as i32,
            ROOT => self.root = (value as u32 % 12) as u8,
            SCALE => self.mode = (value as usize).min(MODES.len() - 1),
            id => {
                let pc = id.wrapping_sub(MAP_BASE) as usize;
                if let Some(cell) = self.matrix.get_mut(pc) {
                    *cell = (value as u32).min(BLOCKED) as u8;
                }
            }
        }
    }

    fn reset(&mut self) {
        self.clear();
    }
}

/// The output voice id of `key`: one per key sounding.
fn out_voice(key: u8) -> VoiceId {
    VoiceId::new(u32::from(key)).unwrap_or_default()
}

/// `key` put on `scale` by `rule` (module doc), as a MIDI key that may fall outside 0..=127;
/// `None`: not played. An empty scale lets every key through.
pub fn quantize(key: u8, scale: PitchSet, rule: u32) -> Option<i32> {
    let key = i32::from(key);
    let inside = |k: i32| scale.contains(k.rem_euclid(12) as u8);
    if scale.is_empty() || inside(key) {
        return Some(key);
    }
    // A scale that is not empty holds a pitch class within 11 semitones either way.
    let below = (1..12).map(|d| key - d).find(|&k| inside(k));
    let above = (1..12).map(|d| key + d).find(|&k| inside(k));
    match rule {
        BLOCK => None,
        DOWN => below,
        UP => above,
        _ => match (below, above) {
            (Some(b), Some(a)) => Some(if key - b <= a - key { b } else { a }),
            (b, a) => b.or(a),
        },
    }
}
