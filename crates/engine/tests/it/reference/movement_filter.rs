//! The Filter as it was after #686 and before its movement moved into the shared block
//! (`crates/engine/src/devices/filter.rs` at commit fff08a72 on `develop`), frozen: the
//! reference render the Filter on `engine::dsp::movement` (#688) is compared with, bit for bit,
//! with every movement parameter moving (`tests/it/filter_movement.rs`). Only its doc, its
//! registry line and its factory are cut out, and the imports point at the crate from outside.
//! Do not edit.

#![allow(dead_code)]

use engine::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, Event, ParamDescriptor, Scale, Unit,
    event_frame,
};
use engine::dsp::envelope_follower::EnvelopeFollower;
use engine::dsp::lfo::{self, DIVISION_KEYS, Lfo, SHAPE_KEYS, Shape};
use engine::msg::{DeviceKind, ParamId};
use engine::param::Smoother;
use engine::time::TimeSignature;

/// Ids of the parameters (appended: the ids of DV-2, 0 to 3, never move).
pub const MODE: u32 = 0;
pub const CUTOFF: u32 = 1;
pub const RESONANCE: u32 = 2;
pub const MIX: u32 = 3;
pub const DRIVE: u32 = 4;
pub const LFO_SHAPE: u32 = 5;
pub const LFO_SYNC: u32 = 6;
pub const LFO_RATE: u32 = 7;
pub const LFO_DIVISION: u32 = 8;
pub const LFO_PHASE: u32 = 9;
pub const LFO_STEREO: u32 = 10;
pub const LFO_DEPTH: u32 = 11;
pub const LFO_RESTART: u32 = 12;
pub const ENV_AMOUNT: u32 = 13;
pub const ENV_ATTACK: u32 = 14;
pub const ENV_RELEASE: u32 = 15;

/// The values of `mode`.
pub const LOW_PASS: u32 = 0;
pub const HIGH_PASS: u32 = 1;
pub const BAND_PASS: u32 = 2;
pub const NOTCH: u32 = 3;

/// Lowest and highest cutoff, in hertz.
pub const CUTOFF_MIN: f64 = 20.0;
pub const CUTOFF_MAX: f64 = 20_000.0;
/// The cutoff never goes above this share of the sample rate.
pub const CUTOFF_NYQUIST_SHARE: f32 = 0.45;
/// Damping at no resonance (a flat second-order response) and at full resonance: `1 / K_MIN`
/// is the highest peak gain, so the output is bounded.
pub const K_FLAT: f32 = core::f32::consts::SQRT_2;
pub const K_MIN: f32 = 0.04;
/// Time the four outputs take to crossfade when `mode` changes, in seconds (a choice
/// declares no smoothing: the crossfade is the device's own).
const MODE_SECONDS: f64 = 0.01;
/// The drive's gain into the saturation at full drive (`a` of the module doc).
pub const DRIVE_MAX: f32 = 16.0;
/// A drive at or below this is none: the stage is skipped (its gain `sqrt(1 + a) / a` would
/// overflow on a vanishing drive).
pub const DRIVE_FLOOR: f32 = 1e-6;
/// Fastest move of the LFO's term of the cutoff, in octaves per second.
pub const MOD_SLEW_OCTAVES_PER_SECOND: f32 = 2000.0;
/// Deepest LFO and largest envelope amount, in octaves.
pub const DEPTH_MAX: f64 = 5.0;

/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;

const VALUES_SYNC: &[&str] = &["free", "synced"];
const VALUES_RESTART: &[&str] = &["off", "on"];

/// The descriptor of the filter (CT-5): `faulk.filter`, displayed "Filter" (one English name,
/// never translated). The defaults are the values of [`Filter::new`]: low-pass, 1 kHz, a
/// resonance of 0.2, fully wet, no drive, no movement (LFO depth and envelope amount 0).
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
            description: "What the filter lets through: the sound below the cutoff (low-pass), above it (high-pass), around it (band-pass) or all but it (notch). A change fades over 10 ms.",
            unit: Unit::None,
            min: 0.0,
            max: 3.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["lowpass", "highpass", "bandpass", "notch"],
        },
        ParamDescriptor {
            id: ParamId(CUTOFF),
            key: "cutoff",
            description: "Frequency, in hertz, where the filter starts to cut, from 20 Hz to 20 kHz: the centre the LFO and the envelope move the cutoff around.",
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
        ParamDescriptor {
            id: ParamId(DRIVE),
            key: "drive",
            description: "Soft saturation before the filter, from 0 (none) to 1: louder and rounder, so a strong resonance bites instead of whistling.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(LFO_SHAPE),
            key: "lfo_shape",
            description: "Shape of the LFO that moves the cutoff: sine, triangle, rising saw, falling saw, square, or random (a new value at each cycle).",
            unit: Unit::None,
            min: 0.0,
            max: 5.0,
            default: 1.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: SHAPE_KEYS,
        },
        ParamDescriptor {
            id: ParamId(LFO_SYNC),
            key: "lfo_sync",
            description: "Whether the LFO runs at its own rate in hertz (free) or follows the tempo (synced), each cycle on the same beat of every bar.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 1.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_SYNC,
        },
        ParamDescriptor {
            id: ParamId(LFO_RATE),
            key: "lfo_rate",
            description: "Speed of the LFO in free mode, in cycles per second, from 0.01 to 20 Hz.",
            unit: Unit::Hertz,
            min: 0.01,
            max: 20.0,
            default: 1.0,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(LFO_DIVISION),
            key: "lfo_division",
            description: "Length of one LFO cycle in synced mode, a note value from 1/32 to 8 bars.",
            unit: Unit::None,
            min: 0.0,
            max: 12.0,
            default: 6.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &DIVISION_KEYS,
        },
        ParamDescriptor {
            id: ParamId(LFO_PHASE),
            key: "lfo_phase",
            description: "Where in its cycle the LFO starts, from 0 to 360 degrees: in synced mode, where in the bar the cutoff peaks.",
            unit: Unit::Degrees,
            min: 0.0,
            max: 360.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(LFO_STEREO),
            key: "lfo_stereo",
            description: "Offset of the right channel's LFO, from 0 to 180 degrees: at 180 the two channels open in opposition.",
            unit: Unit::Degrees,
            min: 0.0,
            max: 180.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(LFO_DEPTH),
            key: "lfo_depth",
            description: "How far the LFO moves the cutoff either side of its centre, in octaves, from 0 (no movement) to 5.",
            unit: Unit::Octaves,
            min: 0.0,
            max: DEPTH_MAX,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(LFO_RESTART),
            key: "lfo_restart",
            description: "In free mode, whether the LFO starts again from its phase each time playback starts (on) or runs on freely (off).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 1.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_RESTART,
        },
        ParamDescriptor {
            id: ParamId(ENV_AMOUNT),
            key: "env_amount",
            description: "How far the level of the incoming sound moves the cutoff, in octaves at full level, from -5 (louder closes) to +5 (louder opens); 0 for none.",
            unit: Unit::Octaves,
            min: -DEPTH_MAX,
            max: DEPTH_MAX,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(ENV_ATTACK),
            key: "env_attack",
            description: "Time the envelope follower takes to rise with the sound, from 1 ms to 500 ms.",
            unit: Unit::Seconds,
            min: 0.001,
            max: 0.5,
            default: 0.01,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(ENV_RELEASE),
            key: "env_release",
            description: "Time the envelope follower takes to fall back when the sound gets quieter, from 10 ms to 2 s.",
            unit: Unit::Seconds,
            min: 0.01,
            max: 2.0,
            default: 0.15,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
    ],
};

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

/// A cutoff the filter can use at `sample_rate`: in `[20 Hz, 0.45 * sample_rate]`, 1 kHz for a
/// non-finite one.
pub fn clamp_cutoff(hz: f32, sample_rate: f32) -> f32 {
    let top = CUTOFF_NYQUIST_SHARE * sample_rate;
    if hz.is_finite() {
        hz.clamp(CUTOFF_MIN as f32, top.max(CUTOFF_MIN as f32))
    } else {
        1000.0
    }
}

/// The four weights (low, high, band, notch) of a `mode` value.
fn weights(mode: u32) -> [f32; 4] {
    let mut w = [0.0; 4];
    w[(mode as usize).min(3)] = 1.0;
    w
}

/// The coefficients of one sample.
#[derive(Clone, Copy, Debug)]
pub struct Coefficients {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Coefficients {
    pub fn new(cutoff_hz: f32, resonance: f32, sample_rate: f32) -> Self {
        Self::with_damping(cutoff_hz, damping(resonance), sample_rate)
    }

    fn with_damping(cutoff_hz: f32, k: f32, sample_rate: f32) -> Self {
        let hz = clamp_cutoff(cutoff_hz, sample_rate);
        let g = (core::f32::consts::PI * hz / sample_rate).tan();
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
pub struct Channel {
    ic1: f32,
    ic2: f32,
}

impl Channel {
    /// One sample: `(low, high, band)` outputs of the sanitized input `x`.
    #[inline]
    pub fn tick(&mut self, x: f32, c: &Coefficients) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        (v2, x - c.k * v1 - v2, c.k * v1)
    }
}

/// The drive stage of one sample, `gain = sqrt(1 + a) / a` (module doc).
#[inline]
fn drive(x: f32, a: f32, gain: f32) -> f32 {
    (a * x).tanh() * gain
}

/// The settings of the LFO that are not ramped.
#[derive(Clone, Copy, Debug)]
struct LfoSettings {
    shape: Shape,
    synced: bool,
    rate_hz: f32,
    division: u32,
    /// `lfo_phase` and `lfo_stereo`, in cycles.
    phase: f64,
    stereo: f64,
    restart: bool,
}

#[derive(Debug)]
pub struct Filter {
    sample_rate: f32,
    /// The cutoff in hertz (the centre of the movement).
    cutoff: Smoother,
    resonance: Smoother,
    mix: Smoother,
    drive: Smoother,
    depth: Smoother,
    amount: Smoother,
    /// The weights of the low-pass, the high-pass, the band-pass and the notch outputs.
    weight: [Smoother; 4],
    mode_frames: u32,
    ramp_frames: u32,
    /// The coefficients of the left and the right channel (the same without movement).
    coefficients: [Coefficients; 2],
    /// Whether the coefficients must be recomputed once more (a ramp ran, a value was set or
    /// the cutoff moved).
    stale: bool,
    left: Channel,
    right: Channel,
    lfo: Lfo,
    settings: LfoSettings,
    /// The time signature of the block (a division in bars).
    signature: TimeSignature,
    env: EnvelopeFollower,
    /// The LFO's term of each channel after the slew limit, in octaves.
    swing: [f32; 2],
    /// The limiter takes the LFO's value at once on the next frame (module doc).
    snap_swing: bool,
    /// The resonance the cached damping was computed for, and that damping.
    damping_of: f32,
    k: f32,
    /// The cutoff each channel used on the last frame, in hertz.
    effective: [f32; 2],
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
        let ramp_frames = DESCRIPTOR
            .param(ParamId(CUTOFF))
            .map_or(0, |d| d.smoothing_frames(sample_rate));
        let mode_frames = (MODE_SECONDS * f64::from(sample_rate)).round() as u32;
        let w = weights(default(MODE) as u32);
        let cutoff = default(CUTOFF) as f32;
        let resonance = default(RESONANCE) as f32;
        let coefficients = Coefficients::new(cutoff, resonance, sample_rate);
        Self {
            sample_rate,
            cutoff: Smoother::new(cutoff),
            resonance: Smoother::new(resonance),
            mix: Smoother::new(default(MIX) as f32),
            drive: Smoother::new(default(DRIVE) as f32),
            depth: Smoother::new(default(LFO_DEPTH) as f32),
            amount: Smoother::new(default(ENV_AMOUNT) as f32),
            weight: w.map(Smoother::new),
            mode_frames,
            ramp_frames,
            coefficients: [coefficients; 2],
            stale: false,
            left: Channel::default(),
            right: Channel::default(),
            lfo: Lfo::new(),
            settings: LfoSettings {
                shape: Shape::from_index(default(LFO_SHAPE) as u32),
                synced: default(LFO_SYNC) >= 0.5,
                rate_hz: default(LFO_RATE) as f32,
                division: default(LFO_DIVISION) as u32,
                phase: default(LFO_PHASE) / 360.0,
                stereo: default(LFO_STEREO) / 360.0,
                restart: default(LFO_RESTART) >= 0.5,
            },
            signature: TimeSignature::COMMON,
            env: EnvelopeFollower::new(
                default(ENV_ATTACK) as f32,
                default(ENV_RELEASE) as f32,
                sample_rate,
            ),
            swing: [0.0; 2],
            snap_swing: true,
            damping_of: resonance,
            k: damping(resonance),
            effective: [clamp_cutoff(cutoff, sample_rate); 2],
        }
    }

    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        let ramp = self.ramp_frames;
        let s = &mut self.settings;
        match id.0 {
            MODE => {
                let w = weights(value.round() as u32);
                for (smoother, target) in self.weight.iter_mut().zip(w) {
                    smoother.set(target, self.mode_frames);
                }
            }
            CUTOFF => {
                self.cutoff.set(value, ramp);
                self.stale = true;
            }
            RESONANCE => {
                self.resonance.set(value, ramp);
                self.stale = true;
            }
            MIX => self.mix.set(value, ramp),
            DRIVE => self.drive.set(value, ramp),
            LFO_SHAPE => s.shape = Shape::from_index(value.round() as u32),
            LFO_SYNC => s.synced = value >= 0.5,
            LFO_RATE => s.rate_hz = value,
            LFO_DIVISION => s.division = value.round() as u32,
            LFO_PHASE => s.phase = f64::from(value) / 360.0,
            LFO_STEREO => s.stereo = f64::from(value) / 360.0,
            LFO_DEPTH => self.depth.set(value, ramp),
            LFO_RESTART => s.restart = value >= 0.5,
            ENV_AMOUNT => self.amount.set(value, ramp),
            ENV_ATTACK => self.env.set_attack(value, self.sample_rate),
            ENV_RELEASE => self.env.set_release(value, self.sample_rate),
            _ => {}
        }
    }

    /// The cutoff the next frame uses as its centre, in hertz (the smoother's value, ramp
    /// included).
    pub fn cutoff_hz(&self) -> f32 {
        self.cutoff.current()
    }

    /// The cutoff the left and the right channel used on the last frame, in hertz: the centre
    /// moved by the LFO and the envelope, clamped.
    pub fn effective_cutoffs(&self) -> [f32; 2] {
        self.effective
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let sr = self.sample_rate;
        let slew = MOD_SLEW_OCTAVES_PER_SECOND / sr;
        let s = self.settings;
        let period = lfo::division_ticks(s.division, self.signature);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let moving = !self.cutoff.is_still() || !self.resonance.is_still();
            let cutoff = self.cutoff.next_value();
            let resonance = self.resonance.next_value();
            let mix = self.mix.next_value().clamp(0.0, 1.0);
            let [wl, wh, wb, wn] = self.weight.each_mut().map(Smoother::next_value);
            let drive_amount = self.drive.next_value();
            let depth = self.depth.next_value();
            let amount = self.amount.next_value();
            let (dry_l, dry_r) = (sanitize(*l), sanitize(*r));

            // The movement, in octaves.
            let env = self.env.next(dry_l.abs().max(dry_r.abs()));
            let target = if depth > 0.0 {
                let base = self.lfo.phase(s.synced, period) + s.phase;
                [
                    lfo::value(s.shape, base) * depth,
                    lfo::value(s.shape, base + s.stereo) * depth,
                ]
            } else {
                [0.0; 2]
            };
            if self.snap_swing {
                self.swing = target;
                self.snap_swing = false;
            } else {
                for (swing, t) in self.swing.iter_mut().zip(target) {
                    *swing += (t - *swing).clamp(-slew, slew);
                }
            }
            self.lfo.advance(s.rate_hz, sr);
            let e = env * amount;
            let m = [self.swing[0] + e, self.swing[1] + e];

            if m[0] != 0.0 || m[1] != 0.0 {
                if resonance != self.damping_of {
                    self.damping_of = resonance;
                    self.k = damping(resonance);
                }
                let hz_l = clamp_cutoff(cutoff * m[0].exp2(), sr);
                let c_l = Coefficients::with_damping(hz_l, self.k, sr);
                let (hz_r, c_r) = if m[1] == m[0] {
                    (hz_l, c_l)
                } else {
                    let hz = clamp_cutoff(cutoff * m[1].exp2(), sr);
                    (hz, Coefficients::with_damping(hz, self.k, sr))
                };
                self.coefficients = [c_l, c_r];
                self.effective = [hz_l, hz_r];
                self.stale = true;
            } else if moving || self.stale {
                let c = Coefficients::new(cutoff, resonance, sr);
                self.coefficients = [c; 2];
                self.effective = [clamp_cutoff(cutoff, sr); 2];
                self.stale = moving;
            }

            let (in_l, in_r) = if drive_amount > DRIVE_FLOOR {
                let a = DRIVE_MAX * drive_amount;
                let gain = (1.0 + a).sqrt() / a;
                (drive(dry_l, a, gain), drive(dry_r, a, gain))
            } else {
                (dry_l, dry_r)
            };
            let [c_l, c_r] = self.coefficients;
            let (ll, lh, lb) = self.left.tick(in_l, &c_l);
            let (rl, rh, rb) = self.right.tick(in_r, &c_r);
            let mut wet_l = wl * ll + wh * lh + wb * lb;
            let mut wet_r = wl * rl + wh * rh + wb * rb;
            if wn != 0.0 {
                wet_l += wn * (ll + lh);
                wet_r += wn * (rl + rh);
            }
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
        let s = self.settings;
        if self
            .lfo
            .begin(&ctx.transport, self.sample_rate, s.restart && !s.synced)
        {
            self.snap_swing = true;
        }
        self.signature = ctx.transport.signature;
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

    /// Empties the filter, sets the LFO and the envelope back to their start and snaps the
    /// ramps; the parameter values are kept.
    fn reset(&mut self) {
        self.left = Channel::default();
        self.right = Channel::default();
        self.cutoff.snap();
        self.resonance.snap();
        self.mix.snap();
        self.drive.snap();
        self.depth.snap();
        self.amount.snap();
        for w in &mut self.weight {
            w.snap();
        }
        self.lfo.reset();
        self.env.reset();
        self.swing = [0.0; 2];
        self.snap_swing = true;
        let c = Coefficients::new(
            self.cutoff.current(),
            self.resonance.current(),
            self.sample_rate,
        );
        self.coefficients = [c; 2];
        self.effective = [clamp_cutoff(self.cutoff.current(), self.sample_rate); 2];
        self.stale = false;
    }

    /// The cutoff the left channel used on the last frame, in hertz.
    fn readout(&self) -> Option<(ParamId, f32)> {
        Some((ParamId(CUTOFF), self.effective[0]))
    }
}
