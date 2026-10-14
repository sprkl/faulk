//! Polyphonic oscillator synth: one oscillator and one ADSR envelope per voice.

use core::f32::consts::TAU;

use crate::device::{DeviceDescriptor, DeviceRole, ParamDescriptor, Scale, Unit};
use crate::msg::{DeviceKind, ParamId};

/// Number of voices allocated at construction. Notes beyond this steal a voice.
pub const MAX_VOICES: usize = 16;

/// Headroom applied per voice so that a few simultaneous notes stay below full scale.
const VOICE_LEVEL: f32 = 0.25;
/// Shortest attack and release, in seconds, to avoid clicks at note start and end.
const MIN_ENVELOPE_SECONDS: f32 = 0.002;
/// Longest attack, decay and release, in seconds.
const MAX_ENVELOPE_SECONDS: f32 = 30.0;
/// Time constant of the master gain smoothing, in seconds.
const GAIN_SMOOTHING_SECONDS: f32 = 0.01;
/// Duration of the fade applied to a stolen voice, in seconds.
const STEAL_FADE_SECONDS: f32 = 0.003;
/// Samples whose magnitude is below this are flushed to zero (denormal guard).
const DENORMAL_FLOOR: f32 = 1e-20;
/// Time a sustaining voice takes to reach a new sustain level, in seconds, so that
/// moving the sustain knob while notes are held does not click.
const SUSTAIN_SLEW_SECONDS: f32 = 0.005;
/// A smoothed gain closer than this to its target snaps to it (keeps the gain state
/// out of the subnormal range when the target is 0).
const GAIN_SNAP: f32 = 1e-6;

/// The descriptor of the synth (CT-5): `faulk.osc_synth`, displayed "Oscillator". Ids are
/// [`Param::id`], keys the v1 parameter names, ranges the clamps of [`Synth::set_param`],
/// defaults the values of [`Synth::new`] (tested). The smoothing times are the contract's
/// linear ramps the synth must follow once it is a [`crate::device::Device`] (DV-1, AM-3);
/// today its gain follows a one-pole of time constant 10 ms and its sustain a fixed slope
/// that crosses the whole range in 5 ms, so neither lands as the descriptor declares.
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
            unit: Unit::None,
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
    /// Fast fade-out before a stolen voice restarts on its new note.
    Steal,
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    stage: Stage,
    note: u8,
    amplitude: f32,
    phase: f32,
    phase_inc: f32,
    level: f32,
    release_step: f32,
    /// Monotonic counter value at note on, used to pick the oldest voice to steal.
    started_at: u64,
    /// Note queued while the voice fades out after being stolen.
    pending: Option<(u8, f32)>,
}

impl Voice {
    const IDLE: Self = Self {
        stage: Stage::Idle,
        note: 0,
        amplitude: 0.0,
        phase: 0.0,
        phase_inc: 0.0,
        level: 0.0,
        release_step: 0.0,
        started_at: 0,
        pending: None,
    };
}

#[derive(Clone, Copy, Debug)]
struct Envelope {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

/// Polyphonic synth. All memory is allocated in [`Synth::new`].
#[derive(Clone, Debug)]
pub struct Synth {
    sample_rate: f32,
    voices: [Voice; MAX_VOICES],
    waveform: Waveform,
    envelope: Envelope,
    gain_target: f32,
    gain: f32,
    gain_coeff: f32,
    note_counter: u64,
}

impl Synth {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        Self {
            sample_rate,
            voices: [Voice::IDLE; MAX_VOICES],
            waveform: Waveform::Saw,
            envelope: Envelope {
                attack: 0.005,
                decay: 0.2,
                sustain: 0.7,
                release: 0.3,
            },
            gain_target: 0.8,
            gain: 0.8,
            gain_coeff: 1.0 - (-1.0 / (GAIN_SMOOTHING_SECONDS * sample_rate)).exp(),
            note_counter: 0,
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Current value of a parameter, in the unit [`Synth::set_param`] takes.
    pub fn param(&self, param: Param) -> f32 {
        match param {
            Param::Waveform => self.waveform.index() as f32,
            Param::Attack => self.envelope.attack,
            Param::Decay => self.envelope.decay,
            Param::Sustain => self.envelope.sustain,
            Param::Release => self.envelope.release,
            Param::Gain => self.gain_target,
        }
    }

    /// Sets a parameter. Values are clamped to their range; NaN is ignored.
    pub fn set_param(&mut self, param: Param, value: f32) {
        if !value.is_finite() {
            return;
        }
        let seconds = value.clamp(MIN_ENVELOPE_SECONDS, MAX_ENVELOPE_SECONDS);
        match param {
            // Rounded and clamped to the four waveforms, as the descriptor's step says.
            Param::Waveform => {
                self.waveform = Waveform::from_index(value.round().clamp(0.0, 3.0) as u32)
            }
            Param::Attack => self.envelope.attack = seconds,
            Param::Decay => self.envelope.decay = seconds,
            Param::Sustain => self.envelope.sustain = value.clamp(0.0, 1.0),
            Param::Release => self.envelope.release = seconds,
            Param::Gain => self.gain_target = value.clamp(0.0, 1.0),
        }
    }

    /// Starts a note. `note` is a MIDI note number (0–127), `velocity` 1–127;
    /// a velocity of 0 is a note off, as in MIDI.
    pub fn note_on(&mut self, note: u8, velocity: u8) {
        let note = note.min(127);
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        let amplitude = f32::from(velocity.min(127)) / 127.0;
        self.note_counter = self.note_counter.wrapping_add(1);

        // Retrigger a voice already playing this note, else take a free one, else steal.
        let index = self
            .voices
            .iter()
            .position(|v| v.stage != Stage::Idle && v.note == note && v.pending.is_none())
            .or_else(|| self.voices.iter().position(|v| v.stage == Stage::Idle))
            .unwrap_or_else(|| self.voice_to_steal());

        let counter = self.note_counter;
        let voice = &mut self.voices[index];
        if voice.stage == Stage::Idle {
            Self::start_voice(voice, note, amplitude, counter, self.sample_rate);
        } else {
            // Fade out what the voice plays, then start the new note.
            voice.pending = Some((note, amplitude));
            voice.started_at = counter;
            voice.stage = Stage::Steal;
            voice.release_step = voice.level / (STEAL_FADE_SECONDS * self.sample_rate);
        }
    }

    /// Releases every voice playing `note`.
    pub fn note_off(&mut self, note: u8) {
        let release_samples = self.envelope.release * self.sample_rate;
        for voice in &mut self.voices {
            if let Some((pending_note, _)) = voice.pending
                && pending_note == note
            {
                voice.pending = None;
            }
            if voice.note == note
                && matches!(voice.stage, Stage::Attack | Stage::Decay | Stage::Sustain)
            {
                voice.stage = Stage::Release;
                voice.release_step = voice.level / release_samples;
            }
        }
    }

    /// Releases every voice.
    pub fn all_notes_off(&mut self) {
        for note in 0..=127 {
            self.note_off(note);
        }
    }

    /// True when a voice plays `note` and has not been released or stolen.
    pub fn is_held(&self, note: u8) -> bool {
        self.voices.iter().any(|v| {
            v.note == note
                && v.pending.is_none()
                && matches!(v.stage, Stage::Attack | Stage::Decay | Stage::Sustain)
        })
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
        let envelope = self.envelope;
        let waveform = self.waveform;
        let attack_step = 1.0 / (envelope.attack * sample_rate);
        let decay_step = (1.0 - envelope.sustain) / (envelope.decay * sample_rate);
        let slew_step = 1.0 / (SUSTAIN_SLEW_SECONDS * sample_rate);

        for frame in 0..frames {
            self.gain += (self.gain_target - self.gain) * self.gain_coeff;
            if (self.gain - self.gain_target).abs() < GAIN_SNAP {
                self.gain = self.gain_target;
            }
            let mut sum = 0.0f32;
            for i in 0..MAX_VOICES {
                let voice = &mut self.voices[i];
                if voice.stage == Stage::Idle {
                    continue;
                }
                advance_envelope(voice, attack_step, decay_step, slew_step, envelope.sustain);
                if voice.stage == Stage::Idle {
                    if let Some((note, amplitude)) = voice.pending.take() {
                        let started_at = voice.started_at;
                        Self::start_voice(voice, note, amplitude, started_at, sample_rate);
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
            let mut sample = (sum * self.gain).tanh();
            if !sample.is_finite() || sample.abs() < DENORMAL_FLOOR {
                sample = 0.0;
            }
            left[frame] = sample;
            right[frame] = sample;
        }
    }

    fn start_voice(voice: &mut Voice, note: u8, amplitude: f32, counter: u64, sample_rate: f32) {
        let frequency = 440.0 * ((f32::from(note) - 69.0) / 12.0).exp2();
        *voice = Voice {
            stage: Stage::Attack,
            note,
            amplitude,
            phase: 0.0,
            phase_inc: (frequency / sample_rate).min(0.5),
            level: 0.0,
            release_step: 0.0,
            started_at: counter,
            pending: None,
        };
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

fn advance_envelope(
    voice: &mut Voice,
    attack_step: f32,
    decay_step: f32,
    slew_step: f32,
    sustain: f32,
) {
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
                // Sustain raised above the current level: slew up instead of jumping.
                voice.stage = Stage::Sustain;
            } else {
                voice.level = (voice.level - decay_step).max(sustain);
                if voice.level <= sustain {
                    voice.stage = Stage::Sustain;
                }
            }
        }
        Stage::Sustain => {
            // Follow sustain changes smoothly.
            let delta = sustain - voice.level;
            voice.level += delta.clamp(-slew_step, slew_step);
        }
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
