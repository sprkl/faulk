//! Measurement helpers (off the audio path): an FFT, the aliasing report of a periodic tone,
//! signal checks, and a WAV writer.

use crate::halfband::Decimator;
use crate::osc::{NO_SYNC, Osc};
use crate::synth::Oversample;

/// In-place radix-2 FFT of `re`/`im` (length a power of two).
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * core::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let a = start + k;
                let b = a + len / 2;
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Power spectrum (one side) of `x` under a four-term Blackman-Harris window.
pub fn power_spectrum(x: &[f32]) -> Vec<f64> {
    let n = x.len();
    let mut re = vec![0.0; n];
    let mut im = vec![0.0; n];
    for (i, (r, &v)) in re.iter_mut().zip(x).enumerate() {
        let t = 2.0 * core::f64::consts::PI * i as f64 / n as f64;
        let w = 0.35875 - 0.48829 * t.cos() + 0.14128 * (2.0 * t).cos() - 0.01168 * (3.0 * t).cos();
        *r = v as f64 * w;
    }
    fft(&mut re, &mut im);
    (0..n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect()
}

/// Aliasing of a tone of fundamental `f0`: everything that is not near a harmonic.
#[derive(Clone, Copy, Debug)]
pub struct AliasReport {
    /// Loudest alias component against the fundamental, in dB (dBc).
    pub worst_dbc: f64,
    pub worst_hz: f64,
    /// Loudest alias component below `audible_hz`, in dBc.
    pub worst_audible_dbc: f64,
    pub worst_audible_hz: f64,
    /// Total alias power against total harmonic power, in dB.
    pub alias_to_harmonics_db: f64,
}

pub fn alias_report(x: &[f32], fs: f64, f0: f64, audible_hz: f64) -> AliasReport {
    let p = power_spectrum(x);
    let n = x.len() as f64;
    let df = fs / n;
    // Main lobe of the window: ±4 bins; keep a little more.
    let lobe = 6.0 * df;
    let fund_bin = (f0 / df).round() as usize;
    let fund = p[fund_bin.saturating_sub(6)..=fund_bin + 6]
        .iter()
        .cloned()
        .fold(0.0, f64::max);
    let (mut harm, mut alias) = (0.0, 0.0);
    let (mut worst, mut worst_hz, mut worst_a, mut worst_a_hz) = (0.0f64, 0.0, 0.0f64, 0.0);
    for (k, &pk) in p.iter().enumerate() {
        let f = k as f64 * df;
        if f < 30.0 {
            continue;
        }
        let h = (f / f0).round();
        if h >= 1.0 && (f - h * f0).abs() <= lobe {
            harm += pk;
        } else {
            alias += pk;
            if pk > worst {
                worst = pk;
                worst_hz = f;
            }
            if f < audible_hz && pk > worst_a {
                worst_a = pk;
                worst_a_hz = f;
            }
        }
    }
    let db = |r: f64| 10.0 * r.max(1e-30).log10();
    AliasReport {
        worst_dbc: db(worst / fund),
        worst_hz,
        worst_audible_dbc: db(worst_a / fund),
        worst_audible_hz: worst_a_hz,
        alias_to_harmonics_db: db(alias / harm),
    }
}

/// The oscillator under test for the aliasing measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SawKind {
    Naive,
    PolyBlep,
}

/// A saw at `hz`, rendered at `fs × os` and brought back to `fs` (2×: the half-band
/// decimator), `frames` output frames after a warm-up.
pub fn saw(kind: SawKind, os: Oversample, hz: f32, fs: f32, frames: usize) -> Vec<f32> {
    let rate = fs * os.factor() as f32;
    let inc = hz / rate;
    let mut osc = Osc::new(0.0);
    let mut dec = Decimator::new();
    let next = |osc: &mut Osc| match kind {
        SawKind::Naive => osc.step_naive_saw(inc),
        SawKind::PolyBlep => osc.step(inc, 1.0, 0.0, 0.5, NO_SYNC).0,
    };
    let warm = 256;
    let mut out = Vec::with_capacity(frames);
    for i in 0..frames + warm {
        let y = match os {
            Oversample::X1 => next(&mut osc),
            Oversample::X2 => {
                let a = next(&mut osc);
                let b = next(&mut osc);
                dec.process(a, b)
            }
        };
        if i >= warm {
            out.push(y);
        }
    }
    out
}

/// Peak magnitude.
pub fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

pub fn rms(x: &[f32]) -> f32 {
    let s: f64 = x.iter().map(|&v| (v as f64) * (v as f64)).sum();
    (s / x.len().max(1) as f64).sqrt() as f32
}

/// Number of samples that are NaN or infinite, and that are subnormal (non-zero and below
/// the smallest normal f32).
pub fn bad_samples(x: &[f32]) -> (usize, usize) {
    let non_finite = x.iter().filter(|v| !v.is_finite()).count();
    let denormal = x.iter().filter(|v| v.is_subnormal()).count();
    (non_finite, denormal)
}

/// A 24-bit stereo PCM WAV file.
pub fn wav_24(left: &[f32], right: &[f32], fs: u32) -> Vec<u8> {
    let frames = left.len().min(right.len());
    let data = frames * 6;
    let mut out = Vec::with_capacity(44 + data);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&fs.to_le_bytes());
    out.extend_from_slice(&(fs * 6).to_le_bytes());
    out.extend_from_slice(&6u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    for i in 0..frames {
        for v in [left[i], right[i]] {
            let s = (v.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
            out.extend_from_slice(&s.to_le_bytes()[..3]);
        }
    }
    out
}
