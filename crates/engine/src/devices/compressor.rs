//! The compressor, `faulk.compressor`, displayed "Compressor" (DV-5, extended by #157): a
//! stereo-linked, feed-forward compressor with a threshold, a ratio, a knee, an attack, a
//! manual or automatic release, peak or RMS detection, a high-pass on what it listens to, a
//! look-ahead, a makeup gain and a dry/compressed mix. A [`Device`] of role effect.
//!
//! # Signal
//!
//! Per frame, one gain for both channels (the stereo image does not move):
//!
//! 1. The input is sanitized like the echo's: a non-finite sample is 0, a magnitude beyond 1e6
//!    is clamped, a magnitude below 1e-20 is flushed. The sanitized frame `x` is written to a
//!    delay line.
//! 2. The **detection signal** `d` is `x`, or with `sidechain_hpf` on, `x` through a 12 dB
//!    per octave Butterworth high-pass at `sidechain_freq` on each channel (the state-variable
//!    filter of the Filter device, `k = sqrt 2`): the compressor stops listening to the lows,
//!    the sound itself is not filtered.
//! 3. The **level** in decibels: with `detection` on peak, `x_db = 20 log10(max(|d_L|, |d_R|,
//!    1e-6))`; on RMS, `x_db = 10 log10(max(ms, 1e-12))` with the mean square
//!    `ms = a_rms · ms + (1 − a_rms) · max(d_L², d_R²)`, a one-pole of [`RMS_SECONDS`]: a steady
//!    sine reads 3.01 dB under its peak (with a ripple of the one-pole at twice its frequency,
//!    about ±0.35 dB at 100 Hz), a square wave reads its peak.
//! 4. The **static curve**, the textbook one with a soft knee of width `W` (the `knee`
//!    parameter, [`KNEE_DB`] by default) centred on the threshold `T`; with `over = x_db − T`
//!    and `slope = 1 − 1/ratio`, the **gain reduction** in decibels is
//!    - `0` when `2 · over <= −W`;
//!    - `slope · over` when `2 · over >= W`;
//!    - `slope · (over + W/2)² / (2 W)` in between.
//!
//!    With `W = 0` the knee is hard (the middle branch is never reached: no division by 0).
//!
//!    With a look-ahead of `L` frames (step 6) the reduction then goes through a sliding
//!    maximum over the last `L + 1` frames (a monotonic queue in rings prepared off the audio
//!    path): the frame that comes out now, `L` frames back, was detected inside that window,
//!    so the release only starts once the loud part has left the output, not `L` frames
//!    before. With no look-ahead the window is the frame itself.
//! 5. The reduction is smoothed in the decibel domain by two stages. The **release** stage:
//!    - manual, `hold = max(gr, a_r · hold + (1 − a_r) · gr)`: it follows a rise at once and
//!      decays exponentially (a peak hold);
//!    - auto, program dependent: a slow mean of the reduction `S = a_s · S + (1 − a_s) · gr`
//!      ([`AUTO_SLOW_SECONDS`]) and `hold = max(gr, S + a_f · (hold − S))`
//!      ([`AUTO_FAST_SECONDS`]): after a short hit the gain comes back fast; after a long
//!      compression it comes back fast to the sustained reduction, then slowly with it (less
//!      pumping). The `release` parameter is not used.
//!
//!    The output `y` of the **attack** stage follows `hold` down at once, and up through a
//!    one-pole low-pass, `y = a_a · y + (1 − a_a) · hold`. With `a = exp(−1 / (tau ·
//!    sample_rate))` the `attack` and `release` parameters are **time constants** (the time
//!    to cover 63% of the way), each exact whatever the other. A sine settles on the
//!    reduction of its peak; the droop of the hold between two peaks is `exp(−half period /
//!    release)` of the reduction, which is why a very short release on a low note lets the
//!    gain ripple with the wave.
//! 6. The **look-ahead** of `L = round(lookahead · sample_rate)` frames: the gain computed
//!    from the frame `n` applies to the frame `n − L` of the line, so the gain starts closing
//!    `L` frames before a hit reaches the output. The compressed sample is `c = x[n − L] ·
//!    10^((makeup − y) / 20)` and the output `c · m + x[n − L] · (1 − m)`, `m` the `mix`
//!    (exactly `c` at 1, exactly `x[n − L]` at 0).
//!
//! The gain is `makeup − y` decibels, `y >= 0`: **the output never exceeds the delayed input
//! times the makeup gain** (the mix of two samples within that bound stays within it). With
//! no look-ahead the first milliseconds of a sudden loud sound pass before the attack has
//! closed the gain (bounded by that bound, never beyond); a look-ahead longer than a few
//! attack times catches them.
//!
//! The smoothing states are flushed below 1e-6 dB, the mean square and the filter states
//! below 1e-20 (a detection sample under 1e-15 counts as 0 in the mean square, so its square
//! is never subnormal), so no denormal ever appears in a decay.
//!
//! # The MVP's compressor
//!
//! At the defaults of the parameters #157 added (`knee` 6 dB, peak detection, manual release,
//! no high-pass, no look-ahead, mix 1) every added stage is skipped or exact, and the device
//! renders the very samples of the MVP's (DV-5): a project saved before #157 opens and plays
//! the same (`tests/it/compressor.rs` against a frozen copy of the MVP's device, and the
//! WASM fingerprint of a reopened project in `crates/test-support`).
//!
//! # Latency
//!
//! [`Device::latency_frames`] is `L`, 0 by default, at most [`Device::max_latency_frames`]
//! (the longest look-ahead at the device's rate). The graph compensates it (#749,
//! [`crate::graph`] > Latency), as the Ceiling's: the other tracks are delayed to match, and a
//! bypassed, inserted or removed Compressor keeps its timing through the graph's bypass line
//! (its dry side delayed by `L`), so a bypass neither moves the track nor combs.
//! [`MEMORY_BYTES`] counts that line.
//!
//! A change of `lookahead` while the device sounds goes through the [`LatencySwitch`]: one
//! switch per gesture, a fade out, a silence of the new latency, a fade in. A change made
//! before any frame since the last reset applies at once.
//!
//! # Parameters and smoothing
//!
//! `threshold`, `ratio`, `knee`, `makeup` and `mix` follow the 20 ms linear ramp the descriptor
//! declares; `attack`, `release` and `sidechain_freq` are not ramped: their coefficients are
//! recomputed when they are set (the gain smoothing covers the move); `detection`,
//! `release_mode` and `sidechain_hpf` take effect at their frame (switching the high-pass on
//! starts its filter from rest); `lookahead` follows the latency switch. Every value goes
//! through [`ParamDescriptor::sanitize`].
//!
//! # Real time
//!
//! The delay line and the rings of the sliding maximum are allocated by [`factory`] with
//! fallible allocations, sized for the longest look-ahead at the device's sample rate, at
//! most [`MAX_SAMPLE_RATE`] (a non-finite rate or one below 8 kHz is taken as 48 kHz, as in
//! the MVP; above [`MAX_SAMPLE_RATE`] the look-ahead is at most the line's length, shorter
//! than 10 ms). [`Device::reset`] takes constant time and wipes no
//! buffer: the count of frames since the reset hides the old content of the line (an older
//! position reads as silence). It zeroes the reduction and the detector, takes a pending
//! look-ahead and snaps the ramps; the parameter values are kept.

use super::Builtin;
use super::filter::{Channel, Coefficients};
use super::latency_switch::{LatencySwitch, Take};
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MAX_SAMPLE_RATE,
    ParamDescriptor, Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};
use crate::param::Smoother;

pub use super::latency_switch::{FADE_SECONDS, SETTLE_SECONDS};

/// Ids of the parameters (the MVP's, then #157's).
pub const THRESHOLD: u32 = 0;
pub const RATIO: u32 = 1;
pub const ATTACK: u32 = 2;
pub const RELEASE: u32 = 3;
pub const MAKEUP: u32 = 4;
pub const KNEE: u32 = 5;
pub const DETECTION: u32 = 6;
pub const RELEASE_MODE: u32 = 7;
pub const SIDECHAIN_HPF: u32 = 8;
pub const SIDECHAIN_FREQ: u32 = 9;
pub const LOOKAHEAD: u32 = 10;
pub const MIX: u32 = 11;

/// The default width of the soft knee, in decibels, centred on the threshold: the MVP's
/// fixed knee.
pub const KNEE_DB: f32 = 6.0;
/// The widest knee, in decibels.
pub const KNEE_MAX_DB: f64 = 24.0;
/// Highest makeup gain, in decibels: the bound of the output over the input.
pub const MAKEUP_MAX_DB: f64 = 24.0;
/// The longest look-ahead, in seconds.
pub const LOOKAHEAD_MAX_SECONDS: f64 = 0.01;
/// The time constant of the RMS detector's mean square.
pub const RMS_SECONDS: f32 = 0.01;
/// The auto release's fast and slow time constants.
pub const AUTO_FAST_SECONDS: f32 = 0.05;
pub const AUTO_SLOW_SECONDS: f32 = 1.0;
/// The bounds of the sidechain high-pass's frequency, in hertz.
pub const SIDECHAIN_FREQ_MIN: f64 = 20.0;
pub const SIDECHAIN_FREQ_MAX: f64 = 500.0;

/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;
/// The detector never reads below this level (-120 dBFS).
const LEVEL_FLOOR: f32 = 1e-6;
/// A smoothing state below this, in decibels, is 0.
const STATE_FLOOR_DB: f32 = 1e-6;
/// `20 / ln(10)` and `ln(10) / 20`: decibels from a natural log, and back.
const DB_PER_NEPER: f32 = 8.685_89;
const NEPER_PER_DB: f32 = 0.115_129_26;

/// The descriptor of the compressor (CT-5): `faulk.compressor`, displayed "Compressor" (one
/// English name, never translated). The defaults are the values of [`Compressor::new`]:
/// -18 dB, 4:1, a 6 dB knee, 10 ms attack, 150 ms manual release, peak detection, no
/// high-pass, no look-ahead, no makeup, fully compressed. The order of the parameters is the
/// panel's; their ids are stable (the MVP's five, then #157's).
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.compressor",
    kind_id: DeviceKind(5),
    version: 1,
    name: "Compressor",
    role: DeviceRole::Effect,
    memory_bytes: MEMORY_BYTES,
    midi_output: None,
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(THRESHOLD),
            key: "threshold",
            description: "Level, in decibels, above which the compressor turns the sound down, from -60 to 0 dB.",
            unit: Unit::Decibels,
            min: -60.0,
            max: 0.0,
            default: -18.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(RATIO),
            key: "ratio",
            description: "How strongly the sound above the threshold is turned down, from 1 (not at all) to 20: at 4, a signal 8 dB over the threshold comes out 2 dB over it.",
            unit: Unit::None,
            min: 1.0,
            max: 20.0,
            default: 4.0,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(KNEE),
            key: "knee",
            description: "Width, in decibels, of the zone around the threshold where the compression comes in gradually, from 0 (hard: the full ratio right at the threshold) to 24 dB (soft).",
            unit: Unit::Decibels,
            min: 0.0,
            max: KNEE_MAX_DB,
            default: KNEE_DB as f64,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(ATTACK),
            key: "attack",
            description: "Time the compressor takes to turn the sound down once it passes the threshold, from 0.1 ms to 200 ms: short catches the first hit, long lets it through.",
            unit: Unit::Seconds,
            min: 0.0001,
            max: 0.2,
            default: 0.01,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(RELEASE_MODE),
            key: "release_mode",
            description: "How the compressor lets the sound back up: manual (the release time) or auto (fast after a short hit, slower after a long compression, so it pumps less).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["manual", "auto"],
        },
        ParamDescriptor {
            id: ParamId(RELEASE),
            key: "release",
            description: "In manual release, time the compressor takes to let the sound back up once it falls under the threshold, from 10 ms to 2 s.",
            unit: Unit::Seconds,
            min: 0.01,
            max: 2.0,
            default: 0.15,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(DETECTION),
            key: "detection",
            description: "What level the compressor reacts to: peak (each sample, it catches the hits) or RMS (the average over 10 ms, it follows the loudness and lets short hits through).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["peak", "rms"],
        },
        ParamDescriptor {
            id: ParamId(LOOKAHEAD),
            key: "lookahead",
            description: "How far ahead the compressor sees the sound, from 0 to 10 ms, so a short attack catches a hit whole. It delays the sound by that time; the other tracks are delayed to match, so everything stays in time. A change fades out and back in.",
            unit: Unit::Seconds,
            min: 0.0,
            max: LOOKAHEAD_MAX_SECONDS,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(SIDECHAIN_HPF),
            key: "sidechain_hpf",
            description: "When on, the compressor does not react to the lows under the sidechain frequency (a 12 dB per octave high-pass on what it listens to, not on the sound): a kick or a bass no longer pumps everything else.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["off", "on"],
        },
        ParamDescriptor {
            id: ParamId(SIDECHAIN_FREQ),
            key: "sidechain_freq",
            description: "With the sidechain high-pass on, the frequency under which the compressor stops reacting, from 20 to 500 Hz.",
            unit: Unit::Hertz,
            min: SIDECHAIN_FREQ_MIN,
            max: SIDECHAIN_FREQ_MAX,
            default: 100.0,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(MAKEUP),
            key: "makeup",
            description: "Gain added after the compression to make up for the level it took away, from 0 to 24 dB.",
            unit: Unit::Decibels,
            min: 0.0,
            max: MAKEUP_MAX_DB,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(MIX),
            key: "mix",
            description: "Balance between the dry sound (0) and the compressed sound (1): in between is parallel compression, the hits keep their attack and the quiet parts come up.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 1.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
    ],
};

/// The compressor's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a compressor for `setup`, off the audio path: its delay line is allocated here,
/// with fallible allocations.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Compressor::new(setup.sample_rate)?)?)
}

/// The look-ahead, in frames, of `seconds` at `sample_rate` (bounded to the longest).
pub fn lookahead_frames(seconds: f32, sample_rate: f32) -> u32 {
    let longest = (LOOKAHEAD_MAX_SECONDS * f64::from(sample_rate)).round();
    let frames = (f64::from(seconds) * f64::from(sample_rate))
        .round()
        .clamp(0.0, longest);
    if frames.is_finite() { frames as u32 } else { 0 }
}

/// Frames of the line at `sample_rate`: a power of two above the longest look-ahead.
const fn capacity(sample_rate: f32) -> usize {
    ((LOOKAHEAD_MAX_SECONDS * sample_rate as f64) as usize + 2).next_power_of_two()
}

/// Bytes per frame of capacity: the delay line (two channels) and the queue of the sliding
/// maximum (a frame index and a reduction).
const BYTES_PER_FRAME: usize = 3 * size_of::<f32>() + size_of::<u64>();

/// The most latency an instance at `sample_rate` adds: the longest look-ahead, bounded by
/// its line (as [`Compressor::new`] bounds it).
const fn max_latency_at(sample_rate: f32) -> u32 {
    let longest = (LOOKAHEAD_MAX_SECONDS * sample_rate as f64 + 0.5) as u32;
    let line = (capacity(sample_rate) - 2) as u32;
    if longest < line { longest } else { line }
}

/// The most memory an instance takes, at [`MAX_SAMPLE_RATE`], with the graph's bypass line for
/// its latency and its wrapper (#749, [`crate::latency::Latent`]).
pub const MEMORY_BYTES: u64 = (size_of::<Compressor>()
    + capacity(MAX_SAMPLE_RATE) * BYTES_PER_FRAME) as u64
    + crate::latency::Latent::bytes_for(max_latency_at(MAX_SAMPLE_RATE));

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

fn flush_db(x: f32) -> f32 {
    if x < STATE_FLOOR_DB { 0.0 } else { x }
}

/// The one-pole coefficient of a time constant `seconds`.
fn coefficient(seconds: f32, sample_rate: f32) -> f32 {
    (-1.0 / (f64::from(seconds.max(1e-5)) * f64::from(sample_rate))).exp() as f32
}

/// `len` zeroed values, or a typed error when the memory is refused.
fn buffer<T: Copy + Default>(len: usize) -> Result<Box<[T]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| MsgError::OutOfMemory {
            bytes: len.saturating_mul(size_of::<T>()),
        })?;
    v.resize(len, T::default());
    Ok(v.into_boxed_slice())
}

/// The gain reduction, in decibels (>= 0), of a level `x_db` against `threshold_db`, with
/// `slope = 1 - 1/ratio` and a soft knee `knee_db` wide (0: a hard knee).
pub fn gain_reduction_db(x_db: f32, threshold_db: f32, slope: f32, knee_db: f32) -> f32 {
    let over = x_db - threshold_db;
    if 2.0 * over <= -knee_db {
        0.0
    } else if 2.0 * over >= knee_db {
        slope * over
    } else {
        // Reached only with `knee_db > 0`.
        let t = over + 0.5 * knee_db;
        slope * t * t / (2.0 * knee_db)
    }
}

#[derive(Debug)]
pub struct Compressor {
    sample_rate: f32,
    threshold: Smoother,
    ratio: Smoother,
    makeup: Smoother,
    knee: Smoother,
    mix: Smoother,
    /// The ramps of threshold, ratio, makeup, knee and mix.
    ramp_frames: [u32; 5],
    attack: f32,
    release: f32,
    auto_release: bool,
    fast: f32,
    slow: f32,
    rms: bool,
    rms_coefficient: f32,
    sidechain_hpf: bool,
    sidechain: Coefficients,
    /// The detection high-pass's states, left and right.
    hp: [Channel; 2],
    /// The look-ahead in use and the one a change asked for, in frames.
    lookahead: u32,
    wanted: u32,
    switch: LatencySwitch,
    /// The longest look-ahead the line holds, in frames.
    longest: u32,
    /// The delay line, `mask + 1` frames per channel.
    mask: usize,
    line_l: Box<[f32]>,
    line_r: Box<[f32]>,
    /// The sliding maximum's queue: frame indices and reductions, decreasing reductions from
    /// `head` to `tail`.
    queue_at: Box<[u64]>,
    queue_db: Box<[f32]>,
    head: u64,
    tail: u64,
    /// Frames since the last reset.
    frame: u64,
    /// The RMS detector's mean square.
    mean_square: f32,
    /// The release stage, the auto release's slow mean and the attack stage, in decibels of
    /// gain reduction.
    hold: f32,
    sustained: f32,
    reduction: f32,
    /// The mix of the last frame, for the reduction reported.
    mix_now: f32,
}

impl Compressor {
    /// A compressor at its defaults, its line allocated (fallibly) for `sample_rate`.
    pub fn new(sample_rate: f32) -> Result<Self, MsgError> {
        // The MVP's fallback: the coefficients keep the rate the device runs at.
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        let default = |id: u32| DESCRIPTOR.param(ParamId(id)).map_or(0.0, |d| d.default);
        let ramp = |id: u32| {
            DESCRIPTOR
                .param(ParamId(id))
                .map_or(0, |d| d.smoothing_frames(sample_rate))
        };
        let frames = capacity(sample_rate.min(MAX_SAMPLE_RATE));
        let longest = (frames - 2) as u32;
        let lookahead = lookahead_frames(default(LOOKAHEAD) as f32, sample_rate).min(longest);
        Ok(Self {
            sample_rate,
            threshold: Smoother::new(default(THRESHOLD) as f32),
            ratio: Smoother::new(default(RATIO) as f32),
            makeup: Smoother::new(default(MAKEUP) as f32),
            knee: Smoother::new(default(KNEE) as f32),
            mix: Smoother::new(default(MIX) as f32),
            ramp_frames: [
                ramp(THRESHOLD),
                ramp(RATIO),
                ramp(MAKEUP),
                ramp(KNEE),
                ramp(MIX),
            ],
            attack: coefficient(default(ATTACK) as f32, sample_rate),
            release: coefficient(default(RELEASE) as f32, sample_rate),
            auto_release: default(RELEASE_MODE) >= 0.5,
            fast: coefficient(AUTO_FAST_SECONDS, sample_rate),
            slow: coefficient(AUTO_SLOW_SECONDS, sample_rate),
            rms: default(DETECTION) >= 0.5,
            rms_coefficient: coefficient(RMS_SECONDS, sample_rate),
            sidechain_hpf: default(SIDECHAIN_HPF) >= 0.5,
            sidechain: Coefficients::new(default(SIDECHAIN_FREQ) as f32, 0.0, sample_rate),
            hp: [Channel::default(); 2],
            lookahead,
            wanted: lookahead,
            switch: LatencySwitch::new(sample_rate),
            longest,
            mask: frames - 1,
            line_l: buffer(frames)?,
            line_r: buffer(frames)?,
            queue_at: buffer(frames)?,
            queue_db: buffer(frames)?,
            head: 0,
            tail: 0,
            frame: 0,
            mean_square: 0.0,
            hold: 0.0,
            sustained: 0.0,
            reduction: 0.0,
            mix_now: default(MIX) as f32,
        })
    }

    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        match id.0 {
            THRESHOLD => self.threshold.set(value, self.ramp_frames[0]),
            RATIO => self.ratio.set(value, self.ramp_frames[1]),
            ATTACK => self.attack = coefficient(value, self.sample_rate),
            RELEASE => self.release = coefficient(value, self.sample_rate),
            MAKEUP => self.makeup.set(value, self.ramp_frames[2]),
            KNEE => self.knee.set(value, self.ramp_frames[3]),
            DETECTION => self.rms = value >= 0.5,
            RELEASE_MODE => self.auto_release = value >= 0.5,
            SIDECHAIN_HPF => {
                let on = value >= 0.5;
                if on && !self.sidechain_hpf {
                    // The filter starts from rest, not from what it held when last on.
                    self.hp = [Channel::default(); 2];
                }
                self.sidechain_hpf = on;
            }
            SIDECHAIN_FREQ => {
                self.sidechain = Coefficients::new(value, 0.0, self.sample_rate);
            }
            LOOKAHEAD => {
                self.wanted = lookahead_frames(value, self.sample_rate).min(self.longest);
                match self
                    .switch
                    .ask(self.frame == 0, self.wanted != self.lookahead)
                {
                    Take::Later => {}
                    Take::Now => self.take_latency(),
                    Take::NowSilent => {
                        self.take_latency();
                        self.switch.silence(self.lookahead);
                    }
                }
            }
            MIX => self.mix.set(value, self.ramp_frames[4]),
            _ => {}
        }
    }

    /// The latency, in frames, the device adds now: the look-ahead in use.
    pub fn latency(&self) -> u32 {
        self.lookahead
    }

    /// Takes the wanted look-ahead; the sliding maximum starts again (what it held was
    /// detected for the old window).
    fn take_latency(&mut self) {
        self.lookahead = self.wanted;
        self.head = 0;
        self.tail = 0;
    }

    /// The largest of the reductions of the frames `n − L ..= n`, `wanted` the one of `n`.
    #[inline]
    fn window_max(&mut self, n: u64, wanted: f32) -> f32 {
        if self.lookahead == 0 {
            return wanted;
        }
        while self.tail > self.head && self.queue_db[(self.tail - 1) as usize & self.mask] <= wanted
        {
            self.tail -= 1;
        }
        let slot = self.tail as usize & self.mask;
        self.queue_at[slot] = n;
        self.queue_db[slot] = wanted;
        self.tail += 1;
        while self.queue_at[self.head as usize & self.mask] + u64::from(self.lookahead) < n {
            self.head += 1;
        }
        self.queue_db[self.head as usize & self.mask]
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            (*l, *r) = self.frame_of(*l, *r);
        }
    }

    /// The detected level of the frame `(dl, dr)` (the detection signal), in decibels.
    #[inline]
    fn level_db(&mut self, dl: f32, dr: f32) -> f32 {
        let level = dl.abs().max(dr.abs());
        // The mean square runs in both modes, so a switch to RMS reads the program now, not
        // what it held when RMS was last on. Under 1e-15 the square would be subnormal: it
        // counts as silence.
        let square = if level < 1e-15 { 0.0 } else { level * level };
        let ms = self.rms_coefficient * self.mean_square + (1.0 - self.rms_coefficient) * square;
        self.mean_square = flush(ms);
        if self.rms {
            0.5 * DB_PER_NEPER * self.mean_square.max(LEVEL_FLOOR * LEVEL_FLOOR).ln()
        } else {
            DB_PER_NEPER * level.max(LEVEL_FLOOR).ln()
        }
    }

    /// One frame in, one frame out.
    #[inline]
    fn frame_of(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        if self.switch.due() {
            self.take_latency();
            self.switch.silence(self.lookahead);
        }
        let threshold = self.threshold.next_value().clamp(-60.0, 0.0);
        let ratio = self.ratio.next_value().clamp(1.0, 20.0);
        let makeup = self.makeup.next_value().clamp(0.0, MAKEUP_MAX_DB as f32);
        let knee = self.knee.next_value().clamp(0.0, KNEE_MAX_DB as f32);
        let mix = self.mix.next_value().clamp(0.0, 1.0);
        // The gain multiplies the sanitized samples: a NaN or an infinity never comes out.
        let (dry_l, dry_r) = (sanitize(in_l), sanitize(in_r));
        let at = self.frame as usize & self.mask;
        self.line_l[at] = dry_l;
        self.line_r[at] = dry_r;

        let (dl, dr) = if self.sidechain_hpf {
            let c = self.sidechain;
            (self.hp[0].tick(dry_l, &c).1, self.hp[1].tick(dry_r, &c).1)
        } else {
            (dry_l, dry_r)
        };
        let x_db = self.level_db(dl, dr);
        let wanted = gain_reduction_db(x_db, threshold, 1.0 - 1.0 / ratio, knee);
        let wanted = self.window_max(self.frame, wanted);
        // The slow mean runs in both modes, so a switch to auto starts from the program.
        self.sustained = flush_db(self.slow * self.sustained + (1.0 - self.slow) * wanted);
        self.hold = if self.auto_release {
            flush_db(wanted.max(self.sustained + self.fast * (self.hold - self.sustained)))
        } else {
            flush_db(wanted.max(self.release * self.hold + (1.0 - self.release) * wanted))
        };
        self.reduction = if self.hold <= self.reduction {
            self.hold
        } else {
            flush_db(self.attack * self.reduction + (1.0 - self.attack) * self.hold)
        };
        // `reduction >= 0`: the gain never exceeds the makeup.
        let gain = (NEPER_PER_DB * (makeup - self.reduction)).exp();

        // The sample `L` frames back; before the reset, the line holds old content: silence.
        let (xl, xr) = match self.frame.checked_sub(u64::from(self.lookahead)) {
            Some(k) => {
                let k = k as usize & self.mask;
                (self.line_l[k], self.line_r[k])
            }
            None => (0.0, 0.0),
        };
        let (wet_l, wet_r) = (xl * gain, xr * gain);
        let (mut yl, mut yr) = if mix >= 1.0 {
            (wet_l, wet_r)
        } else if mix <= 0.0 {
            (xl, xr)
        } else {
            (
                xl * (1.0 - mix) + wet_l * mix,
                xr * (1.0 - mix) + wet_r * mix,
            )
        };
        self.mix_now = mix;
        let fade = self.switch.gain(self.wanted != self.lookahead);
        yl *= fade;
        yr *= fade;
        self.frame += 1;
        (flush(yl), flush(yr))
    }
}

impl Device for Compressor {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    /// Renders the frames between events and applies each event at its frame
    /// ([`event_frame`]).
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).min(io.left.len()).min(io.right.len());
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

    /// The smoothed reduction now (>= 0, finite), the makeup aside. With a mix under 1, the
    /// reduction of the sum heard: `−20 log10(1 − m + m · 10^(−y/20))`, smaller than the
    /// compressed path's `y` (at mix 0, nothing).
    fn gain_reduction_db(&self) -> f32 {
        let y = if self.reduction.is_finite() {
            self.reduction.max(0.0)
        } else {
            0.0
        };
        let m = self.mix_now;
        if m >= 1.0 || y == 0.0 {
            return y;
        }
        let gain = (1.0 - m) + m * (-NEPER_PER_DB * y).exp();
        let db = -DB_PER_NEPER * gain.max(1e-12).ln();
        if db.is_finite() {
            db.clamp(0.0, y)
        } else {
            0.0
        }
    }

    fn latency_frames(&self) -> u32 {
        self.latency()
    }

    /// The longest look-ahead at the device's rate (#749): the graph sizes its bypass line
    /// from it.
    fn max_latency_frames(&self) -> u32 {
        lookahead_frames(LOOKAHEAD_MAX_SECONDS as f32, self.sample_rate).min(self.longest)
    }

    /// Opens the gain, rests the detector, hides the line's content, takes a pending
    /// look-ahead and snaps the ramps; the parameter values are kept. Constant time: no
    /// buffer is wiped.
    fn reset(&mut self) {
        self.hold = 0.0;
        self.sustained = 0.0;
        self.reduction = 0.0;
        self.mean_square = 0.0;
        self.hp = [Channel::default(); 2];
        self.frame = 0;
        self.take_latency();
        self.switch.reset();
        self.threshold.snap();
        self.ratio.snap();
        self.makeup.snap();
        self.knee.snap();
        self.mix.snap();
    }
}
