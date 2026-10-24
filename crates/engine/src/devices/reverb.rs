//! The reverb, `faulk.reverb`, displayed "Reverb" (DV-4): an algorithmic room with a size, a
//! damping, a pre-delay and a mix. A [`Device`] of role effect.
//!
//! # Signal
//!
//! The classic Schroeder/Moorer network, in stereo. The two channels are summed to a mono
//! input, delayed by the pre-delay, and fed to two banks of [`COMBS`] parallel feedback comb
//! filters (one bank per channel, the right one [`SPREAD`] frames longer per comb, which
//! decorrelates the two outputs into a wide image). Each comb has a one-pole low-pass in its
//! feedback path (the damping: high frequencies die faster than low ones, like the air and the
//! walls of a room). The bank's sum goes through [`ALLPASSES`] series all-pass filters that
//! thicken the echo density. The output is `dry * (1 - mix) + wet * mix`; the dry signal is
//! never touched.
//!
//! # Size and decay
//!
//! `size` (0 to 1) sets the reverberation time `RT60 = 0.3 s * 25^size`: 0.3 s (a small room)
//! at 0, 1.5 s at the default 0.5, 7.5 s (a hall) at 1. It acts on the feedback gain of every
//! comb, `g = 10^(-3 * delay / RT60)`, so each comb loses 60 dB in exactly `RT60` at low
//! frequencies; the lengths of the lines are fixed (a length that moved would sweep the pitch
//! of the tail). The damping shortens the tail of the high frequencies only. The gain of every
//! comb stays below 1 (at most 0.977: the shortest comb, 25 ms, at 7.5 s), so the tail always decays. The input
//! of a comb is scaled by `sqrt(1 - g^2)`: the energy of an impulse response is the same at
//! every size (a longer room is not louder for a drum hit), and white noise comes out at the
//! level it went in.
//!
//! The gains are recomputed every [`CONTROL_PERIOD`] frames while `size` changes (the
//! `exp2` calls stay off the per-frame loop); the counter lives in the device, so the result
//! does not depend on how the host cuts the blocks.
//!
//! The mono input goes through a DC blocker (a high-pass below 10 Hz) so that a constant
//! offset does not pile up in the combs.
//!
//! The input is sanitized before it enters the network: a non-finite sample is 0, a
//! magnitude beyond 1e6 is clamped and a magnitude below 1e-20 is flushed to 0. Every state
//! the network keeps is flushed the same way, so no denormal ever circulates in a loop.
//!
//! # Memory
//!
//! Every line is allocated in the factory ([`Reverb::new`]), for the sample rate of the setup
//! (at most [`MAX_SAMPLE_RATE`]); `process` allocates nothing.
//!
//! [`Device::reset`] takes constant time: the lines are not wiped, `age`, the frames
//! processed since the reset, hides what they hold (a line read before `age` reaches its
//! length returns 0).
//!
//! # Parameters and smoothing
//!
//! All four parameters are continuous and follow the linear ramp their descriptor declares
//! (`mix` 20 ms, `size` and `damping` 50 ms), except `predelay`: like the Echo's delay, the pre-delay in
//! use glides toward its target by at most [`PREDELAY_SLEW`] frame per frame, so the pitch of the
//! reverb bends by 25% at most while it moves, like a tape, and never jumps (a ramp over a fixed
//! time would read the line several times faster than real time on a large move). The poles of the
//! DC blocker and of the damping are scaled to the sample rate, so their cutoffs do not depend on it. Every value goes through
//! [`ParamDescriptor::sanitize`].

use super::Builtin;
use super::osc_synth::LinearRamp;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MAX_SAMPLE_RATE,
    ParamDescriptor, Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

/// Ids of the parameters.
pub const SIZE: u32 = 0;
pub const DAMPING: u32 = 1;
pub const PREDELAY: u32 = 2;
pub const MIX: u32 = 3;

/// Parallel comb filters per channel, and series all-pass filters per channel.
pub const COMBS: usize = 8;
pub const ALLPASSES: usize = 4;
/// Shortest and longest reverberation time, in seconds (`size` 0 and 1).
pub const RT60_MIN: f32 = 0.3;
pub const RT60_MAX: f32 = 7.5;
/// Longest pre-delay, in seconds.
pub const MAX_PREDELAY_SECONDS: f32 = 0.2;

/// Lengths of the lines at 44.1 kHz, in frames (scaled to the sample rate of the setup).
const COMB_LENGTHS: [usize; COMBS] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_LENGTHS: [usize; ALLPASSES] = [556, 441, 341, 225];
/// Frames added to every line of the right channel (at 44.1 kHz).
const SPREAD: usize = 23;
const REFERENCE_RATE: f32 = 44_100.0;
/// Feedback of the all-pass filters.
const ALLPASS_FEEDBACK: f32 = 0.5;
/// Gain of the sum of the combs: `1 / sqrt(COMBS)`, so uncorrelated combs add up to the level
/// of one.
const WET_GAIN: f32 = 0.353_553_4;
/// Largest pole of the damping filter (at `damping` 1) at [`RATE_REFERENCE`].
const DAMPING_POLE_MAX: f32 = 0.9;
/// The sample rate the per-sample constants above are tuned for.
const RATE_REFERENCE: f32 = 48_000.0;
/// Fastest the pre-delay moves, in frames per frame.
const PREDELAY_SLEW: f32 = 0.25;
/// Cutoff of the DC blocker of the input, in hertz.
const DC_HZ: f32 = 8.0;
/// Frames between two recomputations of the comb gains.
const CONTROL_PERIOD: u32 = 16;
/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;
/// Frames kept past the longest pre-delay, for the interpolation.
const GUARD_FRAMES: usize = 4;

/// Frames of a line of `length` at 44.1 kHz, for the sample rate `rate`.
const fn scaled(length: usize, rate: f64) -> usize {
    let frames = length as f64 * rate / REFERENCE_RATE as f64;
    let whole = frames as usize;
    let whole = if (whole as f64) < frames {
        whole + 1
    } else {
        whole
    };
    if whole < 2 { 2 } else { whole }
}

/// Bytes of every line at [`MAX_SAMPLE_RATE`] (an upper bound: `new` rounds to nearest).
const LINES_BYTES: u64 = {
    let rate = MAX_SAMPLE_RATE as f64;
    let mut frames = 0usize;
    let mut i = 0;
    while i < COMBS {
        frames += scaled(COMB_LENGTHS[i], rate) + scaled(COMB_LENGTHS[i] + SPREAD, rate);
        i += 1;
    }
    let mut i = 0;
    while i < ALLPASSES {
        frames += scaled(ALLPASS_LENGTHS[i], rate) + scaled(ALLPASS_LENGTHS[i] + SPREAD, rate);
        i += 1;
    }
    frames += (MAX_PREDELAY_SECONDS as f64 * rate) as usize + GUARD_FRAMES + 2;
    frames as u64 * size_of::<f32>() as u64
};

/// The descriptor of the reverb (CT-5): `faulk.reverb`, displayed "Reverb" (one English name,
/// never translated). The defaults are the values of [`Reverb::new`]: size 0.5 (1.5 s),
/// damping 0.5, 10 ms of pre-delay, 25% mix.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.reverb",
    kind_id: DeviceKind(4),
    version: 1,
    name: "Reverb",
    role: DeviceRole::Effect,
    memory_bytes: size_of::<Reverb>() as u64 + LINES_BYTES,
    midi_output: None,
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(SIZE),
            key: "size",
            description: "Size of the room, from 0 to 1: a small room (about 0.3 seconds of reverberation) up to a hall (about 7.5 seconds).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.5,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.05,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(DAMPING),
            key: "damping",
            description: "How fast the high frequencies die in the tail, from 0 to 1: bright and open at 0, dark and soft at 1.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.5,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.05,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(PREDELAY),
            key: "predelay",
            description: "Time between the dry sound and the start of the reverb, from 0 to 0.2 seconds: more room to hear the sound before its tail.",
            unit: Unit::Seconds,
            min: 0.0,
            max: MAX_PREDELAY_SECONDS as f64,
            default: 0.01,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(MIX),
            key: "mix",
            description: "Balance between the dry sound (0) and the reverb (1).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.25,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
    ],
};

/// The reverb's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a reverb for `setup`, off the audio path: every line is allocated here, with a
/// fallible allocation.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Reverb::new(setup.sample_rate)?)?)
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

/// A circular line of `buf.len()` frames: the sample at `pos` is the oldest one.
#[derive(Debug)]
struct Line {
    buf: Box<[f32]>,
    pos: usize,
}

impl Line {
    /// A line of `frames` zeroed samples, or a typed error when the memory is refused.
    fn new(frames: usize) -> Result<Self, MsgError> {
        let mut v = Vec::new();
        v.try_reserve_exact(frames)
            .map_err(|_| MsgError::OutOfMemory {
                bytes: frames.saturating_mul(size_of::<f32>()),
            })?;
        v.resize(frames, 0.0);
        Ok(Self {
            buf: v.into_boxed_slice(),
            pos: 0,
        })
    }

    /// The oldest sample, or 0 while it predates the last reset (`age` frames processed).
    fn oldest(&self, age: u32) -> f32 {
        if age as usize >= self.buf.len() {
            self.buf[self.pos]
        } else {
            0.0
        }
    }

    /// Replaces the oldest sample by `x` and moves on.
    fn push(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
    }
}

/// A comb filter with a damped feedback.
#[derive(Debug)]
struct Comb {
    line: Line,
    /// State of the damping low-pass.
    lowpass: f32,
    /// Feedback gain and input gain, from `size`.
    gain: f32,
    input: f32,
}

/// One channel of the network.
#[derive(Debug)]
struct Channel {
    combs: [Comb; COMBS],
    allpasses: [Line; ALLPASSES],
}

impl Channel {
    fn new(rate: f32, extra: usize) -> Result<Self, MsgError> {
        let rate = f64::from(rate.min(MAX_SAMPLE_RATE));
        let length = |base: usize| {
            let frames = ((base + extra) as f64 * rate / f64::from(REFERENCE_RATE)).round();
            (frames as usize).max(2)
        };
        let comb = |i: usize| -> Result<Comb, MsgError> {
            Ok(Comb {
                line: Line::new(length(COMB_LENGTHS[i]))?,
                lowpass: 0.0,
                gain: 0.0,
                input: 0.0,
            })
        };
        // Built in order: the first failure is returned, what was built is freed.
        let combs = [
            comb(0)?,
            comb(1)?,
            comb(2)?,
            comb(3)?,
            comb(4)?,
            comb(5)?,
            comb(6)?,
            comb(7)?,
        ];
        let allpasses = [
            Line::new(length(ALLPASS_LENGTHS[0]))?,
            Line::new(length(ALLPASS_LENGTHS[1]))?,
            Line::new(length(ALLPASS_LENGTHS[2]))?,
            Line::new(length(ALLPASS_LENGTHS[3]))?,
        ];
        Ok(Self { combs, allpasses })
    }

    /// Sets the feedback of every comb for a reverberation time of `rt60` seconds.
    fn set_decay(&mut self, rate: f32, rt60: f32) {
        // 10^(-3 t / rt60) = 2^(-3 log2(10) t / rt60)
        const K: f32 = -3.0 * core::f32::consts::LOG2_10;
        for comb in &mut self.combs {
            let t = comb.line.buf.len() as f32 / rate;
            let gain = (K * t / rt60).exp2().clamp(0.0, 0.999);
            comb.gain = gain;
            comb.input = (1.0 - gain * gain).max(0.0).sqrt();
        }
    }

    fn clear_filters(&mut self) {
        for comb in &mut self.combs {
            comb.lowpass = 0.0;
        }
    }

    /// One frame of the network for the mono input `x`.
    fn tick(&mut self, x: f32, pole: f32, age: u32) -> f32 {
        let mut sum = 0.0;
        for comb in &mut self.combs {
            let out = comb.line.oldest(age);
            sum += out;
            comb.lowpass = flush(out + (comb.lowpass - out) * pole);
            comb.line
                .push(flush(x * comb.input + comb.lowpass * comb.gain));
        }
        let mut y = sum * WET_GAIN;
        for line in &mut self.allpasses {
            let buffered = line.oldest(age);
            line.push(flush(y + buffered * ALLPASS_FEEDBACK));
            y = buffered * (1.0 - ALLPASS_FEEDBACK * ALLPASS_FEEDBACK) - y * ALLPASS_FEEDBACK;
        }
        flush(y)
    }
}

#[derive(Debug)]
pub struct Reverb {
    sample_rate: f32,
    left: Channel,
    right: Channel,
    predelay: Line,
    size: LinearRamp,
    damping: LinearRamp,
    /// The pre-delay in use and the one it glides toward, in frames.
    predelay_delay: f32,
    predelay_target: f32,
    /// The DC blocker's pole, scaled to the sample rate.
    dc_pole: f32,
    /// The damping filter's pole and the `damping` it was computed for. The pole is raised to
    /// `RATE_REFERENCE / sample_rate`: the cutoff stays where it is at 48 kHz.
    pole: f32,
    pole_for: f32,
    mix: LinearRamp,
    ramp_frames: [u32; 3],
    /// The `size` the comb gains were computed for.
    decay_size: f32,
    /// Frames processed, to space the gain updates (wraps; only its remainder matters).
    clock: u32,
    /// State of the DC blocker of the input.
    dc_in: f32,
    dc_out: f32,
    /// Frames processed since the build or the last reset, saturating: a line read before
    /// `age` reaches its length holds nothing of the present.
    age: u32,
}

impl Reverb {
    /// A reverb at its defaults, with its lines allocated for `sample_rate`.
    pub fn new(sample_rate: f32) -> Result<Self, MsgError> {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        let default = |id: u32| DESCRIPTOR.param(ParamId(id)).map_or(0.0, |d| d.default) as f32;
        let ramp = |id: u32| {
            DESCRIPTOR
                .param(ParamId(id))
                .map_or(0, |d| d.smoothing_frames(sample_rate))
        };
        let predelay_frames = (MAX_PREDELAY_SECONDS * sample_rate.min(MAX_SAMPLE_RATE)).ceil()
            as usize
            + GUARD_FRAMES;
        let mut reverb = Self {
            sample_rate,
            left: Channel::new(sample_rate, 0)?,
            right: Channel::new(sample_rate, SPREAD)?,
            predelay: Line::new(predelay_frames)?,
            size: LinearRamp::new(default(SIZE)),
            damping: LinearRamp::new(default(DAMPING)),
            predelay_delay: default(PREDELAY) * sample_rate,
            predelay_target: default(PREDELAY) * sample_rate,
            dc_pole: 1.0 - core::f32::consts::TAU * DC_HZ / sample_rate,
            pole: 0.0,
            pole_for: -1.0,
            mix: LinearRamp::new(default(MIX)),
            ramp_frames: [ramp(SIZE), ramp(DAMPING), ramp(MIX)],
            decay_size: default(SIZE),
            clock: 0,
            dc_in: 0.0,
            dc_out: 0.0,
            age: 0,
        };
        reverb.apply_decay(reverb.decay_size);
        Ok(reverb)
    }

    /// The mono input without its constant component: a steady offset would pile up in the
    /// feedback combs (their gain at 0 Hz is up to 10) and come out as a loud offset.
    fn block_dc(&mut self, x: f32) -> f32 {
        let y = flush(x - self.dc_in + self.dc_pole * self.dc_out);
        self.dc_in = x;
        self.dc_out = y;
        y
    }

    /// Computes the damping filter's pole for `damping` (0 to 1).
    fn set_pole(&mut self, damping: f32) {
        let at_reference = damping * DAMPING_POLE_MAX;
        self.pole = if at_reference > 0.0 {
            at_reference
                .powf(RATE_REFERENCE / self.sample_rate)
                .min(0.999)
        } else {
            0.0
        };
        self.pole_for = damping;
    }

    /// Computes the comb gains for `size`.
    fn apply_decay(&mut self, size: f32) {
        let rt60 = RT60_MIN * (size.clamp(0.0, 1.0) * (RT60_MAX / RT60_MIN).log2()).exp2();
        self.left.set_decay(self.sample_rate, rt60);
        self.right.set_decay(self.sample_rate, rt60);
        self.decay_size = size;
    }

    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        match id.0 {
            SIZE => self.size.set(value, self.ramp_frames[0]),
            DAMPING => self.damping.set(value, self.ramp_frames[1]),
            PREDELAY => self.predelay_target = value * self.sample_rate,
            MIX => self.mix.set(value, self.ramp_frames[2]),
            _ => {}
        }
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let len = self.predelay.buf.len();
        let max_delay = (len - GUARD_FRAMES) as f32;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let size = self.size.next_value();
            if self.clock.is_multiple_of(CONTROL_PERIOD) && size != self.decay_size {
                self.apply_decay(size);
            }
            self.clock = self.clock.wrapping_add(1);
            let damping = self.damping.next_value().clamp(0.0, 1.0);
            if damping != self.pole_for {
                self.set_pole(damping);
            }
            let pole = self.pole;
            let mix = self.mix.next_value().clamp(0.0, 1.0);
            let target = self.predelay_target.clamp(0.0, max_delay);
            let distance = target - self.predelay_delay;
            self.predelay_delay += distance.clamp(-PREDELAY_SLEW, PREDELAY_SLEW);
            let delay = self.predelay_delay.clamp(0.0, max_delay);

            let (dry_l, dry_r) = (sanitize(*l), sanitize(*r));
            self.age = self.age.saturating_add(1);
            // Pre-delay: write the present, read `delay` frames back (linear interpolation).
            let mono = self.block_dc((dry_l + dry_r) * 0.5);
            self.predelay.push(mono);
            let whole = delay as usize;
            let frac = delay - whole as f32;
            // `push` moved on: the present is one before `pos`.
            let at = |back: usize| {
                let pos = self.predelay.pos;
                let index = if pos > back {
                    pos - back - 1
                } else {
                    pos + len - back - 1
                };
                self.predelay.buf[index]
            };
            let age = self.age as usize;
            let newer = if whole < age { at(whole) } else { 0.0 };
            let older = if whole + 1 < age { at(whole + 1) } else { 0.0 };
            let x = newer + (older - newer) * frac;

            let wet_l = self.left.tick(x, pole, self.age - 1);
            let wet_r = self.right.tick(x, pole, self.age - 1);
            *l = flush(dry_l * (1.0 - mix) + wet_l * mix);
            *r = flush(dry_r * (1.0 - mix) + wet_r * mix);
        }
    }
}

impl Device for Reverb {
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

    /// Empties the tail and snaps the ramps; the parameter values are kept. Constant time:
    /// the lines are not touched, `age` hides what they hold (only the damping filters, 16
    /// numbers, are zeroed).
    fn reset(&mut self) {
        self.age = 0;
        self.dc_in = 0.0;
        self.dc_out = 0.0;
        self.left.clear_filters();
        self.right.clear_filters();
        self.size.snap();
        self.damping.snap();
        let damping = self.damping.next_value();
        self.set_pole(damping);
        self.predelay_delay = self
            .predelay_target
            .clamp(0.0, (self.predelay.buf.len() - GUARD_FRAMES) as f32);
        self.mix.snap();
        let size = self.size.next_value();
        self.apply_decay(size);
        self.size.snap();
    }
}
