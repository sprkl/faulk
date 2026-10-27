//! What a delay does when its time changes while it sounds: the `time_change` setting shared
//! by the Echo (`faulk.echo`) and the Reverb's pre-delay (`faulk.reverb`), issues #342 and
//! #353. One mechanism, two devices: a device owns a [`DelayMotion`], tells it the delay it
//! wants ([`DelayMotion::set_target`]) and asks, for each frame, where to read its line
//! ([`DelayMotion::advance`]).
//!
//! The three settings (parameter values [`GLIDE`], [`FADE`], [`JUMP`], labels
//! [`VALUES`]):
//!
//! - **Glide** (default): the delay in use moves toward its target one frame at a time, by
//!   the device's [`GlideLaw`]. The pitch of what is read bends while it moves, like a tape,
//!   and never jumps.
//! - **Fade**: the line is read at the old and at the new delay at once and the two are
//!   crossfaded over [`FADE_SECONDS`] (gains on a smoothstep curve, summing to 1: the level
//!   dips by 3 dB mid-fade on unrelated material but never exceeds the loudest tap): the pitch
//!   is kept. A change that arrives while a fade runs waits for it to end, then a new fade
//!   starts toward the latest target (no click, at most [`FADE_SECONDS`] late).
//! - **Jump**: the delay is the target at once, at the frame the change arrives. The output
//!   steps (a click on sustained sound) by design.
//!
//! Switching the setting while a fade runs lets that fade finish first (except to Jump, which
//! cuts it). [`DelayMotion::snap`] puts the delay on its target with no fade: the first block
//! of a device, or after a reset.
//!
//! Nothing here allocates, locks or panics: it is plain arithmetic on a few numbers.

/// The values of the `time_change` parameter.
pub const GLIDE: f32 = 0.0;
pub const FADE: f32 = 1.0;
pub const JUMP: f32 = 2.0;

/// The names of the values, in the order of the parameter's values (labels are
/// `<device>.<name>`).
pub const VALUES: &[&str] = &["glide", "fade", "jump"];

/// Length of the crossfade of the Fade setting, in seconds.
pub const FADE_SECONDS: f32 = 0.05;

/// A distance below this, in frames, is reached.
const REACHED: f64 = 1e-3_f32 as f64;

/// The values of `time_change`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeChange {
    Glide,
    Fade,
    Jump,
}

impl TimeChange {
    /// The setting of a (sanitized) parameter value: an unknown value is Glide.
    pub fn from_value(value: f32) -> Self {
        match value as u32 {
            1 => Self::Fade,
            2 => Self::Jump,
            _ => Self::Glide,
        }
    }
}

/// How the delay moves toward its target under Glide, each frame (a device's own law).
#[derive(Clone, Copy, Debug)]
pub enum GlideLaw {
    /// The Echo's: a share of the distance (an exponential approach), no less than `min` and
    /// no more than `max` frames.
    Exponential { share: f64, min: f64, max: f64 },
    /// The Reverb's: `step` frames at most, whatever the distance (a ramp over a fixed time
    /// would read the line several times faster than real time on a large move).
    Constant { step: f64 },
}

/// Where to read the line for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Read {
    /// `delay` frames back.
    At(f64),
    /// Both `from` and `to` frames back, mixed with gains that sum to 1.
    Crossfade {
        from: f64,
        to: f64,
        from_gain: f32,
        to_gain: f32,
    },
}

/// The delay in use, the delay it moves toward and the crossfade in progress.
#[derive(Debug)]
pub struct DelayMotion {
    law: GlideLaw,
    mode: TimeChange,
    /// The delay in use and the one it moves toward, in frames (`f64`: a step of 0.002 frame
    /// must still move a delay of several seconds).
    delay: f64,
    target: f64,
    /// A crossfade runs between the delays `from` and `to`, `pos` frames in out of `len`.
    fading: bool,
    from: f64,
    to: f64,
    pos: u32,
    len: u32,
}

impl DelayMotion {
    /// A motion at rest on `delay` frames, Glide, with the Glide law of the device.
    pub fn new(sample_rate: f32, law: GlideLaw, delay: f64) -> Self {
        Self {
            law,
            mode: TimeChange::Glide,
            delay,
            target: delay,
            fading: false,
            from: delay,
            to: delay,
            pos: 0,
            len: ((FADE_SECONDS * sample_rate).round() as u32).max(1),
        }
    }

    /// The delay the device wants, in frames (already within its line).
    pub fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    /// The setting. Jump cuts a running fade (the delay lands on the target); the others let
    /// it finish.
    pub fn set_mode(&mut self, mode: TimeChange) {
        self.mode = mode;
        if mode == TimeChange::Jump {
            self.fading = false;
        }
    }

    /// The delay lands on the target, with no fade.
    pub fn snap(&mut self) {
        self.delay = self.target;
        self.fading = false;
    }

    /// Moves the delay by one frame as the setting says and tells where to read.
    pub fn advance(&mut self) -> Read {
        if self.fading {
            // A fade runs (Fade, or a setting left while it ran): it finishes first.
            let s = self.pos as f32 / self.len as f32;
            let curve = s * s * (3.0 - 2.0 * s);
            // The gains sum to 1: the mix never exceeds its loudest tap, so a feedback loop
            // gain stays below 1 even when the taps are correlated (a bass, a small step).
            let read = Read::Crossfade {
                from: self.from,
                to: self.to,
                from_gain: 1.0 - curve,
                to_gain: curve,
            };
            self.pos += 1;
            if self.pos >= self.len {
                self.fading = false;
                self.delay = self.to;
            }
            return read;
        }
        let diff = self.target - self.delay;
        let distance = diff.abs();
        match self.mode {
            TimeChange::Jump => self.delay = self.target,
            TimeChange::Fade if distance > REACHED => {
                self.fading = true;
                self.from = self.delay;
                self.to = self.target;
                self.pos = 0;
                return self.advance();
            }
            _ => match self.law {
                GlideLaw::Exponential { share, min, max } => {
                    if distance <= REACHED {
                        self.delay = self.target;
                    } else {
                        let step = (distance * share).clamp(min, max).min(distance);
                        self.delay += step.copysign(diff);
                    }
                }
                GlideLaw::Constant { step } => {
                    // In `f32`, rounded at every step, as the Reverb's pre-delay always was:
                    // an `f64` accumulator would land one ulp away on some glides.
                    let (delay, target) = (self.delay as f32, self.target as f32);
                    let step = step as f32;
                    self.delay = f64::from(delay + (target - delay).clamp(-step, step));
                }
            },
        }
        Read::At(self.delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAW: GlideLaw = GlideLaw::Constant { step: 0.25 };

    #[test]
    fn values_round_trip_and_unknown_is_glide() {
        assert_eq!(TimeChange::from_value(GLIDE), TimeChange::Glide);
        assert_eq!(TimeChange::from_value(FADE), TimeChange::Fade);
        assert_eq!(TimeChange::from_value(JUMP), TimeChange::Jump);
        assert_eq!(TimeChange::from_value(f32::NAN), TimeChange::Glide);
        assert_eq!(TimeChange::from_value(-3.0), TimeChange::Glide);
        assert_eq!(VALUES.len(), 3);
    }

    #[test]
    fn glide_moves_by_the_law_and_lands_on_the_target() {
        let mut m = DelayMotion::new(48_000.0, LAW, 100.0);
        m.set_target(101.0);
        let seen: Vec<Read> = (0..6).map(|_| m.advance()).collect();
        assert_eq!(seen[0], Read::At(100.25));
        assert_eq!(seen[3], Read::At(101.0));
        assert_eq!(seen[5], Read::At(101.0));
    }

    #[test]
    fn jump_lands_at_once_and_cuts_a_fade() {
        let mut m = DelayMotion::new(48_000.0, LAW, 100.0);
        m.set_mode(TimeChange::Fade);
        m.set_target(500.0);
        assert!(matches!(m.advance(), Read::Crossfade { .. }));
        m.set_mode(TimeChange::Jump);
        assert_eq!(m.advance(), Read::At(500.0));
    }

    #[test]
    fn fade_gains_sum_to_one_and_the_fade_ends_on_the_target() {
        let mut m = DelayMotion::new(48_000.0, LAW, 100.0);
        m.set_mode(TimeChange::Fade);
        m.set_target(500.0);
        for _ in 0..2_400 {
            let Read::Crossfade {
                from,
                to,
                from_gain,
                to_gain,
            } = m.advance()
            else {
                panic!("a fade runs")
            };
            assert_eq!((from, to), (100.0, 500.0));
            assert!((from_gain + to_gain - 1.0).abs() < 1e-6);
            assert!((0.0..=1.0).contains(&to_gain));
        }
        assert_eq!(m.advance(), Read::At(500.0));
    }

    #[test]
    fn a_change_during_a_fade_waits_then_fades_toward_the_latest() {
        let mut m = DelayMotion::new(48_000.0, LAW, 100.0);
        m.set_mode(TimeChange::Fade);
        m.set_target(500.0);
        let _ = m.advance();
        m.set_target(300.0);
        for _ in 1..2_400 {
            let Read::Crossfade { to, .. } = m.advance() else {
                panic!("the first fade runs to its end")
            };
            assert_eq!(to, 500.0);
        }
        let Read::Crossfade { from, to, .. } = m.advance() else {
            panic!("a second fade starts")
        };
        assert_eq!((from, to), (500.0, 300.0));
    }

    #[test]
    fn snap_puts_the_delay_on_the_target_and_ends_a_fade() {
        let mut m = DelayMotion::new(48_000.0, LAW, 100.0);
        m.set_mode(TimeChange::Fade);
        m.set_target(500.0);
        let _ = m.advance();
        m.snap();
        assert_eq!(m.advance(), Read::At(500.0));
    }
}
