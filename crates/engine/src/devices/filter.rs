//! The filter, `faulk.filter`, displayed "Filter" (DV-2): a multimode state-variable filter
//! (low-pass, high-pass, band-pass at 12 dB per octave) with a cutoff, a resonance and a
//! dry/wet mix. A [`Device`] of role effect, the same filter on both channels.
//!
//! # Signal
//!
//! A zero-delay-feedback (topology-preserving) state-variable filter: with `g = tan(pi *
//! cutoff / sample_rate)` and the damping `k`, each channel keeps two integrator states and
//! one sample runs
//!
//! ```text
//! v3 = x - ic2;  v1 = a1 * ic1 + a2 * v3;  v2 = ic2 + a2 * ic1 + a3 * v3
//! ic1 = 2 v1 - ic1;  ic2 = 2 v2 - ic2
//! low-pass = v2;  band-pass = k * v1;  high-pass = x - k * v1 - v2
//! ```
//!
//! with `a1 = 1 / (1 + g (g + k))`, `a2 = g a1`, `a3 = g a2`. The transfer functions, with
//! `w = f / cutoff`: low-pass `1 / (1 - w^2 + j k w)`, high-pass `-w^2 / (1 - w^2 + j k w)`,
//! band-pass `j k w / (1 - w^2 + j k w)`. The band-pass is **normalised to unity gain at the
//! cutoff** (the raw band output peaks at `1 / k`). Low-pass and high-pass fall at 12 dB per
//! octave away from the cutoff; the band-pass falls at 6 dB per octave on each side of it
//! (its two skirts are 12 dB per octave of the pair).
//!
//! # Resonance and stability
//!
//! `resonance` (0 to 1) sets the damping on an exponential law between `k = sqrt(2)` (0: no
//! peak, a maximally flat low-pass, -3 dB at the cutoff, about -12.3 dB one octave away) and
//! `k = K_MIN` (1: a peak of about +28 dB at the cutoff). `K_MIN` is above zero on purpose:
//! the filter never reaches self-oscillation, its impulse response always decays, and the
//! output is bounded by the peak gain `1 / K_MIN` times the (clamped) input. The structure is
//! stable for any `g > 0` and `k > 0`, also while they move. The cutoff is clamped to
//! `[20 Hz, 0.45 * sample_rate]`, so it stays under Nyquist at every sample rate.
//!
//! The input is sanitized like the echo's: a non-finite sample is 0, a magnitude beyond 1e6
//! is clamped, a magnitude below 1e-20 is flushed. The integrator states are flushed below
//! 1e-20 on every sample, so no denormal ever appears in the decay.
//!
//! # Parameters and smoothing
//!
//! `cutoff` is smoothed in **octaves** (the ramp runs on `log2(cutoff)`, so a sweep is
//! even to the ear), `resonance` and `mix` linearly, each over the 20 ms the descriptor
//! declares; a change of `mode` (a choice, which declares no smoothing) crossfades the three
//! outputs over 10 ms, so it does not click. The coefficients are recomputed per sample while a ramp runs and kept when it does
//! not, so the render does not depend on how the host cuts blocks. [`Device::reset`] zeroes
//! the states and snaps the ramps; the parameter values are kept. Every value goes through
//! [`ParamDescriptor::sanitize`].

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, ParamDescriptor,
    Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};
use crate::param::Smoother;

/// Ids of the parameters.
pub const MODE: u32 = 0;
pub const CUTOFF: u32 = 1;
pub const RESONANCE: u32 = 2;
pub const MIX: u32 = 3;

/// The values of `mode`.
pub const LOW_PASS: u32 = 0;
pub const HIGH_PASS: u32 = 1;
pub const BAND_PASS: u32 = 2;

/// Lowest and highest cutoff, in hertz.
pub const CUTOFF_MIN: f64 = 20.0;
pub const CUTOFF_MAX: f64 = 20_000.0;
/// The cutoff never goes above this share of the sample rate.
const CUTOFF_NYQUIST_SHARE: f32 = 0.45;
/// Damping at no resonance (a flat second-order response) and at full resonance: `1 / K_MIN`
/// is the highest peak gain, so the output is bounded.
pub const K_FLAT: f32 = core::f32::consts::SQRT_2;
pub const K_MIN: f32 = 0.04;
/// Time the three outputs take to crossfade when `mode` changes, in seconds (a choice
/// declares no smoothing: the crossfade is the device's own).
const MODE_SECONDS: f64 = 0.01;

/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;

/// The descriptor of the filter (CT-5): `faulk.filter`, displayed "Filter" (one English name,
/// never translated). The defaults are the values of [`Filter::new`]: low-pass, 1 kHz, a
/// resonance of 0.2, fully wet.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.filter",
    kind_id: DeviceKind(6),
    version: 1,
    name: "Filter",
    role: DeviceRole::Effect,
    memory_bytes: size_of::<Filter>() as u64,
    midi_output: None,
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(MODE),
            key: "mode",
            description: "What the filter lets through: the sound below the cutoff (low-pass), above it (high-pass) or around it (band-pass).",
            unit: Unit::None,
            min: 0.0,
            max: 2.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["lowpass", "highpass", "bandpass"],
        },
        ParamDescriptor {
            id: ParamId(CUTOFF),
            key: "cutoff",
            description: "Frequency, in hertz, where the filter starts to cut, from 20 Hz to 20 kHz.",
            unit: Unit::Hertz,
            min: CUTOFF_MIN,
            max: CUTOFF_MAX,
            default: 1000.0,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(RESONANCE),
            key: "resonance",
            description: "Boost of the sound around the cutoff, from 0 (none) to 1 (a sharp peak that stays stable).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.2,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(MIX),
            key: "mix",
            description: "Balance between the dry sound (0) and the filtered sound (1).",
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

/// The filter's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a filter for `setup`, off the audio path.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Filter::new(setup.sample_rate))?)
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

/// The damping of a `resonance` from 0 to 1: exponential between [`K_FLAT`] and [`K_MIN`].
pub fn damping(resonance: f32) -> f32 {
    let r = if resonance.is_finite() {
        resonance.clamp(0.0, 1.0)
    } else {
        0.0
    };
    K_FLAT * (K_MIN / K_FLAT).powf(r)
}

/// The three weights (low, high, band) of a `mode` value.
fn weights(mode: u32) -> [f32; 3] {
    let mut w = [0.0; 3];
    w[(mode as usize).min(2)] = 1.0;
    w
}

/// The coefficients of one sample.
#[derive(Clone, Copy, Debug)]
struct Coefficients {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Coefficients {
    fn new(cutoff_hz: f32, resonance: f32, sample_rate: f32) -> Self {
        let top = CUTOFF_NYQUIST_SHARE * sample_rate;
        let hz = if cutoff_hz.is_finite() {
            cutoff_hz.clamp(CUTOFF_MIN as f32, top.max(CUTOFF_MIN as f32))
        } else {
            1000.0
        };
        let g = (core::f32::consts::PI * hz / sample_rate).tan();
        let k = damping(resonance);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self {
            a1,
            a2,
            a3: g * a2,
            k,
        }
    }
}

/// The two integrator states of one channel.
#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    ic1: f32,
    ic2: f32,
}

impl Channel {
    /// One sample: `(low, high, band)` outputs of the sanitized input `x`.
    #[inline]
    fn tick(&mut self, x: f32, c: &Coefficients) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        (v2, x - c.k * v1 - v2, c.k * v1)
    }
}

#[derive(Debug)]
pub struct Filter {
    sample_rate: f32,
    /// `log2` of the cutoff in hertz.
    cutoff: Smoother,
    resonance: Smoother,
    mix: Smoother,
    /// The weights of the low-pass, the high-pass and the band-pass outputs.
    weight: [Smoother; 3],
    ramp_frames: [u32; 4],
    coefficients: Coefficients,
    /// Whether the coefficients must be recomputed once more (a ramp ran or a value was set).
    stale: bool,
    left: Channel,
    right: Channel,
}

impl Filter {
    /// A filter at its defaults.
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
        let mode_frames = (MODE_SECONDS * f64::from(sample_rate)).round() as u32;
        let w = weights(default(MODE) as u32);
        let cutoff = (default(CUTOFF) as f32).log2();
        let resonance = default(RESONANCE) as f32;
        Self {
            sample_rate,
            cutoff: Smoother::new(cutoff),
            resonance: Smoother::new(resonance),
            mix: Smoother::new(default(MIX) as f32),
            weight: [
                Smoother::new(w[0]),
                Smoother::new(w[1]),
                Smoother::new(w[2]),
            ],
            ramp_frames: [mode_frames, ramp(CUTOFF), ramp(RESONANCE), ramp(MIX)],
            coefficients: Coefficients::new(cutoff.exp2(), resonance, sample_rate),
            stale: false,
            left: Channel::default(),
            right: Channel::default(),
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
            MODE => {
                let w = weights(value.round() as u32);
                for (smoother, target) in self.weight.iter_mut().zip(w) {
                    smoother.set(target, self.ramp_frames[0]);
                }
            }
            CUTOFF => {
                self.cutoff.set(value.log2(), self.ramp_frames[1]);
                self.stale = true;
            }
            RESONANCE => {
                self.resonance.set(value, self.ramp_frames[2]);
                self.stale = true;
            }
            MIX => self.mix.set(value, self.ramp_frames[3]),
            _ => {}
        }
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let moving = !self.cutoff.is_still() || !self.resonance.is_still();
            let cutoff = self.cutoff.next_value();
            let resonance = self.resonance.next_value();
            if moving || self.stale {
                self.coefficients = Coefficients::new(cutoff.exp2(), resonance, self.sample_rate);
                self.stale = moving;
            }
            let mix = self.mix.next_value().clamp(0.0, 1.0);
            let [wl, wh, wb] = self.weight.each_mut().map(Smoother::next_value);
            let (dry_l, dry_r) = (sanitize(*l), sanitize(*r));
            let c = self.coefficients;
            let (ll, lh, lb) = self.left.tick(dry_l, &c);
            let (rl, rh, rb) = self.right.tick(dry_r, &c);
            let wet_l = wl * ll + wh * lh + wb * lb;
            let wet_r = wl * rl + wh * rh + wb * rb;
            *l = flush(dry_l + mix * (wet_l - dry_l));
            *r = flush(dry_r + mix * (wet_r - dry_r));
        }
    }
}

impl Device for Filter {
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

    /// Empties the filter and snaps the ramps; the parameter values are kept.
    fn reset(&mut self) {
        self.left = Channel::default();
        self.right = Channel::default();
        self.cutoff.snap();
        self.resonance.snap();
        self.mix.snap();
        for w in &mut self.weight {
            w.snap();
        }
        self.coefficients = Coefficients::new(
            self.cutoff.current().exp2(),
            self.resonance.current(),
            self.sample_rate,
        );
        self.stale = false;
    }
}
