//! The brick-wall limiter, `faulk.ceiling`, displayed "Ceiling" (#159): a stereo-linked,
//! look-ahead peak limiter for the end of a chain. It pushes the signal up by an input gain
//! and guarantees that no output sample goes above the ceiling. A [`Device`] of role effect.
//!
//! # Signal
//!
//! Per frame, one gain for both channels (the stereo image does not move):
//!
//! 1. `x = sanitize(input) · g_in`, the input gain applied, written to a delay line. The input
//!    is sanitized like the Compressor's: a non-finite sample is 0, a magnitude beyond 1e6 is
//!    clamped, a magnitude below 1e-20 is flushed.
//! 2. The **detector** reads `p = max(|x_L|, |x_R|)`. With `true_peak` on it reads instead
//!    the largest magnitude reconstructed on both sides of the sample: a 4× polyphase
//!    interpolator ([`TP_TAPS`] taps per phase, a Kaiser-windowed sinc, the table computed in
//!    [`Ceiling::new`], off the audio path) gives the values at the quarters, and a parabola
//!    through each local extremum and its two neighbours refines its height (the quarters
//!    alone read up to 0.5 dB low at 20 kHz and 48 kHz). The interpolator needs [`TP_DELAY`]
//!    future frames, so the detector then speaks of the sample `TP_DELAY` frames back; and
//!    the look-ahead is then at least [`TP_MIN_LOOKAHEAD`] frames (a gain that moves within
//!    fewer frames spreads the spectrum enough for the reconstructed peaks to go over).
//! 3. The **wanted gain** is `r = min(1, c / p)`, `c` the ceiling as a linear gain.
//! 4. The **look-ahead** of `L = round(lookahead · sample_rate)` frames: the sliding minimum
//!    `m[n] = min(r[n−L ..= n])` (a monotonic queue in a ring prepared off the audio path),
//!    then the release (below) gives `e[n] <= m[n]`, then the moving average
//!    `a[n] = mean(e[n−L ..= n])`. The audio comes out [`Ceiling::latency`] frames later:
//!    `y[n] = x[n − L] · a[n]` (`x[n − L − TP_DELAY]` with `true_peak`). Every `m[k]`
//!    averaged into `a[n]` is a minimum over a window that holds the sample being output, so
//!    `a[n]` is at most the wanted gain of that sample: the gain is down **on the very
//!    sample**, after a ramp of `L + 1` frames (the attack). With a look-ahead of 0 the
//!    attack is immediate, like the engine's output protection.
//! 5. The **release**, in decibels of reduction `t = −20 log10(m) >= 0`, with a state
//!    `R >= t` at every frame (so `e = 10^(−R/20) <= m`):
//!    - manual: `R = max(t, a_r · R + (1 − a_r) · t)`, `a_r = exp(−1 / (release · sample_rate))`:
//!      the `release` parameter is a time constant (63% of the way), as the Compressor's;
//!    - auto, program dependent: a slow mean of the reduction `S = a_s · S + (1 − a_s) · t`
//!      ([`AUTO_SLOW_SECONDS`]) and `R = max(t, S + a_f · (R − S))` ([`AUTO_FAST_SECONDS`]):
//!      after a short peak the gain comes back fast; under sustained limiting it comes back
//!      fast to the sustained reduction, then slowly with it (less pumping).
//! 6. The output is `y · comp` (`comp` is 1, or `1 / g_in` of that very sample with `compensate` on: the input
//!    gain then changes how hard the signal is limited, not its level), then **clamped to
//!    `±c`**: no output sample has a magnitude above the ceiling, whatever the signal and the
//!    settings. The clamp covers the rounding, and a ceiling that ramps down while delayed
//!    samples are in flight: those samples were measured against the higher ceiling of their
//!    detection, so a fast downward move of the ceiling clips them (at most the ceiling's
//!    move during the latency: 5 ms of a 20 ms ramp of 12 dB is 3 dB). With `true_peak` on,
//!    the reconstructed peak between the output samples stays near the ceiling (measured by
//!    the device tests on band-limited material, not guaranteed by construction: the gain
//!    varies inside the interpolator's span, and content close to the Nyquist frequency has
//!    no finite reconstruction).
//!
//! Below the ceiling with no input gain the wanted gain is exactly 1, the reduction exactly
//! 0 and the average exactly 1.0 (the running sum is set back to its exact value when no
//! reduced frame is left in the window): **the output is the input delayed by the
//! latency, bit for bit**.
//!
//! # Latency
//!
//! [`Device::latency_frames`] is `L`; with `true_peak` on, `max(L, TP_MIN_LOOKAHEAD) + TP_DELAY`. The engine does
//! not compensate a device's latency yet: a Ceiling on one track plays that track this much
//! later than the others (72 frames, 1.5 ms, at the defaults and 48 kHz). On the master
//! chain, its place, everything is delayed together. For the same reason the graph's 5 ms
//! crossfade of a bypass, an insert or a removal mixes the undelayed dry signal with the
//! delayed one (a short comb), and the track moves by the latency.
//!
//! A change of `lookahead` or `true_peak` changes the latency. While the device sounds, the
//! change waits until no other one came for [`SETTLE_SECONDS`] (a drag of the control or an
//! automation lane sends many: one switch for the whole gesture, the old latency meanwhile),
//! then the output fades out over [`FADE_SECONDS`], the new latency is taken (the windows
//! start again), the output stays silent for the new latency (the samples still in the line
//! were measured with the old windows, so they are never heard), then fades back in over
//! [`FADE_SECONDS`]. A change made before any frame since the last reset (a device being
//! inserted with its state) applies at once. A change during the fade out only moves the
//! target; one during the silence takes the newer latency and starts the silence again; one
//! during the fade in waits again (the fade in goes on meanwhile). A value that brings the
//! latency back to the one in use cancels the wait. The fades only lower the output: the
//! ceiling holds through them.
//!
//! # Parameters and smoothing
//!
//! `ceiling` and `gain` follow the 20 ms linear ramp their descriptors declare; `lookahead`
//! and `true_peak` follow the switch above; `release` is not ramped (its coefficient is
//! recomputed); the other switches take effect at their frame. Every value goes through
//! [`ParamDescriptor::sanitize`].
//!
//! # Real time
//!
//! Every buffer is allocated by [`factory`] with fallible allocations, sized for the
//! longest look-ahead at the device's sample rate (a rate outside 8 kHz to
//! [`MAX_SAMPLE_RATE`] is taken as 48 kHz). [`Device::reset`] takes constant time and wipes no
//! buffer: the count of frames since the reset (a `u64`, which does not wrap in any session)
//! hides the old content of the line and of the rings (an older position reads as silence or
//! as "no reduction"). The smoothing states are flushed below 1e-6 dB: no denormal comes out
//! of the exponential decay.

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, MAX_SAMPLE_RATE,
    ParamDescriptor, Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};
use crate::param::Smoother;

/// Ids of the parameters.
pub const CEILING: u32 = 0;
pub const GAIN: u32 = 1;
pub const COMPENSATE: u32 = 2;
pub const LOOKAHEAD: u32 = 3;
pub const RELEASE_MODE: u32 = 4;
pub const RELEASE: u32 = 5;
pub const TRUE_PEAK: u32 = 6;

/// The longest look-ahead, in seconds.
pub const LOOKAHEAD_MAX_SECONDS: f64 = 0.005;
/// The highest input gain, in decibels.
pub const GAIN_MAX_DB: f64 = 24.0;
/// Frames the true-peak detector looks ahead: half its interpolator's span.
pub const TP_DELAY: u32 = 12;
/// Taps per phase of the true-peak interpolator.
pub const TP_TAPS: usize = 2 * TP_DELAY as usize;
/// Phases of the true-peak interpolator (4× oversampling); phase 0 is the sample itself.
pub const TP_PHASES: usize = 4;
/// The shortest look-ahead with the true-peak detection, in frames: a gain that moves
/// faster spreads the signal's spectrum and its reconstructed peaks go over.
pub const TP_MIN_LOOKAHEAD: u32 = 16;
/// The Kaiser window's shape of the interpolator.
const TP_KAISER_BETA: f64 = 7.0;
/// How long a change of latency waits for the next one before it fades out.
pub const SETTLE_SECONDS: f32 = 0.05;
/// Fade out, then in, around a change of latency.
pub const FADE_SECONDS: f32 = 0.005;
/// The auto release's fast and slow time constants.
pub const AUTO_FAST_SECONDS: f32 = 0.02;
pub const AUTO_SLOW_SECONDS: f32 = 0.5;

/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;
/// A reduction state below this, in decibels, is 0.
const STATE_FLOOR_DB: f32 = 1e-6;
/// `20 / ln(10)` and `ln(10) / 20`: decibels from a natural log, and back.
const DB_PER_NEPER: f32 = 8.685_89;
const NEPER_PER_DB: f32 = 0.115_129_26;

/// The descriptor of the limiter (CT-5): `faulk.ceiling`, displayed "Ceiling" (one English
/// name, never translated). The defaults are the values of [`Ceiling::new`].
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.ceiling",
    kind_id: DeviceKind(7),
    version: 1,
    name: "Ceiling",
    role: DeviceRole::Effect,
    memory_bytes: MEMORY_BYTES,
    midi_output: None,
    named_keys: &[],
    params: &[
        ParamDescriptor {
            id: ParamId(CEILING),
            key: "ceiling",
            description: "Highest level the output may reach, in decibels of full scale, from -12 to 0 dB: no sample goes above it.",
            unit: Unit::Decibels,
            min: -12.0,
            max: 0.0,
            default: -0.3,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(GAIN),
            key: "gain",
            description: "Gain added before the limiter, from 0 to 24 dB: it pushes the sound against the ceiling, so it gets louder without going over.",
            unit: Unit::Decibels,
            min: 0.0,
            max: GAIN_MAX_DB,
            default: 0.0,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.02,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(COMPENSATE),
            key: "compensate",
            description: "When on, the output is turned down by the input gain: the gain changes how hard the sound is limited, not how loud it is, to compare by ear.",
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
            id: ParamId(LOOKAHEAD),
            key: "lookahead",
            description: "How far ahead the limiter sees the peaks, from 0 to 5 ms: it turns down smoothly before each one. It delays this track by that time: the other tracks are not delayed to match. A change fades out and back in.",
            unit: Unit::Seconds,
            min: 0.0,
            max: LOOKAHEAD_MAX_SECONDS,
            default: 0.0015,
            scale: Scale::Linear,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(RELEASE_MODE),
            key: "release_mode",
            description: "How the limiter lets the sound back up after a peak: manual (the release time) or auto (fast after a short peak, slower under sustained limiting).",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 1.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["manual", "auto"],
        },
        ParamDescriptor {
            id: ParamId(RELEASE),
            key: "release",
            description: "In manual release, time the limiter takes to let the sound back up after a peak, from 1 ms to 1 s: short is louder, long is smoother.",
            unit: Unit::Seconds,
            min: 0.001,
            max: 1.0,
            default: 0.1,
            scale: Scale::Log,
            step: 0.0,
            smoothing_seconds: 0.0,
            values: &[],
        },
        ParamDescriptor {
            id: ParamId(TRUE_PEAK),
            key: "true_peak",
            description: "When on, the limiter also catches the peaks between samples (4x oversampled detection), for a converter or an encoder. It adds 12 samples of delay and a look-ahead of at least 16 samples. A change fades out and back in.",
            unit: Unit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: Scale::Linear,
            step: 1.0,
            smoothing_seconds: 0.0,
            values: &["off", "on"],
        },
    ],
};

/// The limiter's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds a limiter for `setup`, off the audio path: its delay line and rings are allocated
/// here, with fallible allocations.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Ceiling::new(setup.sample_rate)?)?)
}

/// The look-ahead, in frames, of `seconds` at `sample_rate` (bounded to the longest).
pub fn lookahead_frames(seconds: f32, sample_rate: f32) -> u32 {
    let longest = (LOOKAHEAD_MAX_SECONDS * f64::from(sample_rate)).round();
    let frames = (f64::from(seconds) * f64::from(sample_rate))
        .round()
        .clamp(0.0, longest);
    if frames.is_finite() { frames as u32 } else { 0 }
}

/// Frames of the line and of the rings at `sample_rate`: a power of two above the longest
/// latency plus the interpolator's span.
const fn capacity(sample_rate: f32) -> usize {
    let longest = (LOOKAHEAD_MAX_SECONDS * sample_rate as f64) as usize + 1;
    (longest + 2 * TP_DELAY as usize + 2).next_power_of_two()
}

/// Bytes per frame of capacity: the delay line (two channels) and its compensation, the
/// averaged gains, and the queue of the sliding minimum (a frame index and a gain).
const BYTES_PER_FRAME: usize =
    3 * size_of::<f32>() + size_of::<f32>() + size_of::<u64>() + size_of::<f32>();

/// The most memory an instance takes, at [`MAX_SAMPLE_RATE`].
const MEMORY_BYTES: u64 =
    (size_of::<Ceiling>() + capacity(MAX_SAMPLE_RATE) * BYTES_PER_FRAME) as u64;

/// The index of the first frame after a reset: frame indices start here so that the
/// interpolator's oldest tap is never below 0; an index below it reads as silence.
const FLOOR: u64 = TP_TAPS as u64;

/// A gain of `db` decibels, as the device computes it (the ceiling's linear value).
pub fn db_to_gain(db: f32) -> f32 {
    (NEPER_PER_DB * db).exp()
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

/// The zeroth-order modified Bessel function of the first kind (the Kaiser window).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while k < 64.0 {
        let half = x / (2.0 * k);
        term *= half * half;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
        k += 1.0;
    }
    sum
}

/// The interpolator's table: for phase `j` (1 to 3), the taps that give the value at
/// `k + j/4` from the samples `k − TP_DELAY + 1 ..= k + TP_DELAY`, each phase normalised to
/// a gain of 1 at DC.
fn true_peak_table() -> [[f32; TP_TAPS]; TP_PHASES - 1] {
    let mut table = [[0f32; TP_TAPS]; TP_PHASES - 1];
    let half = f64::from(TP_DELAY);
    let norm = bessel_i0(TP_KAISER_BETA);
    for (j, row) in table.iter_mut().enumerate() {
        let frac = (j + 1) as f64 / TP_PHASES as f64;
        let mut taps = [0f64; TP_TAPS];
        for (i, tap) in taps.iter_mut().enumerate() {
            // The distance from the sample `k − TP_DELAY + 1 + i` to the point `k + frac`.
            let t = frac + half - 1.0 - i as f64;
            let sinc = if t == 0.0 {
                1.0
            } else {
                (core::f64::consts::PI * t).sin() / (core::f64::consts::PI * t)
            };
            let u = (t / half).clamp(-1.0, 1.0);
            let window = bessel_i0(TP_KAISER_BETA * (1.0 - u * u).sqrt()) / norm;
            *tap = sinc * window;
        }
        let dc: f64 = taps.iter().sum();
        for (slot, tap) in row.iter_mut().zip(taps) {
            *slot = (tap / dc) as f32;
        }
    }
    table
}

/// The look-ahead (in frames) and the true-peak switch in use for the ones asked: with the
/// true-peak detection, the look-ahead is at least [`TP_MIN_LOOKAHEAD`].
fn effective((lookahead, true_peak): (u32, bool)) -> (u32, bool) {
    if true_peak {
        (lookahead.max(TP_MIN_LOOKAHEAD), true)
    } else {
        (lookahead, false)
    }
}

/// What the output is doing around a change of latency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Steady,
    /// A change of latency waits for these many more frames without a new one (a drag of
    /// the control sends many), fading back in meanwhile if it was.
    Settle(u32),
    /// Fading out towards the switch.
    Out,
    /// Silent for these many more frames after the switch.
    Mute(u32),
    /// Fading back in.
    In,
}

#[derive(Debug)]
pub struct Ceiling {
    sample_rate: f32,
    ceiling: Smoother,
    gain: Smoother,
    ramp_frames: [u32; 2],
    /// The last decibel values read and their linear gains (the exponential is computed
    /// only when the value moves).
    ceiling_db: f32,
    ceiling_lin: f32,
    gain_db: f32,
    gain_lin: f32,
    compensate: bool,
    auto_release: bool,
    release: f32,
    fast: f32,
    slow: f32,
    /// The look-ahead and the true-peak switch in use, and the ones a change asked for.
    lookahead: u32,
    true_peak: bool,
    wanted: (u32, bool),
    /// The interpolator's table.
    table: [[f32; TP_TAPS]; TP_PHASES - 1],
    /// Ring buffers of `mask + 1` frames.
    mask: usize,
    line_l: Box<[f32]>,
    line_r: Box<[f32]>,
    /// One over the input gain each sample of the line was pushed by (the compensation).
    inv_gain: Box<[f32]>,
    /// The released gains `e`, for the moving average.
    gains: Box<[f32]>,
    /// The sliding minimum's queue: frame indices and wanted gains, increasing gains from
    /// `head` to `tail`.
    queue_at: Box<[u64]>,
    queue_gain: Box<[f32]>,
    head: u64,
    tail: u64,
    /// Frames since the last reset, and the frame the windows started at.
    frame: u64,
    window_from: u64,
    /// The moving average's running sum and the count of gains below 1 in its window.
    sum: f64,
    below: u32,
    /// The release's states, in decibels of reduction.
    reduction: f32,
    sustained: f32,
    /// The largest reconstructed magnitude of the previous span (the true-peak detector),
    /// and the last value reconstructed in it, per channel.
    between: f32,
    last_l: f32,
    last_r: f32,
    phase: Phase,
    fade: f32,
    fade_step: f32,
    settle_frames: u32,
    /// The smallest average gain of the last block, for the visual feedback.
    block_gain: f32,
    reported_db: f32,
}

impl Ceiling {
    /// A limiter at its defaults, its buffers allocated (fallibly) for `sample_rate`.
    pub fn new(sample_rate: f32) -> Result<Self, MsgError> {
        let sample_rate =
            if sample_rate.is_finite() && (8_000.0..=MAX_SAMPLE_RATE).contains(&sample_rate) {
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
        let frames = capacity(sample_rate);
        let asked = (
            lookahead_frames(default(LOOKAHEAD), sample_rate),
            default(TRUE_PEAK) >= 0.5,
        );
        let (lookahead, true_peak) = effective(asked);
        let mut device = Self {
            sample_rate,
            ceiling: Smoother::new(default(CEILING)),
            gain: Smoother::new(default(GAIN)),
            ramp_frames: [ramp(CEILING), ramp(GAIN)],
            ceiling_db: default(CEILING),
            ceiling_lin: db_to_gain(default(CEILING)),
            gain_db: default(GAIN),
            gain_lin: db_to_gain(default(GAIN)),
            compensate: default(COMPENSATE) >= 0.5,
            auto_release: default(RELEASE_MODE) >= 0.5,
            release: coefficient(default(RELEASE), sample_rate),
            fast: coefficient(AUTO_FAST_SECONDS, sample_rate),
            slow: coefficient(AUTO_SLOW_SECONDS, sample_rate),
            lookahead,
            true_peak,
            wanted: asked,
            table: true_peak_table(),
            mask: frames - 1,
            line_l: buffer(frames)?,
            line_r: buffer(frames)?,
            inv_gain: buffer(frames)?,
            gains: buffer(frames)?,
            queue_at: buffer(frames)?,
            queue_gain: buffer(frames)?,
            head: 0,
            tail: 0,
            frame: 0,
            window_from: 0,
            sum: 0.0,
            below: 0,
            reduction: 0.0,
            sustained: 0.0,
            between: 0.0,
            last_l: 0.0,
            last_r: 0.0,
            phase: Phase::Steady,
            fade: 1.0,
            fade_step: 1.0 / (FADE_SECONDS * sample_rate),
            settle_frames: (SETTLE_SECONDS * sample_rate) as u32,
            block_gain: 1.0,
            reported_db: 0.0,
        };
        device.restart_windows();
        Ok(device)
    }

    /// The latency, in frames, the device adds now: the look-ahead, plus [`TP_DELAY`] with
    /// the true-peak detection.
    pub fn latency(&self) -> u32 {
        self.lookahead + if self.true_peak { TP_DELAY } else { 0 }
    }

    /// The windows start again at the current frame, with no reduction.
    fn restart_windows(&mut self) {
        self.window_from = self.frame;
        self.head = 0;
        self.tail = 0;
        self.sum = f64::from(self.lookahead) + 1.0;
        self.below = 0;
        self.reduction = 0.0;
        self.sustained = 0.0;
        self.between = 0.0;
        self.last_l = 0.0;
        self.last_r = 0.0;
    }

    /// Silent for the latency just taken (nothing older than it may be heard), then the fade
    /// in; with no latency nothing is in flight: straight to the fade in.
    fn mute(&mut self) {
        let latency = self.latency();
        self.phase = if latency > 0 {
            Phase::Mute(latency)
        } else {
            Phase::In
        };
    }

    /// Takes the wanted latency.
    fn switch(&mut self) {
        (self.lookahead, self.true_peak) = effective(self.wanted);
        self.restart_windows();
    }

    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        match id.0 {
            CEILING => self.ceiling.set(value, self.ramp_frames[0]),
            GAIN => self.gain.set(value, self.ramp_frames[1]),
            COMPENSATE => self.compensate = value >= 0.5,
            LOOKAHEAD => {
                self.ask_latency((lookahead_frames(value, self.sample_rate), self.wanted.1));
            }
            RELEASE_MODE => self.auto_release = value >= 0.5,
            RELEASE => self.release = coefficient(value, self.sample_rate),
            TRUE_PEAK => self.ask_latency((self.wanted.0, value >= 0.5)),
            _ => {}
        }
    }

    /// A change of look-ahead or of true-peak detection: at once when nothing went through
    /// since the reset, else through the settle, the fade and the silence.
    fn ask_latency(&mut self, wanted: (u32, bool)) {
        self.wanted = wanted;
        if self.frame == 0 {
            self.switch();
            self.phase = Phase::Steady;
            self.fade = 1.0;
            return;
        }
        let differs = effective(wanted) != (self.lookahead, self.true_peak);
        match self.phase {
            // A drag sends many values: the switch waits until they stop.
            Phase::Steady | Phase::In | Phase::Settle(_) => {
                self.phase = if differs {
                    Phase::Settle(self.settle_frames)
                } else if self.phase == Phase::Steady {
                    Phase::Steady
                } else {
                    Phase::In
                };
            }
            // Taken back during the fade out: it fades back in, nothing switches.
            Phase::Out => {
                if !differs {
                    self.phase = Phase::In;
                }
            }
            Phase::Mute(_) => {
                if differs {
                    self.switch();
                    self.mute();
                }
            }
        }
    }

    /// The largest magnitude reconstructed from the sample `k` (included) to the sample
    /// `k + 1` (excluded) of one channel, `k = frame − TP_DELAY`, and the value reconstructed
    /// last in that span (the neighbour of the next span's first point). `last` is the one
    /// of the previous span.
    ///
    /// The interpolator gives the values at the quarters; a parabola through each local
    /// extremum and its two neighbours refines its height.
    fn span_peak(&self, line: &[f32], frame: u64, last: f32) -> (f32, f32) {
        // The taps read `k − TP_DELAY + 1 ..= k + TP_DELAY`, the last one being `frame`.
        let first = frame + 1 - TP_TAPS as u64;
        let start = first as usize & self.mask;
        let mut copy = [0f32; TP_TAPS];
        let samples: &[f32] = if first >= FLOOR && start + TP_TAPS <= line.len() {
            // The common case: the taps are one slice of the ring.
            &line[start..start + TP_TAPS]
        } else if first >= FLOOR {
            for (i, s) in copy.iter_mut().enumerate() {
                *s = line[(start + i) & self.mask];
            }
            &copy
        } else {
            for (i, s) in copy.iter_mut().enumerate() {
                let at = first + i as u64;
                // Before the reset the line holds old content: silence.
                if at >= FLOOR {
                    *s = line[at as usize & self.mask];
                }
            }
            &copy
        };
        let mid = TP_DELAY as usize;
        // `last`, the sample `k`, the three quarters, the sample `k + 1`.
        let mut points = [last, samples[mid - 1], 0.0, 0.0, 0.0, samples[mid]];
        for (row, point) in self.table.iter().zip(&mut points[2..5]) {
            *point = row.iter().zip(samples).map(|(h, x)| h * x).sum();
        }
        let mut peak = 0f32;
        for m in 1..5 {
            let (a, b, c) = (points[m - 1], points[m], points[m + 1]);
            peak = peak.max(b.abs());
            if b.abs() >= a.abs() && b.abs() >= c.abs() {
                // The vertex of the parabola through the three points.
                let curve = a - 2.0 * b + c;
                if curve.abs() > 1e-12 {
                    let top = b - 0.125 * (a - c) * (a - c) / curve;
                    if top.is_finite() {
                        peak = peak.max(top.abs().min(2.0 * b.abs()));
                    }
                }
            }
        }
        (peak, points[4])
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (yl, yr) = self.frame_of(*l, *r);
            *l = yl;
            *r = yr;
        }
    }

    /// One frame in, one frame out.
    #[inline]
    fn frame_of(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        if self.phase == Phase::Out && self.fade <= 0.0 {
            self.switch();
            self.mute();
        }
        let ceiling_db = self.ceiling.next_value().clamp(-12.0, 0.0);
        if ceiling_db != self.ceiling_db {
            self.ceiling_db = ceiling_db;
            self.ceiling_lin = db_to_gain(ceiling_db);
        }
        let gain_db = self.gain.next_value().clamp(0.0, GAIN_MAX_DB as f32);
        if gain_db != self.gain_db {
            self.gain_db = gain_db;
            self.gain_lin = db_to_gain(gain_db);
        }
        let c = self.ceiling_lin;
        // Frame indices start at the floor (see `FLOOR`).
        let n = self.frame + FLOOR;
        let at = n as usize & self.mask;
        let (xl, xr) = (
            sanitize(in_l) * self.gain_lin,
            sanitize(in_r) * self.gain_lin,
        );
        self.line_l[at] = xl;
        self.line_r[at] = xr;
        self.inv_gain[at] = 1.0 / self.gain_lin;

        // The detector, for the sample `n − TP_DELAY` with the true peak.
        let peak = if self.true_peak {
            let (left, last_l) = self.span_peak(&self.line_l, n, self.last_l);
            let (right, last_r) = self.span_peak(&self.line_r, n, self.last_r);
            (self.last_l, self.last_r) = (last_l, last_r);
            let now = left.max(right);
            // The sample `k` ends the previous span and starts this one.
            let peak = self.between.max(now);
            self.between = now;
            peak
        } else {
            xl.abs().max(xr.abs())
        };
        let wanted = if peak > c { c / peak } else { 1.0 };

        // The sliding minimum over `[n − L, n]`.
        let lookahead = u64::from(self.lookahead);
        while self.tail > self.head
            && self.queue_gain[(self.tail - 1) as usize & self.mask] >= wanted
        {
            self.tail -= 1;
        }
        let slot = self.tail as usize & self.mask;
        self.queue_at[slot] = n;
        self.queue_gain[slot] = wanted;
        self.tail += 1;
        while self.queue_at[self.head as usize & self.mask] + lookahead < n {
            self.head += 1;
        }
        let minimum = self.queue_gain[self.head as usize & self.mask];

        // The release, in decibels of reduction: `reduction >= t`.
        let t = if minimum < 1.0 {
            -DB_PER_NEPER * minimum.ln()
        } else {
            0.0
        };
        self.reduction = if self.auto_release {
            self.sustained = flush_db(self.slow * self.sustained + (1.0 - self.slow) * t);
            flush_db(t.max(self.sustained + self.fast * (self.reduction - self.sustained)))
        } else {
            flush_db(t.max(self.release * self.reduction + (1.0 - self.release) * t))
        };
        let released = if self.reduction > 0.0 {
            (-NEPER_PER_DB * self.reduction).exp().min(minimum)
        } else {
            minimum
        };

        // The moving average over `[n − L, n]`.
        self.gains[at] = released;
        self.sum += f64::from(released);
        if released < 1.0 {
            self.below += 1;
        }
        let old = match n.checked_sub(lookahead + 1) {
            Some(k) if k >= self.window_from + FLOOR => self.gains[k as usize & self.mask],
            _ => 1.0,
        };
        self.sum -= f64::from(old);
        if old < 1.0 {
            self.below = self.below.saturating_sub(1);
        }
        if self.below == 0 {
            self.sum = lookahead as f64 + 1.0;
        }
        let average = ((self.sum / (lookahead as f64 + 1.0)) as f32).clamp(0.0, 1.0);
        self.block_gain = self.block_gain.min(average);

        // The delayed sample, its gain, the fade, the ceiling.
        let latency = u64::from(self.latency());
        let (dl, dr, inv) = match n.checked_sub(latency) {
            Some(k) if k >= FLOOR => {
                let k = k as usize & self.mask;
                (self.line_l[k], self.line_r[k], self.inv_gain[k])
            }
            _ => (0.0, 0.0, 1.0),
        };
        let mut g = average;
        if self.compensate {
            // The gain this very sample was pushed by, not the one of now (a ramp moves it
            // while the sample is in flight).
            g *= inv;
        }
        match self.phase {
            Phase::Steady => {}
            Phase::Settle(left) => {
                self.fade = (self.fade + self.fade_step).min(1.0);
                g *= self.fade;
                self.phase = if left > 1 {
                    Phase::Settle(left - 1)
                } else if effective(self.wanted) != (self.lookahead, self.true_peak) {
                    Phase::Out
                } else {
                    Phase::In
                };
            }
            Phase::Out => {
                self.fade = (self.fade - self.fade_step).max(0.0);
                g *= self.fade;
            }
            Phase::Mute(left) => {
                g = 0.0;
                self.phase = if left <= 1 {
                    Phase::In
                } else {
                    Phase::Mute(left - 1)
                };
            }
            Phase::In => {
                self.fade = (self.fade + self.fade_step).min(1.0);
                g *= self.fade;
                if self.fade >= 1.0 {
                    self.phase = Phase::Steady;
                }
            }
        }
        self.frame += 1;
        (flush((dl * g).clamp(-c, c)), flush((dr * g).clamp(-c, c)))
    }
}

impl Device for Ceiling {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    /// Renders the frames between events and applies each event at its frame
    /// ([`event_frame`]).
    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = (ctx.frames as usize).min(io.left.len()).min(io.right.len());
        self.block_gain = 1.0;
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
        let db = -DB_PER_NEPER * self.block_gain.max(1e-12).ln();
        self.reported_db = if db.is_finite() { db.max(0.0) } else { 0.0 };
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        self.set_value(id, value);
    }

    /// The largest reduction of the last block (>= 0, finite).
    fn gain_reduction_db(&self) -> f32 {
        self.reported_db
    }

    fn latency_frames(&self) -> u32 {
        self.latency()
    }

    /// Opens the gain, hides the line's content, takes a pending latency and snaps the
    /// ramps; the parameter values are kept. Constant time: no buffer is wiped.
    fn reset(&mut self) {
        self.frame = 0;
        self.switch();
        self.phase = Phase::Steady;
        self.fade = 1.0;
        self.ceiling.snap();
        self.gain.snap();
        self.block_gain = 1.0;
        self.reported_db = 0.0;
    }
}
