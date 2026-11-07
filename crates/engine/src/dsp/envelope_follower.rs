//! An envelope follower: the level of a signal, rising with an attack time and falling with
//! a release time.
//!
//! The caller hands it a rectified level per frame (a peak, such as `max(|left|, |right|)`).
//! The level is clamped to `[0, 1]` (a non-finite one counts as 0), then followed by a
//! one-pole whose coefficient is the attack's while the input is above the state and the
//! release's otherwise: `state = x + c * (state - x)`, `c = exp(-1 / (tau * sample_rate))`.
//! `attack` and `release` are **time constants** (the time to cover 63% of the way). The
//! state is flushed to 0 below 1e-20, so its decay never leaves a denormal. A change of a
//! time recomputes its coefficient and moves nothing else: the level never jumps.

/// A state below this is 0.
const FLOOR: f32 = 1e-20;
/// Shortest time constant, in seconds.
const MIN_SECONDS: f32 = 1e-5;

/// The one-pole coefficient of a time constant `seconds` at `sample_rate`.
pub fn coefficient(seconds: f32, sample_rate: f32) -> f32 {
    let seconds = if seconds.is_finite() {
        seconds.max(MIN_SECONDS)
    } else {
        MIN_SECONDS
    };
    let rate = if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate
    } else {
        48_000.0
    };
    (-1.0 / (f64::from(seconds) * f64::from(rate))).exp() as f32
}

/// The follower's state and its two coefficients.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeFollower {
    level: f32,
    attack: f32,
    release: f32,
}

impl EnvelopeFollower {
    /// A follower at level 0 with these time constants, in seconds.
    pub fn new(attack_seconds: f32, release_seconds: f32, sample_rate: f32) -> Self {
        Self {
            level: 0.0,
            attack: coefficient(attack_seconds, sample_rate),
            release: coefficient(release_seconds, sample_rate),
        }
    }

    /// Sets the attack time constant, in seconds.
    pub fn set_attack(&mut self, seconds: f32, sample_rate: f32) {
        self.attack = coefficient(seconds, sample_rate);
    }

    /// Sets the release time constant, in seconds.
    pub fn set_release(&mut self, seconds: f32, sample_rate: f32) {
        self.release = coefficient(seconds, sample_rate);
    }

    /// Follows one frame's rectified level `x` and returns the new level, in `[0, 1]`.
    #[inline]
    pub fn next(&mut self, x: f32) -> f32 {
        let x = if x.is_finite() {
            x.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let c = if x > self.level {
            self.attack
        } else {
            self.release
        };
        let level = x + c * (self.level - x);
        self.level = if level < FLOOR { 0.0 } else { level.min(1.0) };
        self.level
    }

    /// The level after the last frame.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// Back to level 0; the times are kept.
    pub fn reset(&mut self) {
        self.level = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    #[test]
    fn a_step_reaches_63_percent_after_one_attack_time_and_falls_back_after_one_release() {
        let mut f = EnvelopeFollower::new(0.005, 0.15, SR);
        let attack = (0.005 * SR) as usize;
        let mut level = 0.0;
        for _ in 0..attack {
            level = f.next(1.0);
        }
        assert!((level - 0.632).abs() < 0.01, "{level}");
        for _ in 0..10 * attack {
            f.next(1.0);
        }
        let release = (0.15 * SR) as usize;
        for _ in 0..release {
            level = f.next(0.0);
        }
        assert!((level - 0.368).abs() < 0.01, "{level}");
    }

    #[test]
    fn it_stays_in_range_flushes_and_ignores_garbage() {
        let mut f = EnvelopeFollower::new(0.001, 0.01, SR);
        for x in [f32::NAN, f32::INFINITY, -3.0, 1e9, 2.0] {
            let l = f.next(x);
            assert!((0.0..=1.0).contains(&l), "{x}: {l}");
        }
        for _ in 0..SR as usize {
            f.next(0.0);
        }
        assert_eq!(f.level(), 0.0);
        f.set_attack(f32::NAN, SR);
        f.set_release(-1.0, 0.0);
        assert!(f.next(0.5).is_finite());
        f.reset();
        assert_eq!(f.level(), 0.0);
    }

    #[test]
    fn a_new_time_does_not_move_the_level() {
        let mut f = EnvelopeFollower::new(0.01, 0.1, SR);
        for _ in 0..100 {
            f.next(0.8);
        }
        let before = f.level();
        f.set_attack(0.4, SR);
        f.set_release(1.5, SR);
        assert_eq!(f.level(), before);
    }
}
