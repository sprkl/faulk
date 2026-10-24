//! Analog-style envelope: each stage is a one-pole (an RC charge or discharge), not a line.
//!
//! - **Attack** charges towards 1.3 and stops at 1: a curve that rises fast and bends over
//!   near the top, as a capacitor charging towards a voltage above the comparator's. The
//!   attack time is the time from 0 to 1.
//! - **Decay** and **release** fall exponentially; their time is the time to cover 99 % of
//!   the way (five time constants would be 99.3 %; here `τ = time / 4.6`).
//! - A retriggered note starts its attack from the current level, never from zero.
//!
//! Coefficients are computed at the gate (one `exp` per stage), never per sample.

use crate::math::{exp, flush};

const ATTACK_TARGET: f32 = 1.3;
/// ln(1.3 / 0.3): time constants from 0 to 1 when charging towards 1.3.
const ATTACK_SPAN: f32 = 1.466_337;
/// ln(100): time constants to cover 99 % of the way.
const FALL_SPAN: f32 = 4.605_17;
/// The release ends (the voice is free) below this level, -80 dB.
pub const END_LEVEL: f32 = 1e-4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Release,
}

/// Stage times in seconds and the sustain level (0 – 1).
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Env {
    pub stage: Stage,
    pub level: f32,
    attack: f32,
    decay: f32,
    release: f32,
    sustain: f32,
}

fn coef(seconds: f32, span: f32, rate: f32) -> f32 {
    let tau = (seconds.max(0.0005) / span) * rate;
    1.0 - exp(-1.0 / tau)
}

impl Env {
    /// Starts (or restarts) the attack. `rate` is the rate at which [`Env::tick`] is called,
    /// `scale` stretches every time (per-voice tolerance).
    pub fn gate_on(&mut self, adsr: &Adsr, rate: f32, scale: f32) {
        self.attack = coef(adsr.attack * scale, ATTACK_SPAN, rate);
        self.decay = coef(adsr.decay * scale, FALL_SPAN, rate);
        self.release = coef(adsr.release * scale, FALL_SPAN, rate);
        self.sustain = adsr.sustain.clamp(0.0, 1.0);
        self.stage = Stage::Attack;
    }

    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    #[inline(always)]
    pub fn tick(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += (ATTACK_TARGET - self.level) * self.attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.level += (self.sustain - self.level) * self.decay,
            Stage::Release => {
                self.level -= self.level * self.release;
                if self.level < END_LEVEL {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level = flush(self.level);
        self.level
    }

    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attack_reaches_one_in_its_time() {
        let rate = 48_000.0;
        let mut e = Env::default();
        let adsr = Adsr {
            attack: 0.01,
            decay: 0.1,
            sustain: 0.5,
            release: 0.2,
        };
        e.gate_on(&adsr, rate, 1.0);
        let mut n = 0;
        while e.stage == Stage::Attack {
            e.tick();
            n += 1;
        }
        assert!((470..=490).contains(&n), "{n}");
        for _ in 0..48_000 {
            e.tick();
        }
        assert!((e.level - 0.5).abs() < 1e-3);
        e.gate_off();
        let mut n = 0;
        while !e.is_idle() {
            e.tick();
            n += 1;
        }
        // From 0.5 to 1e-4 is ln(5000) / ln(100) = 1.85 release times.
        assert!((17_000..=18_500).contains(&n), "{n}");
    }
}
