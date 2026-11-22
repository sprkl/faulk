//! The noise gate, `faulk.sluice`, displayed "Sluice" (#158): a stereo-linked gate with an
//! opening and a closing threshold (a hysteresis), an attack, a hold, a release, a depth, an
//! inverted mode, a high-pass and a low-pass on what it listens to, an optional look-ahead and
//! an external detection source. A [`Device`] of role effect.
//!
//! # Signal
//!
//! Per frame, one gain for both channels (the stereo image does not move):
//!
//! 1. The input is sanitized like the compressor's: a non-finite sample is 0, a magnitude
//!    beyond 1e6 is clamped, a magnitude below 1e-20 is flushed. The sanitized frame is
//!    written to a delay line.
//! 2. The **detection signal** is that frame, or with a detection source (#952,
//!    [`Device::sidechain`]) the frame of the other track the graph gives
//!    ([`Device::process_keyed`]), sanitized alike: the gate follows that track, the gain still
//!    applies to this one. It goes through a 12 dB per octave Butterworth high-pass at `hpf`
//!    and a low-pass at `lpf` on each channel (the state-variable filter of the Filter device)
//!    unless the parameter is at its end that means off (20 Hz, 20 kHz): the gate stops
//!    listening to the lows or the highs, the sound itself is not filtered.
//! 3. The **level**: a peak follower, `e = max(|d|, e · a)` over the larger channel with `a`
//!    the one-pole coefficient of [`DETECT_RELEASE_SECONDS`] (it follows a rise at once and
//!    falls in 8 ms per neper): a sine of 1 kHz reads within 0.5 dB of its peak, a bass note
//!    ripples with the wave (a hold or the gap between the thresholds covers it). In decibels,
//!    `20 log10(max(e, 1e-6))`.
//! 4. The **trigger**, with a hysteresis: it fires when the level reaches `threshold` and
//!    stays fired while the level is at or above `close` (the closing threshold; one above the
//!    opening threshold is taken as the opening one). Under `close` it holds for `hold` seconds,
//!    then lets go. A level that swings between the two thresholds changes nothing.
//! 5. The **reduction** `r`, in decibels, from 0 (the signal passes) to the `depth`: it moves
//!    linearly in decibels towards its target, at `depth / time` per second, so a move across
//!    the whole depth lasts exactly `attack` (the trigger fired) or `release` (the trigger let
//!    go). In the normal mode a fired trigger opens the gate (target 0) and an unfired one
//!    closes it (target `depth`); in the inverted mode (the erase) it is the other way round.
//!    A depth change by the user follows the 20 ms ramp, and `r` never stays above it. At the
//!    largest depth ([`DEPTH_MAX_DB`]) a closed gate is silent: the gain is exactly 0.
//! 6. The **look-ahead** of `L = round(lookahead · sample_rate)` frames: the gain computed from
//!    the frame `n` applies to the frame `n − L` of the line, so the gate starts opening `L`
//!    frames before a hit reaches the output. With `r = 0` the output is exactly the delayed
//!    input.
//!
//! # Feedback
//!
//! [`Device::gain_reduction_db`] stays 0: the strip reports the largest reduction of its devices,
//! and a gate closed at 80 dB would swamp a compressor's meter. [`Device::readout`] is the gate's
//! own state and reduction under the id
//! [`STATE_READOUT`] (not a parameter): `r` while the trigger lets the sound pass, `-1 - r`
//! while it closes it, so the card tells open from closed through the hold, the release and the
//! inverted mode without guessing from the reduction.
//!
//! The output never exceeds the delayed input in magnitude (the gain is at most 1), and the
//! smoothing states are flushed under 1e-20 (the level) and 1e-6 dB (the reduction), so no
//! denormal appears in a decay.
//!
//! # Latency
//!
//! [`Device::latency_frames`] is `L`, 0 by default, at most [`Device::max_latency_frames`]. The
//! graph compensates it (#749, [`crate::graph`] > Latency) as the compressor's, and a bypassed,
//! inserted or removed gate keeps its timing through the graph's bypass line.
//! [`MEMORY_BYTES`] counts that line. A change of `lookahead` while the device sounds goes
//! through the [`LatencySwitch`]: a fade out, a silence of the new latency, a fade in. The
//! detection of an external source is not latency compensated (a source with a look-ahead is
//! heard that late), as the compressor's.
//!
//! # Parameters and smoothing
//!
//! `threshold`, `close` and `depth` follow the 20 ms linear ramp the descriptor declares;
//! `attack`, `hold`, `release`, `hpf` and `lpf` are not ramped: their coefficients are
//! recomputed when they are set (the reduction's own motion covers it); `invert` takes effect
//! at its frame; `lookahead` follows the latency switch. Every value goes through
//! [`ParamDescriptor::sanitize`].
//!
//! # Real time
//!
//! The delay line is allocated by [`factory`] with fallible allocations, sized for the longest
//! look-ahead at the device's sample rate, at most [`MAX_SAMPLE_RATE`]. [`Device::reset`] takes
//! constant time and wipes no buffer: the count of frames since the reset hides the old content
//! of the line. It closes the gate (opens it when inverted), rests the detector, takes a pending
//! look-ahead and snaps the ramps; the parameter values are kept.

use super::Builtin;
use super::filter::{Channel, Coefficients};
use super::latency_switch::{LatencySwitch, Take};
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, Key,
    MAX_SAMPLE_RATE, ParamDescriptor, Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, TrackSlot, try_box};
use crate::param::Smoother;

pub use super::latency_switch::{FADE_SECONDS, SETTLE_SECONDS};

/// Ids of the parameters.
pub const THRESHOLD: u32 = 0;
pub const CLOSE: u32 = 1;
pub const DEPTH: u32 = 2;
pub const ATTACK: u32 = 3;
pub const HOLD: u32 = 4;
pub const RELEASE: u32 = 5;
pub const INVERT: u32 = 6;
pub const HPF: u32 = 7;
pub const LPF: u32 = 8;
pub const LOOKAHEAD: u32 = 9;
/// The id of the device's readout (not a parameter: no control is drawn for it): the gate's
/// state and reduction for the visual feedback ([`Device::readout`]).
pub const STATE_READOUT: u32 = 100;

/// The deepest depth, in decibels: a closed gate at this depth is silent (the gain is 0).
pub const DEPTH_MAX_DB: f64 = 80.0;
/// The longest look-ahead, in seconds.
pub const LOOKAHEAD_MAX_SECONDS: f64 = 0.005;
/// The time constant of the peak follower's fall.
pub const DETECT_RELEASE_SECONDS: f32 = 0.008;
/// The detection high-pass is off at this frequency, in hertz, and the low-pass at its top.
pub const HPF_OFF_HZ: f64 = 20.0;
pub const HPF_MAX_HZ: f64 = 2000.0;
pub const LPF_MIN_HZ: f64 = 500.0;
pub const LPF_OFF_HZ: f64 = 20_000.0;

/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;
/// The detector never reads below this level (-120 dBFS).
const LEVEL_FLOOR: f32 = 1e-6;
/// A reduction below this, in decibels, is 0.
const STATE_FLOOR_DB: f32 = 1e-6;
/// `20 / ln(10)` and `ln(10) / 20`: decibels from a natural log, and back.
const DB_PER_NEPER: f32 = 8.685_89;
const NEPER_PER_DB: f32 = 0.115_129_26;

/// The descriptor of the gate (CT-5): `faulk.sluice`, displayed "Sluice" (one English name,
/// never translated). The defaults are the values of [`Sluice::new`]: opens at -30 dB, closes
/// at -36 dB, silent when closed, a 1 ms attack, a 50 ms hold, a 100 ms release, not inverted,
/// no filter on the detection, no look-ahead. The order of the parameters is the panel's.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    sidechain: true,
    kind: "faulk.sluice",
    kind_id: DeviceKind(18),
    version: 1,
    name: "Sluice",
    role: DeviceRole::Effect,
    memory_bytes: MEMORY_BYTES,
    midi_output: None,
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(THRESHOLD),
            key: "threshold",
            description: "Level, in decibels, at which the gate opens, from -80 to 0 dB: the sound passes once what the gate listens to reaches it.",
            unit: Unit::Decibels,
            min: -80.0,
            max: 0.0,
            default: -30.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(CLOSE),
            key: "close",
            description: "Level, in decibels, under which the open gate starts to close, from -80 to 0 dB. Between it and the opening threshold the gate stays as it is, so a wobbling level does not make it flutter. Above the opening threshold it counts as equal.",
            unit: Unit::Decibels,
            min: -80.0,
            max: 0.0,
            default: -36.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(DEPTH),
            key: "depth",
            description: "How far the closed gate turns the sound down, from 0 dB (not at all) to 80 dB, the largest, where it is silent.",
            unit: Unit::Decibels,
            min: 0.0,
            max: DEPTH_MAX_DB,
            default: DEPTH_MAX_DB,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(ATTACK),
            key: "attack",
            description: "Time the gate takes to open fully once the level reaches the opening threshold, from 0.01 ms to 100 ms: short keeps the first hit, long softens it.",
            unit: Unit::Seconds,
            min: 0.00001,
            max: 0.1,
            default: 0.001,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(HOLD),
            key: "hold",
            description: "Time the gate stays open after the level falls under the closing threshold, from 0 to 2 s: it keeps a note or a hit from being cut between two peaks.",
            unit: Unit::Seconds,
            min: 0.0,
            max: 2.0,
            default: 0.05,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(RELEASE),
            key: "release",
            description: "Time the gate takes to close fully once the hold is over, from 1 ms to 5 s: short cuts the tail, long lets it ring out.",
            unit: Unit::Seconds,
            min: 0.001,
            max: 5.0,
            default: 0.1,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(INVERT),
            key: "invert",
            description: "When on, the gate works the other way round: it turns the sound down while what it listens to is above the threshold, and lets it through under it, to erase a loud part.",
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
            id: ParamId(HPF),
            key: "hpf",
            description: "Frequency under which the gate stops listening, from 20 to 2000 Hz (a 12 dB per octave high-pass on what it listens to, not on the sound): a rumble no longer holds it open. 20 Hz is off.",
            unit: Unit::Hertz,
            min: HPF_OFF_HZ,
            max: HPF_MAX_HZ,
            default: HPF_OFF_HZ,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(LPF),
            key: "lpf",
            description: "Frequency above which the gate stops listening, from 500 to 20000 Hz (a 12 dB per octave low-pass on what it listens to, not on the sound): a hiss or a cymbal no longer opens it. 20000 Hz is off.",
            unit: Unit::Hertz,
            min: LPF_MIN_HZ,
            max: LPF_OFF_HZ,
            default: LPF_OFF_HZ,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(LOOKAHEAD),
            key: "lookahead",
            description: "How far ahead the gate sees the sound, from 0 to 5 ms, so it is already open when a hit arrives. It delays the sound by that time; the other tracks are delayed to match, so everything stays in time. A change fades out and back in.",
            unit: Unit::Seconds,
            min: 0.0,
            max: LOOKAHEAD_MAX_SECONDS,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
    ],
};

/// The gate's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a gate for `setup`, off the audio path: its delay line is allocated here, with
/// fallible allocations.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Sluice::new(setup.sample_rate)?)?)
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

/// Bytes per frame of capacity: the delay line, two channels.
const BYTES_PER_FRAME: usize = 2 * size_of::<f32>();

/// The most latency an instance at `sample_rate` adds: the longest look-ahead, bounded by
/// its line (as [`Sluice::new`] bounds it).
const fn max_latency_at(sample_rate: f32) -> u32 {
    let longest = (LOOKAHEAD_MAX_SECONDS * sample_rate as f64 + 0.5) as u32;
    let line = (capacity(sample_rate) - 2) as u32;
    if longest < line { longest } else { line }
}

/// The most memory an instance takes, at [`MAX_SAMPLE_RATE`], with the graph's bypass line for
/// its latency and its wrapper (#749, [`crate::latency::Latent`]).
pub const MEMORY_BYTES: u64 = (size_of::<Sluice>() + capacity(MAX_SAMPLE_RATE) * BYTES_PER_FRAME)
    as u64
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

#[derive(Debug)]
pub struct Sluice {
    sample_rate: f32,
    threshold: Smoother,
    close: Smoother,
    depth: Smoother,
    /// The ramps of threshold, close and depth.
    ramp_frames: [u32; 3],
    /// The attack and the release, in frames (at least one), and the hold, in frames.
    attack_frames: f32,
    release_frames: f32,
    hold_frames: u32,
    invert: bool,
    hpf_on: bool,
    lpf_on: bool,
    hpf: Coefficients,
    lpf: Coefficients,
    /// The detection filters' states, left and right.
    hp: [Channel; 2],
    lp: [Channel; 2],
    detect_release: f32,
    /// The peak follower.
    envelope: f32,
    /// The trigger (fired while the level is above the thresholds) and the frames of hold left.
    fired: bool,
    hold_left: u32,
    /// The gain taken off now, in decibels (0 to the depth).
    reduction: f32,
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
    /// Frames since the last reset.
    frame: u64,
    /// The track the detection listens to (#952), `None` for this one.
    source: Option<TrackSlot>,
}

impl Sluice {
    /// A gate at its defaults, its line allocated (fallibly) for `sample_rate`.
    pub fn new(sample_rate: f32) -> Result<Self, MsgError> {
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
        let mut gate = Self {
            sample_rate,
            threshold: Smoother::new(default(THRESHOLD) as f32),
            close: Smoother::new(default(CLOSE) as f32),
            depth: Smoother::new(default(DEPTH) as f32),
            ramp_frames: [ramp(THRESHOLD), ramp(CLOSE), ramp(DEPTH)],
            attack_frames: 1.0,
            release_frames: 1.0,
            hold_frames: 0,
            invert: default(INVERT) >= 0.5,
            hpf_on: false,
            lpf_on: false,
            hpf: Coefficients::new(default(HPF) as f32, 0.0, sample_rate),
            lpf: Coefficients::new(default(LPF) as f32, 0.0, sample_rate),
            hp: [Channel::default(); 2],
            lp: [Channel::default(); 2],
            detect_release: coefficient(DETECT_RELEASE_SECONDS, sample_rate),
            envelope: 0.0,
            fired: false,
            hold_left: 0,
            reduction: 0.0,
            lookahead,
            wanted: lookahead,
            switch: LatencySwitch::new(sample_rate),
            longest,
            mask: frames - 1,
            line_l: buffer(frames)?,
            line_r: buffer(frames)?,
            frame: 0,
            source: None,
        };
        gate.attack_frames = gate.frames_of(default(ATTACK) as f32);
        gate.release_frames = gate.frames_of(default(RELEASE) as f32);
        gate.hold_frames = gate.hold_of(default(HOLD) as f32);
        gate.reduction = gate.closed_reduction();
        Ok(gate)
    }

    /// `seconds` in frames, at least one (a time under a frame is one step).
    fn frames_of(&self, seconds: f32) -> f32 {
        (seconds * self.sample_rate).max(1.0)
    }

    fn hold_of(&self, seconds: f32) -> u32 {
        (f64::from(seconds) * f64::from(self.sample_rate)).round() as u32
    }

    /// The reduction of a gate at rest with nothing to hear: the depth, or none inverted.
    fn closed_reduction(&self) -> f32 {
        if self.invert {
            0.0
        } else {
            self.depth.current().clamp(0.0, DEPTH_MAX_DB as f32)
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
            CLOSE => self.close.set(value, self.ramp_frames[1]),
            DEPTH => self.depth.set(value, self.ramp_frames[2]),
            ATTACK => self.attack_frames = self.frames_of(value),
            HOLD => self.hold_frames = self.hold_of(value),
            RELEASE => self.release_frames = self.frames_of(value),
            INVERT => self.invert = value >= 0.5,
            HPF => {
                let on = f64::from(value) > HPF_OFF_HZ + 0.01;
                if on && !self.hpf_on {
                    // The filter starts from rest, not from what it held when last on.
                    self.hp = [Channel::default(); 2];
                }
                self.hpf_on = on;
                self.hpf = Coefficients::new(value, 0.0, self.sample_rate);
            }
            LPF => {
                let on = f64::from(value) < LPF_OFF_HZ - 0.01;
                if on && !self.lpf_on {
                    self.lp = [Channel::default(); 2];
                }
                self.lpf_on = on;
                self.lpf = Coefficients::new(value, 0.0, self.sample_rate);
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
            _ => {}
        }
    }

    /// The latency, in frames, the device adds now: the look-ahead in use.
    pub fn latency(&self) -> u32 {
        self.lookahead
    }

    /// Takes the wanted look-ahead.
    fn take_latency(&mut self) {
        self.lookahead = self.wanted;
    }

    /// Renders the frames of `left` and `right` (same length), the detection listening to
    /// `key` from its frame `at` on when there is one (past its end: silence), else to the
    /// input.
    fn render(&mut self, left: &mut [f32], right: &mut [f32], key: Option<(Key<'_>, usize)>) {
        match key {
            None => {
                for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                    (*l, *r) = self.frame_of(*l, *r, None);
                }
            }
            Some((key, at)) => {
                for (n, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                    let k = at + n;
                    let kl = key.left.get(k).copied().unwrap_or(0.0);
                    let kr = key.right.get(k).copied().unwrap_or(0.0);
                    (*l, *r) = self.frame_of(*l, *r, Some((kl, kr)));
                }
            }
        }
    }

    /// One frame in, one frame out; `key` is the frame of the track the detection listens to
    /// (#952), `None` for the input itself.
    #[inline]
    fn frame_of(&mut self, in_l: f32, in_r: f32, key: Option<(f32, f32)>) -> (f32, f32) {
        if self.switch.due() {
            self.take_latency();
            self.switch.silence(self.lookahead);
        }
        let threshold = self.threshold.next_value().clamp(-80.0, 0.0);
        let close = self.close.next_value().clamp(-80.0, 0.0).min(threshold);
        let depth = self.depth.next_value().clamp(0.0, DEPTH_MAX_DB as f32);
        // The gain multiplies the sanitized samples: a NaN or an infinity never comes out.
        let (dry_l, dry_r) = (sanitize(in_l), sanitize(in_r));
        let at = self.frame as usize & self.mask;
        self.line_l[at] = dry_l;
        self.line_r[at] = dry_r;

        // What the detection hears: the input, or the other track's signal, sanitized alike.
        let (mut dl, mut dr) = match key {
            None => (dry_l, dry_r),
            Some((kl, kr)) => (sanitize(kl), sanitize(kr)),
        };
        if self.hpf_on {
            let c = self.hpf;
            (dl, dr) = (self.hp[0].tick(dl, &c).1, self.hp[1].tick(dr, &c).1);
        }
        if self.lpf_on {
            let c = self.lpf;
            (dl, dr) = (self.lp[0].tick(dl, &c).0, self.lp[1].tick(dr, &c).0);
        }
        let level = dl.abs().max(dr.abs());
        self.envelope = flush(level.max(self.envelope * self.detect_release));
        let level_db = DB_PER_NEPER * self.envelope.max(LEVEL_FLOOR).ln();

        // The trigger: fires at the opening threshold, holds above the closing one, then
        // lets go after the hold.
        if level_db >= threshold {
            self.fired = true;
            self.hold_left = self.hold_frames;
        } else if self.fired {
            if level_db >= close {
                self.hold_left = self.hold_frames;
            } else if self.hold_left > 0 {
                self.hold_left -= 1;
            } else {
                self.fired = false;
            }
        }

        // The reduction moves in decibels towards its target: the depth when the gate is
        // closed, 0 when it is open; across the whole depth in the attack or the release.
        let passing = self.fired != self.invert;
        let target = if passing { 0.0 } else { depth };
        let time = if self.fired {
            self.attack_frames
        } else {
            self.release_frames
        };
        let step = depth / time;
        let r = self.reduction;
        let r = if r < target {
            (r + step).min(target)
        } else {
            (r - step).max(target)
        };
        let r = r.min(depth);
        self.reduction = if r < STATE_FLOOR_DB { 0.0 } else { r };
        // `reduction >= 0`: the gain never exceeds 1; at the largest depth it is silence.
        let gain = if self.reduction >= DEPTH_MAX_DB as f32 {
            0.0
        } else {
            (-NEPER_PER_DB * self.reduction).exp()
        };

        // The sample `L` frames back; before the reset, the line holds old content: silence.
        let (xl, xr) = match self.frame.checked_sub(u64::from(self.lookahead)) {
            Some(k) => {
                let k = k as usize & self.mask;
                (self.line_l[k], self.line_r[k])
            }
            None => (0.0, 0.0),
        };
        let fade = self.switch.gain(self.wanted != self.lookahead);
        let g = gain * fade;
        self.frame += 1;
        (flush(xl * g), flush(xr * g))
    }
}

impl Device for Sluice {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    /// Renders the frames between events and applies each event at its frame
    /// ([`event_frame`]); the detection listens to the input.
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        self.process_keyed(ctx, events, io, None);
    }

    /// [`Device::process`] with the detection listening to `key` (#952): the gate follows the
    /// other track, the gain still applies to this one.
    fn process_keyed(
        &mut self,
        ctx: &BlockCtx,
        events: &[Event],
        io: &mut AudioIo<'_>,
        key: Option<Key<'_>>,
    ) {
        let frames = (ctx.frames as usize).min(io.left.len()).min(io.right.len());
        let mut cursor = 0;
        for event in events {
            let at = event_frame(event.offset(), cursor, frames);
            if at > cursor {
                let k = key.map(|k| (k, cursor));
                self.render(&mut io.left[cursor..at], &mut io.right[cursor..at], k);
                cursor = at;
            }
            if let Event::ParamValue { id, value, .. } = *event {
                self.set_value(id, value);
            }
        }
        if frames > cursor {
            let k = key.map(|k| (k, cursor));
            self.render(
                &mut io.left[cursor..frames],
                &mut io.right[cursor..frames],
                k,
            );
        }
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        self.set_value(id, value);
    }

    /// The gate's own state and reduction for its card (the strip's `gain_reduction_db` is the
    /// largest of the devices', which would mix the gate's up to 80 dB with a compressor's):
    /// `r` when the gate is open or opening (the trigger says the sound passes), `-1 - r` when
    /// it is closed or closing, `r` the reduction in decibels now. A hold, a release and the
    /// inverted mode are all in the trigger's decision, not guessed from the reduction.
    fn readout(&self) -> Option<(ParamId, f32)> {
        let r = if self.reduction.is_finite() {
            self.reduction.clamp(0.0, DEPTH_MAX_DB as f32)
        } else {
            0.0
        };
        let passing = self.fired != self.invert;
        Some((ParamId(STATE_READOUT), if passing { r } else { -1.0 - r }))
    }

    fn latency_frames(&self) -> u32 {
        self.latency()
    }

    fn sidechain(&self) -> Option<TrackSlot> {
        self.source
    }

    fn set_sidechain(&mut self, source: Option<TrackSlot>) {
        self.source = source;
    }

    /// The longest look-ahead at the device's rate (#749): the graph sizes its bypass line
    /// from it.
    fn max_latency_frames(&self) -> u32 {
        lookahead_frames(LOOKAHEAD_MAX_SECONDS as f32, self.sample_rate).min(self.longest)
    }

    /// Closes the gate (opens it when inverted), rests the detector, hides the line's content,
    /// takes a pending look-ahead and snaps the ramps; the parameter values are kept.
    /// Constant time: no buffer is wiped.
    fn reset(&mut self) {
        self.threshold.snap();
        self.close.snap();
        self.depth.snap();
        self.envelope = 0.0;
        self.fired = false;
        self.hold_left = 0;
        self.hp = [Channel::default(); 2];
        self.lp = [Channel::default(); 2];
        self.frame = 0;
        self.take_latency();
        self.switch.reset();
        self.reduction = self.closed_reduction();
    }
}
