//! Ripple, `faulk.ripple` (#688): a phaser (cascaded all-pass stages) and a flanger (a short
//! delay line with feedback) in one device, their notches moved by a tempo LFO and an envelope
//! follower, the Filter's movement ([`crate::dsp::movement`], #686). A [`Device`] of role
//! effect, stereo, latency 0.
//!
//! # Phaser
//!
//! Each channel runs a chain of `N` first-order all-pass stages (`stages`: 4, 6, 8 or 12),
//! `H(z) = (a + z^-1) / (1 + a z^-1)` with `a = (t - 1) / (t + 1)` and `t = tan(pi f / fs)`: the
//! bilinear transform of the analog all-pass, gain 1 at every frequency, +1 at DC, phase
//! `-2 atan(tan(pi f / fs) / t)`. Each stage runs `y = a u + z; z = u - a y`. Stage `i` of `N`
//! sits at `f_i = centre * 2^(spread * (i / (N - 1) - 1/2))`, every `f_i` clamped to
//! `[20 Hz, 0.45 fs]`, so `|a| < 1` strictly (between -0.9994 at 20 Hz and 192 kHz and 0.73 at
//! 0.45 fs): each stage is stable whatever its coefficient does. At `spread` 0 all the stages
//! share the centre. The wet signal is the chain's output.
//!
//! Mixed with the dry sound in equal parts (mix 100 %), the chain digs a notch wherever its
//! phase is an odd multiple of pi: with spread 0 and no feedback, at the `N / 2` frequencies
//! `tan(pi f_k / fs) = t tan(pi (2k + 1) / (2N))`, `k = 0 .. N/2 - 1` (four stages around 1 kHz
//! at 48 kHz: 414.7 Hz and 2 398 Hz), and lets the sound through at gain `|cos(phi / 2)|`
//! between them (1 where the phase is a multiple of 2 pi).
//!
//! # Flanger
//!
//! Each channel runs a delay line read at `delay` (0.1 to 20 ms, and at least one frame) with
//! linear interpolation; the line takes `x + feedback * sat(y)` where `y` is what it reads, the
//! wet signal. With feedback `g` and mix 100 % the response is
//! `(1 + (1 - g) z^-D) / (2 (1 - g z^-D))`: peaks every `1 / D` hertz (1 kHz at 1 ms) and notches
//! half-way between them.
//!
//! # Feedback, polarity and mix
//!
//! `feedback` (-95 % to +95 %) loops the wet signal back into the input: around the whole
//! chain with one frame of delay (the phaser), through the line (the flanger). It passes through
//! `sat`, the identity up to a magnitude of 2 and a smooth bend to at most 4 above
//! ([`saturate`]), so the signal fed back is bounded whatever happens. The flanger's line holds
//! at most `|x| + 0.95 * 4` (each read is a convex combination of what it holds), and so does
//! its output; the phaser's chain is fed at most `|x| + 3.8` and each stage, with `|a| < 1`,
//! keeps a bounded input bounded, also while its coefficient moves. Neither mode runs away or
//! oscillates by itself. As a last guard every state is clamped to `±1e9` (never reached by a
//! bounded input in the tests) and flushed below 1e-20, so no infinity, NaN or denormal ever
//! appears.
//!
//! `polarity` (positive or negative) is the sign of the wet signal in the mix: negative moves
//! the flanger's notches onto the multiples of `1 / D` (a notch at DC) and the phaser's onto
//! the even multiples of pi. `mix` (0 to 100 %, `m` = `mix / 100`) gives
//! `out = (dry + m * polarity * wet) / (1 + m)`: at 0 the output **is** the sanitized input, bit
//! for bit; at 100 % dry and wet in equal parts, the deepest notches. Unlike the Filter's mix,
//! 100 % is not the wet signal alone: an all-pass chain alone only shifts the phase, the notches
//! are the dry and the wet together.
//!
//! # Movement
//!
//! The modulation `m` of each channel, in octaves, moves the phaser's centre to `centre * 2^m`
//! and the flanger's delay to `delay * 2^-m`: positive moves the notches up in both modes, and an
//! automation lane on `centre` or `delay` moves the middle of the swing (as in the Filter). The
//! LFO's phase is the transport's position in synced mode (the same at each pass of a loop,
//! after a seek and in an offline export). While nothing moves (no LFO depth, no envelope amount,
//! no ramp running) the phaser's coefficients are computed once; otherwise on every frame, for
//! each channel. [`Device::readout`] reports the effective centre of the left channel in hertz
//! in phaser mode, its effective delay in seconds in flanger mode.
//!
//! # Changes
//!
//! `centre` (in hertz), `spread`, `delay` (in seconds), `feedback` and `mix` follow the
//! contract's linear ramp over the 20 ms their descriptor declares. The choices declare no
//! smoothing and fade by themselves over 10 ms:
//!
//! - `mode` crossfades the phaser's and the flanger's wet signals. A mode that is fully faded
//!   out is not computed; when it is chosen again it starts from cleared states (a mode still
//!   sounding, chosen back within the fade, keeps its states).
//! - `stages` crossfades the output tap of the chain (after stage 4, 6, 8 or 12) while the chain
//!   runs to the largest tap still sounding, and the stages glide to the layout of the new
//!   count over the same 10 ms (a layout matters only with a spread; the stages past the new
//!   count go on with its progression while they fade). A stage that starts to run starts
//!   from zero. The
//!   phaser's feedback takes the crossfaded output. A new count during a fade aims every tap at
//!   its new weight from where it is.
//! - `polarity` ramps the sign from +1 to -1 (or back) over 10 ms.
//!
//! The render does not depend on how the host cuts blocks. [`Device::reset`] clears the states
//! and snaps the ramps; the parameter values are kept. Every value goes through
//! [`ParamDescriptor::sanitize`]. The input is sanitized like the Filter's: a non-finite sample
//! is 0, a magnitude beyond 1e6 is clamped, below 1e-20 flushed.
//!
//! # Real time
//!
//! The two delay lines are allocated by the factory, off the audio path, for 20 ms at the
//! setup's rate (at most [`MAX_SAMPLE_RATE`]), and counted in the descriptor's `memory_bytes`;
//! `process` allocates nothing, locks nothing and never panics (indices are masked). The
//! all-pass coefficients and the powers of two are computed in `f64` and rounded once to `f32`,
//! so that the native and the WebAssembly builds, whose libm differ in the last bits, agree:
//! a stage's pole sits close to the unit circle at low frequencies and would carry a one-bit
//! difference for a long time.

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MAX_SAMPLE_RATE,
    ParamDescriptor, Scale, Unit, event_frame,
};
use crate::dsp::lfo::Shape;
use crate::dsp::movement::{self, Ids, Movement};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};
use crate::param::Smoother;

/// Ids of the parameters (never move).
pub const MODE: u32 = 0;
pub const STAGES: u32 = 1;
pub const CENTRE: u32 = 2;
pub const SPREAD: u32 = 3;
pub const DELAY: u32 = 4;
pub const FEEDBACK: u32 = 5;
pub const POLARITY: u32 = 6;
pub const MIX: u32 = 7;
pub const LFO_SHAPE: u32 = 8;
pub const LFO_SYNC: u32 = 9;
pub const LFO_RATE: u32 = 10;
pub const LFO_DIVISION: u32 = 11;
pub const LFO_PHASE: u32 = 12;
pub const LFO_STEREO: u32 = 13;
pub const LFO_DEPTH: u32 = 14;
pub const LFO_RESTART: u32 = 15;
pub const ENV_AMOUNT: u32 = 16;
pub const ENV_ATTACK: u32 = 17;
pub const ENV_RELEASE: u32 = 18;

/// The values of `mode`.
pub const PHASER: u32 = 0;
pub const FLANGER: u32 = 1;
/// The values of `polarity`.
pub const POSITIVE: u32 = 0;
pub const NEGATIVE: u32 = 1;

/// The stage counts of the values of `stages`, in order.
pub const STAGE_COUNTS: [usize; 4] = [4, 6, 8, 12];
const MAX_STAGES: usize = 12;

/// Lowest and highest centre, in hertz.
pub const CENTRE_MIN: f64 = 20.0;
pub const CENTRE_MAX: f64 = 20_000.0;
/// No stage's frequency goes above this share of the sample rate.
pub const NYQUIST_SHARE: f32 = 0.45;
/// Widest spread of the stages, in octaves.
pub const SPREAD_MAX: f64 = 4.0;
/// Shortest and longest delay of the flanger, in seconds.
pub const DELAY_MIN: f64 = 0.0001;
pub const DELAY_MAX: f64 = 0.02;
/// Largest feedback, in percent either way.
pub const FEEDBACK_MAX: f64 = 95.0;
/// The saturator of the feedback is the identity up to this magnitude, and stays under
/// [`SAT_LIMIT`].
pub const SAT_KNEE: f32 = 2.0;
pub const SAT_LIMIT: f32 = 4.0;
/// Time the choices take to fade, in seconds.
const FADE_SECONDS: f64 = 0.01;

/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;
/// The last guard on every state (module doc).
pub const STATE_LIMIT: f32 = 1e9;

/// Frames of a delay line at `sample_rate`: 20 ms and a margin for the interpolation, a power
/// of two (the indices are masked).
const fn line_frames(sample_rate: f32) -> usize {
    ((DELAY_MAX * sample_rate as f64) as usize + 4).next_power_of_two()
}

/// Bytes of the two delay lines at [`MAX_SAMPLE_RATE`].
const LINES_BYTES: u64 = 2 * size_of::<f32>() as u64 * line_frames(MAX_SAMPLE_RATE) as u64;

/// The ids of the movement's parameters.
const MOVEMENT: Ids = Ids {
    shape: LFO_SHAPE,
    sync: LFO_SYNC,
    rate: LFO_RATE,
    division: LFO_DIVISION,
    phase: LFO_PHASE,
    stereo: LFO_STEREO,
    depth: LFO_DEPTH,
    restart: LFO_RESTART,
    amount: ENV_AMOUNT,
    attack: ENV_ATTACK,
    release: ENV_RELEASE,
};

/// The note value of the default LFO: a whole note (one bar of 4/4).
const DEFAULT_DIVISION: u32 = 9;

/// The descriptor of Ripple (CT-5): `faulk.ripple`, displayed "Ripple" (one English name, never
/// translated). The defaults are the values of [`Ripple::new`]: a four-stage phaser around
/// 800 Hz, 30 % feedback, mix 100 %, swept two octaves either way by a sine over each bar.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.ripple",
    kind_id: DeviceKind(15),
    version: 1,
    name: "Ripple",
    role: DeviceRole::Effect,
    memory_bytes: size_of::<Ripple>() as u64 + LINES_BYTES,
    midi_output: None,
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(MODE),
            key: "mode",
            description: "Phaser (a chain of all-pass stages: soft, vocal notches) or flanger (a short delay with feedback: a metallic comb). A change fades over 10 ms.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["phaser", "flanger"],
        },
        ParamDescriptor {
            id: ParamId(STAGES),
            key: "stages",
            description: "Phaser: number of all-pass stages, 4, 6, 8 or 12; each pair of stages adds one notch. A change fades over 10 ms.",
            unit: Unit::None,
            min: 0.0,
            max: 3.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["four", "six", "eight", "twelve"],
        },
        ParamDescriptor {
            id: ParamId(CENTRE),
            key: "centre",
            description: "Phaser: centre frequency of the stages, from 20 Hz to 20 kHz: the middle the LFO and the envelope move the notches around.",
            unit: Unit::Hertz,
            min: CENTRE_MIN,
            max: CENTRE_MAX,
            default: 800.0,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(SPREAD),
            key: "spread",
            description: "Phaser: how far apart the stages sit around the centre, from 0 (all at the centre, evenly spaced notches) to 4 octaves (wider, uneven notches).",
            unit: Unit::Octaves,
            min: 0.0,
            max: SPREAD_MAX,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(DELAY),
            key: "delay",
            description: "Flanger: delay time, from 0.1 to 20 ms: the peaks of the comb sit every 1/delay hertz (1 kHz at 1 ms). The middle the LFO and the envelope move it around.",
            unit: Unit::Seconds,
            min: DELAY_MIN,
            max: DELAY_MAX,
            default: 0.002,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(FEEDBACK),
            key: "feedback",
            description: "Share of the effect fed back into its input, from -95 % to +95 %: sharper, ringing notches; negative feedback moves the peaks. Bounded: it never runs away.",
            unit: Unit::Percent,
            min: -FEEDBACK_MAX,
            max: FEEDBACK_MAX,
            default: 30.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(POLARITY),
            key: "polarity",
            description: "Sign of the effect in the mix: positive, or negative (the notches move between the former ones; the flanger then cuts the lowest frequencies). A change fades over 10 ms.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["positive", "negative"],
        },
        ParamDescriptor {
            id: ParamId(MIX),
            key: "mix",
            description: "How much of the effect joins the dry sound, from 0 (dry only) to 100 % (dry and effect in equal parts: the deepest notches).",
            unit: Unit::Percent,
            min: 0.0,
            max: 100.0,
            default: 100.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        movement::lfo_shape(
            LFO_SHAPE,
            "Shape of the LFO that sweeps the notches: sine, triangle, rising saw, falling saw, square, or random (a new value at each cycle).",
            Shape::Sine,
        ),
        movement::lfo_sync(
            LFO_SYNC,
            "Whether the LFO runs at its own rate in hertz (free) or follows the tempo (synced), each cycle on the same beat of every bar.",
            true,
        ),
        movement::lfo_rate(
            LFO_RATE,
            "Speed of the LFO in free mode, in cycles per second, from 0.01 to 20 Hz.",
            0.5,
        ),
        movement::lfo_division(
            LFO_DIVISION,
            "Length of one LFO cycle in synced mode, a note value from 1/32 to 8 bars.",
            DEFAULT_DIVISION,
        ),
        movement::lfo_phase(
            LFO_PHASE,
            "Where in its cycle the LFO starts, from 0 to 360 degrees: in synced mode, where in the bar the notches are highest.",
        ),
        movement::lfo_stereo(
            LFO_STEREO,
            "Offset of the right channel's LFO, from 0 to 180 degrees: at 180 the notches of the two channels move in opposition.",
        ),
        movement::lfo_depth(
            LFO_DEPTH,
            "How far the LFO sweeps the centre (phaser) or the delay (flanger) either side of its value, in octaves, from 0 (no sweep) to 5.",
            2.0,
        ),
        movement::lfo_restart(
            LFO_RESTART,
            "In free mode, whether the LFO starts again from its phase each time playback starts (on) or runs on freely (off).",
        ),
        movement::env_amount(
            ENV_AMOUNT,
            "How far the level of the incoming sound moves the notches, in octaves at full level, from -5 (louder lowers them) to +5 (louder raises them); 0 for none.",
        ),
        movement::env_attack(
            ENV_ATTACK,
            "Time the envelope follower takes to rise with the sound, from 1 ms to 500 ms.",
        ),
        movement::env_release(
            ENV_RELEASE,
            "Time the envelope follower takes to fall back when the sound gets quieter, from 10 ms to 2 s.",
        ),
    ],
};

/// Ripple's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a Ripple for `setup`, off the audio path.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Ripple::new(setup.sample_rate)?)?)
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

/// A state as it is kept: flushed below 1e-20, clamped to [`STATE_LIMIT`] (0 if not finite,
/// which a bounded input never gives).
#[inline]
fn keep(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR || x.is_nan() {
        0.0
    } else {
        x.clamp(-STATE_LIMIT, STATE_LIMIT)
    }
}

/// The saturator of the feedback: `y` up to a magnitude of [`SAT_KNEE`], then
/// `knee + knee * t / (t + knee)` with `t = |y| - knee` (continuous, with its slope), always
/// under [`SAT_LIMIT`].
#[inline]
pub fn saturate(y: f32) -> f32 {
    let a = y.abs();
    if a <= SAT_KNEE {
        y
    } else {
        let t = a - SAT_KNEE;
        let bent = SAT_KNEE + SAT_KNEE * t / (t + SAT_KNEE);
        bent.copysign(y)
    }
}

/// A stage frequency the phaser can use at `sample_rate`: in `[20 Hz, 0.45 * sample_rate]`,
/// 1 kHz for a non-finite one.
pub fn clamp_frequency(hz: f32, sample_rate: f32) -> f32 {
    let top = NYQUIST_SHARE * sample_rate;
    if hz.is_finite() {
        hz.clamp(CENTRE_MIN as f32, top.max(CENTRE_MIN as f32))
    } else {
        1000.0
    }
}

/// The all-pass coefficient of a stage at `hz` (clamped) and `sample_rate`, computed in `f64`
/// and rounded once: the libm of the native targets and of wasm32 differ in their last bits,
/// which a pole near the unit circle would carry along (module doc, Real time).
#[inline]
pub fn coefficient(hz: f32, sample_rate: f32) -> f32 {
    let t = (core::f64::consts::PI * f64::from(clamp_frequency(hz, sample_rate))
        / f64::from(sample_rate))
    .tan();
    ((t - 1.0) / (t + 1.0)) as f32
}

/// `2^x`, computed in `f64` and rounded once (as [`coefficient`]).
#[inline]
fn exp2(x: f32) -> f32 {
    f64::from(x).exp2() as f32
}

/// Where stage `i` of `n` sits in the spread: from -1/2 (the first) to +1/2 (the last).
fn position(i: usize, n: usize) -> f32 {
    if n < 2 {
        0.0
    } else {
        i as f32 / (n - 1) as f32 - 0.5
    }
}

/// The positions of the stages of a chain of `n`.
fn layout(n: usize) -> [f32; MAX_STAGES] {
    core::array::from_fn(|i| position(i, n))
}

/// The index of `STAGE_COUNTS` of a tap after stage `count`, if any.
fn tap_of(count: usize) -> Option<usize> {
    STAGE_COUNTS.iter().position(|c| *c == count)
}

/// The one-hot weights of a `stages` value.
fn tap_weights(index: usize) -> [f32; 4] {
    let mut w = [0.0; 4];
    w[index.min(3)] = 1.0;
    w
}

/// The all-pass chain of one channel.
#[derive(Clone, Copy, Debug, Default)]
struct Chain {
    z: [f32; MAX_STAGES],
    /// The chain's output on the last frame (what the feedback takes).
    last: f32,
}

#[derive(Debug)]
pub struct Ripple {
    sample_rate: f32,
    centre: Smoother,
    spread: Smoother,
    /// The flanger's delay, in seconds.
    delay: Smoother,
    /// Feedback and mix as fractions (the parameters are in percent).
    feedback: Smoother,
    mix: Smoother,
    /// The sign of the wet signal, from +1 to -1.
    polarity: Smoother,
    /// The weights of the phaser and of the flanger.
    modes: [Smoother; 2],
    /// The weights of the chain's taps after 4, 6, 8 and 12 stages.
    taps: [Smoother; 4],
    /// The position of each stage in the spread ([`position`]), fading to the new count's.
    layout: [Smoother; MAX_STAGES],
    ramp_frames: u32,
    fade_frames: u32,
    movement: Movement,
    chains: [Chain; 2],
    /// The coefficients of each channel's stages.
    coefficients: [[f32; MAX_STAGES]; 2],
    /// The stages' ratios to the centre, the spread they were computed for, and whether the
    /// layout moved since.
    ratios: [f32; MAX_STAGES],
    ratios_of: f32,
    layout_moved: bool,
    /// Whether the phaser's coefficients must be computed once more.
    stale: bool,
    /// The stages the chain ran on the last frame.
    ran: usize,
    lines: [Box<[f32]>; 2],
    mask: usize,
    write: usize,
    /// The longest delay the lines hold, in frames.
    max_delay_frames: f32,
    /// The centre (hertz) and the delay (seconds) each channel used on the last frame.
    centre_used: [f32; 2],
    delay_used: [f32; 2],
}

impl Ripple {
    /// A Ripple at its defaults, its delay lines allocated (off the audio path).
    pub fn new(sample_rate: f32) -> Result<Self, MsgError> {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        let frames = line_frames(sample_rate.min(MAX_SAMPLE_RATE));
        let default = |id: u32| DESCRIPTOR.param(ParamId(id)).map_or(0.0, |d| d.default);
        let ramp_frames = DESCRIPTOR
            .param(ParamId(CENTRE))
            .map_or(0, |d| d.smoothing_frames(sample_rate));
        let fade_frames = (FADE_SECONDS * f64::from(sample_rate)).round() as u32;
        let mode = default(MODE) as usize;
        let stage_index = default(STAGES) as usize;
        let centre = default(CENTRE) as f32;
        let delay = default(DELAY) as f32;
        let mut modes = [Smoother::new(0.0); 2];
        modes[mode.min(1)] = Smoother::new(1.0);
        let mut ripple = Self {
            sample_rate,
            centre: Smoother::new(centre),
            spread: Smoother::new(default(SPREAD) as f32),
            delay: Smoother::new(delay),
            feedback: Smoother::new(default(FEEDBACK) as f32 / 100.0),
            mix: Smoother::new(default(MIX) as f32 / 100.0),
            polarity: Smoother::new(if default(POLARITY) >= 0.5 { -1.0 } else { 1.0 }),
            modes,
            taps: tap_weights(stage_index).map(Smoother::new),
            layout: layout(STAGE_COUNTS[stage_index.min(3)]).map(Smoother::new),
            ramp_frames,
            fade_frames,
            movement: Movement::new(&DESCRIPTOR, &MOVEMENT, sample_rate),
            chains: [Chain::default(); 2],
            coefficients: [[0.0; MAX_STAGES]; 2],
            ratios: [1.0; MAX_STAGES],
            ratios_of: f32::NAN,
            layout_moved: true,
            stale: true,
            ran: 0,
            lines: [line(frames)?, line(frames)?],
            mask: frames - 1,
            write: 0,
            max_delay_frames: (frames - 3) as f32,
            centre_used: [clamp_frequency(centre, sample_rate); 2],
            delay_used: [delay; 2],
        };
        ripple.stale = true;
        Ok(ripple)
    }

    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        let ramp = self.ramp_frames;
        let fade = self.fade_frames;
        match id.0 {
            MODE => {
                let chosen = (value.round() as usize).min(1);
                for (i, w) in self.modes.iter_mut().enumerate() {
                    let target = if i == chosen { 1.0 } else { 0.0 };
                    // A mode fully faded out (at 0 and aimed at 0: it is not computed) starts
                    // again from cleared states. `aim` below leaves a restated mode alone, so a
                    // repeated value (a seek, an automation lane, an agent) never restarts a
                    // ramp that would hide this state.
                    if target > 0.0 && w.current() == 0.0 && w.target() == 0.0 {
                        if i == PHASER as usize {
                            self.chains = [Chain::default(); 2];
                            self.ran = 0;
                            self.stale = true;
                        } else {
                            for l in &mut self.lines {
                                l.fill(0.0);
                            }
                        }
                    }
                    w.aim(target, fade);
                }
            }
            STAGES => {
                let index = (value.round() as usize).min(3);
                for (s, target) in self.layout.iter_mut().zip(layout(STAGE_COUNTS[index])) {
                    s.set(target, fade);
                }
                for (tap, target) in self.taps.iter_mut().zip(tap_weights(index)) {
                    tap.set(target, fade);
                }
                self.stale = true;
            }
            CENTRE => {
                self.centre.set(value, ramp);
                self.stale = true;
            }
            SPREAD => {
                self.spread.set(value, ramp);
                self.stale = true;
            }
            DELAY => self.delay.set(value, ramp),
            FEEDBACK => self.feedback.set(value / 100.0, ramp),
            POLARITY => self
                .polarity
                .set(if value >= 0.5 { -1.0 } else { 1.0 }, fade),
            MIX => self.mix.set(value / 100.0, ramp),
            other => {
                self.movement
                    .set(&MOVEMENT, other, value, ramp, self.sample_rate);
            }
        }
    }

    /// The centre (hertz) the left and the right channel used on the last frame.
    pub fn centres_used(&self) -> [f32; 2] {
        self.centre_used
    }

    /// The delay (seconds) the left and the right channel used on the last frame.
    pub fn delays_used(&self) -> [f32; 2] {
        self.delay_used
    }

    /// The coefficients of the phaser's stages for the modulation `m` of each channel.
    fn update_coefficients(&mut self, centre: f32, spread: f32, run: usize, m: [f32; 2]) {
        let sr = self.sample_rate;
        if self.layout_moved || self.ratios_of != spread {
            for (r, at) in self.ratios.iter_mut().zip(&self.layout) {
                *r = if spread == 0.0 {
                    1.0
                } else {
                    exp2(spread * at.current())
                };
            }
            self.ratios_of = spread;
            self.layout_moved = false;
        }
        for ch in 0..2 {
            if ch == 1 && m[1] == m[0] {
                self.coefficients[1] = self.coefficients[0];
                self.centre_used[1] = self.centre_used[0];
                break;
            }
            let c = if m[ch] == 0.0 {
                centre
            } else {
                centre * exp2(m[ch])
            };
            let c = clamp_frequency(c, sr);
            self.centre_used[ch] = c;
            if spread == 0.0 {
                let a = coefficient(c, sr);
                self.coefficients[ch][..run].fill(a);
            } else {
                for i in 0..run {
                    self.coefficients[ch][i] = coefficient(c * self.ratios[i], sr);
                }
            }
        }
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let sr = self.sample_rate;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let laying = self.layout.iter().any(|s| !s.is_still());
            if laying {
                for s in &mut self.layout {
                    s.next_value();
                }
                self.layout_moved = true;
            }
            let ramping = laying || !self.centre.is_still() || !self.spread.is_still();
            let centre = self.centre.next_value();
            let spread = self.spread.next_value();
            let delay = self.delay.next_value();
            let fb = self.feedback.next_value();
            let mix = self.mix.next_value().clamp(0.0, 1.0);
            let pol = self.polarity.next_value();
            let w = self.modes.each_mut().map(Smoother::next_value);
            let taps = self.taps.each_mut().map(Smoother::next_value);
            let dry = [sanitize(*l), sanitize(*r)];
            let m = self.movement.next(dry[0].abs().max(dry[1].abs()), sr);

            let mut wet = [0.0f32; 2];
            if w[0] != 0.0 || self.modes[0].target() != 0.0 {
                // The chain runs to the largest tap still sounding or aimed at.
                let mut run = 0;
                for (k, count) in STAGE_COUNTS.iter().enumerate() {
                    if taps[k] != 0.0 || self.taps[k].target() != 0.0 {
                        run = *count;
                    }
                }
                if run > self.ran {
                    for chain in &mut self.chains {
                        chain.z[self.ran..run].fill(0.0);
                    }
                }
                self.ran = run;
                if m[0] != 0.0 || m[1] != 0.0 {
                    self.update_coefficients(centre, spread, run, m);
                    self.stale = true;
                } else if ramping || self.stale {
                    self.update_coefficients(centre, spread, run, [0.0; 2]);
                    self.stale = ramping;
                }
                for (ch, out) in wet.iter_mut().enumerate() {
                    let chain = &mut self.chains[ch];
                    let a = &self.coefficients[ch];
                    let mut u = dry[ch] + fb * saturate(chain.last);
                    let mut y = 0.0;
                    for (i, (a, z)) in a[..run].iter().zip(&mut chain.z[..run]).enumerate() {
                        let s = a * u + *z;
                        *z = keep(u - a * s);
                        u = s;
                        if let Some(k) = tap_of(i + 1) {
                            y += taps[k] * s;
                        }
                    }
                    let y = keep(y);
                    chain.last = y;
                    *out = w[0] * y;
                }
            } else {
                self.stale = true;
            }
            if w[1] != 0.0 || self.modes[1].target() != 0.0 {
                let mask = self.mask;
                for (ch, out) in wet.iter_mut().enumerate() {
                    let d = if m[ch] == 0.0 {
                        delay
                    } else {
                        delay * exp2(-m[ch])
                    };
                    let d = d.clamp(DELAY_MIN as f32, DELAY_MAX as f32);
                    self.delay_used[ch] = d;
                    let frames = (d * sr).clamp(1.0, self.max_delay_frames);
                    let whole = frames as usize;
                    let frac = frames - whole as f32;
                    let line = &mut self.lines[ch];
                    let i0 = (self.write + mask + 1 - whole) & mask;
                    let i1 = (i0 + mask) & mask;
                    let (x0, x1) = (line[i0], line[i1]);
                    let y = x0 + frac * (x1 - x0);
                    line[self.write & mask] = keep(dry[ch] + fb * saturate(y));
                    *out += w[1] * y;
                }
                self.write = (self.write + 1) & mask;
            }
            let gain = 1.0 / (1.0 + mix);
            *l = keep((dry[0] + mix * pol * wet[0]) * gain);
            *r = keep((dry[1] + mix * pol * wet[1]) * gain);
        }
    }
}

impl Device for Ripple {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    /// Renders the frames between events and applies each event at its frame
    /// ([`event_frame`]).
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).min(io.left.len()).min(io.right.len());
        self.movement.begin(&ctx.transport, self.sample_rate);
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

    /// Clears the chains and the lines, sets the LFO and the envelope back to their start and
    /// snaps the ramps; the parameter values are kept.
    fn reset(&mut self) {
        self.centre.snap();
        self.spread.snap();
        self.delay.snap();
        self.feedback.snap();
        self.mix.snap();
        self.polarity.snap();
        for w in self
            .modes
            .iter_mut()
            .chain(self.taps.iter_mut())
            .chain(self.layout.iter_mut())
        {
            w.snap();
        }
        self.layout_moved = true;
        self.movement.reset();
        self.chains = [Chain::default(); 2];
        self.ran = 0;
        for l in &mut self.lines {
            l.fill(0.0);
        }
        self.write = 0;
        self.stale = true;
        self.centre_used = [clamp_frequency(self.centre.current(), self.sample_rate); 2];
        self.delay_used = [self.delay.current(); 2];
    }

    /// The effective centre of the left channel in hertz (phaser), or its effective delay in
    /// seconds (flanger).
    fn readout(&self) -> Option<(ParamId, f32)> {
        if self.modes[FLANGER as usize].target() > 0.5 {
            Some((ParamId(DELAY), self.delay_used[0]))
        } else {
            Some((ParamId(CENTRE), self.centre_used[0]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saturator_is_the_identity_up_to_the_knee_and_stays_under_the_limit() {
        for y in [0.0, 0.5, -1.0, 1.999, 2.0, -2.0] {
            assert_eq!(saturate(y), y);
        }
        let mut last = 2.0;
        for i in 1..10_000 {
            let y = 2.0 + i as f32 * 0.01;
            let s = saturate(y);
            assert!(s > last - 1e-6 && s < SAT_LIMIT, "{y}: {s}");
            assert_eq!(saturate(-y), -s);
            last = s;
        }
        assert!(saturate(1e30) <= SAT_LIMIT);
        // Its slope is 1 at the knee: no corner.
        let slope = (saturate(2.001) - saturate(2.0)) / 0.001;
        assert!((slope - 1.0).abs() < 0.01, "{slope}");
    }

    #[test]
    fn every_coefficient_is_strictly_inside_the_unit_circle() {
        for sr in [8_000.0, 44_100.0, 48_000.0, 96_000.0, MAX_SAMPLE_RATE] {
            for hz in [0.0, 1.0, 20.0, 1000.0, 20_000.0, 1e9, f32::NAN] {
                let a = coefficient(hz, sr);
                assert!(a.abs() < 1.0, "{hz} Hz at {sr}: {a}");
            }
        }
    }

    #[test]
    fn the_lines_fit_twenty_milliseconds_at_every_rate() {
        for sr in [8_000.0, 48_000.0, MAX_SAMPLE_RATE] {
            let frames = line_frames(sr);
            assert!(frames.is_power_of_two());
            assert!(frames as f32 - 3.0 >= DELAY_MAX as f32 * sr);
        }
        assert_eq!(LINES_BYTES, 2 * 4 * 4096);
    }
}
