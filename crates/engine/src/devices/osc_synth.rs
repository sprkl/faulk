//! The oscillator synth, `faulk.osc_synth`, displayed "Oscillator" (DV-1): a polyphonic
//! oscillator with one ADSR envelope per voice, a [`Device`] of role instrument.
//!
//! # Notes
//!
//! A note is its [`VoiceId`] (CT-5): a `NoteOn` of a voice id already sounding retriggers
//! that voice (a 3 ms fade, then the new note), else takes a free voice, else steals one (the
//! oldest released, else the oldest, after the same fade); a `NoteOff` releases every voice
//! of its id, whatever its `key`. The pitch is `key + tune_cents / 100` semitones. A velocity
//! of 0 (or not finite) releases the voice id, as MIDI's note-on of velocity 0 does.
//!
//! # Parameters and smoothing
//!
//! Every value goes through [`ParamDescriptor::sanitize`]: the waveform index is rounded and
//! clamped to the four waveforms, times are clamped to 2 ms – 30 s, levels to 0 – 1. The
//! waveform and the envelope times apply at their exact offset; **gain and sustain follow
//! the linear ramps the descriptor declares** (10 ms and 5 ms). Before DV-1 the gain
//! followed a one-pole of time constant 10 ms and a held voice slewed to a new sustain at a
//! fixed slope (the whole range in 5 ms). DV-1 chose the declared ramps: with constant gain
//! and sustain the render is the same sample for sample, and while they move both curves
//! are click-free and settle within the same few milliseconds (the reference render test,
//! `tests/osc_synth.rs`, bounds the difference).
//!
//! # The legacy calls
//!
//! [`Synth::note_on`], [`Synth::note_off`], [`Synth::set_param`] with a [`Param`] and
//! [`Synth::render`] are the calls the worklet and the helper made before the engine
//! channel (MO-2). They run the same code as [`Device::process`]: a key's voice id is
//! `key + 1`, the graph's mapping of a live note.

use core::f32::consts::TAU;

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, ParamDescriptor,
    Scale, Unit, VoiceId, clamp_tune_cents, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

/// Number of voices allocated at construction. Notes beyond this steal a voice.
pub const MAX_VOICES: usize = 16;

/// Headroom applied per voice so that a few simultaneous notes stay below full scale.
const VOICE_LEVEL: f32 = 0.25;
/// Duration of the fade applied to a stolen voice, in seconds.
const STEAL_FADE_SECONDS: f32 = 0.003;
/// Samples whose magnitude is below this are flushed to zero (denormal guard).
const DENORMAL_FLOOR: f32 = 1e-20;

/// The descriptor of the synth (CT-5): `faulk.osc_synth`, displayed "Oscillator". Ids are
/// [`Param::id`], keys the v1 parameter names, defaults the values of [`Synth::new`]
/// (tested). The device clamps and rounds every value with [`ParamDescriptor::sanitize`] and
/// smooths gain and sustain as declared here (module doc).
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.osc_synth",
    kind_id: DeviceKind(1),
    version: 1,
    name: "Oscillator",
    role: DeviceRole::Instrument,
    memory_bytes: size_of::<Synth>() as u64,
    midi_output: None,
    params: &[
        ParamDescriptor {
            id: ParamId(0),
            key: "waveform",
            unit: Unit::None,
            min: 0.0,
            max: 3.0,
            default: 1.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["sine", "saw", "square", "triangle"],
        },
        envelope_time(1, "attack", 0.005),
        envelope_time(2, "decay", 0.2),
        ParamDescriptor {
            id: ParamId(3),
            key: "sustain",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.7,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.005,
            values: &[],
        },
        envelope_time(4, "release", 0.3),
        ParamDescriptor {
            id: ParamId(5),
            key: "gain",
            unit: Unit::Gain,
            min: 0.0,
            max: 1.0,
            default: 0.8,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.01,
            values: &[],
        },
    ],
};

/// The synth's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a synth for `setup`, off the audio path (fallible allocation).
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Synth::new(setup.sample_rate))?)
}

/// An envelope time: read when a stage starts, so never smoothed.
const fn envelope_time(id: u32, key: &'static str, default: f64) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        unit: Unit::Seconds,
        min: 0.002,
        max: 30.0,
        default,
        scale: Scale::Log,
        step: 0.0,
        smoothing_seconds: 0.0,
        values: &[],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Saw,
    Square,
    Triangle,
}

impl Waveform {
    /// Maps an index (0 = sine, 1 = saw, 2 = square, 3 = triangle) to a waveform.
    /// Out-of-range values fall back to sine.
    pub fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Saw,
            2 => Self::Square,
            3 => Self::Triangle,
            _ => Self::Sine,
        }
    }

    pub fn index(self) -> u32 {
        match self {
            Self::Sine => 0,
            Self::Saw => 1,
            Self::Square => 2,
            Self::Triangle => 3,
        }
    }
}

/// Synth parameters settable at any time, including while notes play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    /// Waveform index, see [`Waveform::from_index`].
    Waveform,
    /// Attack time in seconds.
    Attack,
    /// Decay time in seconds.
    Decay,
    /// Sustain level, 0 to 1.
    Sustain,
    /// Release time in seconds.
    Release,
    /// Master gain, 0 to 1 (linear).
    Gain,
}

impl Param {
    /// Stable numeric id used across the WebAssembly boundary.
    pub fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            0 => Self::Waveform,
            1 => Self::Attack,
            2 => Self::Decay,
            3 => Self::Sustain,
            4 => Self::Release,
            5 => Self::Gain,
            _ => return None,
        })
    }

    pub fn id(self) -> u32 {
        match self {
            Self::Waveform => 0,
            Self::Attack => 1,
            Self::Decay => 2,
            Self::Sustain => 3,
            Self::Release => 4,
            Self::Gain => 5,
        }
    }
}

/// A linear ramp under CT-5's smoothing rule: `frames` frames after a target is set, the
/// value is the target; in between, `start + (target − start) · i / frames`. A new target
/// starts a new ramp from the current value. (AM-3 writes the shared smoother.)
#[derive(Clone, Copy, Debug)]
pub(super) struct LinearRamp {
    start: f32,
    pub(super) target: f32,
    frames: u32,
    done: u32,
}

impl LinearRamp {
    pub(super) const fn new(value: f32) -> Self {
        Self {
            start: value,
            target: value,
            frames: 0,
            done: 0,
        }
    }

    /// The value of the next frame.
    pub(super) fn current(&self) -> f32 {
        if self.done >= self.frames {
            self.target
        } else {
            self.start + (self.target - self.start) * (self.done as f32 / self.frames as f32)
        }
    }

    /// Aims at `target` over `frames` frames from the current value (0 frames: a jump).
    pub(super) fn set(&mut self, target: f32, frames: u32) {
        self.start = self.current();
        self.target = target;
        self.frames = frames;
        self.done = 0;
    }

    /// Ends the ramp at its target.
    pub(super) fn snap(&mut self) {
        self.start = self.target;
        self.frames = 0;
        self.done = 0;
    }

    /// The value of this frame, then one frame on.
    pub(super) fn next_value(&mut self) -> f32 {
        let value = self.current();
        if self.done < self.frames {
            self.done += 1;
        }
        value
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
    /// Fast fade-out before a stolen or retriggered voice restarts on its new note.
    Steal,
}

/// A note to start: on a free voice at once, or once its voice has faded out.
#[derive(Clone, Copy, Debug)]
struct Pending {
    voice: VoiceId,
    key: u8,
    amplitude: f32,
    cents: f32,
}

/// The voice id an idle voice carries: never compared, since only sounding voices are.
const NO_VOICE: VoiceId = match VoiceId::new(0) {
    Some(id) => id,
    None => panic!("0 is a voice id"),
};

#[derive(Clone, Copy, Debug)]
struct Voice {
    stage: Stage,
    /// The note's voice id (CT-5): what a `NoteOff` names.
    id: VoiceId,
    key: u8,
    amplitude: f32,
    phase: f32,
    phase_inc: f32,
    level: f32,
    release_step: f32,
    /// Monotonic counter value at note on, used to pick the oldest voice to steal.
    started_at: u64,
    /// Note queued while the voice fades out.
    pending: Option<Pending>,
}

impl Voice {
    const IDLE: Self = Self {
        stage: Stage::Idle,
        id: NO_VOICE,
        key: 0,
        amplitude: 0.0,
        phase: 0.0,
        phase_inc: 0.0,
        level: 0.0,
        release_step: 0.0,
        started_at: 0,
        pending: None,
    };

    /// Attack, decay or sustain: the note is held.
    fn is_held(&self) -> bool {
        matches!(self.stage, Stage::Attack | Stage::Decay | Stage::Sustain)
    }
}

/// The voice id of a key played through the legacy calls: the key plus one, as the graph
/// maps a live note.
fn key_voice(key: u8) -> VoiceId {
    VoiceId::new(u32::from(key) + 1).unwrap_or(NO_VOICE)
}

/// Polyphonic synth. All memory is allocated in [`Synth::new`].
#[derive(Clone, Debug)]
pub struct Synth {
    sample_rate: f32,
    voices: [Voice; MAX_VOICES],
    waveform: Waveform,
    attack: f32,
    decay: f32,
    release: f32,
    sustain: LinearRamp,
    gain: LinearRamp,
    /// Frames of a sustain ramp and of a gain ramp at this rate.
    sustain_frames: u32,
    gain_frames: u32,
    note_counter: u64,
}

impl Synth {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        // Every value starts at its descriptor's default (the one source of the defaults).
        let desc = |p: Param| DESCRIPTOR.param(ParamId(p.id()));
        let default = |p: Param| desc(p).map_or(0.0, |d| d.default as f32);
        let frames = |p: Param| desc(p).map_or(0, |d| d.smoothing_frames(sample_rate));
        Self {
            sample_rate,
            voices: [Voice::IDLE; MAX_VOICES],
            waveform: Waveform::from_index(default(Param::Waveform) as u32),
            attack: default(Param::Attack),
            decay: default(Param::Decay),
            release: default(Param::Release),
            sustain: LinearRamp::new(default(Param::Sustain)),
            gain: LinearRamp::new(default(Param::Gain)),
            sustain_frames: frames(Param::Sustain),
            gain_frames: frames(Param::Gain),
            note_counter: 0,
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Current value of a parameter (the target of a smoothed one), in the unit
    /// [`Synth::set_param`] takes.
    pub fn param(&self, param: Param) -> f32 {
        match param {
            Param::Waveform => self.waveform.index() as f32,
            Param::Attack => self.attack,
            Param::Decay => self.decay,
            Param::Sustain => self.sustain.target,
            Param::Release => self.release,
            Param::Gain => self.gain.target,
        }
    }

    /// Sets a parameter (legacy call): [`Device::set_param`] with its id.
    pub fn set_param(&mut self, param: Param, value: f32) {
        self.set_value(ParamId(param.id()), value);
    }

    /// Starts a note (legacy call): `note` is a MIDI note number (0–127), `velocity` 1–127;
    /// a velocity of 0 is a note off, as in MIDI. Its voice id is `note + 1`.
    pub fn note_on(&mut self, note: u8, velocity: u8) {
        let note = note.min(127);
        let amplitude = f32::from(velocity.min(127)) / 127.0;
        self.start_note(key_voice(note), note, amplitude, 0.0);
    }

    /// Releases the note [`Synth::note_on`] started on `note` (legacy call).
    pub fn note_off(&mut self, note: u8) {
        self.release_voice(key_voice(note.min(127)));
    }

    /// Releases every voice and drops every note waiting for its voice.
    pub fn all_notes_off(&mut self) {
        let release_samples = self.release * self.sample_rate;
        for voice in &mut self.voices {
            voice.pending = None;
            if voice.is_held() {
                voice.stage = Stage::Release;
                voice.release_step = voice.level / release_samples;
            }
        }
    }

    /// True when a voice plays `note` and has not been released or stolen.
    pub fn is_held(&self, note: u8) -> bool {
        self.voices
            .iter()
            .any(|v| v.key == note && v.pending.is_none() && v.is_held())
    }

    /// Number of voices currently producing sound.
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.stage != Stage::Idle)
            .count()
    }

    /// Renders `left.len()` frames into both channels (mono synth, same signal).
    /// If the slices differ in length, only the common length is rendered.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let sample_rate = self.sample_rate;
        let waveform = self.waveform;
        let attack_step = 1.0 / (self.attack * sample_rate);
        let decay_samples = self.decay * sample_rate;

        let mut last_sustain = f32::NAN;
        let mut decay_step = 0.0;
        for frame in 0..frames {
            let gain = self.gain.next_value();
            let sustain = self.sustain.next_value();
            // Recomputed only while the sustain ramp moves (the same value otherwise).
            if sustain != last_sustain {
                decay_step = (1.0 - sustain) / decay_samples;
                last_sustain = sustain;
            }
            let mut sum = 0.0f32;
            for voice in &mut self.voices {
                if voice.stage == Stage::Idle {
                    continue;
                }
                advance_envelope(voice, attack_step, decay_step, sustain);
                if voice.stage == Stage::Idle {
                    if let Some(next) = voice.pending.take() {
                        let started_at = voice.started_at;
                        start_voice(voice, next, started_at, sample_rate);
                    }
                    continue;
                }
                sum += oscillator(waveform, voice.phase, voice.phase_inc)
                    * voice.level
                    * voice.amplitude
                    * VOICE_LEVEL;
                voice.phase += voice.phase_inc;
                if voice.phase >= 1.0 {
                    voice.phase -= 1.0;
                }
            }
            let mut sample = (sum * gain).tanh();
            if !sample.is_finite() || sample.abs() < DENORMAL_FLOOR {
                sample = 0.0;
            }
            left[frame] = sample;
            right[frame] = sample;
        }
    }

    /// Applies a parameter value: unknown ids and non-finite values are ignored, the rest is
    /// clamped and rounded by the descriptor; gain and sustain start their ramp.
    fn set_value(&mut self, id: ParamId, value: f32) {
        let (Some(param), Some(desc)) = (Param::from_id(id.0), DESCRIPTOR.param(id)) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        match param {
            Param::Waveform => self.waveform = Waveform::from_index(value as u32),
            Param::Attack => self.attack = value,
            Param::Decay => self.decay = value,
            Param::Sustain => self.sustain.set(value, self.sustain_frames),
            Param::Release => self.release = value,
            Param::Gain => self.gain.set(value, self.gain_frames),
        }
    }

    /// Starts the note of `id`: retriggers the voice that holds it, else a free voice, else
    /// steals one. A velocity of 0 or not finite releases `id` instead.
    fn start_note(&mut self, id: VoiceId, key: u8, velocity: f32, tune_cents: f32) {
        if !(velocity.is_finite() && velocity > 0.0) {
            self.release_voice(id);
            return;
        }
        let next = Pending {
            voice: id,
            key: key.min(127),
            amplitude: velocity.min(1.0),
            cents: clamp_tune_cents(tune_cents),
        };
        self.note_counter = self.note_counter.wrapping_add(1);

        let index = self
            .voices
            .iter()
            .position(|v| v.stage != Stage::Idle && v.id == id && v.pending.is_none())
            .or_else(|| self.voices.iter().position(|v| v.stage == Stage::Idle))
            .unwrap_or_else(|| self.voice_to_steal());

        let counter = self.note_counter;
        let sample_rate = self.sample_rate;
        let voice = &mut self.voices[index];
        if voice.stage == Stage::Idle {
            start_voice(voice, next, counter, sample_rate);
        } else {
            // Fade out what the voice plays, then start the new note.
            voice.pending = Some(next);
            voice.started_at = counter;
            voice.stage = Stage::Steal;
            voice.release_step = voice.level / (STEAL_FADE_SECONDS * sample_rate);
        }
    }

    /// Releases every voice of `id`, and drops a note of `id` waiting for its voice.
    fn release_voice(&mut self, id: VoiceId) {
        let release_samples = self.release * self.sample_rate;
        for voice in &mut self.voices {
            if voice.pending.is_some_and(|p| p.voice == id) {
                voice.pending = None;
            }
            if voice.id == id && voice.is_held() {
                voice.stage = Stage::Release;
                voice.release_step = voice.level / release_samples;
            }
        }
    }

    fn apply(&mut self, event: &Event) {
        match *event {
            Event::NoteOn {
                voice,
                key,
                velocity,
                tune_cents,
                ..
            } => self.start_note(voice, key, velocity, tune_cents),
            Event::NoteOff { voice, .. } => self.release_voice(voice),
            Event::AllNotesOff { .. } => self.all_notes_off(),
            Event::ParamValue { id, value, .. } => self.set_value(id, value),
        }
    }

    /// Picks the voice to steal: the oldest released voice, else the oldest voice.
    fn voice_to_steal(&self) -> usize {
        let oldest = |releasing: bool| {
            self.voices
                .iter()
                .enumerate()
                .filter(|(_, v)| !releasing || v.stage == Stage::Release)
                .min_by_key(|(_, v)| v.started_at)
                .map(|(i, _)| i)
        };
        oldest(true).or_else(|| oldest(false)).unwrap_or(0)
    }
}

impl Device for Synth {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    /// Renders the frames between events and applies each event at its frame
    /// ([`event_frame`]: an offset beyond the block is its last frame, an unsorted one is
    /// "now").
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).min(io.left.len()).min(io.right.len());
        let mut cursor = 0;
        for event in events {
            let at = event_frame(event.offset(), cursor, frames);
            if at > cursor {
                self.render(&mut io.left[cursor..at], &mut io.right[cursor..at]);
                cursor = at;
            }
            self.apply(event);
        }
        if frames > cursor {
            self.render(&mut io.left[cursor..frames], &mut io.right[cursor..frames]);
        }
        // The voice count of the host's report (interim until the meters of MO-5).
        crate::host::add_block_voices(self.active_voices() as u32);
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        self.set_value(id, value);
    }

    /// Silences every voice at once and snaps gain and sustain to their targets; the
    /// parameter values are kept.
    fn reset(&mut self) {
        self.voices = [Voice::IDLE; MAX_VOICES];
        self.sustain.snap();
        self.gain.snap();
        self.note_counter = 0;
    }
}

fn start_voice(voice: &mut Voice, note: Pending, counter: u64, sample_rate: f32) {
    let pitch = f32::from(note.key) + note.cents / 100.0;
    let frequency = 440.0 * ((pitch - 69.0) / 12.0).exp2();
    *voice = Voice {
        stage: Stage::Attack,
        id: note.voice,
        key: note.key,
        amplitude: note.amplitude,
        phase: 0.0,
        phase_inc: (frequency / sample_rate).min(0.5),
        level: 0.0,
        release_step: 0.0,
        started_at: counter,
        pending: None,
    };
}

fn advance_envelope(voice: &mut Voice, attack_step: f32, decay_step: f32, sustain: f32) {
    match voice.stage {
        Stage::Attack => {
            voice.level += attack_step;
            if voice.level >= 1.0 {
                voice.level = 1.0;
                voice.stage = Stage::Decay;
            }
        }
        Stage::Decay => {
            if voice.level <= sustain {
                // Sustain raised above the current level: the voice joins its ramp.
                voice.stage = Stage::Sustain;
            } else {
                voice.level = (voice.level - decay_step).max(sustain);
                if voice.level <= sustain {
                    voice.stage = Stage::Sustain;
                }
            }
        }
        // A held voice follows the smoothed sustain (its ramp is the declared smoothing).
        Stage::Sustain => voice.level = sustain,
        Stage::Release | Stage::Steal => {
            voice.level -= voice.release_step;
            if voice.level <= 0.0 || voice.release_step <= 0.0 {
                voice.level = 0.0;
                voice.stage = Stage::Idle;
            }
        }
        Stage::Idle => {}
    }
}

/// One sample of the waveform at `phase` (0 to 1). Saw and square use PolyBLEP
/// to reduce aliasing.
fn oscillator(waveform: Waveform, phase: f32, phase_inc: f32) -> f32 {
    match waveform {
        Waveform::Sine => (phase * TAU).sin(),
        Waveform::Saw => 2.0 * phase - 1.0 - poly_blep(phase, phase_inc),
        Waveform::Square => {
            let naive = if phase < 0.5 { 1.0 } else { -1.0 };
            let shifted = if phase >= 0.5 {
                phase - 0.5
            } else {
                phase + 0.5
            };
            naive + poly_blep(phase, phase_inc) - poly_blep(shifted, phase_inc)
        }
        Waveform::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
    }
}

/// Polynomial band-limited step correction around a discontinuity at phase 0.
fn poly_blep(phase: f32, phase_inc: f32) -> f32 {
    if phase_inc <= 0.0 {
        return 0.0;
    }
    if phase < phase_inc {
        let t = phase / phase_inc;
        t + t - t * t - 1.0
    } else if phase > 1.0 - phase_inc {
        let t = (phase - 1.0) / phase_inc;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}
