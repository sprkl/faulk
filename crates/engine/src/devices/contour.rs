//! The equaliser, `faulk.contour`, displayed "Contour" (#156): a seven-band parametric EQ
//! with a high-pass, a low shelf, three bells, a high shelf, a low-pass and an output gain.
//! A [`Device`] of role effect, the same filters on both channels, with no latency.
//!
//! # Bands
//!
//! In the order the signal runs through them, the keys' prefix in brackets:
//!
//! - the **high-pass** (`hp`): a slope of 12, 24 or 48 dB per octave (`hp_slope`);
//! - the **low shelf** (`low`), the three **bells** (`bell1`, `bell2`, `bell3`), the **high
//!   shelf** (`high`): a gain of ±24 dB;
//! - the **low-pass** (`lp`), with its slope like the high-pass;
//! - then the **output** gain, ±24 dB.
//!
//! Each band has its switch (`<band>_on`), its frequency (20 Hz to 20 kHz, log) and its Q (0.1
//! to 18, log). The parameter ids are stable: each band takes four, in that order, then the
//! output is 28 ([`band_param`]).
//!
//! # Signal
//!
//! Each band is a zero-delay-feedback (topology-preserving) state-variable filter, the
//! structure of the Filter (`devices/filter.rs`): with `g` and the damping `k`, one sample is
//!
//! ```text
//! v3 = x - ic2;  v1 = a1 ic1 + a2 v3;  v2 = ic2 + a2 ic1 + a3 v3
//! ic1 = 2 v1 - ic1;  ic2 = 2 v2 - ic2;  y = m0 x + m1 v1 + m2 v2
//! ```
//!
//! with `a1 = 1 / (1 + g (g + k))`, `a2 = g a1`, `a3 = g a2`. Its transfer function is
//! `(m0 (s^2 + k s + 1) + m1 s + m2) / (s^2 + k s + 1)` at `s = j tan(pi f / fs) / g`: the
//! analog prototype through the bilinear transform, exact at the band's frequency
//! ([`Contour::response_db`] evaluates it). With `t = tan(pi freq / fs)` and `A = 10^(gain / 40)`:
//!
//! - bell: `g = t`, `k = 1 / (Q A)`, `m = (1, k (A^2 - 1), 0)`: the peaking response
//!   `(s^2 + s A / Q + 1) / (s^2 + s / (A Q) + 1)`, whose gain at its frequency is exactly the
//!   set gain and whose `Q` is the frequency over the bandwidth between the points at half the
//!   gain in decibels;
//! - low shelf: `g = t / sqrt(A)`, `k = 1 / Q`, `m = (1, k (A - 1), A^2 - 1)`: the set gain far
//!   below the frequency, half of it (in decibels) at the frequency, 0 dB far above;
//! - high shelf: `g = t sqrt(A)`, `k = 1 / Q`, `m = (A^2, k (1 - A) A, 1 - A^2)`: the mirror;
//! - high-pass `m = (1, -k, -1)` and low-pass `m = (0, 0, 1)`: a slope of 12 dB per octave is one
//!   section with `k = 1 / Q`; 24 is two sections and 48 four, with the quality factors of a
//!   Butterworth filter of order 4 and 8 ([`BUTTERWORTH`]). The band's `Q` scales the section of
//!   highest quality factor by `Q / sqrt(1/2)`, so at the default `Q` (0.707) the response is a
//!   Butterworth's (flat, -3 dB at the frequency, then 12, 24 or 48 dB per octave).
//!
//! With every gain at 0 dB (`A = 1`) a bell or a shelf is `y = 1 x + 0 v1 + 0 v2`, and the output
//! gain is exactly 1: the device at its defaults (high-pass and low-pass off) gives out its input
//! unchanged, bit for bit (apart from the sanitising below).
//!
//! # Stability
//!
//! The structure is stable for any `g > 0` and `k > 0`, also while they move from one sample to
//! the next. The frequency is clamped to `[20 Hz, 0.45 fs]`, so it stays under Nyquist at every
//! sample rate. The input is sanitised like the Filter's: a non-finite sample is 0, a magnitude
//! beyond 1e6 is clamped, one below 1e-20 is flushed. Every state is flushed below 1e-20 on every
//! sample, so the decay never makes a denormal. The highest gain of the whole chain is bounded by
//! the bounds of its parameters (each bell or shelf at most +24 dB, the output +24 dB, a cut band
//! at full Q a peak of about `Q_last` (a section's quality factor)).
//!
//! # Parameters and smoothing
//!
//! Frequencies, gains, quality factors and the output follow the contract's linear ramp over the
//! 20 ms their descriptor declares (frequencies in hertz, gains in decibels). While a band's ramp
//! runs its coefficients are recomputed every sample, and kept when it does not, so the render
//! does not depend on how the host cuts blocks. A switch (`<band>_on`) or a slope change (both
//! choices, which declare no smoothing: their descriptions say what they do) crossfades over 10 ms:
//! a band fades between its input and its output, a slope between the cascades of the two slopes.
//! A band (or a cascade) that is off and still is not computed; its state starts empty when it
//! fades in again. [`Device::reset`] empties every state and snaps the ramps; the values are kept.
//! Every value goes through [`ParamDescriptor::sanitize`].

use super::Builtin;
use crate::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, ParamDescriptor,
    Scale, Unit, event_frame,
};
use crate::msg::{DeviceKind, MsgError, ParamId, try_box};
use crate::param::Smoother;

/// The bands, in the order the signal runs through them.
pub const HIGH_PASS: usize = 0;
pub const LOW_SHELF: usize = 1;
pub const BELL_1: usize = 2;
pub const BELL_2: usize = 3;
pub const BELL_3: usize = 4;
pub const HIGH_SHELF: usize = 5;
pub const LOW_PASS: usize = 6;
/// Number of bands.
pub const BANDS: usize = 7;

/// What a band does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    HighPass,
    LowShelf,
    Bell,
    HighShelf,
    LowPass,
}

impl Shape {
    /// A cut band (high-pass, low-pass): a slope and no gain.
    pub const fn is_cut(self) -> bool {
        matches!(self, Shape::HighPass | Shape::LowPass)
    }
}

/// The shape of each band.
pub const SHAPES: [Shape; BANDS] = [
    Shape::HighPass,
    Shape::LowShelf,
    Shape::Bell,
    Shape::Bell,
    Shape::Bell,
    Shape::HighShelf,
    Shape::LowPass,
];

/// What a band's parameter sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandParam {
    On,
    Freq,
    /// The gain of a bell or a shelf.
    Gain,
    Q,
    /// The slope of a cut band.
    Slope,
}

/// The parameter id of `what` of `band`: four ids per band, `on`, `freq`, then `gain` and `q`
/// for a bell or a shelf, `q` and `slope` for a cut band. `None` for a gain of a cut band or a
/// slope of another band.
pub const fn band_param(band: usize, what: BandParam) -> Option<u32> {
    if band >= BANDS {
        return None;
    }
    let base = (band * 4) as u32;
    let cut = SHAPES[band].is_cut();
    match (what, cut) {
        (BandParam::On, _) => Some(base),
        (BandParam::Freq, _) => Some(base + 1),
        (BandParam::Gain, false) => Some(base + 2),
        (BandParam::Q, false) => Some(base + 3),
        (BandParam::Q, true) => Some(base + 2),
        (BandParam::Slope, true) => Some(base + 3),
        _ => None,
    }
}

/// The id of the output gain.
pub const OUTPUT: u32 = 28;

/// The values of a slope.
pub const DB12: u32 = 0;
pub const DB24: u32 = 1;
pub const DB48: u32 = 2;

/// Lowest and highest frequency, in hertz.
pub const FREQ_MIN: f64 = 20.0;
pub const FREQ_MAX: f64 = 20_000.0;
/// Bounds of a gain and of the output, in decibels.
pub const GAIN_MAX_DB: f64 = 24.0;
/// Bounds of a quality factor.
pub const Q_MIN: f64 = 0.1;
pub const Q_MAX: f64 = 18.0;
/// The default quality factor of the shelves and the cut bands: a Butterworth's.
pub const Q_FLAT: f64 = 0.707;
/// A frequency never goes above this share of the sample rate.
const NYQUIST_SHARE: f32 = 0.45;
/// Time a switch or a slope change takes to crossfade, in seconds.
const FADE_SECONDS: f64 = 0.01;

/// The quality factors of the sections of a cut band, per slope (12, 24, 48 dB per octave): a
/// Butterworth filter of order 2, 4 and 8, sorted so that the highest is last (the one the
/// band's `Q` scales). Unused entries are 0.
pub const BUTTERWORTH: [[f32; 4]; 3] = [
    [core::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0, 0.0],
    [0.541_196_1, 1.306_563, 0.0, 0.0],
    [0.509_795_6, 0.601_344_9, 0.899_976_2, 2.562_915_4],
];
/// Sections of each slope's cascade.
const SECTIONS: [usize; 3] = [1, 2, 4];
/// Sections a band holds: one cascade per slope, side by side (1 + 2 + 4).
const BAND_SECTIONS: usize = 7;
/// Where each slope's cascade starts among a band's sections.
const CASCADE_START: [usize; 3] = [0, 1, 3];

/// The input is clamped to this magnitude; below [`DENORMAL_FLOOR`] it is flushed.
const INPUT_LIMIT: f32 = 1e6;
const DENORMAL_FLOOR: f32 = 1e-20;

const fn switch(
    id: u32,
    key: &'static str,
    description: &'static str,
    on: bool,
) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit: Unit::None,
        min: 0.0,
        max: 1.0,
        default: if on { 1.0 } else { 0.0 },
        scale: Scale::Linear,
        step: 1.0,
        smoothing_seconds: 0.0,
        values: &["off", "on"],
    }
}

const fn freq(
    id: u32,
    key: &'static str,
    description: &'static str,
    default: f64,
) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit: Unit::Hertz,
        min: FREQ_MIN,
        max: FREQ_MAX,
        default,
        scale: Scale::Log,
        step: 0.0,
        smoothing_seconds: 0.02,
        values: &[],
    }
}

const fn gain(id: u32, key: &'static str, description: &'static str) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit: Unit::Decibels,
        min: -GAIN_MAX_DB,
        max: GAIN_MAX_DB,
        default: 0.0,
        scale: Scale::Linear,
        step: 0.0,
        smoothing_seconds: 0.02,
        values: &[],
    }
}

const fn q(id: u32, key: &'static str, description: &'static str, default: f64) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit: Unit::None,
        min: Q_MIN,
        max: Q_MAX,
        default,
        scale: Scale::Log,
        step: 0.0,
        smoothing_seconds: 0.02,
        values: &[],
    }
}

const fn slope(id: u32, key: &'static str, description: &'static str) -> ParamDescriptor {
    ParamDescriptor {
        id: ParamId(id),
        key,
        description,
        unit: Unit::None,
        min: 0.0,
        max: 2.0,
        default: DB24 as f64,
        scale: Scale::Linear,
        step: 1.0,
        smoothing_seconds: 0.0,
        values: &["db12", "db24", "db48"],
    }
}

/// The descriptor of the equaliser: `faulk.contour`, displayed "Contour" (one English name,
/// never translated). The defaults are the values of [`Contour::new`]: the high-pass and the
/// low-pass off, the shelves and the bells on at 0 dB, so the device is transparent.
pub const DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.contour",
    kind_id: DeviceKind(7),
    version: 1,
    name: "Contour",
    role: DeviceRole::Effect,
    memory_bytes: size_of::<Contour>() as u64,
    midi_output: None,
    named_keys: &[],
    params: &[
        switch(
            0,
            "hp_on",
            "Switches the low cut (high-pass) on or off. A switch fades over 10 ms.",
            false,
        ),
        freq(
            1,
            "hp_freq",
            "Frequency, in hertz, below which the low cut takes the sound away, from 20 Hz to 20 kHz.",
            30.0,
        ),
        q(
            2,
            "hp_q",
            "Resonance of the low cut at its frequency, from 0.1 to 18: 0.707 is flat, higher values add a peak just above the cut.",
            Q_FLAT,
        ),
        slope(
            3,
            "hp_slope",
            "How steeply the low cut falls below its frequency: 12, 24 or 48 dB per octave. A change fades over 10 ms.",
        ),
        switch(
            4,
            "low_on",
            "Switches the low shelf on or off. A switch fades over 10 ms.",
            true,
        ),
        freq(
            5,
            "low_freq",
            "Frequency, in hertz, of the low shelf's corner, from 20 Hz to 20 kHz: the shelf's gain applies below it.",
            100.0,
        ),
        gain(
            6,
            "low_gain",
            "Gain of the low shelf, from -24 to +24 dB: what the sound below its frequency gains or loses.",
        ),
        q(
            7,
            "low_q",
            "Steepness of the low shelf's corner, from 0.1 to 18: 0.707 is smooth, higher values add a bump at the corner.",
            Q_FLAT,
        ),
        switch(
            8,
            "bell1_on",
            "Switches bell 1 on or off. A switch fades over 10 ms.",
            true,
        ),
        freq(
            9,
            "bell1_freq",
            "Centre frequency of bell 1, in hertz, from 20 Hz to 20 kHz.",
            250.0,
        ),
        gain(
            10,
            "bell1_gain",
            "Gain of bell 1 at its centre frequency, from -24 to +24 dB.",
        ),
        q(
            11,
            "bell1_q",
            "Width of bell 1, from 0.1 (wide) to 18 (narrow): its centre frequency over the bandwidth at half its gain.",
            1.0,
        ),
        switch(
            12,
            "bell2_on",
            "Switches bell 2 on or off. A switch fades over 10 ms.",
            true,
        ),
        freq(
            13,
            "bell2_freq",
            "Centre frequency of bell 2, in hertz, from 20 Hz to 20 kHz.",
            1000.0,
        ),
        gain(
            14,
            "bell2_gain",
            "Gain of bell 2 at its centre frequency, from -24 to +24 dB.",
        ),
        q(
            15,
            "bell2_q",
            "Width of bell 2, from 0.1 (wide) to 18 (narrow): its centre frequency over the bandwidth at half its gain.",
            1.0,
        ),
        switch(
            16,
            "bell3_on",
            "Switches bell 3 on or off. A switch fades over 10 ms.",
            true,
        ),
        freq(
            17,
            "bell3_freq",
            "Centre frequency of bell 3, in hertz, from 20 Hz to 20 kHz.",
            4000.0,
        ),
        gain(
            18,
            "bell3_gain",
            "Gain of bell 3 at its centre frequency, from -24 to +24 dB.",
        ),
        q(
            19,
            "bell3_q",
            "Width of bell 3, from 0.1 (wide) to 18 (narrow): its centre frequency over the bandwidth at half its gain.",
            1.0,
        ),
        switch(
            20,
            "high_on",
            "Switches the high shelf on or off. A switch fades over 10 ms.",
            true,
        ),
        freq(
            21,
            "high_freq",
            "Frequency, in hertz, of the high shelf's corner, from 20 Hz to 20 kHz: the shelf's gain applies above it.",
            8000.0,
        ),
        gain(
            22,
            "high_gain",
            "Gain of the high shelf, from -24 to +24 dB: what the sound above its frequency gains or loses.",
        ),
        q(
            23,
            "high_q",
            "Steepness of the high shelf's corner, from 0.1 to 18: 0.707 is smooth, higher values add a bump at the corner.",
            Q_FLAT,
        ),
        switch(
            24,
            "lp_on",
            "Switches the high cut (low-pass) on or off. A switch fades over 10 ms.",
            false,
        ),
        freq(
            25,
            "lp_freq",
            "Frequency, in hertz, above which the high cut takes the sound away, from 20 Hz to 20 kHz.",
            18_000.0,
        ),
        q(
            26,
            "lp_q",
            "Resonance of the high cut at its frequency, from 0.1 to 18: 0.707 is flat, higher values add a peak just below the cut.",
            Q_FLAT,
        ),
        slope(
            27,
            "lp_slope",
            "How steeply the high cut falls above its frequency: 12, 24 or 48 dB per octave. A change fades over 10 ms.",
        ),
        gain(
            OUTPUT,
            "output",
            "Gain of the whole equaliser's output, from -24 to +24 dB.",
        ),
    ],
};

/// The equaliser's line of the registry ([`super::REGISTRY`]).
pub const BUILTIN: Builtin = Builtin {
    descriptor: &DESCRIPTOR,
    factory,
};

/// Builds an equaliser for `setup`, off the audio path.
pub fn factory(setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Contour::new(setup.sample_rate))?)
}

fn sanitize(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
    }
}

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR { 0.0 } else { x }
}

/// `10^(db / 20)`, exactly 1 at 0 dB.
fn db_to_gain(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// The coefficients of one section.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Coefficients {
    a1: f32,
    a2: f32,
    a3: f32,
    m0: f32,
    m1: f32,
    m2: f32,
}

impl Coefficients {
    const IDENTITY: Self = Self {
        a1: 1.0,
        a2: 0.0,
        a3: 0.0,
        m0: 1.0,
        m1: 0.0,
        m2: 0.0,
    };

    fn new(g: f32, k: f32, m: (f32, f32, f32)) -> Self {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self {
            a1,
            a2,
            a3: g * a2,
            m0: m.0,
            m1: m.1,
            m2: m.2,
        }
    }
}

/// The two integrator states of one channel of one section.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct State {
    ic1: f32,
    ic2: f32,
}

impl State {
    #[inline]
    fn tick(&mut self, x: f32, c: &Coefficients) -> f32 {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        c.m0 * x + c.m1 * v1 + c.m2 * v2
    }
}

/// The settings of one band, as its coefficients are computed from them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandSettings {
    pub shape: Shape,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

/// A quality factor as the filters use it: clamped to its range, [`Q_FLAT`] when not finite.
fn finite_q(q: f32) -> f32 {
    if q.is_finite() {
        q.clamp(Q_MIN as f32, Q_MAX as f32)
    } else {
        Q_FLAT as f32
    }
}

/// `g`, `k` and the mix of a bell or a shelf, or `g` of a cut band (`k` and the mix per section).
fn prototype(s: &BandSettings, sample_rate: f32) -> (f32, f32, (f32, f32, f32)) {
    let top = (NYQUIST_SHARE * sample_rate).max(FREQ_MIN as f32);
    let hz = if s.freq_hz.is_finite() {
        s.freq_hz.clamp(FREQ_MIN as f32, top)
    } else {
        1000.0
    };
    let q = finite_q(s.q);
    let db = if s.gain_db.is_finite() {
        s.gain_db.clamp(-GAIN_MAX_DB as f32, GAIN_MAX_DB as f32)
    } else {
        0.0
    };
    let t = (core::f32::consts::PI * hz / sample_rate).tan();
    let a = if db == 0.0 {
        1.0
    } else {
        10f32.powf(db / 40.0)
    };
    match s.shape {
        Shape::Bell => {
            let k = 1.0 / (q * a);
            (t, k, (1.0, k * (a * a - 1.0), 0.0))
        }
        Shape::LowShelf => {
            let k = 1.0 / q;
            (t / a.sqrt(), k, (1.0, k * (a - 1.0), a * a - 1.0))
        }
        Shape::HighShelf => {
            let k = 1.0 / q;
            (t * a.sqrt(), k, (a * a, k * (1.0 - a) * a, 1.0 - a * a))
        }
        Shape::HighPass | Shape::LowPass => (t, 1.0 / q, (0.0, 0.0, 0.0)),
    }
}

/// The quality factor of section `i` of the cascade of `slope` for a band `Q` of `q`.
fn section_q(slope: usize, i: usize, q: f32) -> f32 {
    let bw = BUTTERWORTH[slope][i];
    if i + 1 == SECTIONS[slope] {
        bw * q / core::f32::consts::FRAC_1_SQRT_2
    } else {
        bw
    }
}

/// The mix of a cut section with damping `k`.
fn cut_mix(shape: Shape, k: f32) -> (f32, f32, f32) {
    if shape == Shape::HighPass {
        (1.0, -k, -1.0)
    } else {
        (0.0, 0.0, 1.0)
    }
}

/// The magnitude, squared, of the transfer function of a section at `w = tan(pi f / fs) / g`.
fn section_power(w: f32, k: f32, m: (f32, f32, f32)) -> f64 {
    let (w, k) = (f64::from(w), f64::from(k));
    let (m0, m1, m2) = (f64::from(m.0), f64::from(m.1), f64::from(m.2));
    // Denominator 1 - w^2 + j k w; numerator m0 (1 - w^2 + j k w) + j m1 w + m2.
    let (dr, di) = (1.0 - w * w, k * w);
    let (nr, ni) = (m0 * dr + m2, m0 * di + m1 * w);
    (nr * nr + ni * ni) / (dr * dr + di * di)
}

/// One band: its smoothed settings, its coefficients and the states of both channels.
#[derive(Clone, Copy, Debug)]
struct Band {
    shape: Shape,
    on: Smoother,
    freq: Smoother,
    gain: Smoother,
    q: Smoother,
    /// The weights of the three cascades of a cut band (12, 24, 48 dB per octave); a bell or a
    /// shelf uses the first one only, held at 1.
    slope: [Smoother; 3],
    coefficients: [Coefficients; BAND_SECTIONS],
    /// `[section][channel]`.
    states: [[State; 2]; BAND_SECTIONS],
    /// Whether the band, and each cascade, was computed at the last sample.
    active: bool,
    cascade_active: [bool; 3],
    /// Whether the coefficients must be recomputed once more.
    stale: bool,
    /// The cascades whose coefficients are those of the current settings (a cut band computes
    /// only the cascades it hears).
    computed: [bool; 3],
}

impl Band {
    fn settings(&self) -> BandSettings {
        BandSettings {
            shape: self.shape,
            freq_hz: self.freq.current(),
            gain_db: self.gain.current(),
            q: self.q.current(),
        }
    }

    /// Computes the coefficients of `s`: a bell's or a shelf's section, or the sections of the
    /// cascades of a cut band that `cascades` names.
    fn compute(&mut self, s: &BandSettings, sample_rate: f32, cascades: [bool; 3]) {
        let (g, k, m) = prototype(s, sample_rate);
        if s.shape.is_cut() {
            let q = finite_q(s.q);
            for slope in (0..3).filter(|c| cascades[*c]) {
                for i in 0..SECTIONS[slope] {
                    let k = 1.0 / section_q(slope, i, q);
                    self.coefficients[CASCADE_START[slope] + i] =
                        Coefficients::new(g, k, cut_mix(s.shape, k));
                }
            }
        } else {
            self.coefficients[0] = Coefficients::new(g, k, m);
        }
        self.computed = cascades;
    }

    /// The band's output for one stereo frame.
    #[inline]
    fn tick(&mut self, l: f32, r: f32, sample_rate: f32) -> (f32, f32) {
        let won = self.on.next_value();
        let moving = !self.freq.is_still() || !self.gain.is_still() || !self.q.is_still();
        let s = BandSettings {
            shape: self.shape,
            freq_hz: self.freq.next_value(),
            gain_db: self.gain.next_value(),
            q: self.q.next_value(),
        };
        let weights = self.slope.each_mut().map(Smoother::next_value);
        if won == 0.0 && self.on.is_still() {
            self.active = false;
            return (l, r);
        }
        if !self.active {
            self.states = [[State::default(); 2]; BAND_SECTIONS];
            self.cascade_active = [false; 3];
            self.active = true;
            self.stale = true;
        }
        // The cascades heard now: the weight of a slope that is not 0, or that still moves.
        let heard = core::array::from_fn(|c| weights[c] != 0.0 || !self.slope[c].is_still());
        if moving || self.stale || heard != self.computed {
            self.compute(&s, sample_rate, heard);
            self.stale = moving;
        }
        let (mut out_l, mut out_r) = (0.0, 0.0);
        for (c, &w) in weights.iter().enumerate() {
            if w == 0.0 && self.slope[c].is_still() {
                self.cascade_active[c] = false;
                continue;
            }
            let range = CASCADE_START[c]..CASCADE_START[c] + SECTIONS[c];
            if !self.cascade_active[c] {
                for st in &mut self.states[range.clone()] {
                    *st = [State::default(); 2];
                }
                self.cascade_active[c] = true;
            }
            let (mut yl, mut yr) = (l, r);
            for i in range {
                let co = self.coefficients[i];
                yl = self.states[i][0].tick(yl, &co);
                yr = self.states[i][1].tick(yr, &co);
            }
            out_l += w * yl;
            out_r += w * yr;
        }
        (l + won * (out_l - l), r + won * (out_r - r))
    }
}

#[derive(Debug)]
pub struct Contour {
    sample_rate: f32,
    bands: [Band; BANDS],
    output: Smoother,
    output_gain: f32,
    /// Whether the output gain must be taken once more (the frame after its ramp ends).
    output_stale: bool,
    fade_frames: u32,
    ramp_frames: u32,
}

impl Contour {
    /// An equaliser at its defaults.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        let default = |id: Option<u32>| {
            id.and_then(|id| DESCRIPTOR.param(ParamId(id)))
                .map_or(0.0, |d| d.default as f32)
        };
        let bands = core::array::from_fn(|b| {
            let shape = SHAPES[b];
            let slope = if shape.is_cut() {
                default(band_param(b, BandParam::Slope)) as usize
            } else {
                0
            };
            let mut band = Band {
                shape,
                on: Smoother::new(default(band_param(b, BandParam::On))),
                freq: Smoother::new(default(band_param(b, BandParam::Freq))),
                gain: Smoother::new(default(band_param(b, BandParam::Gain))),
                q: Smoother::new(default(band_param(b, BandParam::Q))),
                slope: core::array::from_fn(|c| Smoother::new(if c == slope { 1.0 } else { 0.0 })),
                coefficients: [Coefficients::IDENTITY; BAND_SECTIONS],
                states: [[State::default(); 2]; BAND_SECTIONS],
                active: false,
                cascade_active: [false; 3],
                stale: true,
                computed: [false; 3],
            };
            band.compute(&band.settings(), sample_rate, [true; 3]);
            band
        });
        let ramp = DESCRIPTOR
            .param(ParamId(OUTPUT))
            .map_or(0, |d| d.smoothing_frames(sample_rate));
        Self {
            sample_rate,
            bands,
            output: Smoother::new(0.0),
            output_gain: 1.0,
            output_stale: false,
            fade_frames: (FADE_SECONDS * f64::from(sample_rate)).round() as u32,
            ramp_frames: ramp,
        }
    }

    fn set_value(&mut self, id: ParamId, value: f32) {
        let Some(desc) = DESCRIPTOR.param(id) else {
            return;
        };
        let Some(value) = desc.sanitize(value) else {
            return;
        };
        if id.0 == OUTPUT {
            self.output.set(value, self.ramp_frames);
            return;
        }
        let b = (id.0 / 4) as usize;
        let Some(band) = self.bands.get_mut(b) else {
            return;
        };
        let (fade, ramp) = (self.fade_frames, self.ramp_frames);
        // The inverse of `band_param`: four ids per band, the last two by the band's shape.
        let what = match (id.0 % 4, band.shape.is_cut()) {
            (0, _) => Some(BandParam::On),
            (1, _) => Some(BandParam::Freq),
            (2, false) => Some(BandParam::Gain),
            (2, true) | (3, false) => Some(BandParam::Q),
            (3, true) => Some(BandParam::Slope),
            _ => None,
        };
        match what {
            Some(BandParam::On) => band.on.set(value.round().clamp(0.0, 1.0), fade),
            Some(BandParam::Freq) => band.freq.set(value, ramp),
            Some(BandParam::Gain) => band.gain.set(value, ramp),
            Some(BandParam::Q) => band.q.set(value, ramp),
            Some(BandParam::Slope) => {
                let chosen = (value.round() as usize).min(2);
                for (c, w) in band.slope.iter_mut().enumerate() {
                    w.set(if c == chosen { 1.0 } else { 0.0 }, fade);
                }
            }
            None => {}
        }
    }

    /// The settings the next frame of `band` uses (ramps included); `None` beyond the bands.
    pub fn band(&self, band: usize) -> Option<BandSettings> {
        self.bands.get(band).map(Band::settings)
    }

    /// The gain, in decibels, of the whole equaliser at `hz`, as its coefficients stand (the
    /// target values once the ramps are snapped): the product of the transfer functions of the
    /// bands that are on, with the slope each cut band settles on, and the output gain.
    pub fn response_db(&self, hz: f32) -> f32 {
        let mut power = 1.0f64;
        // From 1 Hz up: at 0 Hz a cut band's power is 0, whose logarithm is not finite.
        let t = (core::f32::consts::PI * hz.clamp(1.0, 0.499 * self.sample_rate)
            / self.sample_rate)
            .tan();
        for band in &self.bands {
            if band.on.target() < 0.5 {
                continue;
            }
            let s = BandSettings {
                shape: band.shape,
                freq_hz: band.freq.target(),
                gain_db: band.gain.target(),
                q: band.q.target(),
            };
            let (g, k, m) = prototype(&s, self.sample_rate);
            let w = t / g;
            if band.shape.is_cut() {
                let slope = band
                    .slope
                    .iter()
                    .position(|w| w.target() >= 0.5)
                    .unwrap_or(0);
                for i in 0..SECTIONS[slope] {
                    let k = 1.0 / section_q(slope, i, finite_q(s.q));
                    power *= section_power(w, k, cut_mix(band.shape, k));
                }
            } else {
                power *= section_power(w, k, m);
            }
        }
        (10.0 * power.max(1e-30).log10()) as f32 + self.output.target()
    }

    /// Renders the frames of `left` and `right` (same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let sr = self.sample_rate;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (mut x, mut y) = (sanitize(*l), sanitize(*r));
            for band in &mut self.bands {
                (x, y) = band.tick(x, y, sr);
            }
            // The gain follows the ramp, and is taken once more at its target the frame after the
            // ramp ends (the last frame of a ramp is one step short of it).
            let moving = !self.output.is_still();
            let db = self.output.next_value();
            if moving || self.output_stale {
                self.output_gain = db_to_gain(db);
                self.output_stale = moving;
            }
            *l = flush(x * self.output_gain);
            *r = flush(y * self.output_gain);
        }
    }
}

impl Device for Contour {
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

    /// Empties every band and snaps the ramps; the parameter values are kept.
    fn reset(&mut self) {
        let sr = self.sample_rate;
        for band in &mut self.bands {
            band.on.snap();
            band.freq.snap();
            band.gain.snap();
            band.q.snap();
            for w in &mut band.slope {
                w.snap();
            }
            band.states = [[State::default(); 2]; BAND_SECTIONS];
            band.active = false;
            band.cascade_active = [false; 3];
            band.compute(&band.settings(), sr, [true; 3]);
            band.stale = false;
        }
        self.output.snap();
        self.output_gain = db_to_gain(self.output.current());
        self.output_stale = false;
    }
}
