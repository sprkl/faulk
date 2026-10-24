//! The echo, `faulk.echo`, displayed "Echo" (DV-3): a stereo delay whose time is free (in
//! seconds) or locked to the project's tempo (a note value), with a feedback and a mix. A
//! [`Device`] of role effect.
//!
//! # Signal
//!
//! Two independent delay lines, left to left and right to right. For each frame the line is
//! read `delay` frames back (linear interpolation, so an integer delay returns the exact
//! sample), the output is `dry * (1 - mix) + wet * mix` and the line takes `dry + wet *
//! feedback`. The feedback stops at [`FEEDBACK_MAX`] (0.95): the loop gain is below 1, so a
//! repeat is always quieter than the one before and the tail never runs away.
//!
//! The input is sanitized before it enters the line: a non-finite sample is 0, a magnitude
//! beyond 1e6 is clamped and a magnitude below 1e-20 is flushed to 0 (no denormal ever
//! circulates in the feedback loop). The output is flushed the same way: a cancellation
//! between the dry and the wet signal never leaves a denormal.
//!
//! [`Device::reset`] takes constant time: the lines are not wiped, a count of the frames
//! written since the reset hides what they hold (an effect switched on again on the audio
//! thread must not cost a block of memory writes).
//!
//! # Time
//!
//! In **free** mode the delay is the `time` parameter. In **synced** mode it is the length of
//! the `division` at the tempo of the block (`ctx.transport.tempo`, which the host takes from
//! the project, playing or not): `beats * 60 / bpm` seconds, `beats` counted in quarter
//! notes. The delay is at least 1 frame and at most [`MAX_DELAY_SECONDS`]: a synced time
//! longer than that (a whole note below 60 BPM) is shortened to it.
//!
//! The delay in use glides toward its target, so that a tempo change, a new time or a switch
//! of mode never makes the output jump: each frame it moves by a share of the distance
//! (a time constant of [`GLIDE_SECONDS`]), no less than [`MIN_SLEW`] and no more than
//! [`MAX_SLEW`] frames (the pitch of the
//! repeats moves by 25% at most while it glides, like a tape). The first block after the
//! device is built, or reset, starts at the target (the echo of a note is at its exact place
//! from the start).
//!
//! # Memory
//!
//! Both lines are allocated in the factory (`prepare`), for [`MAX_DELAY_SECONDS`] at the
//! sample rate of the setup (at most [`MAX_SAMPLE_RATE`]); `process` allocates nothing.
//!
//! # Parameters and smoothing
//!
//! `mode` and `division` are stepped (never smoothed); `time` takes effect through the
//! glide above; `feedback` and `mix` follow the 20 ms linear ramp the descriptor declares.
//! Every value goes through [`ParamDescriptor::sanitize`].

use super::Builtin;
use super::osc_synth::LinearRamp;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MAX_SAMPLE_RATE,
    ParamDescriptor, Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};

/// Ids of the parameters.
pub const MODE: u32 = 0;
pub const TIME: u32 = 1;
pub const DIVISION: u32 = 2;
pub const FEEDBACK: u32 = 3;
pub const MIX: u32 = 4;

/// Longest delay, in seconds.
pub const MAX_DELAY_SECONDS: f32 = 4.0;
/// Highest feedback: the loop gain stays below 1.
pub const FEEDBACK_MAX: f32 = 0.95;

/// The note values of `division`, in quarter notes, in the order of the parameter's values.
pub const DIVISION_BEATS: [f64; 10] = [
    0.125,
    0.25,
    1.0 / 3.0,
    0.5,
    2.0 / 3.0,
    0.75,
    1.0,
    1.5,
    2.0,
    4.0,
];

/// Time constant of the glide of the delay, in seconds.
const GLIDE_SECONDS: f32 = 0.05;
/// Fastest and slowest the delay moves, in frames per frame.
const MAX_SLEW: f32 = 0.25;
const MIN_SLEW: f32 = 0.002;
/// A distance below this, in frames, is reached.
const REACHED: f32 = 1e-3;
/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;
/// Frames kept past the longest delay, for the interpolation and the write head.
const GUARD_FRAMES: usize = 4;

const VALUES_MODE: &[&str] = &["free", "synced"];
const VALUES_DIVISION: &[&str] = &[
    "thirty_second",
    "sixteenth",
    "eighth_triplet",
    "eighth",
    "quarter_triplet",
    "eighth_dotted",
    "quarter",
    "quarter_dotted",
    "half",
    "whole",
];

/// Bytes of the two delay lines at [`MAX_SAMPLE_RATE`].
const LINES_BYTES: u64 = 2
    * ((MAX_DELAY_SECONDS as f64 * MAX_SAMPLE_RATE as f64) as u64 + GUARD_FRAMES as u64 + 1)
    * size_of::<f32>() as u64;

/// The descriptor of the echo (CT-5): `faulk.echo`, displayed "Echo" (one English name,
/// never translated). The defaults are the values of [`Echo::new`]: synced on a dotted
/// eighth, 40% feedback, 30% mix, 300 ms when set free.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.echo",
    kind_id: DeviceKind(3),
    version: 1,
    name: "Echo",
    role: DeviceRole::Effect,
    memory_bytes: size_of::<Echo>() as u64 + LINES_BYTES,
    midi_output: None,
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(MODE),
            key: "mode",
            description: "Whether the delay time is free (the time setting, in seconds) or locked to the project tempo (the note value setting).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 1.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_MODE,
        },
        ParamDescriptor {
            id: ParamId(TIME),
            key: "time",
            description: "Delay time in free mode, from 1 millisecond to 4 seconds.",
            unit: Unit::Seconds,
            min: 0.001,
            max: 4.0,
            default: 0.3,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(DIVISION),
            key: "division",
            description: "Delay time in synced mode, as a note value of the project tempo: 1/32 to a whole note, with triplets (T) and dotted values (.). Times beyond 4 seconds are shortened to 4 seconds.",
            unit: Unit::None,
            min: 0.0,
            max: 9.0,
            default: 5.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: VALUES_DIVISION,
        },
        ParamDescriptor {
            id: ParamId(FEEDBACK),
            key: "feedback",
            description: "Share of the echo fed back into the delay, from 0 to 0.95: more repeats, each one softer.",
            unit: Unit::None,
            min: 0.0,
            max: FEEDBACK_MAX as f64,
            default: 0.4,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(MIX),
            key: "mix",
            description: "Balance between the dry sound (0) and the echoes (1).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.3,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
    ],
};

/// The echo's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds an echo for `setup`, off the audio path: both lines are allocated here, with a
/// fallible allocation.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Echo::new(setup.sample_rate)?)?)
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

#[derive(Debug)]
pub struct Echo {
    sample_rate: f32,
    left: Box<[f32]>,
    right: Box<[f32]>,
    /// Frames written since the build or the last reset, up to the length of a line: the
    /// line holds nothing older, so a read further back is silence. A reset only zeroes this
    /// count, whatever the size of the lines.
    filled: usize,
    write: usize,
    synced: bool,
    time: f32,
    division: usize,
    feedback: LinearRamp,
    mix: LinearRamp,
    ramp_frames: [u32; 2],
    bpm: f64,
    /// The longest delay the lines hold, in frames.
    max_delay: f32,
    /// The delay in use and the one it glides toward, in frames (`f64`: a step of 0.002 frame must
    /// still move a delay of several seconds).
    delay: f64,
    target: f64,
    glide: f32,
    /// The next block starts at the target (built or reset).
    snap: bool,
}

impl Echo {
    /// An echo at its defaults, with its lines allocated for `sample_rate`.
    pub fn new(sample_rate: f32) -> Result<Self, MsgError> {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        // Above the rate the descriptor declares its memory for, the lines stop at that
        // size: the longest delay is shorter, the times stay right.
        let frames =
            (MAX_DELAY_SECONDS * sample_rate.min(MAX_SAMPLE_RATE)).ceil() as usize + GUARD_FRAMES;
        let default = |id: u32| DESCRIPTOR.param(ParamId(id)).map_or(0.0, |d| d.default);
        let ramp = |id: u32| {
            DESCRIPTOR
                .param(ParamId(id))
                .map_or(0, |d| d.smoothing_frames(sample_rate))
        };
        let mut echo = Self {
            sample_rate,
            left: line(frames)?,
            right: line(frames)?,
            filled: 0,
            write: 0,
            synced: default(MODE) >= 0.5,
            time: default(TIME) as f32,
            division: default(DIVISION) as usize,
            feedback: LinearRamp::new(default(FEEDBACK) as f32),
            mix: LinearRamp::new(default(MIX) as f32),
            ramp_frames: [ramp(FEEDBACK), ramp(MIX)],
            bpm: crate::time::Tempo::DEFAULT_BPM,
            max_delay: (frames - GUARD_FRAMES) as f32,
            delay: 1.0,
            target: 1.0,
            glide: 1.0 / (GLIDE_SECONDS * sample_rate),
            snap: true,
        };
        echo.retarget();
        Ok(echo)
    }

    /// The delay the parameters ask for at the current tempo, in frames, within the lines.
    fn retarget(&mut self) {
        let seconds = if self.synced {
            let beats = DIVISION_BEATS
                .get(self.division)
                .copied()
                .unwrap_or(DIVISION_BEATS[0]);
            beats * 60.0 / self.bpm
        } else {
            f64::from(self.time)
        };
        let frames = seconds * f64::from(self.sample_rate);
        // `max` and `min` swallow a NaN into the bound.
        self.target = frames.max(1.0).min(f64::from(self.max_delay));
    }

    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        match id.0 {
            MODE => self.synced = value >= 0.5,
            TIME => self.time = value,
            DIVISION => self.division = value as usize,
            FEEDBACK => self.feedback.set(value, self.ramp_frames[0]),
            MIX => self.mix.set(value, self.ramp_frames[1]),
            _ => return,
        }
        self.retarget();
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let len = self.left.len();
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let diff = self.target - self.delay;
            let distance = diff.abs();
            if distance <= f64::from(REACHED) {
                self.delay = self.target;
            } else {
                let step = (distance * f64::from(self.glide))
                    .clamp(f64::from(MIN_SLEW), f64::from(MAX_SLEW))
                    .min(distance);
                self.delay += step.copysign(diff);
            }
            // `delay` is within [1, max_delay]: both indexes are inside the line.
            let whole = (self.delay as usize).clamp(1, len - GUARD_FRAMES);
            let frac = ((self.delay - whole as f64) as f32).clamp(0.0, 1.0);
            let newer = if self.write >= whole {
                self.write - whole
            } else {
                self.write + len - whole
            };
            let older = if newer == 0 { len - 1 } else { newer - 1 };
            let feedback = self.feedback.next_value().clamp(0.0, FEEDBACK_MAX);
            let mix = self.mix.next_value().clamp(0.0, 1.0);
            let (dry_l, dry_r) = (sanitize(*l), sanitize(*r));
            // What was written before the last reset is not there.
            let (a, b) = (whole <= self.filled, whole < self.filled);
            let tap = |line: &[f32]| {
                let (x, y) = (
                    if a { line[newer] } else { 0.0 },
                    if b { line[older] } else { 0.0 },
                );
                x + (y - x) * frac
            };
            let (wet_l, wet_r) = (tap(&self.left), tap(&self.right));
            self.left[self.write] = flush(dry_l + wet_l * feedback);
            self.right[self.write] = flush(dry_r + wet_r * feedback);
            *l = flush(dry_l * (1.0 - mix) + wet_l * mix);
            *r = flush(dry_r * (1.0 - mix) + wet_r * mix);
            self.write += 1;
            if self.write == len {
                self.write = 0;
            }
            self.filled = (self.filled + 1).min(len);
        }
    }
}

fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR { 0.0 } else { x }
}

impl Device for Echo {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    /// Renders the frames between events and applies each event at its frame
    /// ([`event_frame`]); the tempo is the block's.
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).min(io.left.len()).min(io.right.len());
        self.bpm = ctx.transport.tempo.bpm();
        self.retarget();
        if self.snap {
            self.delay = self.target;
            self.snap = false;
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

    /// Empties both lines (the tail ends), snaps the ramps and starts the next block at the
    /// target delay; the parameter values are kept. Constant time: the lines are not
    /// touched, `filled` hides what they hold.
    fn reset(&mut self) {
        self.filled = 0;
        self.feedback.snap();
        self.mix.snap();
        self.snap = true;
    }
}
