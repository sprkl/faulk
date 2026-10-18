//! A rendered signal and the assertions on it.

use std::ops::Range;
use std::path::PathBuf;

/// Seconds-based assertions panic with a message that names the window and the value, so
/// a failing test says what it measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Render {
    pub sample_rate: f32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

/// The magnitude spectrum of a window of the left channel.
#[derive(Clone, Debug, PartialEq)]
pub struct Spectrum {
    /// Width of a bin, in Hz.
    pub bin_hz: f32,
    /// Magnitudes of bins `0..=n/2`, normalised so that a full-scale sine reads about 1.
    pub magnitudes: Vec<f32>,
}

impl Spectrum {
    /// The frequency of the strongest bin above DC, refined by parabolic interpolation.
    pub fn dominant_hz(&self) -> f32 {
        let m = &self.magnitudes;
        let (k, _) = m
            .iter()
            .enumerate()
            .skip(1)
            .fold((1, 0.0f32), |b, (i, v)| if *v > b.1 { (i, *v) } else { b });
        let offset = if k > 1 && k + 1 < m.len() {
            let (a, b, c) = (m[k - 1], m[k], m[k + 1]);
            let d = a - 2.0 * b + c;
            if d.abs() > f32::EPSILON {
                0.5 * (a - c) / d
            } else {
                0.0
            }
        } else {
            0.0
        };
        (k as f32 + offset) * self.bin_hz
    }

    /// The magnitude of the strongest bin within `band` Hz (it reads up to 1.4 dB low when
    /// the frequency falls between two bins: the Hann window's scalloping loss).
    pub fn peak_in(&self, band: Range<f32>) -> f32 {
        self.magnitudes
            .iter()
            .enumerate()
            .filter(|(i, _)| band.contains(&(*i as f32 * self.bin_hz)))
            .fold(0.0f32, |m, (_, v)| m.max(*v))
    }
}

impl Render {
    pub fn new(sample_rate: f32, left: Vec<f32>, right: Vec<f32>) -> Render {
        assert_eq!(left.len(), right.len(), "the channels differ in length");
        Render {
            sample_rate,
            left,
            right,
        }
    }

    pub fn frames(&self) -> usize {
        self.left.len()
    }

    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.sample_rate
    }

    fn span(&self, window: &Range<f32>) -> Range<usize> {
        let frame = |s: f32| ((s * self.sample_rate).round() as usize).min(self.frames());
        let (a, b) = (frame(window.start), frame(window.end));
        assert!(
            a < b,
            "empty window {window:?} in a {} s render",
            self.seconds()
        );
        a..b
    }

    fn both(&self, span: Range<usize>) -> impl Iterator<Item = f32> + '_ {
        self.left[span.clone()]
            .iter()
            .chain(&self.right[span])
            .copied()
    }

    fn whole(&self) -> Range<f32> {
        0.0..self.seconds()
    }

    /// The largest absolute sample of both channels, over the whole render.
    pub fn peak(&self) -> f32 {
        self.peak_in(self.whole())
    }

    /// The largest absolute sample of both channels within `window` (seconds).
    pub fn peak_in(&self, window: Range<f32>) -> f32 {
        let span = self.span(&window);
        self.both(span).fold(0.0, |m, s| m.max(s.abs()))
    }

    /// The root mean square of both channels, over the whole render.
    pub fn rms(&self) -> f32 {
        self.rms_in(self.whole())
    }

    /// The root mean square of both channels within `window` (seconds).
    pub fn rms_in(&self, window: Range<f32>) -> f32 {
        let span = self.span(&window);
        let n = (span.len() * 2) as f64;
        let sum: f64 = self.both(span).map(|s| f64::from(s) * f64::from(s)).sum();
        (sum / n).sqrt() as f32
    }

    /// No NaN and no infinity in any sample.
    pub fn assert_finite(&self) {
        for (name, ch) in [("left", &self.left), ("right", &self.right)] {
            if let Some(i) = ch.iter().position(|s| !s.is_finite()) {
                panic!("{name} sample {i} is not finite: {}", ch[i]);
            }
        }
    }

    /// No subnormal float in any sample (they slow the audio thread down on some CPUs).
    pub fn assert_no_denormals(&self) {
        for (name, ch) in [("left", &self.left), ("right", &self.right)] {
            if let Some(i) = ch.iter().position(|s| s.is_subnormal()) {
                panic!("{name} sample {i} is denormal: {:e}", ch[i]);
            }
        }
    }

    /// Finite, no denormal, and within full scale.
    pub fn assert_clean(&self) {
        self.assert_finite();
        self.assert_no_denormals();
        assert!(
            self.peak() <= 1.0,
            "the signal exceeds full scale: {}",
            self.peak()
        );
    }

    /// Exact silence (every sample 0.0) within `window` (seconds).
    pub fn assert_silent(&self, window: Range<f32>) {
        let span = self.span(&window);
        let start = span.start;
        if let Some(i) = self.left[span.clone()].iter().position(|s| *s != 0.0) {
            panic!(
                "left sample {} is not silent: {}",
                start + i,
                self.left[start + i]
            );
        }
        if let Some(i) = self.right[span].iter().position(|s| *s != 0.0) {
            panic!(
                "right sample {} is not silent: {}",
                start + i,
                self.right[start + i]
            );
        }
    }

    /// The peak within `window` is at least `min`: the signal is audible there.
    pub fn assert_audible(&self, window: Range<f32>, min: f32) {
        let peak = self.peak_in(window.clone());
        assert!(peak >= min, "peak {peak} within {window:?} is below {min}");
    }

    /// The time (seconds) of the first sample, at or after `from` seconds, whose
    /// absolute value exceeds `threshold`; `None` when the signal never does.
    pub fn onset(&self, threshold: f32, from: f32) -> Option<f32> {
        let start = ((from * self.sample_rate).round() as usize).min(self.frames());
        (start..self.frames())
            .find(|&i| self.left[i].abs() > threshold || self.right[i].abs() > threshold)
            .map(|i| i as f32 / self.sample_rate)
    }

    /// The first onset above `threshold` at or after `from` is within `tolerance`
    /// seconds of `expected`.
    pub fn assert_onset_at(&self, expected: f32, tolerance: f32, threshold: f32, from: f32) {
        let at = self
            .onset(threshold, from)
            .unwrap_or_else(|| panic!("no onset above {threshold} after {from} s"));
        assert!(
            (at - expected).abs() <= tolerance,
            "onset at {at} s, expected {expected} s (+-{tolerance} s)"
        );
    }

    /// The magnitude spectrum of the left channel within `window` (seconds): the
    /// longest power-of-two run of frames (at most 65 536) from its start, Hann
    /// windowed.
    pub fn spectrum(&self, window: Range<f32>) -> Spectrum {
        let span = self.span(&window);
        let n = (1usize << span.len().ilog2()).min(1 << 16);
        assert!(n >= 64, "window {window:?} is too short for a spectrum");
        let mut re: Vec<f64> = Vec::with_capacity(n);
        let mut gain = 0.0;
        for (i, s) in self.left[span.start..span.start + n].iter().enumerate() {
            let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            gain += w;
            re.push(f64::from(*s) * w);
        }
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        Spectrum {
            bin_hz: self.sample_rate / n as f32,
            magnitudes: (0..=n / 2)
                .map(|k| (2.0 * re[k].hypot(im[k]) / gain) as f32)
                .collect(),
        }
    }

    /// The largest absolute difference between two renders, with the sample index.
    pub fn max_difference(&self, other: &Render) -> (f32, usize) {
        assert_eq!(
            self.frames(),
            other.frames(),
            "the renders differ in length"
        );
        let diffs = self
            .left
            .iter()
            .zip(&other.left)
            .chain(self.right.iter().zip(&other.right))
            .map(|(a, b)| (a - b).abs());
        diffs
            .enumerate()
            .fold((0.0, 0), |m, (i, d)| if d > m.0 { (d, i) } else { m })
    }

    /// The two renders agree within `tolerance` on every sample.
    pub fn assert_close(&self, other: &Render, tolerance: f32) {
        let (diff, index) = self.max_difference(other);
        assert!(
            diff <= tolerance,
            "largest difference {diff} at sample {index} (tolerance {tolerance})"
        );
    }

    /// Writes the render as a 32-bit float stereo WAV file to
    /// `.qa/renders/<name>.wav` at the repository root (git-ignored), for listening, and
    /// returns its path.
    pub fn write_wav(&self, name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.qa/renders");
        std::fs::create_dir_all(&dir).expect("the renders folder is created");
        let path = dir.join(format!("{name}.wav"));
        std::fs::write(&path, self.wav()).expect("the WAV file is written");
        path
    }

    /// The bytes of the WAV file of [`Render::write_wav`].
    pub fn wav(&self) -> Vec<u8> {
        let data = (self.frames() * 8) as u32;
        let mut out = Vec::with_capacity(44 + data as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&(self.sample_rate as u32).to_le_bytes());
        out.extend_from_slice(&(self.sample_rate as u32 * 8).to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        for (l, r) in self.left.iter().zip(&self.right) {
            out.extend_from_slice(&l.to_le_bytes());
            out.extend_from_slice(&r.to_le_bytes());
        }
        out
    }
}

/// In-place radix-2 FFT; the length is a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, secs: f32, amp: f32) -> Render {
        let sr = 48_000.0;
        let n = (secs * sr) as usize;
        let s: Vec<f32> = (0..n)
            .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / sr).sin())
            .collect();
        Render::new(sr, s.clone(), s)
    }

    #[test]
    fn a_sine_reads_its_level_and_frequency() {
        let r = sine(1_000.0, 1.0, 0.5);
        assert!((r.peak() - 0.5).abs() < 1e-3);
        assert!((r.rms() - 0.5 / 2f32.sqrt()).abs() < 1e-3);
        let sp = r.spectrum(0.0..1.0);
        assert!(
            (sp.dominant_hz() - 1_000.0).abs() < 1.0,
            "{}",
            sp.dominant_hz()
        );
        assert!((sp.peak_in(900.0..1_100.0) - 0.5).abs() < 0.08);
        assert!(sp.peak_in(3_000.0..4_000.0) < 0.001);
        r.assert_clean();
    }

    #[test]
    fn onset_and_silence_are_located() {
        let mut r = sine(440.0, 1.0, 0.5);
        for i in 0..24_000 {
            r.left[i] = 0.0;
            r.right[i] = 0.0;
        }
        r.assert_silent(0.0..0.5);
        r.assert_onset_at(0.5, 0.001, 0.05, 0.0);
        assert_eq!(r.onset(2.0, 0.0), None);
    }

    #[test]
    #[should_panic(expected = "not silent")]
    fn silence_refuses_a_single_stray_sample() {
        let mut r = Render::new(48_000.0, vec![0.0; 100], vec![0.0; 100]);
        r.right[40] = 1e-30;
        r.assert_silent(0.0..0.002);
    }

    #[test]
    #[should_panic(expected = "not finite")]
    fn nan_is_caught() {
        let mut r = sine(440.0, 0.1, 0.5);
        r.left[7] = f32::NAN;
        r.assert_finite();
    }

    #[test]
    #[should_panic(expected = "denormal")]
    fn denormals_are_caught() {
        let mut r = sine(440.0, 0.1, 0.5);
        r.right[9] = 1e-40;
        r.assert_no_denormals();
    }

    #[test]
    fn close_and_different_renders_are_told_apart() {
        let a = sine(440.0, 0.1, 0.5);
        let mut b = a.clone();
        a.assert_close(&b, 0.0);
        b.left[10] += 1e-3;
        assert_eq!(a.max_difference(&b).1, 10);
        assert!(std::panic::catch_unwind(|| a.assert_close(&b, 1e-5)).is_err());
    }

    #[test]
    fn the_wav_header_matches_the_data() {
        let r = sine(440.0, 0.01, 0.5);
        let wav = r.wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(wav.len(), 44 + r.frames() * 8);
        let riff = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff, wav.len() - 8);
        let first = f32::from_le_bytes(wav[44..48].try_into().unwrap());
        assert_eq!(first, r.left[0]);
    }
}
