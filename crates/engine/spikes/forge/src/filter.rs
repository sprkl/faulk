//! The two filter models: a nonlinear four-pole ladder low-pass and a two-pole state-variable
//! filter, both zero-delay-feedback (topology-preserving transform: trapezoidal integrators,
//! the feedback loop solved within the sample instead of delayed by one).
//!
//! Both take the prewarped coefficient `g = tan(π · fc / fs)`, which the voice computes at
//! control rate and interpolates per sample. The caller keeps `g ≤ 1` (cutoff ≤ fs / 4):
//! then each one-pole stage of the ladder has `G = g / (1 + g) ≤ 1/2` and maps an input
//! bounded by 1 to an output and a state bounded by 1, so the ladder's output is bounded by
//! construction whatever the resonance (see [`Ladder::process`]).

use crate::math::{flush, tan_pi, tanh};

/// The highest feedback of the ladder (resonance 1). Above 4 the linear filter
/// self-oscillates; the `tanh` in the loop bounds the oscillation.
pub const LADDER_MAX_K: f32 = 4.3;

/// Four-pole (24 dB/oct) ladder low-pass with one saturating stage in the feedback loop.
///
/// The loop input is `u = tanh(x − k · y4)`, where `y4` depends on `u` within the same
/// sample. Linear prediction of `u`, one Newton step on `u − tanh(x − k (G⁴ u + Σ)) = 0`,
/// then a final evaluation of the `tanh`: so `|u| ≤ 1` always, and with `G ≤ 1/2` every
/// stage stays within ±1. The Newton step divides by `1 + k G⁴ (1 − t²) ≥ 1`, never by
/// zero. Passband loss at high resonance is partly made up at the input (`comp`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Ladder {
    s: [f32; 4],
}

impl Ladder {
    pub fn reset(&mut self) {
        self.s = [0.0; 4];
    }

    /// Filters one sample. `big_g` the stage gain `G = g / (1 + g)` (≤ 1/2, see
    /// [`stage_gain`]; the voice interpolates `G` itself, which saves a division per
    /// sample), `k` the feedback (0 – [`LADDER_MAX_K`]), `x` the input already scaled by
    /// the drive.
    #[inline(always)]
    pub fn process(&mut self, x: f32, big_g: f32, k: f32) -> f32 {
        let inv = 1.0 - big_g;
        let [s1, s2, s3, s4] = self.s;
        let g2 = big_g * big_g;
        let g4 = g2 * g2;
        // y4 = G⁴ u + Σ, Σ from the states.
        let sigma = (big_g * g2 * s1 + g2 * s2 + big_g * s3 + s4) * inv;
        let input = x * (1.0 + 0.5 * k);
        // Linear solution, then one Newton step on the saturated loop.
        let mut u = tanh((input - k * sigma) / (1.0 + k * g4));
        let t = tanh(input - k * (g4 * u + sigma));
        u -= (u - t) / (1.0 + k * g4 * (1.0 - t * t));
        let u = tanh(input - k * (g4 * u + sigma));

        let v1 = (u - s1) * big_g;
        let y1 = v1 + s1;
        let v2 = (y1 - s2) * big_g;
        let y2 = v2 + s2;
        let v3 = (y2 - s3) * big_g;
        let y3 = v3 + s3;
        let v4 = (y3 - s4) * big_g;
        let y4 = v4 + s4;
        self.s = [y1 + v1, y2 + v2, y3 + v3, y4 + v4];
        y4
    }

    pub fn has_subnormal(&self) -> bool {
        self.s.iter().any(|v| v.is_subnormal())
    }

    /// Flushes the states that decayed towards denormals (once per control block).
    pub fn flush(&mut self) {
        for s in &mut self.s {
            *s = flush(*s);
        }
    }
}

/// Two-pole (12 dB/oct) state-variable filter, linear core (the trapezoidal SVF), low-pass
/// output. Its character comes from the saturation around it (input drive, output stage),
/// not from inside the loop: softer and cleaner than the ladder. Damping never goes below
/// [`SVF_MIN_DAMPING`] (Q = 50), so it rings but does not self-oscillate.
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

/// The lowest damping `k = 1/Q` of the state-variable filter (resonance 1).
pub const SVF_MIN_DAMPING: f32 = 0.02;

impl Svf {
    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    /// Filters one sample. `g` the prewarped coefficient, `k` the damping (1/Q).
    #[inline(always)]
    pub fn process(&mut self, x: f32, g: f32, k: f32) -> f32 {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + g * a1 * v3;
        let v2 = self.ic2 + g * v1;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v2
    }

    pub fn has_subnormal(&self) -> bool {
        self.ic1.is_subnormal() || self.ic2.is_subnormal()
    }

    pub fn flush(&mut self) {
        self.ic1 = flush(self.ic1);
        self.ic2 = flush(self.ic2);
    }
}

/// The ladder's stage gain `G = g / (1 + g)` for a prewarped coefficient `g`.
#[inline]
pub fn stage_gain(g: f32) -> f32 {
    g / (1.0 + g)
}

/// The prewarped coefficient of a cutoff, clamped to 10 Hz – `fs / 4`.
#[inline]
pub fn coefficient(cutoff_hz: f32, fs: f32) -> f32 {
    let fc = cutoff_hz.clamp(10.0, fs * 0.25);
    tan_pi(fc / fs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gain of the filter at `hz` for a small sine (the linear regime), in dB.
    fn gain_db(mut f: impl FnMut(f32) -> f32, hz: f32, fs: f32) -> f32 {
        let n = (fs * 0.5) as usize;
        let mut peak = 0.0f32;
        for i in 0..n {
            let x = 0.01 * (core::f32::consts::TAU * hz * i as f32 / fs).sin();
            let y = f(x);
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        20.0 * (peak / 0.01).log10()
    }

    #[test]
    fn ladder_rolls_off_24_db_per_octave() {
        let fs = 96_000.0;
        let g = coefficient(500.0, fs);
        let mut l = Ladder::default();
        let at2k = gain_db(|x| l.process(x, stage_gain(g), 0.0), 2_000.0, fs);
        l.reset();
        let at4k = gain_db(|x| l.process(x, stage_gain(g), 0.0), 4_000.0, fs);
        let slope = at2k - at4k;
        assert!((21.0..27.0).contains(&slope), "slope {slope} dB/oct");
    }

    #[test]
    fn svf_rolls_off_12_db_per_octave() {
        let fs = 96_000.0;
        let g = coefficient(500.0, fs);
        let mut f = Svf::default();
        let at2k = gain_db(|x| f.process(x, g, 1.414), 2_000.0, fs);
        f.reset();
        let at4k = gain_db(|x| f.process(x, g, 1.414), 4_000.0, fs);
        let slope = at2k - at4k;
        assert!((10.0..14.0).contains(&slope), "slope {slope} dB/oct");
    }
}
