//! The compressor, `faulk.compressor`, displayed "Compressor" (DV-5): a stereo-linked,
//! feed-forward peak compressor with a threshold, a ratio, an attack, a release and a makeup
//! gain. A [`Device`] of role effect.
//!
//! # Signal
//!
//! For each frame the detector takes the larger magnitude of both channels (one gain for
//! both, so the stereo image does not move), in decibels: `x = 20 log10(max(level, 1e-6))`.
//! The **static curve** is the textbook one with a fixed soft knee of [`KNEE_DB`] (6 dB) centred
//! on the threshold `T`; with `over = x - T` and `slope = 1 - 1/ratio`, the **gain reduction**
//! in decibels is
//!
//! - `0` when `2 * over <= -KNEE_DB`;
//! - `slope * (over + KNEE_DB / 2)^2 / (2 * KNEE_DB)` inside the knee;
//! - `slope * over` above it.
//!
//! The reduction is smoothed in the decibel domain by two stages: the **release** stage
//! `hold = max(gr, a_r * hold + (1 - a_r) * gr)` follows a rise at once and decays
//! exponentially (a peak hold); the output `y` of the **attack** stage follows `hold` down
//! at once, and up through a one-pole low-pass, `y = a_a * y + (1 - a_a) * hold`. With `a =
//! exp(-1 / (tau * sample_rate))` the `attack` and `release` parameters are **time
//! constants** (the time to cover 63% of the way), each exact whatever the other. A sine
//! settles on the reduction of its peak (a single branching smoother on the raw reduction
//! would settle lower, by an amount that depends on the attack/release ratio); the droop of
//! the hold between two peaks is `exp(-half period / release)` of the reduction, which is
//! why a very short release on a low note lets the gain ripple with the wave.
//!
//! The gain applied is `makeup - y` decibels, `y >= 0`: **the output never exceeds the input
//! times the makeup gain**, whatever the signal and the settings. There is no look-ahead: the
//! first milliseconds of a sudden loud sound pass before the attack has closed the gain
//! (bounded by that bound, never beyond).
//!
//! The input is sanitized like the echo's: a non-finite sample is 0, a magnitude beyond 1e6
//! is clamped, a magnitude below 1e-20 is flushed. The smoothing states are flushed below
//! 1e-6 dB, so no denormal ever appears in the exponential decay.
//!
//! [`Device::reset`] zeroes the reduction and snaps the ramps (constant time, no memory);
//! the parameter values are kept.
//!
//! # Parameters and smoothing
//!
//! `threshold`, `ratio` and `makeup` follow the 20 ms linear ramp the descriptor declares;
//! `attack` and `release` are not ramped: their one-pole coefficient is recomputed when they
//! are set, which moves the smoothing time without any discontinuity of the gain. Every
//! value goes through [`ParamDescriptor::sanitize`].

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, ParamDescriptor,
    Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};
use crate::param::Smoother;

/// Ids of the parameters.
pub const THRESHOLD: u32 = 0;
pub const RATIO: u32 = 1;
pub const ATTACK: u32 = 2;
pub const RELEASE: u32 = 3;
pub const MAKEUP: u32 = 4;

/// Width of the soft knee, in decibels, centred on the threshold.
pub const KNEE_DB: f32 = 6.0;
/// Highest makeup gain, in decibels: the bound of the output over the input.
pub const MAKEUP_MAX_DB: f64 = 24.0;

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
/// -18 dB, 4:1, 10 ms attack, 150 ms release, no makeup.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.compressor",
    kind_id: DeviceKind(5),
    version: 1,
    name: "Compressor",
    role: DeviceRole::Effect,
    memory_bytes: size_of::<Compressor>() as u64,
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
            id: ParamId(RELEASE),
            key: "release",
            description: "Time the compressor takes to let the sound back up once it falls under the threshold, from 10 ms to 2 s.",
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
    ],
};

/// The compressor's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a compressor for `setup`, off the audio path.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Compressor::new(setup.sample_rate))?)
}

fn sanitize(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
    }
}

fn flush_db(x: f32) -> f32 {
    if x < STATE_FLOOR_DB { 0.0 } else { x }
}

/// The one-pole coefficient of a time constant `seconds`.
fn coefficient(seconds: f32, sample_rate: f32) -> f32 {
    (-1.0 / (f64::from(seconds.max(1e-5)) * f64::from(sample_rate))).exp() as f32
}

/// The gain reduction, in decibels (>= 0), of a level `x_db` against `threshold_db`, with the
/// soft knee and `slope = 1 - 1/ratio`.
pub fn gain_reduction_db(x_db: f32, threshold_db: f32, slope: f32) -> f32 {
    let over = x_db - threshold_db;
    if 2.0 * over <= -KNEE_DB {
        0.0
    } else if 2.0 * over >= KNEE_DB {
        slope * over
    } else {
        let t = over + 0.5 * KNEE_DB;
        slope * t * t / (2.0 * KNEE_DB)
    }
}

#[derive(Debug)]
pub struct Compressor {
    sample_rate: f32,
    threshold: Smoother,
    ratio: Smoother,
    makeup: Smoother,
    ramp_frames: [u32; 3],
    attack: f32,
    release: f32,
    /// The release stage and the attack stage, in decibels of gain reduction.
    hold: f32,
    reduction: f32,
}

impl Compressor {
    /// A compressor at its defaults.
    pub fn new(sample_rate: f32) -> Self {
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
        Self {
            sample_rate,
            threshold: Smoother::new(default(THRESHOLD) as f32),
            ratio: Smoother::new(default(RATIO) as f32),
            makeup: Smoother::new(default(MAKEUP) as f32),
            ramp_frames: [ramp(THRESHOLD), ramp(RATIO), ramp(MAKEUP)],
            attack: coefficient(default(ATTACK) as f32, sample_rate),
            release: coefficient(default(RELEASE) as f32, sample_rate),
            hold: 0.0,
            reduction: 0.0,
        }
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
            _ => {}
        }
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let threshold = self.threshold.next_value().clamp(-60.0, 0.0);
            let ratio = self.ratio.next_value().clamp(1.0, 20.0);
            let makeup = self.makeup.next_value().clamp(0.0, MAKEUP_MAX_DB as f32);
            // The gain multiplies the sanitized samples: a NaN or an infinity never comes out.
            let (dry_l, dry_r) = (sanitize(*l), sanitize(*r));
            let level = dry_l.abs().max(dry_r.abs()).max(LEVEL_FLOOR);
            let x_db = DB_PER_NEPER * level.ln();
            let wanted = gain_reduction_db(x_db, threshold, 1.0 - 1.0 / ratio);
            self.hold =
                flush_db(wanted.max(self.release * self.hold + (1.0 - self.release) * wanted));
            self.reduction = if self.hold <= self.reduction {
                self.hold
            } else {
                flush_db(self.attack * self.reduction + (1.0 - self.attack) * self.hold)
            };
            // `reduction >= 0`: the gain never exceeds the makeup.
            let gain = (NEPER_PER_DB * (makeup - self.reduction)).exp();
            *l = flush(dry_l * gain);
            *r = flush(dry_r * gain);
        }
    }
}

fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR { 0.0 } else { x }
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

    /// The smoothed reduction now (>= 0, finite).
    fn gain_reduction_db(&self) -> f32 {
        if self.reduction.is_finite() {
            self.reduction.max(0.0)
        } else {
            0.0
        }
    }

    /// Opens the gain (no reduction) and snaps the ramps; the parameter values are kept.
    fn reset(&mut self) {
        self.hold = 0.0;
        self.reduction = 0.0;
        self.threshold.snap();
        self.ratio.snap();
        self.makeup.snap();
    }
}
