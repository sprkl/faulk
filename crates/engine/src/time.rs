//! Musical time: the contract shared by the engine and the project (CT-2).
//!
//! Units:
//!
//! - [`Tick`]: musical position or duration, an integer, [`PPQ`] (960) ticks per quarter
//!   note. 960 divides by 2, 3, 4, 5, 6, 8, 10, 12, 15 and 16, so triplets and
//!   quintuplets are exact. Integers: no drift, stable diffs.
//! - [`Frame`]: position or count *inside an audio file*, an integer sample index that does
//!   not depend on the tempo.
//! - Seconds and samples are `f64` and exist only at the engine's edge: the engine places
//!   every event at a fractional sample position derived from its tick, and splits the
//!   block at that offset.
//!
//! The tempo is constant per project at the MVP ([`Tempo`], 20 to 999 BPM); the grid
//! ([`TimeSignature`], [`Quantize`]) is anchored at tick 0. Nothing here allocates, locks
//! or panics: every function is total, out-of-range results saturate.
//!
//! # Rounding rules
//!
//! Every conversion is a pure function of the absolute position (never accumulated), so a
//! position converted twice gives the same value and there is no drift over time.
//!
//! - tick -> seconds, tick -> sample: `f64`, computed as `tick * 60 * rate / (bpm * 960)`
//!   in two operations, so the result is within a few ulps of the exact value (exact when
//!   the samples-per-tick ratio is an integer, as at 120 BPM and 48 kHz: 25 samples per
//!   tick). The engine keeps this fractional position; [`Tempo::tick_to_sample_rounded`]
//!   gives the nearest integer sample (ties away from zero, `f64::round`, the same on every
//!   target).
//! - seconds -> tick, sample -> tick, frames -> ticks: nearest tick, ties away from zero.
//!   A non-finite input (NaN, ±infinity) gives [`Tick::ZERO`]; a finite value beyond `i64`
//!   saturates to `i64::MIN` or `i64::MAX`.
//! - The round trip tick -> fractional sample -> tick is the identity for `|tick| <= 2^40`
//!   at every tempo and rate in range (the relative error of the two `f64` operations is
//!   far below half a tick). The round trip sample -> tick -> sample is *not* the identity:
//!   at the rates the engine runs at (44.1 kHz and above) several samples share one tick
//!   (a tick lasts at least 2.76 samples at 999 BPM and 44.1 kHz), which is why the engine
//!   never stores an event by its integer sample. [`SampleRate`] accepts any positive rate
//!   so that a file at an unusual rate can still be converted; below 2 kHz a tick is
//!   shorter than a sample and several ticks share one sample instead.
//! - A quantization boundary is `None` when the next grid point lies beyond `i64::MAX`:
//!   the grid never saturates to a tick that is off the grid.
//! - An audio clip has no length in ticks on disk; its end tick is computed from its frame
//!   count at the current tempo ([`Tempo::audio_end_tick`]), rounded to the nearest tick.

use core::ops::{Add, AddAssign, Sub, SubAssign};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Ticks per quarter note (pulses per quarter).
pub const PPQ: i64 = 960;

// ---------------------------------------------------------------------------------------
// Tick
// ---------------------------------------------------------------------------------------

/// A musical position or duration, in ticks ([`PPQ`] per quarter note).
///
/// Arithmetic saturates instead of overflowing. The inner value is public so that the
/// project and the UI read it as a plain integer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(transparent)]
pub struct Tick(pub i64);

impl Tick {
    /// The origin of musical time.
    pub const ZERO: Tick = Tick(0);
    /// One quarter note.
    pub const QUARTER: Tick = Tick(PPQ);

    /// Wraps a raw tick count.
    pub const fn new(ticks: i64) -> Self {
        Tick(ticks)
    }

    /// The raw tick count.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Saturating addition.
    pub const fn saturating_add(self, other: Tick) -> Tick {
        Tick(self.0.saturating_add(other.0))
    }

    /// Saturating subtraction.
    pub const fn saturating_sub(self, other: Tick) -> Tick {
        Tick(self.0.saturating_sub(other.0))
    }
}

impl Add for Tick {
    type Output = Tick;
    fn add(self, other: Tick) -> Tick {
        self.saturating_add(other)
    }
}

impl Sub for Tick {
    type Output = Tick;
    fn sub(self, other: Tick) -> Tick {
        self.saturating_sub(other)
    }
}

impl AddAssign for Tick {
    fn add_assign(&mut self, other: Tick) {
        *self = *self + other;
    }
}

impl SubAssign for Tick {
    fn sub_assign(&mut self, other: Tick) {
        *self = *self - other;
    }
}

// ---------------------------------------------------------------------------------------
// Frame
// ---------------------------------------------------------------------------------------

/// A position or a count inside an audio file, in sample frames, independent of the tempo.
///
/// Arithmetic saturates instead of overflowing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(transparent))]
#[repr(transparent)]
pub struct Frame(pub u64);

impl Frame {
    /// The start of a file.
    pub const ZERO: Frame = Frame(0);

    /// Wraps a raw frame count.
    pub const fn new(frames: u64) -> Self {
        Frame(frames)
    }

    /// The raw frame count.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Saturating addition.
    pub const fn saturating_add(self, other: Frame) -> Frame {
        Frame(self.0.saturating_add(other.0))
    }

    /// Saturating subtraction (stops at zero).
    pub const fn saturating_sub(self, other: Frame) -> Frame {
        Frame(self.0.saturating_sub(other.0))
    }
}

impl Add for Frame {
    type Output = Frame;
    fn add(self, other: Frame) -> Frame {
        self.saturating_add(other)
    }
}

impl Sub for Frame {
    type Output = Frame;
    fn sub(self, other: Frame) -> Frame {
        self.saturating_sub(other)
    }
}

impl AddAssign for Frame {
    fn add_assign(&mut self, other: Frame) {
        *self = *self + other;
    }
}

impl SubAssign for Frame {
    fn sub_assign(&mut self, other: Frame) {
        *self = *self - other;
    }
}

// ---------------------------------------------------------------------------------------
// SampleRate
// ---------------------------------------------------------------------------------------

/// A sample rate in hertz, never zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "u32", into = "u32")
)]
pub struct SampleRate(u32);

impl SampleRate {
    /// 44.1 kHz.
    pub const HZ_44100: SampleRate = SampleRate(44_100);
    /// 48 kHz.
    pub const HZ_48000: SampleRate = SampleRate(48_000);
    /// 96 kHz.
    pub const HZ_96000: SampleRate = SampleRate(96_000);

    /// A sample rate; `None` for zero.
    pub const fn new(hz: u32) -> Option<Self> {
        if hz == 0 { None } else { Some(SampleRate(hz)) }
    }

    /// The rate in hertz.
    pub const fn hz(self) -> u32 {
        self.0
    }
}

impl Default for SampleRate {
    fn default() -> Self {
        SampleRate::HZ_48000
    }
}

impl TryFrom<u32> for SampleRate {
    type Error = TimeError;
    fn try_from(hz: u32) -> Result<Self, TimeError> {
        SampleRate::new(hz).ok_or(TimeError::SampleRate)
    }
}

impl From<SampleRate> for u32 {
    fn from(rate: SampleRate) -> u32 {
        rate.0
    }
}

// ---------------------------------------------------------------------------------------
// Tempo
// ---------------------------------------------------------------------------------------

/// A tempo in beats (quarter notes) per minute, finite and within `[20, 999]`.
///
/// Constant per project at the MVP. Carries every conversion between ticks, seconds,
/// samples and frames (see the module documentation for the rounding rules).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "f64", into = "f64")
)]
pub struct Tempo(f64);

impl Tempo {
    /// Slowest tempo, in BPM.
    pub const MIN: f64 = 20.0;
    /// Fastest tempo, in BPM.
    pub const MAX: f64 = 999.0;
    /// The tempo of a new project, in BPM.
    pub const DEFAULT_BPM: f64 = 120.0;

    /// A tempo; `None` when `bpm` is not finite or is outside `[20, 999]`.
    pub fn new(bpm: f64) -> Option<Self> {
        if bpm.is_finite() && (Tempo::MIN..=Tempo::MAX).contains(&bpm) {
            Some(Tempo(bpm))
        } else {
            None
        }
    }

    /// A tempo saturated to `[20, 999]`; NaN gives the default tempo.
    pub fn clamped(bpm: f64) -> Self {
        if bpm.is_nan() {
            Tempo(Tempo::DEFAULT_BPM)
        } else {
            Tempo(bpm.clamp(Tempo::MIN, Tempo::MAX))
        }
    }

    /// The tempo in BPM.
    pub const fn bpm(self) -> f64 {
        self.0
    }

    /// Ticks elapsed in one second.
    pub fn ticks_per_second(self) -> f64 {
        self.0 * PPQ as f64 / 60.0
    }

    /// Duration of one tick, in seconds.
    pub fn seconds_per_tick(self) -> f64 {
        60.0 / (self.0 * PPQ as f64)
    }

    /// Duration of one tick, in samples at `rate`.
    pub fn samples_per_tick(self, rate: SampleRate) -> f64 {
        60.0 * f64::from(rate.hz()) / (self.0 * PPQ as f64)
    }

    /// The time of a tick, in seconds from tick 0 (`f64`, within a few ulps of the exact
    /// value; see the module documentation).
    pub fn tick_to_seconds(self, tick: Tick) -> f64 {
        (tick.0 as f64 * 60.0) / (self.0 * PPQ as f64)
    }

    /// The nearest tick to a time in seconds (ties away from zero). Non-finite gives
    /// [`Tick::ZERO`]; out of range saturates.
    pub fn seconds_to_tick(self, seconds: f64) -> Tick {
        round_to_tick(seconds, self.0 * PPQ as f64 / 60.0)
    }

    /// The fractional sample position of a tick at `rate`, from tick 0.
    ///
    /// This is the position the engine uses to place an event inside a block.
    pub fn tick_to_sample(self, tick: Tick, rate: SampleRate) -> f64 {
        (tick.0 as f64 * (60.0 * f64::from(rate.hz()))) / (self.0 * PPQ as f64)
    }

    /// The nearest integer sample of a tick at `rate` (ties away from zero, saturating).
    pub fn tick_to_sample_rounded(self, tick: Tick, rate: SampleRate) -> i64 {
        self.tick_to_sample(tick, rate).round() as i64
    }

    /// The nearest tick to a sample position at `rate` (ties away from zero). Non-finite
    /// gives [`Tick::ZERO`]; out of range saturates.
    pub fn sample_to_tick(self, sample: f64, rate: SampleRate) -> Tick {
        round_to_tick(sample, self.0 * PPQ as f64 / (60.0 * f64::from(rate.hz())))
    }

    /// The duration in ticks of `frames` sample frames of an audio file at `rate`, rounded
    /// to the nearest tick. Saturates for durations beyond `i64` ticks.
    pub fn frames_to_ticks(self, frames: Frame, rate: SampleRate) -> Tick {
        self.sample_to_tick(frames.0 as f64, rate)
    }

    /// The end tick of an audio clip that starts at `start` and plays `frames` frames at
    /// `rate`: `start + frames_to_ticks(frames)`, saturating.
    ///
    /// With a constant tempo this equals "start in seconds, plus `frames / rate`, back to
    /// ticks" (the rule of the project model); a tempo map would make the two differ.
    pub fn audio_end_tick(self, start: Tick, frames: Frame, rate: SampleRate) -> Tick {
        start + self.frames_to_ticks(frames, rate)
    }
}

impl Default for Tempo {
    fn default() -> Self {
        Tempo(Tempo::DEFAULT_BPM)
    }
}

impl TryFrom<f64> for Tempo {
    type Error = TimeError;
    fn try_from(bpm: f64) -> Result<Self, TimeError> {
        Tempo::new(bpm).ok_or(TimeError::Tempo)
    }
}

impl From<Tempo> for f64 {
    fn from(tempo: Tempo) -> f64 {
        tempo.0
    }
}

/// Nearest tick to `input * scale` (ties away from zero). A non-finite input gives zero;
/// a finite input whose product overflows `f64` or `i64` saturates by sign (`as`
/// saturates, and an infinite product of a finite input is sent to the matching end).
fn round_to_tick(input: f64, scale: f64) -> Tick {
    if !input.is_finite() {
        return Tick::ZERO;
    }
    let ticks = input * scale;
    if ticks.is_finite() {
        Tick(ticks.round() as i64)
    } else if ticks > 0.0 {
        Tick(i64::MAX)
    } else {
        Tick(i64::MIN)
    }
}

// ---------------------------------------------------------------------------------------
// TimeSignature
// ---------------------------------------------------------------------------------------

/// A time signature `numerator / denominator`: the numerator in `1..=32`, the denominator
/// a power of two in `1..=32`. The beat is the denominator's note value (3/8 has an
/// eighth-note beat of 480 ticks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "TimeSignatureRaw", into = "TimeSignatureRaw")
)]
pub struct TimeSignature {
    numerator: u8,
    denominator: u8,
}

/// Bounds of a [`TimeSignature`]'s numerator and denominator.
pub const MAX_SIGNATURE_PART: u8 = 32;

impl TimeSignature {
    /// Common time, 4/4.
    pub const COMMON: TimeSignature = TimeSignature {
        numerator: 4,
        denominator: 4,
    };

    /// A time signature; `None` when the numerator is not in `1..=32` or the denominator
    /// is not a power of two in `1..=32`.
    pub const fn new(numerator: u8, denominator: u8) -> Option<Self> {
        let numerator_ok = numerator >= 1 && numerator <= MAX_SIGNATURE_PART;
        let denominator_ok =
            denominator >= 1 && denominator <= MAX_SIGNATURE_PART && denominator.is_power_of_two();
        if numerator_ok && denominator_ok {
            Some(TimeSignature {
                numerator,
                denominator,
            })
        } else {
            None
        }
    }

    /// Beats per bar.
    pub const fn numerator(self) -> u8 {
        self.numerator
    }

    /// The note value of one beat (4 = quarter note).
    pub const fn denominator(self) -> u8 {
        self.denominator
    }

    /// Ticks in one beat: `PPQ * 4 / denominator`, exact for every accepted denominator.
    pub const fn ticks_per_beat(self) -> Tick {
        Tick(PPQ * 4 / self.denominator as i64)
    }

    /// Ticks in one bar: `numerator * ticks_per_beat`.
    pub const fn ticks_per_bar(self) -> Tick {
        Tick(self.numerator as i64 * self.ticks_per_beat().0)
    }
}

impl Default for TimeSignature {
    fn default() -> Self {
        TimeSignature::COMMON
    }
}

/// The on-disk shape of a [`TimeSignature`], validated on the way in.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TimeSignatureRaw {
    /// Beats per bar.
    pub numerator: u8,
    /// The note value of one beat.
    pub denominator: u8,
}

impl TryFrom<TimeSignatureRaw> for TimeSignature {
    type Error = TimeError;
    fn try_from(raw: TimeSignatureRaw) -> Result<Self, TimeError> {
        TimeSignature::new(raw.numerator, raw.denominator).ok_or(TimeError::TimeSignature)
    }
}

impl From<TimeSignature> for TimeSignatureRaw {
    fn from(sig: TimeSignature) -> Self {
        TimeSignatureRaw {
            numerator: sig.numerator,
            denominator: sig.denominator,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Quantize
// ---------------------------------------------------------------------------------------

/// The launch quantization: the grid a launch snaps to. Global to the project at the MVP,
/// one bar by default (founder's decision 8).
///
/// The grid is anchored at tick 0; [`Quantize::Sixteenth`] is a sixteenth note whatever
/// the signature, [`Quantize::Beat`] and the bars follow the [`TimeSignature`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Quantize {
    /// No quantization: a launch happens at once.
    None,
    /// A sixteenth note (240 ticks).
    Sixteenth,
    /// One beat of the time signature.
    Beat,
    /// One bar.
    #[default]
    Bar,
    /// Two bars.
    #[cfg_attr(feature = "serde", serde(rename = "bars_2"))]
    Bars2,
    /// Four bars.
    #[cfg_attr(feature = "serde", serde(rename = "bars_4"))]
    Bars4,
    /// Eight bars.
    #[cfg_attr(feature = "serde", serde(rename = "bars_8"))]
    Bars8,
}

impl Quantize {
    /// Every value, in the order of the UI's menu.
    pub const ALL: [Quantize; 7] = [
        Quantize::None,
        Quantize::Sixteenth,
        Quantize::Beat,
        Quantize::Bar,
        Quantize::Bars2,
        Quantize::Bars4,
        Quantize::Bars8,
    ];

    /// The grid step in ticks under `sig`; `None` for [`Quantize::None`].
    pub const fn grid(self, sig: TimeSignature) -> Option<Tick> {
        let bar = sig.ticks_per_bar().0;
        match self {
            Quantize::None => None,
            Quantize::Sixteenth => Some(Tick(PPQ / 4)),
            Quantize::Beat => Some(sig.ticks_per_beat()),
            Quantize::Bar => Some(Tick(bar)),
            Quantize::Bars2 => Some(Tick(bar * 2)),
            Quantize::Bars4 => Some(Tick(bar * 4)),
            Quantize::Bars8 => Some(Tick(bar * 8)),
        }
    }

    /// The first grid point at or after `pos`: a launch asked exactly on a boundary happens
    /// now. [`Quantize::None`] has no grid and gives `Some(pos)` (launch at once). Negative
    /// positions (count-in) use the same grid anchored at tick 0. `None` when the next
    /// grid point lies beyond `i64::MAX`.
    pub fn next_boundary_at_or_after(self, pos: Tick, sig: TimeSignature) -> Option<Tick> {
        match self.grid(sig) {
            None => Some(pos),
            Some(grid) => {
                let rem = pos.0.rem_euclid(grid.0);
                if rem == 0 {
                    Some(pos)
                } else {
                    pos.0.checked_add(grid.0 - rem).map(Tick)
                }
            }
        }
    }

    /// The first grid point strictly after `pos`. `None` when there is none: under
    /// [`Quantize::None`] (no grid, so a loop that steps from boundary to boundary ends at
    /// once instead of spinning on `pos`) or when the next grid point lies beyond
    /// `i64::MAX`.
    pub fn next_boundary_after(self, pos: Tick, sig: TimeSignature) -> Option<Tick> {
        let grid = self.grid(sig)?;
        pos.0
            .checked_add(grid.0 - pos.0.rem_euclid(grid.0))
            .map(Tick)
    }
}

// ---------------------------------------------------------------------------------------
// LoopRegion
// ---------------------------------------------------------------------------------------

/// The transport's loop: a half-open region `[start, end)` in ticks, never empty, and
/// never longer than `i64::MAX` ticks so that its length is itself a [`Tick`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "LoopRegionRaw", into = "LoopRegionRaw")
)]
pub struct LoopRegion {
    start: Tick,
    end: Tick,
}

impl LoopRegion {
    /// A loop region; `None` when `end <= start` or when `end - start` exceeds `i64::MAX`.
    pub const fn new(start: Tick, end: Tick) -> Option<Self> {
        match end.0.checked_sub(start.0) {
            Some(len) if len > 0 => Some(LoopRegion { start, end }),
            _ => None,
        }
    }

    /// The first tick of the loop.
    pub const fn start(self) -> Tick {
        self.start
    }

    /// The tick after the last tick of the loop.
    pub const fn end(self) -> Tick {
        self.end
    }

    /// The length of the loop, in ticks (always positive, exact: `start + len() == end`).
    pub const fn len(self) -> Tick {
        Tick(self.end.0 - self.start.0)
    }

    /// Whether `tick` is inside `[start, end)`.
    pub const fn contains(self, tick: Tick) -> bool {
        tick.0 >= self.start.0 && tick.0 < self.end.0
    }

    /// Maps any tick into the loop: ticks inside are unchanged, the others wrap modulo the
    /// loop length (`end` maps to `start`).
    pub fn wrap(self, tick: Tick) -> Tick {
        if self.contains(tick) {
            return tick;
        }
        // Exact i128 arithmetic: no overflow for any i64 input.
        let start = i128::from(self.start.0);
        let len = i128::from(self.end.0) - start;
        let offset = (i128::from(tick.0) - start).rem_euclid(len);
        Tick((start + offset) as i64)
    }
}

/// The on-disk shape of a [`LoopRegion`], validated on the way in.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LoopRegionRaw {
    /// The first tick of the loop.
    pub start: Tick,
    /// The tick after the last tick of the loop.
    pub end: Tick,
}

impl TryFrom<LoopRegionRaw> for LoopRegion {
    type Error = TimeError;
    fn try_from(raw: LoopRegionRaw) -> Result<Self, TimeError> {
        LoopRegion::new(raw.start, raw.end).ok_or(TimeError::LoopRegion)
    }
}

impl From<LoopRegion> for LoopRegionRaw {
    fn from(region: LoopRegion) -> Self {
        LoopRegionRaw {
            start: region.start,
            end: region.end,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------

/// Why a time value was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    /// A sample rate of zero.
    SampleRate,
    /// A tempo that is not finite or is outside `[20, 999]` BPM.
    Tempo,
    /// A numerator outside `1..=32`, or a denominator that is not a power of two in `1..=32`.
    TimeSignature,
    /// A loop region whose end is not after its start, or longer than `i64::MAX` ticks.
    LoopRegion,
}

impl core::fmt::Display for TimeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            TimeError::SampleRate => "sample rate must be positive",
            TimeError::Tempo => "tempo must be a finite number of BPM between 20 and 999",
            TimeError::TimeSignature => {
                "time signature needs a numerator in 1..=32 and a power-of-two denominator in 1..=32"
            }
            TimeError::LoopRegion => {
                "loop region end must be after its start, at most i64::MAX ticks later"
            }
        })
    }
}

impl std::error::Error for TimeError {}
