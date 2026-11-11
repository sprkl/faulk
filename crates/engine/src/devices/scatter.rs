//! Scatter, `faulk.scatter`, displayed "Scatter" (#691): a MIDI effect that moves a note's
//! pitch. Each note it receives has a `chance` of coming out at another pitch, one of
//! `choices` pitches `interval` apart, above, below or around it, in semitones or in the
//! project's scale. A melody or an arpeggio repeated along the song changes from one repetition
//! to the next, the same way on every play.
//!
//! # The choices
//!
//! Choice `k` (1 to `choices`) lies `k × interval` steps above the received note (`up`), below
//! it (`down`) or, `around`, alternately above and below: `+1, −1, +2, −2, …` steps (`k` odd:
//! `+(k + 1) / 2`, `k` even: `−k / 2`), so one choice around is one step above and two choices
//! are one step on each side.
//!
//! With `mode` at `semitones` a step is `interval` semitones. With `mode` at `scale` and a
//! project scale set ([`Transport::scale`]), a step is `interval` **degrees**: the received
//! note is taken to its degree (the scale's note at or below it, the octave wrapping), and the
//! note comes out on the scale's note that many degrees away, so a moved note is always in the
//! scale, even when the received one is not (in F minor, two degrees up from F4 is A♭4, and
//! from F♯4 too). Without a project scale, `scale` reads semitones, as `semitones` does.
//!
//! A note that loses the chance draw comes out as received, in either mode: Scatter never
//! silences a note, and at a chance of 0 the stream goes through unchanged (bringing every note
//! into the scale is another device's work, #692). A choice that would fall outside 0 to 127 is
//! not taken: in semitones the note comes out as received; in the scale it comes out on its own
//! degree (the scale's note at or below it, or the next one up when that is below 0), so that a
//! moved note is in the scale whatever its choice. Velocity, voice id and fine tuning
//! (`tune_cents`) are passed through.
//!
//! # The draws
//!
//! Each received note-on draws from a hash of the `seed`, the note's place and its key
//! ([`super::draw`]): the first number, against `chance / 100`, decides whether the note moves;
//! with `order` at `random`, the second picks the choice (`floor(u × choices)`). While the
//! transport plays, the place is the note's tick in the song, so the same note at the same place
//! moves the same way on every play, from wherever the play starts, and in the export, natively
//! and in WebAssembly; a live note played meanwhile shifts nothing, and a loop plays the same
//! draw on every pass (the lead's decision of 2026-11-06). While it is stopped, the place is a
//! count of the note-ons received stopped, restarted by [`Device::reset`].
//!
//! With `order` at `cycle`, each moved note takes the next choice, wrapping: a count of the
//! moved notes, restarted at [`Device::reset`] and when the transport starts playing. A turn
//! follows the notes, not their place: it goes on across the passes of a loop and a seek, so
//! only a play from the same start repeats it (the chance draw stays by place).
//!
//! # Notes held, none stuck
//!
//! A fixed table, built in the factory, holds the received notes that sound ([`HELD`] of them)
//! with the key each came out at. A received note-off comes out at that key and voice, whatever
//! the settings are now: a setting applies to the notes received after it. A note-on for a voice
//! already held (a broken guarantee) ends the held note first. A note received while the table
//! is full goes through unchanged, as does a note-off for a voice the table does not hold: an
//! instrument releases by voice id, which Scatter never changes. An `AllNotesOff` (a stop, a
//! change of the chain) empties the table and goes on. [`Device::reset`] empties the table in
//! O(1) (a generation counter: the entries of an older generation hold nothing): the engine
//! resets a MIDI effect when it is switched on or off (#219), and the host ends the instrument's
//! notes when Scatter is switched, removed or replaced.
//!
//! # Output bounds
//!
//! A received note-on writes at most a note-off (a repeated voice) and its note-on; a note-off
//! and an `AllNotesOff` one event each: `per_input` 2, `per_block` 0.

use super::Builtin;
use super::draw::{Draw, Place};
use super::held::HeldTable;
use super::params::{choice, stepped};
pub use super::scale::scale_step;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MidiOutput,
    ParamDescriptor, PitchSet, Unit, VoiceId, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

/// Ids of the parameters.
pub const CHANCE: u32 = 0;
pub const CHOICES: u32 = 1;
pub const INTERVAL: u32 = 2;
pub const DIRECTION: u32 = 3;
pub const MODE: u32 = 4;
pub const ORDER: u32 = 5;
pub const SEED: u32 = 6;

/// The values of `direction`.
pub const UP: u32 = 0;
pub const DOWN: u32 = 1;
pub const AROUND: u32 = 2;
/// The values of `mode`.
pub const SEMITONES: u32 = 0;
pub const IN_SCALE: u32 = 1;
/// The values of `order`.
pub const RANDOM: u32 = 0;
pub const CYCLE: u32 = 1;

/// Most choices.
pub const MAX_CHOICES: u32 = 24;
/// Received notes held at once.
pub const HELD: usize = 128;

const PARAMS: [ParamDescriptor; 7] = [
    stepped(
        CHANCE,
        "chance",
        "Chance that a received note comes out at another pitch, from 0 (never: the notes go through unchanged) to 100 % (every note): a repeatable draw, set by the seed.",
        Unit::Percent,
        0.0,
        100.0,
        50.0,
        1.0,
    ),
    stepped(
        CHOICES,
        "choices",
        "How many pitches a moved note can take, from 1 to 24, each one interval further from the received note.",
        Unit::None,
        1.0,
        MAX_CHOICES as f64,
        2.0,
        1.0,
    ),
    stepped(
        INTERVAL,
        "interval",
        "Distance between two choices, from 1 to 12: semitones, or degrees of the project's scale in scale mode (12 semitones is an octave).",
        Unit::None,
        1.0,
        12.0,
        12.0,
        1.0,
    ),
    choice(
        DIRECTION,
        "direction",
        "Where the choices lie: above the received note, below it, or around it (alternately above and below).",
        AROUND as f64,
        &["up", "down", "around"],
    ),
    choice(
        MODE,
        "mode",
        "How the interval is read: in semitones, or in degrees of the project's scale, so that every moved note is in the scale. Without a project scale, in semitones.",
        SEMITONES as f64,
        &["semitones", "scale"],
    ),
    choice(
        ORDER,
        "order",
        "How a moved note picks its choice: at random (a repeatable draw, set by the seed), or in turn, each moved note taking the next choice.",
        RANDOM as f64,
        &["random", "cycle"],
    ),
    stepped(
        SEED,
        "seed",
        "Seed of the draws, from 0 to 1000: a note draws from the seed, its position in the song and its pitch, so the same seed gives the same notes at the same place on every play, from any start; another seed, another draw.",
        Unit::None,
        0.0,
        1000.0,
        0.0,
        1.0,
    ),
];

/// The descriptor of Scatter (CT-5): `faulk.scatter`, displayed "Scatter" (one English name,
/// never translated, founder's decision of 2026-11-06). The defaults move half the notes an
/// octave up or down, which sounds right in any key; the presets hold the rest.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.scatter",
    kind_id: DeviceKind(11),
    version: 1,
    name: "Scatter",
    role: DeviceRole::MidiEffect,
    // The device, its table, and the most output the host gives it (`MidiOutput::capacity` is
    // at most `MAX_MIDI_OUTPUT_EVENTS`).
    memory_bytes: (size_of::<Scatter>()
        + HeldTable::<u8, HELD>::BYTES
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
    factory: Scatter::build,
};

/// The device (module doc).
pub struct Scatter {
    /// 0 to 1.
    chance: f64,
    choices: u32,
    interval: i32,
    direction: u32,
    in_scale: bool,
    cycle: bool,
    seed: u64,
    /// The received notes that sound, with the key each came out at.
    held: HeldTable<u8, HELD>,
    /// Note-ons received while the transport is stopped, since the last reset: their place.
    stopped: u64,
    /// Notes moved in `cycle` order since the last reset or the last start of the transport.
    moved: u64,
    playing: bool,
}

impl Scatter {
    pub fn new(_setup: &DeviceSetup) -> Result<Self, MsgError> {
        let held = HeldTable::new()?;
        let mut scatter = Self {
            chance: 0.0,
            choices: 1,
            interval: 1,
            direction: UP,
            in_scale: false,
            cycle: false,
            seed: 0,
            held,
            stopped: 0,
            moved: 0,
            playing: false,
        };
        for p in DESCRIPTOR.params {
            scatter.set_param(p.id, p.default as f32);
        }
        Ok(scatter)
    }

    pub fn build(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
        Ok(try_box(Self::new(setup)?)?)
    }

    /// The key a received note-on at `place` comes out at (module doc).
    fn target(&mut self, key: u8, place: Place, scale: PitchSet) -> u8 {
        let draw = Draw::new(self.seed, place, key);
        if !draw.chance(0, self.chance) {
            return key;
        }
        let choices = self.choices.clamp(1, MAX_CHOICES);
        let index = if self.cycle {
            let i = (self.moved % u64::from(choices)) as u32;
            self.moved = self.moved.wrapping_add(1);
            i
        } else {
            draw.pick(1, choices)
        };
        let steps = steps(index + 1, self.direction) * self.interval;
        let on_keyboard = |k: i32| (0..=127).contains(&k);
        if !self.in_scale || scale.is_empty() {
            let target = i32::from(key) + steps;
            return if on_keyboard(target) {
                target as u8
            } else {
                key
            };
        }
        // In the scale, a choice off the keyboard leaves the note on its own degree (the
        // scale's note at or below it, or the next one up when that is below 0): a moved note
        // is always in the scale.
        [
            scale_step(key, steps, scale),
            scale_step(key, 0, scale),
            scale_step(key, 1, scale),
        ]
        .into_iter()
        .find(|&k| on_keyboard(k))
        .map_or(key, |k| k as u8)
    }

    #[allow(clippy::too_many_arguments)]
    fn note_on(
        &mut self,
        at: usize,
        voice: VoiceId,
        key: u8,
        velocity: f32,
        tune_cents: f32,
        place: Place,
        scale: PitchSet,
        out: &mut crate::device::EventBuffer,
    ) {
        // One pass over the table: the entry of this voice if it is held, else the first free.
        let (mut free, same) = self.held.free_and_find(voice.get());
        // A note-on for a voice already held (a broken guarantee) ends that one first, and
        // takes its entry.
        if let Some(entry) = same {
            if let Some(held_key) = self.held.get_mut(entry).map(|k| *k) {
                out.push(Event::NoteOff {
                    offset: at as u32,
                    voice,
                    key: held_key,
                });
            }
            self.held.release(entry);
            free = Some(entry);
        }
        let key = key.min(127);
        // A note beyond a full table still takes its turn in `cycle` order and its count when
        // stopped, so it shifts nothing of the next ones'.
        let moved = self.target(key, place, scale);
        let out_key = match free {
            Some(entry) => {
                self.held.insert(entry, voice.get(), moved);
                moved
            }
            // The table is full: the note goes through unchanged, as its note-off will.
            None => key,
        };
        out.push(Event::NoteOn {
            offset: at as u32,
            voice,
            key: out_key,
            velocity,
            tune_cents,
        });
    }

    fn note_off(
        &mut self,
        at: usize,
        voice: VoiceId,
        key: u8,
        out: &mut crate::device::EventBuffer,
    ) {
        let key = match self.held.find(voice.get()) {
            Some(entry) => {
                let held_key = self.held.get_mut(entry).map_or(key, |k| *k);
                self.held.release(entry);
                held_key
            }
            None => key,
        };
        out.push(Event::NoteOff {
            offset: at as u32,
            voice,
            key,
        });
    }
}

impl Device for Scatter {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).max(1);
        if ctx.transport.playing && !self.playing {
            self.moved = 0;
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
                    if !ctx.transport.playing {
                        self.stopped = self.stopped.wrapping_add(1);
                    }
                    let place = Place::of(ctx, at, self.stopped);
                    self.note_on(
                        at,
                        voice,
                        key,
                        velocity,
                        tune_cents,
                        place,
                        scale,
                        io.events_out,
                    );
                }
                Event::NoteOff { voice, key, .. } => self.note_off(at, voice, key, io.events_out),
                Event::AllNotesOff { .. } => {
                    self.held.clear();
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
            CHANCE => self.chance = f64::from(value) / 100.0,
            CHOICES => self.choices = value as u32,
            INTERVAL => self.interval = value as i32,
            DIRECTION => self.direction = value as u32,
            MODE => self.in_scale = value as u32 == IN_SCALE,
            ORDER => self.cycle = value as u32 == CYCLE,
            _ => self.seed = value as u64,
        }
    }

    fn reset(&mut self) {
        self.held.clear();
        self.stopped = 0;
        self.moved = 0;
        self.playing = false;
    }
}

/// The steps of choice `k` (1-based) in `direction` (module doc).
pub fn steps(k: u32, direction: u32) -> i32 {
    let k = k as i32;
    match direction {
        UP => k,
        DOWN => -k,
        _ => {
            if k % 2 == 1 {
                (k + 1) / 2
            } else {
                -(k / 2)
            }
        }
    }
}
