//! Bloom, `faulk.bloom`, displayed "Bloom" (#696): a chorus, a three-voice ensemble and a
//! vibrato, from modulated delay lines. A [`Device`] of role effect.
//!
//! # Signal
//!
//! One delay line per channel (left to left, right to right), read by three **voices**. A
//! voice's delay, in seconds, is `centre + half * lfo`, with `lfo` in `[-1, 1]`, `half =
//! depth * MAX_SWEEP_SECONDS / 2` and, once settled, `centre = delay + half`:
//!
//! `delay + depth * MAX_SWEEP_SECONDS * (1 + lfo) / 2`
//!
//! the `delay` setting is the shortest delay and the voice sweeps from it to `delay + depth *
//! 10 ms` and back (the peak-to-peak excursion is `depth` times [`MAX_SWEEP_SECONDS`]). The
//! three voices of a channel are a third of a cycle apart (0, 120
//! and 240 degrees); the right channel's voices are `stereo_phase` degrees further on.
//!
//! The **mode** chooses what is heard:
//!
//! - chorus: the first voice, mixed with the dry sound by an equal-power crossfade, `dry *
//!   cos(mix * pi / 2) + wet * sin(mix * pi / 2)` (the moving copy is mostly unrelated to the
//!   dry sound, so the level holds as the mix moves; exactly the dry sound at 0);
//! - ensemble: the mean of the three voices (never louder than its input), mixed the same way;
//! - vibrato: the first voice alone, without the dry sound (the pitch wavers); `mix` does not
//!   act in this mode.
//!
//! A change of mode moves the weights of the dry sound and of the voices over
//! [`MODE_RAMP_SECONDS`]: it never clicks.
//!
//! **Interpolation**: each voice reads its line at a fractional delay through a 4-point cubic
//! Hermite (Catmull-Rom) interpolator: exact on a whole number of frames, without state (the
//! delay can move freely) and with a flatter top end than a linear interpolation, whose
//! low-pass changes with the fraction (heard as a flutter of the tone as the delay sweeps).
//!
//! **Speed limit**: the fastest a voice's delay moves under its LFO is that of a sine at the
//! highest free rate and full depth, 10 Hz over 10 ms: `pi * 10 * 0.01 = 0.314` frames per
//! frame ([`MAX_SWEEP_SPEED`]), a pitch swing of 31.4 %. A synced note value can be faster than
//! 10 Hz (a 1/32 is 16 Hz at 120 BPM, 133 Hz at 999 BPM): then the sweep is narrowed so that it
//! never moves faster than that (`sweep <= MAX_SWEEP_SPEED / (pi * cycles per frame)`); the
//! cycle stays on the beat, the depth gives way.
//!
//! **Centre**: the pitch a voice gives is `1 - d'` (the speed of its read position). Moving
//! the read position to follow a new setting is heard as a bend: gliding from 12 ms to 1 ms
//! over 100 ms sounded 2 to 4 semitones sharp, "between two notes" (founder, 2026-11-07, on the
//! Vibrato preset loaded over a sounding note). So a change never moves a voice: a new `depth`
//! (or a speed limit that narrows the sweep) changes `half` at once, and a jump of the LFO's
//! value (a new shape, stereo phase, synchronisation or note value, a seek, a start of playback)
//! changes `lfo` at once; each voice's centre is then taken again as `delay_now - half * lfo`,
//! so the voice stays where it is and swings by the new `half` from its first oscillation. The
//! centre then drifts toward `delay + half` at [`MAX_DRIFT`] frames per frame, a pitch offset of
//! 5 cents at most while it moves (a new `delay` only moves the centre: up to about 10 s for the
//! whole 29 ms range, so `delay` is a setting, not a sweep). When the ends of the line leave no
//! room for the whole swing around that centre (a deeper swing at a short delay), the centre is
//! the one that keeps the voice in place with the widest swing that fits, and the swing widens
//! as the centre drifts. While the lines are silent the centres go there at once (nothing to
//! hear).
//!
//! **Slew limit**: before it is read, each voice's delay moves by at most [`MAX_SLEW`] frames
//! per frame: a safety net that the movement above never reaches (speed limit, centre drift). The
//! read position never steps, so no change of a setting clicks.
//!
//! **Tone**: a one-pole high-pass (topology-preserving transform) on the wet signal of each
//! channel, from 20 Hz to 2 kHz. It sits in the feedback loop, so the low end does not pile up
//! in the repeats.
//!
//! **Feedback**: the line takes `dry + feedback * wet` (the wet signal after the tone filter),
//! from -90 % to +90 %: a short `delay` with feedback gives a soft flanger. A sample written to
//! the line is clamped to 1e6, so the loop is bounded whatever the settings.
//!
//! The input is sanitized like the Echo's: a non-finite sample is 0, a magnitude beyond 1e6 is
//! clamped, a magnitude below 1e-20 is flushed to 0; the line, the filter states and the output
//! are flushed the same way (no denormal). The dry sound is not delayed: Bloom adds no latency
//! and declares none.
//!
//! # Modulation
//!
//! Bloom's LFO ([`Lfo`]) has two shapes, sine (`sin(2 pi p)`) and triangle (0 at the start of the
//! cycle, +1 at a quarter, -1 at three quarters), the formulas and keys of the Filter's LFO block
//! of #686, so that Bloom can take that block once it lands (the module's `Lfo` has its API:
//! `begin` at the start of a block, `phase`, `advance` by one frame).
//!
//! - **Free**: the phase moves by `rate / sample_rate` per frame, from 0.05 to 10 Hz, and starts
//!   again at 0 when the transport starts playing: playback and an export from the same point
//!   sound the same.
//! - **Synced**: while the transport plays, the phase is the transport's position over the length
//!   of the note value (the Echo's ten values, [`super::echo::DIVISION_BEATS`]), cycles counted
//!   from the start of the song: a given tick always has the same phase, after a seek, in a loop
//!   and in an export (a note value that divides the bar starts a cycle on every bar line; a
//!   dotted one, or a whole note in 3/4, does not). The position of a
//!   frame is `anchor + n * ticks_per_frame` from the block where the LFO took the host's
//!   position, taken again when the host's position strays by more than [`RESYNC_TICKS`] (a
//!   seek, a loop's wrap) or the tempo changes: no rounding from block to block, so the block
//!   size changes nothing. While the transport is stopped the LFO keeps moving at the tempo's
//!   rate from where it was (a chorus that stood still would be a fixed comb filter).
//!
//! When the transport starts playing, the LFO takes its starting phase. On the first block after
//! the device is built or reset, and at a start of playback while the lines are silent (nothing
//! but zeros written for a line's length), every voice's delay goes to it at once: the same start
//! as an export, which is what makes the two the same, sample for sample. While a sound rings
//! through Bloom, the voices stay where they are and their centres drift instead (no click).
//!
//! # Memory
//!
//! Both lines are allocated in the factory (`prepare`), for [`MAX_DELAY_SECONDS`] +
//! [`MAX_SWEEP_SECONDS`] at the sample rate of the setup (at most [`MAX_SAMPLE_RATE`]);
//! `process` allocates nothing. [`Device::reset`] takes constant time: the lines are not wiped,
//! a count of the frames written since the reset hides what they hold (as the Echo's). Above
//! that rate the lines stop at the declared size and the delays are kept within them.
//!
//! # Parameters and smoothing
//!
//! `mode`, `sync`, `division` and `shape` are stepped; `rate` is not ramped (the phase is
//! continuous, so a new rate never jumps); `stereo_phase`, `depth` and `delay` declare no ramp
//! (the response is the device's own, "Centre"); `feedback`, `tone` and `mix` follow the 20 ms
//! ramp the descriptor declares. Every value goes through
//! [`ParamDescriptor::sanitize`].

use super::Builtin;
use super::echo::{DIVISION_BEATS, VALUES_DIVISION};
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MAX_SAMPLE_RATE,
    ParamDescriptor, Scale, Transport, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};
use crate::param::Smoother;
use crate::time::PPQ;

/// Ids of the parameters.
pub const MODE: u32 = 0;
pub const SYNC: u32 = 1;
pub const RATE: u32 = 2;
pub const DIVISION: u32 = 3;
pub const DEPTH: u32 = 4;
pub const DELAY: u32 = 5;
pub const SHAPE: u32 = 6;
pub const STEREO_PHASE: u32 = 7;
pub const FEEDBACK: u32 = 8;
pub const TONE: u32 = 9;
pub const MIX: u32 = 10;

/// The values of `mode`.
pub const CHORUS: f32 = 0.0;
pub const ENSEMBLE: f32 = 1.0;
pub const VIBRATO: f32 = 2.0;
/// The values of `shape`.
pub const SINE: f32 = 0.0;
pub const TRIANGLE: f32 = 1.0;

/// Shortest and longest `delay`, in seconds.
pub const MIN_DELAY_SECONDS: f32 = 0.001;
pub const MAX_DELAY_SECONDS: f32 = 0.030;
/// The peak-to-peak sweep of a voice at a depth of 100 %, in seconds.
pub const MAX_SWEEP_SECONDS: f32 = 0.010;
/// Highest feedback, as a share (the parameter is in percent).
pub const FEEDBACK_MAX: f32 = 0.9;
/// Fastest a voice's delay moves, in frames per frame (see the module's "Slew limit").
pub const MAX_SLEW: f64 = 0.5;
/// Fastest a voice's centre drifts toward the one the settings ask for, in frames per frame:
/// a pitch offset of 0.3 % (5 cents) while it moves (see the module's "Centre").
pub const MAX_DRIFT: f64 = 0.003;
/// Fastest the LFO moves a voice's delay, in frames per frame: a sine at 10 Hz over
/// [`MAX_SWEEP_SECONDS`] (see the module's "Speed limit").
pub const MAX_SWEEP_SPEED: f64 = core::f64::consts::PI * 10.0 * MAX_SWEEP_SECONDS as f64;
/// How long a change of mode takes to move the weights of the dry sound and of the voices.
pub const MODE_RAMP_SECONDS: f32 = 0.02;
/// The host's position may stray this far, in ticks, from the one the LFO predicts before the
/// LFO takes it again (the value of #686's LFO block).
pub const RESYNC_TICKS: f64 = 1.0;
/// Voices per channel.
pub const VOICES: usize = 3;

/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;
/// Frames kept past the longest delay, for the interpolation.
const GUARD_FRAMES: usize = 4;
/// The shortest delay read, in frames: the interpolator reads one frame newer than the delay,
/// and the newest frame it may read is the one written a frame ago.
const MIN_READ_FRAMES: f64 = 2.0;
/// The highest cut-off of the tone filter, as a share of the sample rate.
const TONE_MAX_SHARE: f32 = 0.45;

const VALUES_MODE: &[&str] = &["chorus", "ensemble", "vibrato"];
const VALUES_SYNC: &[&str] = &["free", "synced"];
const VALUES_SHAPE: &[&str] = &["sine", "triangle"];

/// Frames of one line at `sample_rate`.
fn line_frames(sample_rate: f32) -> usize {
    ((MAX_DELAY_SECONDS + MAX_SWEEP_SECONDS) * sample_rate.min(MAX_SAMPLE_RATE)).ceil() as usize
        + GUARD_FRAMES
}

/// Bytes of the two lines at [`MAX_SAMPLE_RATE`].
const LINES_BYTES: u64 = 2
    * (((MAX_DELAY_SECONDS + MAX_SWEEP_SECONDS) as f64 * MAX_SAMPLE_RATE as f64) as u64
        + GUARD_FRAMES as u64
        + 1)
    * size_of::<f32>() as u64;

/// The descriptor of Bloom (CT-5): `faulk.bloom`, displayed "Bloom" (one English name, never
/// translated, validated by the founder on 2026-11-06). The defaults are the values of
/// [`Bloom::new`]: a free chorus at 0.8 Hz, 40 % deep from 12 ms, half mixed.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.bloom",
    kind_id: DeviceKind(14),
    version: 1,
    name: "Bloom",
    role: DeviceRole::Effect,
    memory_bytes: size_of::<Bloom>() as u64 + LINES_BYTES,
    midi_output: None,
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(MODE),
            key: "mode",
            description: "Chorus: one moving copy mixed with the dry sound. Ensemble: three copies a third of a cycle apart, wider. Vibrato: the moving copy alone, no dry sound, so the pitch wavers (mix does not act).",
            unit: Unit::None,
            min: 0.0,
            max: 2.0,
            default: CHORUS as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_MODE,
        },
        ParamDescriptor {
            id: ParamId(SYNC),
            key: "sync",
            description: "Whether the movement runs at its own rate in hertz (free) or follows the project tempo (synced), cycles counted from the start of the song.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_SYNC,
        },
        ParamDescriptor {
            id: ParamId(RATE),
            key: "rate",
            description: "Speed of the movement in free mode, in cycles per second, from 0.05 to 10 Hz.",
            unit: Unit::Hertz,
            min: 0.05,
            max: 10.0,
            default: 0.8,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(DIVISION),
            key: "division",
            description: "Length of one cycle in synced mode, as a note value of the project tempo: 1/32 to a whole note, with triplets (T) and dotted values (.).",
            unit: Unit::None,
            min: 0.0,
            max: 9.0,
            default: 9.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_DIVISION,
        },
        ParamDescriptor {
            id: ParamId(DEPTH),
            key: "depth",
            description: "How far the delay moves, from 0 to 100 %: at 100 % it sweeps 10 ms above the delay setting and back. More depth, more pitch movement; synced cycles faster than 10 Hz sweep less. A change never bends the pitch.",
            unit: Unit::Percent,
            min: 0.0,
            max: 100.0,
            default: 40.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(DELAY),
            key: "delay",
            description: "Shortest delay of the moving copies, from 1 to 30 ms: 5 to 20 ms for a chorus; below 5 ms, with feedback, a soft flanger. A change moves there slowly, without a pitch bend.",
            unit: Unit::Seconds,
            min: 0.001,
            max: 0.030,
            default: 0.012,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(SHAPE),
            key: "shape",
            description: "Shape of the movement: sine (smooth) or triangle (the delay moves at a steady speed, so the pitch sits a little sharp, then a little flat).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: SINE as f64,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_SHAPE,
        },
        ParamDescriptor {
            id: ParamId(STEREO_PHASE),
            key: "stereo_phase",
            description: "Offset of the right channel's movement, from 0 to 180 degrees: 0 keeps both channels alike, 180 moves them in opposition, the widest.",
            unit: Unit::None,
            min: 0.0,
            max: 180.0,
            default: 90.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(FEEDBACK),
            key: "feedback",
            description: "Share of the moving copy fed back into the delay, from -90 % to +90 %: a metallic, resonant edge; negative values hollow the sound.",
            unit: Unit::Percent,
            min: -90.0,
            max: 90.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(TONE),
            key: "tone",
            description: "High-pass on the moving copies, from 20 Hz to 2 kHz: higher keeps the low end clean and steady.",
            unit: Unit::Hertz,
            min: 20.0,
            max: 2_000.0,
            default: 20.0,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(MIX),
            key: "mix",
            description: "Balance between the dry sound (0) and the moving copies (1), in chorus and ensemble. Vibrato has no dry sound.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.5,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
    ],
};

/// Bloom's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a Bloom for `setup`, off the audio path: both lines are allocated here, with a
/// fallible allocation.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Bloom::new(setup.sample_rate)?)?)
}

/// A delay line of `frames` zeroed samples, or a typed error when the memory is refused.
fn line(frames: usize) -> Result<Box<[f32]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(frames)
        .map_err(|_| MsgError::OutOfMemory {
            bytes: frames.saturating_mul(size_of::<f32>()),
        })?;
    v.resize(frames, 0.0);
    Ok(v.into_boxed_slice())
}

fn sanitize(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
    }
}

fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR { 0.0 } else { x }
}

/// The value of a shape at `phase` cycles, in `[-1, 1]` (0 for a non-finite phase): the
/// formulas of #686's LFO block (sine, and the triangle in phase with it).
pub fn shape_value(triangle: bool, phase: f64) -> f32 {
    if !phase.is_finite() {
        return 0.0;
    }
    let p = phase - phase.floor();
    if triangle {
        let v = if p < 0.25 {
            4.0 * p
        } else if p < 0.75 {
            2.0 - 4.0 * p
        } else {
            4.0 * p - 4.0
        };
        v as f32
    } else {
        (core::f64::consts::TAU * p).sin() as f32
    }
}

/// Bloom's LFO: a free phase, and the synced position's anchor (the semantics of #686's LFO
/// block, see the module's "Modulation").
#[derive(Clone, Copy, Debug, PartialEq)]
struct Lfo {
    /// The phase in free mode, and in synced mode while the transport is stopped: in `[0, 1)`.
    fraction: f64,
    /// Synced mode while playing: the position of the anchor frame, the frames played since,
    /// the ticks per frame.
    anchor: f64,
    played: u64,
    ticks_per_frame: f64,
    anchored: bool,
    playing: bool,
}

impl Lfo {
    const fn new() -> Self {
        Self {
            fraction: 0.0,
            anchor: 0.0,
            played: 0,
            ticks_per_frame: 0.0,
            anchored: false,
            playing: false,
        }
    }

    /// The start of a block at `transport`: takes the host's position again when needed, and
    /// starts the phase again at 0 when the transport starts playing. Returns whether it
    /// started playing (stopped -> playing).
    fn begin(&mut self, transport: &Transport, sample_rate: f32) -> bool {
        let tpf = transport.tempo.ticks_per_second() / f64::from(sample_rate);
        let tpf = if tpf.is_finite() && tpf > 0.0 {
            tpf
        } else {
            0.0
        };
        let started = transport.playing && !self.playing;
        self.playing = transport.playing;
        if started {
            self.fraction = 0.0;
        }
        if !transport.playing {
            self.anchored = false;
            self.ticks_per_frame = tpf;
            return started;
        }
        let position = if transport.position_ticks.is_finite() {
            transport.position_ticks
        } else {
            0.0
        };
        let predicted = self.anchor + self.played as f64 * self.ticks_per_frame;
        if !self.anchored
            || tpf != self.ticks_per_frame
            || (position - predicted).abs() > RESYNC_TICKS
        {
            self.anchor = position;
            self.played = 0;
            self.ticks_per_frame = tpf;
            self.anchored = true;
        }
        started
    }

    /// The phase of the current frame, in cycles: the position over `period_ticks` when synced
    /// and playing, the free phase otherwise.
    #[inline]
    fn phase(&self, synced: bool, period_ticks: f64) -> f64 {
        if synced && self.playing {
            (self.anchor + self.played as f64 * self.ticks_per_frame) / period_ticks
        } else {
            self.fraction
        }
    }

    /// One frame on: the synced position moves while the transport plays (and the free phase
    /// follows it, so that a stop goes on from there); otherwise the free phase moves by
    /// `rate_hz / sample_rate`, or by the tempo's rate when synced.
    #[inline]
    fn advance(&mut self, synced: bool, rate_hz: f32, period_ticks: f64, sample_rate: f32) {
        if self.playing {
            self.played = self.played.saturating_add(1);
            if synced {
                let p = self.phase(true, period_ticks);
                self.fraction = p - p.floor();
                return;
            }
        }
        let step = if synced {
            self.ticks_per_frame / period_ticks
        } else {
            f64::from(rate_hz) / f64::from(sample_rate)
        };
        let step = if step.is_finite() {
            step.clamp(0.0, 0.5)
        } else {
            0.0
        };
        self.fraction += step;
        if self.fraction >= 1.0 {
            self.fraction -= 1.0;
        }
    }
}

/// A one-pole high-pass, topology-preserving transform.
#[derive(Clone, Copy, Debug, PartialEq)]
struct HighPass {
    state: f32,
}

impl HighPass {
    #[inline]
    fn next(&mut self, x: f32, g: f32) -> f32 {
        let v = (x - self.state) * g;
        let low = v + self.state;
        self.state = flush(low + v);
        x - low
    }
}

/// The gain `g / (1 + g)` of the one-pole at `hz`, `g = tan(pi hz / sample_rate)`.
fn tone_gain(hz: f32, sample_rate: f32) -> f32 {
    let hz = hz.clamp(1.0, TONE_MAX_SHARE * sample_rate);
    let g = (core::f32::consts::PI * hz / sample_rate).tan();
    g / (1.0 + g)
}

#[derive(Debug)]
pub struct Bloom {
    sample_rate: f32,
    lines: [Box<[f32]>; 2],
    write: usize,
    /// Frames written since the build or the last reset, up to the length of a line: the line
    /// holds nothing older, so a read further back is silence. A reset only zeroes this count.
    filled: usize,
    /// The longest delay a voice reads, in frames.
    max_read: f64,
    synced: bool,
    triangle: bool,
    rate: f32,
    division: usize,
    /// The right channel's offset, in cycles.
    stereo: f64,
    /// The depth, as a share, and the shortest delay, in seconds, the parameters ask for.
    depth: f32,
    delay: f32,
    feedback: Smoother,
    tone: Smoother,
    mix: Smoother,
    /// The gains of the dry sound and of the copies at the current mix (equal power).
    mix_gains: (f32, f32),
    /// The mix and the tone the gains were computed for.
    mix_now: f32,
    tone_now: f32,
    /// Whether voices 2 and 3 were read on the last frame (the ensemble heard or fading).
    all_voices: bool,
    /// Frames since a sample other than 0 was written to a line, up to a line's length: at
    /// that length the lines are silent.
    quiet: usize,
    /// 1 when the dry sound is heard (chorus, ensemble), 0 in vibrato.
    dry_weight: Smoother,
    /// 1 in ensemble (the three voices), 0 otherwise (the first voice).
    ensemble_weight: Smoother,
    ramp_frames: [u32; 3],
    mode_frames: u32,
    tone_g: f32,
    filters: [HighPass; 2],
    lfo: Lfo,
    /// The delay each voice reads, in frames: `[channel][voice]`.
    voices: [[f64; VOICES]; 2],
    /// The centre each voice swings around, in frames: `[channel][voice]` (module doc, "Centre").
    centres: [[f64; VOICES]; 2],
    /// Half the sweep of the last frame, in frames.
    half: f64,
    /// The LFO's value jumped (shape, stereo phase, synchronisation, note value, a seek): the
    /// next frame takes the centres again from where the voices are.
    rebase: bool,
    /// The next frame takes the delays the LFO asks for, without the glide.
    snap: bool,
}

impl Bloom {
    /// A Bloom at its defaults, with its lines allocated for `sample_rate`.
    pub fn new(sample_rate: f32) -> Result<Self, MsgError> {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        // Above the rate the descriptor declares its memory for, the lines stop at that size:
        // the longest delay is shorter, the times stay right.
        let frames = line_frames(sample_rate);
        let default = |id: u32| DESCRIPTOR.param(ParamId(id)).map_or(0.0, |d| d.default) as f32;
        let ramp = |id: u32| {
            DESCRIPTOR
                .param(ParamId(id))
                .map_or(0, |d| d.smoothing_frames(sample_rate))
        };
        let mode = default(MODE);
        let bloom = Self {
            sample_rate,
            lines: [line(frames)?, line(frames)?],
            write: 0,
            filled: 0,
            max_read: (frames - GUARD_FRAMES) as f64,
            synced: default(SYNC) >= 0.5,
            triangle: default(SHAPE) >= 0.5,
            rate: default(RATE),
            division: default(DIVISION) as usize,
            stereo: f64::from(default(STEREO_PHASE)) / 360.0,
            depth: default(DEPTH) * 0.01,
            delay: default(DELAY),
            feedback: Smoother::new(default(FEEDBACK)),
            tone: Smoother::new(default(TONE)),
            mix: Smoother::new(default(MIX)),
            mix_gains: mix_gains(default(MIX)),
            mix_now: default(MIX),
            tone_now: default(TONE),
            all_voices: false,
            quiet: frames,
            dry_weight: Smoother::new(dry_weight(mode)),
            ensemble_weight: Smoother::new(ensemble_weight(mode)),
            ramp_frames: [ramp(FEEDBACK), ramp(TONE), ramp(MIX)],
            mode_frames: (MODE_RAMP_SECONDS * sample_rate).round() as u32,
            tone_g: tone_gain(default(TONE), sample_rate),
            filters: [HighPass { state: 0.0 }; 2],
            lfo: Lfo::new(),
            voices: [[0.0; VOICES]; 2],
            centres: [[0.0; VOICES]; 2],
            half: 0.0,
            rebase: false,
            snap: true,
        };
        Ok(bloom)
    }

    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        match id.0 {
            MODE => {
                self.dry_weight.set(dry_weight(value), self.mode_frames);
                self.ensemble_weight
                    .set(ensemble_weight(value), self.mode_frames);
            }
            SYNC => {
                self.synced = value >= 0.5;
                self.rebase = true;
            }
            RATE => self.rate = value,
            DIVISION => {
                self.division = value as usize;
                self.rebase = true;
            }
            DEPTH => self.depth = value * 0.01,
            DELAY => self.delay = value,
            SHAPE => {
                self.triangle = value >= 0.5;
                self.rebase = true;
            }
            STEREO_PHASE => {
                self.stereo = f64::from(value) / 360.0;
                self.rebase = true;
            }
            FEEDBACK => self.feedback.set(value, self.ramp_frames[0]),
            TONE => self.tone.set(value, self.ramp_frames[1]),
            MIX => self.mix.set(value, self.ramp_frames[2]),
            _ => {}
        }
    }

    /// The length of a cycle in synced mode, in ticks.
    fn period_ticks(&self) -> f64 {
        DIVISION_BEATS
            .get(self.division)
            .copied()
            .unwrap_or(DIVISION_BEATS[0])
            * PPQ as f64
    }

    /// The line of `channel` read `delay` frames back, through the 4-point Hermite
    /// interpolator. `delay` is within `[MIN_READ_FRAMES, max_read]`: every index is inside.
    #[inline]
    fn tap(&self, channel: usize, delay: f64) -> f32 {
        let line = &self.lines[channel];
        let len = line.len();
        let whole = delay as usize;
        let t = (delay - whole as f64) as f32;
        // The sample written `k` frames ago, `1 <= k < len`; silence before the last reset.
        let at = |k: usize| {
            if k > self.filled {
                return 0.0;
            }
            let i = self.write + len - k;
            line[if i >= len { i - len } else { i }]
        };
        let (newer, x0, x1, older) = (at(whole - 1), at(whole), at(whole + 1), at(whole + 2));
        let c1 = 0.5 * (x1 - newer);
        let c2 = newer - 2.5 * x0 + 2.0 * x1 - 0.5 * older;
        let c3 = 0.5 * (older - newer) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let rate = self.sample_rate;
        let period = self.period_ticks();
        let len = self.lines[0].len();
        // The LFO's cycles per frame, and the widest sweep that keeps it under the speed limit.
        let cycles = if self.synced {
            self.lfo.ticks_per_frame / period
        } else {
            f64::from(self.rate) / f64::from(rate)
        };
        let widest = if cycles > 0.0 {
            MAX_SWEEP_SPEED / (core::f64::consts::PI * cycles)
        } else {
            f64::MAX
        };
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let feedback = (self.feedback.next_value() * 0.01).clamp(-FEEDBACK_MAX, FEEDBACK_MAX);
            // The gains follow the ramps to their very last value (the target included).
            let mix = self.mix.next_value();
            if mix != self.mix_now {
                self.mix_now = mix;
                self.mix_gains = mix_gains(mix);
            }
            let tone = self.tone.next_value();
            if tone != self.tone_now {
                self.tone_now = tone;
                self.tone_g = tone_gain(tone, rate);
            }
            let dry_w = self.dry_weight.next_value().clamp(0.0, 1.0);
            let ens = self.ensemble_weight.next_value().clamp(0.0, 1.0);
            // Voices 2 and 3 are only read while the ensemble is heard or fades; when it comes
            // back they start at their delays (at a weight of 0, so nothing jumps).
            let all_voices = ens > 0.0 || !self.ensemble_weight.is_still();
            let wake = all_voices && !self.all_voices;
            self.all_voices = all_voices;
            let voices_read = if all_voices { VOICES } else { 1 };
            let shortest = f64::from(self.delay) * f64::from(rate);
            let sweep = (f64::from(self.depth * MAX_SWEEP_SECONDS) * f64::from(rate)).min(widest);
            // Each voice swings by `half` around its centre; the centre drifts toward
            // `shortest + half` (module doc, "Centre").
            let half = 0.5 * sweep;
            let goal = shortest + half;
            let rebase = self.rebase || half != self.half;
            self.rebase = false;
            self.half = half;
            // Silent lines: moving the read position is not heard, the centres go at once.
            let silent = self.quiet >= len;
            let phase = self.lfo.phase(self.synced, period);
            let weights = [1.0 - 2.0 * ens / 3.0, ens / 3.0, ens / 3.0];
            let mut wet = [0.0f32; 2];
            for (channel, (voices, centres)) in self
                .voices
                .iter_mut()
                .zip(self.centres.iter_mut())
                .enumerate()
            {
                let offset = if channel == 1 { self.stereo } else { 0.0 };
                for (v, (voice, centre)) in voices
                    .iter_mut()
                    .zip(centres.iter_mut())
                    .enumerate()
                    .take(voices_read)
                {
                    let p = phase + offset + v as f64 / VOICES as f64;
                    let lfo = f64::from(shape_value(self.triangle, p));
                    let jump = self.snap || silent || (wake && v > 0);
                    if jump {
                        *centre = goal;
                    } else {
                        if rebase {
                            // The voice stays where it is: the new swing starts from there. When
                            // the line's ends leave no room for the whole swing, the centre is
                            // the one that keeps the voice in place with the widest swing that
                            // fits (`swing` below); it widens as the centre drifts.
                            *centre = rebased_centre(*voice, half, lfo, self.max_read, goal);
                        }
                        *centre += (goal - *centre).clamp(-MAX_DRIFT, MAX_DRIFT);
                    }
                    let swing = half
                        .min(*centre - MIN_READ_FRAMES)
                        .min(self.max_read - *centre)
                        .max(0.0);
                    let target = (*centre + swing * lfo)
                        .max(MIN_READ_FRAMES)
                        .min(self.max_read);
                    *voice = if jump {
                        target
                    } else {
                        *voice + (target - *voice).clamp(-MAX_SLEW, MAX_SLEW)
                    };
                }
            }
            for (channel, wet) in wet.iter_mut().enumerate() {
                let mut sum = 0.0;
                for (v, weight) in weights.iter().enumerate().take(voices_read) {
                    if *weight != 0.0 {
                        sum += weight * self.tap(channel, self.voices[channel][v]);
                    }
                }
                *wet = self.filters[channel].next(sum, self.tone_g);
            }
            self.snap = false;
            let dry_gain = dry_w * self.mix_gains.0;
            let wet_gain = dry_w * self.mix_gains.1 + (1.0 - dry_w);
            let mut sounding = false;
            for (channel, sample) in [&mut *l, &mut *r].into_iter().enumerate() {
                let dry = sanitize(*sample);
                let written =
                    flush((dry + wet[channel] * feedback).clamp(-INPUT_LIMIT, INPUT_LIMIT));
                sounding |= written != 0.0;
                self.lines[channel][self.write] = written;
                *sample = flush(dry * dry_gain + wet[channel] * wet_gain);
            }
            self.quiet = if sounding {
                0
            } else {
                (self.quiet + 1).min(len)
            };
            self.write += 1;
            if self.write == len {
                self.write = 0;
            }
            self.filled = (self.filled + 1).min(len);
            self.lfo.advance(self.synced, self.rate, period, rate);
        }
    }
}

/// The gains of the dry sound and of the copies at `mix`, an equal-power crossfade
/// (`cos`, `sin` of `mix * pi / 2`): the copies are mostly unrelated to the dry sound, so the
/// level stays where it was as the mix moves. Exactly (1, 0) at 0 and (0, 1) at 1.
fn mix_gains(mix: f32) -> (f32, f32) {
    if mix <= 0.0 || !mix.is_finite() {
        (1.0, 0.0)
    } else if mix >= 1.0 {
        (0.0, 1.0)
    } else {
        let a = mix * core::f32::consts::FRAC_PI_2;
        (a.cos(), a.sin())
    }
}

/// The centre that keeps a voice at `voice` frames while it swings by up to `half` at `lfo`,
/// within the line (`MIN_READ_FRAMES` to `max_read`): `voice - half * lfo` when the whole swing
/// fits, otherwise the centre whose widest fitting swing still passes through `voice`. At an
/// exact extreme of the LFO (`1 +/- lfo` near 0) the plain centre is kept (the swing then clamps
/// to what fits); a result that is not finite gives `fallback`.
fn rebased_centre(voice: f64, half: f64, lfo: f64, max_read: f64, fallback: f64) -> f64 {
    let at = voice - half * lfo;
    let centre = if at - half < MIN_READ_FRAMES && 1.0 + lfo > 1e-6 {
        (voice + MIN_READ_FRAMES * lfo) / (1.0 + lfo)
    } else if at + half > max_read && 1.0 - lfo > 1e-6 {
        (voice - max_read * lfo) / (1.0 - lfo)
    } else {
        at
    };
    if centre.is_finite() { centre } else { fallback }
}

/// The weight of the dry sound in `mode`.
fn dry_weight(mode: f32) -> f32 {
    if mode >= VIBRATO { 0.0 } else { 1.0 }
}

/// The weight of the ensemble's three voices in `mode`.
fn ensemble_weight(mode: f32) -> f32 {
    if mode == ENSEMBLE { 1.0 } else { 0.0 }
}

impl Device for Bloom {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    /// Renders the frames between events and applies each event at its frame
    /// ([`event_frame`]); the LFO takes the block's transport at its start.
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).min(io.left.len()).min(io.right.len());
        // At the start of playback the voices take their delays at once only when the lines
        // are silent (as when an export starts): a sound ringing through Bloom glides instead.
        // A seek, a loop's wrap or a start of playback can move the LFO's phase: the voices then
        // keep their place and their centres drift (module doc, "Centre").
        let period = self.period_ticks();
        let before = self.lfo.phase(self.synced, period);
        let started = self.lfo.begin(&ctx.transport, self.sample_rate);
        let moved = self.lfo.phase(self.synced, period) - before;
        if (moved - moved.round()).abs() > 1e-9 {
            self.rebase = true;
        }
        if started && self.quiet >= self.lines[0].len() {
            self.snap = true;
            // The tone filter's tail of a silent line is all that is left: an export starts
            // without it.
            self.filters = [HighPass { state: 0.0 }; 2];
        }
        let mut cursor = 0;
        for event in events {
            let at = event_frame(event.offset(), cursor, frames);
            if at > cursor {
                self.render(&mut io.left[cursor..at], &mut io.right[cursor..at]);
                cursor = at;
            }
            if let Event::ParamValue { id, value, .. } = *event {
                self.set_value(id, value);
            }
        }
        if frames > cursor {
            self.render(&mut io.left[cursor..frames], &mut io.right[cursor..frames]);
        }
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        self.set_value(id, value);
    }

    /// Empties both lines and the filters, snaps the ramps, and starts the LFO again (the next
    /// block takes the transport as at a start of playback); the parameter values are kept.
    /// Constant time: the lines are not touched, `filled` hides what they hold.
    fn reset(&mut self) {
        self.filled = 0;
        self.filters = [HighPass { state: 0.0 }; 2];
        for s in [
            &mut self.feedback,
            &mut self.tone,
            &mut self.mix,
            &mut self.dry_weight,
            &mut self.ensemble_weight,
        ] {
            s.snap();
        }
        self.tone_g = tone_gain(self.tone.current(), self.sample_rate);
        self.mix_gains = mix_gains(self.mix.current());
        self.tone_now = self.tone.current();
        self.mix_now = self.mix.current();
        self.quiet = self.lines[0].len();
        self.lfo = Lfo::new();
        self.snap = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rebase_at_an_exact_extreme_on_the_floor_stays_finite() {
        // At lfo = -1 with the voice on the floor, `(2 + 0.3) - 0.3` rounds below 2: the floor
        // branch is taken and its denominator `1 + lfo` is 0.
        let (voice, half, max) = (MIN_READ_FRAMES, 0.3, 1_000.0);
        assert!(
            voice - half * -1.0 - half < MIN_READ_FRAMES,
            "the case is reached"
        );
        let c = rebased_centre(voice, half, -1.0, max, 50.0);
        assert!(c.is_finite(), "{c}");
        assert!((c - (voice + half)).abs() < 1e-9, "{c}");
        // The same at the top of the line, lfo = +1.
        let (voice, half, max) = (1_000.0, 0.3, 1_000.0);
        let c = rebased_centre(voice, half, 1.0, max, 50.0);
        assert!(c.is_finite(), "{c}");
        // Inside the line the voice stays in place: centre + half * lfo = voice.
        for lfo in [-0.9, -0.3, 0.0, 0.4, 0.95] {
            let c = rebased_centre(300.0, 40.0, lfo, 1_000.0, 50.0);
            assert!((c + 40.0 * lfo - 300.0).abs() < 1e-9, "{lfo}: {c}");
        }
        // Near the floor: the widest fitting swing passes through the voice.
        let c = rebased_centre(10.0, 100.0, 0.5, 1_000.0, 50.0);
        let swing = (c - MIN_READ_FRAMES).min(100.0);
        assert!((c + swing * 0.5 - 10.0).abs() < 1e-9, "{c}");
    }
}
