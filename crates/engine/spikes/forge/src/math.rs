//! Small numeric helpers shared by the blocks: a rational `tanh`, a seeded random source and
//! the denormal flush. Everything here is pure arithmetic: no allocation, no panic.

/// Below this magnitude a filter or smoothing state is set to zero (denormal guard).
pub const FLUSH_FLOOR: f32 = 1e-15;

/// A rational approximation of `tanh`, exact at 0 and ±3 with a continuous first derivative
/// there, and clamped to ±1 beyond: `x (27 + x²) / (27 + 9 x²)`. Within 2.5 % of `tanh`
/// (tested), and the same bits natively and in WASM (it uses no library function). A NaN input stays NaN: callers keep their inputs finite.
#[inline(always)]
pub fn tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    (x * (27.0 + x2) / (27.0 + 9.0 * x2)).clamp(-1.0, 1.0)
}

/// Zero when `x` is so small that it would soon become a denormal.
#[inline(always)]
pub fn flush(x: f32) -> f32 {
    if x.abs() < FLUSH_FLOOR { 0.0 } else { x }
}

/// Frequency in hertz of a fractional MIDI key (69 = A4 = 440 Hz).
#[inline]
pub fn key_hz(key: f32) -> f32 {
    440.0 * exp2((key - 69.0) / 12.0)
}

// The transcendental functions below use only `+ − × ÷`, `floor` and bit operations, which
// IEEE 754 makes exact or correctly rounded on every target: they give the same bits
// natively and in WASM, where the library functions (`f32::exp2`, `tan`, `sin`…) come from
// different math libraries and differ in the last bit. A last-bit difference is harmless in
// a smooth computation, but a synth makes discrete decisions on its signals (an envelope
// ends one sample earlier, so a later note takes another voice slot with other tolerances)
// that turn it into a different sound. See the spike report, "Parité".

/// `2^x`, relative error under 1e-7 for |x| ≤ 126 (clamped beyond).
#[inline]
pub fn exp2(x: f32) -> f32 {
    let x = x.clamp(-126.0, 126.0);
    let i = (x + 0.5).floor();
    let f = (x - i) * core::f32::consts::LN_2; // |f| ≤ ln2 / 2
    // e^f by its Taylor series to the 7th degree (error < 4e-9 on this range).
    let p = 1.0
        + f * (1.0
            + f * (0.5
                + f * (1.0 / 6.0
                    + f * (1.0 / 24.0 + f * (1.0 / 120.0 + f * (1.0 / 720.0 + f / 5040.0))))));
    p * f32::from_bits(((i as i32 + 127) as u32) << 23)
}

/// `e^x` (through [`exp2`]).
#[inline]
pub fn exp(x: f32) -> f32 {
    exp2(x * core::f32::consts::LOG2_E)
}

/// `log2(x)` for `x > 0` (clamped to 1e-30 below), absolute error under 1e-6.
#[inline]
pub fn log2(x: f32) -> f32 {
    let bits = x.max(1e-30).to_bits();
    let mut e = ((bits >> 23) & 0xff) as i32 - 127;
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000); // [1, 2)
    if m > core::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    // ln m = 2 atanh(z), z = (m − 1) / (m + 1), |z| ≤ 0.172.
    let z = (m - 1.0) / (m + 1.0);
    let z2 = z * z;
    let ln = 2.0 * z * (1.0 + z2 * (1.0 / 3.0 + z2 * (0.2 + z2 * (1.0 / 7.0 + z2 / 9.0))));
    e as f32 + ln * core::f32::consts::LOG2_E
}

/// `sin(θ)` for |θ| ≤ π/2, Taylor to the 13th degree.
#[inline]
fn sin_poly(t: f32) -> f32 {
    let t2 = t * t;
    t * (1.0
        - t2 / 6.0
            * (1.0
                - t2 / 20.0
                    * (1.0
                        - t2 / 42.0 * (1.0 - t2 / 72.0 * (1.0 - t2 / 110.0 * (1.0 - t2 / 156.0))))))
}

/// `cos(θ)` for |θ| ≤ π/4, Taylor to the 12th degree.
#[inline]
fn cos_poly(t: f32) -> f32 {
    let t2 = t * t;
    1.0 - t2 / 2.0
        * (1.0
            - t2 / 12.0
                * (1.0 - t2 / 30.0 * (1.0 - t2 / 56.0 * (1.0 - t2 / 90.0 * (1.0 - t2 / 132.0)))))
}

/// `sin(2π · turns)`, for any finite `turns`.
#[inline]
pub fn sin_turns(turns: f32) -> f32 {
    let mut t = turns - (turns + 0.5).floor(); // [−0.5, 0.5)
    if t > 0.25 {
        t = 0.5 - t;
    } else if t < -0.25 {
        t = -0.5 - t;
    }
    sin_poly(core::f32::consts::TAU * t)
}

/// `cos(2π · turns)`.
#[inline]
pub fn cos_turns(turns: f32) -> f32 {
    sin_turns(turns + 0.25)
}

/// `tan(π · x)` for `x` in [0, 0.25] (clamped): the prewarping of a cutoff `x = fc / fs`.
#[inline]
pub fn tan_pi(x: f32) -> f32 {
    let t = core::f32::consts::PI * x.clamp(0.0, 0.25);
    sin_poly(t) / cos_poly(t)
}

/// A deterministic random source (SplitMix64). Same seed, same sequence, on every target.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u64);

impl Rng {
    pub const fn new(seed: u64) -> Self {
        Rng(seed)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in [0, 1).
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A value in [-1, 1).
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tanh_is_close_and_bounded() {
        let mut worst = 0.0f32;
        for i in -1000..=1000 {
            let x = i as f32 / 100.0;
            let t = tanh(x);
            assert!(t.abs() <= 1.0);
            worst = worst.max((t - x.tanh()).abs());
        }
        assert!(worst < 0.025, "worst {worst}");
    }

    #[test]
    fn portable_functions_match_the_library() {
        for i in -2000..=2000 {
            let x = i as f32 / 100.0;
            let rel = |a: f32, b: f32| ((a - b) / b).abs();
            assert!(rel(exp2(x), x.exp2()) < 3e-7, "exp2 {x}");
            assert!(rel(exp(x / 4.0), (x / 4.0).exp()) < 3e-7, "exp {x}");
            assert!(
                (sin_turns(x / 7.0) - (core::f32::consts::TAU * x / 7.0).sin()).abs() < 1e-5,
                "sin {x}"
            );
            assert!(
                (cos_turns(x / 7.0) - (core::f32::consts::TAU * x / 7.0).cos()).abs() < 1e-5,
                "cos {x}"
            );
        }
        for i in 1..=10_000 {
            let x = i as f32 * 0.37;
            assert!((log2(x) - x.log2()).abs() < 2e-6, "log2 {x}");
        }
        for i in 0..=250 {
            let x = i as f32 / 1000.0;
            let lib = (core::f32::consts::PI * x).tan();
            assert!(
                ((tan_pi(x) - lib) / lib.max(1e-6)).abs() < 1e-6 || lib < 1e-6,
                "tan {x}"
            );
        }
    }

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..1000 {
            let x = a.bipolar();
            assert_eq!(x, b.bipolar());
            assert!((-1.0..1.0).contains(&x));
        }
    }
}
