//! The 2× decimator: a 63-tap half-band FIR (Kaiser window, β = 9), polyphase. Half its taps
//! are zero, so one output costs 16 multiplications and a few additions. At 96 kHz in, the
//! transition band computed from the window is about 19.5 – 28.5 kHz; tested: flat within
//! 0.1 dB to 18 kHz, at least 80 dB down at 30 and 40 kHz. Latency: 31 input
//! samples (15.5 output samples). It runs once per channel on the sum of the voices (the
//! decimation is linear), so its cost does not grow with the voices.

const TAPS: usize = 63;
const HALF: usize = TAPS / 2; // 31: the centre tap.
const ODD: usize = 16; // taps at odd offsets 1, 3, …, 31 on each side.
const RING: usize = 64;

/// Bessel I0, by its series (construction only).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..50 {
        term *= q / (k * k) as f64;
        sum += term;
    }
    sum
}

#[derive(Clone, Debug)]
pub struct Decimator {
    odd: [f32; ODD],
    /// The last `RING` inputs, written twice (at `pos` and `pos + RING`) so that a window of
    /// 63 consecutive samples is always one contiguous slice.
    buf: [f32; RING * 2],
    pos: usize,
}

impl Default for Decimator {
    fn default() -> Self {
        Self::new()
    }
}

impl Decimator {
    pub fn new() -> Self {
        let beta = 9.0;
        let i0b = bessel_i0(beta);
        let mut odd = [0.0f64; ODD];
        for (j, tap) in odd.iter_mut().enumerate() {
            let k = (2 * j + 1) as f64;
            let r = k / HALF as f64;
            let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0b;
            // sin(π k / 2) is +1, −1, +1… for k = 1, 3, 5… (exact, no library call).
            let sign = if j % 2 == 0 { 1.0 } else { -1.0 };
            let sinc = sign / (core::f64::consts::PI * k);
            *tap = sinc * w;
        }
        // Unity gain at DC: 0.5 + 2 Σ odd = 1.
        let sum: f64 = odd.iter().sum();
        let scale = 0.25 / sum;
        let mut taps = [0.0f32; ODD];
        for (t, o) in taps.iter_mut().zip(odd) {
            *t = (o * scale) as f32;
        }
        Decimator {
            odd: taps,
            buf: [0.0; RING * 2],
            pos: 0,
        }
    }

    pub fn has_subnormal(&self) -> bool {
        self.buf.iter().any(|v| v.is_subnormal())
    }

    #[inline(always)]
    fn push(&mut self, x: f32) {
        self.pos = (self.pos + 1) % RING;
        self.buf[self.pos] = x;
        self.buf[self.pos + RING] = x;
    }

    /// Takes two input samples (2× rate) and returns one output sample.
    #[inline]
    pub fn process(&mut self, x0: f32, x1: f32) -> f32 {
        self.push(x0);
        self.push(x1);
        // The newest sample is at `pos + RING`; the window is the 63 samples ending there.
        let end = self.pos + RING;
        let w = &self.buf[end + 1 - TAPS..=end];
        let mut acc = 0.5 * w[HALF];
        for j in 0..ODD {
            let k = 2 * j + 1;
            acc += self.odd[j] * (w[HALF - k] + w[HALF + k]);
        }
        crate::math::flush(acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(hz: f32) -> f32 {
        let fs = 96_000.0f64;
        let mut d = Decimator::new();
        let mut peak = 0.0f32;
        for i in 0..4_000 {
            // In f64: an f32 phase this large would add its own noise floor.
            let w = core::f64::consts::TAU * hz as f64 / fs;
            let y = d.process(
                (w * (2 * i) as f64).sin() as f32,
                (w * (2 * i + 1) as f64).sin() as f32,
            );
            if i > 200 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn passes_audio_and_stops_the_image_band() {
        assert!((gain(1_000.0) - 1.0).abs() < 0.001);
        assert!((gain(18_000.0) - 1.0).abs() < 0.01);
        assert!(gain(30_000.0) < 1e-4, "{}", gain(30_000.0));
        assert!(gain(40_000.0) < 1e-4);
    }
}
