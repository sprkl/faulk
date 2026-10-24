//! A band-limited oscillator: a blend of saw and pulse, with hard sync, band-limited by a
//! two-point PolyBLEP.
//!
//! Each discontinuity of the naive waveform (the saw's wrap, the pulse's two edges, a sync
//! reset) is located at its fractional time inside the sample interval, and a polynomial
//! residual is added to the two samples around it. The residual of the sample *before* the
//! step is only known once the step is seen, so the oscillator runs one sample late: it
//! returns the previous sample with its residual and keeps the current one pending. Every
//! oscillator of a voice runs one sample late, so they stay aligned, sync included.
//!
//! For a step of height `h` at a fraction `τ` of the interval after sample `n − 1`, the
//! residuals are `+h/2 · (1 − τ)²` on sample `n − 1` and `−h/2 · τ²` on sample `n` (the
//! classic two-point PolyBLEP, written for an arbitrary step).

/// Phase increments are clamped below this (cycles per sample), so that at most one wrap
/// happens per sample.
pub const MAX_INC: f32 = 0.45;

/// One oscillator: phase in [0, 1) and the pending (one sample late) output.
#[derive(Clone, Copy, Debug, Default)]
pub struct Osc {
    phase: f32,
    pending: f32,
}

/// No sync event in this sample (any value ≥ 1 means none).
pub const NO_SYNC: f32 = 2.0;

impl Osc {
    pub fn new(phase: f32) -> Self {
        Osc {
            phase: phase.rem_euclid(1.0),
            pending: 0.0,
        }
    }

    /// Advances one sample and returns `(output, wrap)`: the output of the previous sample,
    /// and the fraction of this interval at which the phase wrapped (≥ 1 when it did not),
    /// to drive a synced oscillator.
    ///
    /// `inc` is the frequency in cycles per sample, `saw` and `pulse` the weights of the two
    /// waveforms, `pw` the pulse width (0.05 – 0.95), `sync` the fraction at which a master
    /// oscillator wrapped in this interval ([`NO_SYNC`] for none).
    #[inline(always)]
    pub fn step(&mut self, inc: f32, saw: f32, pulse: f32, pw: f32, sync: f32) -> (f32, f32) {
        let inc = inc.clamp(1e-7, MAX_INC);
        let pw = pw.clamp(0.05, 0.95);
        let p0 = self.phase;
        let end = sync.min(1.0);
        let mut before = 0.0f32;
        let mut after = 0.0f32;
        let mut edge = |tau: f32, h: f32| {
            let a = 1.0 - tau;
            before += 0.5 * h * a * a;
            after -= 0.5 * h * tau * tau;
        };

        let mut wrap = NO_SYNC;
        // The pulse's falling edge at `pw`.
        if p0 < pw {
            let tau = (pw - p0) / inc;
            if tau < end {
                edge(tau, -2.0 * pulse);
            }
        }
        // The wrap at 1: the saw falls by 2, the pulse rises by 2; then maybe the next
        // falling edge.
        let unwrapped = p0 + inc;
        if unwrapped >= 1.0 {
            let tau = (1.0 - p0) / inc;
            if tau < end {
                edge(tau, 2.0 * pulse - 2.0 * saw);
                wrap = tau;
                let tau2 = (1.0 + pw - p0) / inc;
                if tau2 < end {
                    edge(tau2, -2.0 * pulse);
                }
            }
        }

        let next = if sync < 1.0 {
            // Hard sync: back to phase 0 at `sync`, from wherever the phase was.
            let mut ps = p0 + inc * sync;
            if ps >= 1.0 {
                ps -= 1.0;
            }
            let saw_value = 2.0 * ps - 1.0;
            let pulse_value = if ps < pw { 1.0 } else { -1.0 };
            edge(sync, saw * (-1.0 - saw_value) + pulse * (1.0 - pulse_value));
            (1.0 - sync) * inc
        } else if unwrapped >= 1.0 {
            unwrapped - 1.0
        } else {
            unwrapped
        };
        self.phase = next;

        let naive = saw * (2.0 * next - 1.0) + pulse * if next < pw { 1.0 } else { -1.0 };
        let out = self.pending + before;
        self.pending = naive + after;
        (out, wrap)
    }

    /// The naive (aliasing) saw, for the aliasing comparison only.
    #[inline(always)]
    pub fn step_naive_saw(&mut self, inc: f32) -> f32 {
        let mut p = self.phase + inc;
        if p >= 1.0 {
            p -= 1.0;
        }
        self.phase = p;
        2.0 * p - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saw_stays_bounded_and_centred() {
        let mut osc = Osc::new(0.0);
        let mut sum = 0.0f64;
        let n = 48_000;
        for _ in 0..n {
            let (y, _) = osc.step(4100.0 / 48_000.0, 1.0, 0.0, 0.5, NO_SYNC);
            assert!(y.abs() <= 1.2, "{y}");
            sum += y as f64;
        }
        assert!((sum / n as f64).abs() < 0.01);
    }

    #[test]
    fn sync_resets_the_slave_at_the_master_rate() {
        let mut master = Osc::new(0.0);
        let mut slave = Osc::new(0.0);
        let mut wraps = 0;
        for _ in 0..48_000 {
            let (_, w) = master.step(100.0 / 48_000.0, 1.0, 0.0, 0.5, NO_SYNC);
            if w < 1.0 {
                wraps += 1;
            }
            let (y, _) = slave.step(370.0 / 48_000.0, 1.0, 0.0, 0.5, w);
            assert!(y.is_finite() && y.abs() <= 1.5);
        }
        assert!((99..=101).contains(&wraps));
    }
}
