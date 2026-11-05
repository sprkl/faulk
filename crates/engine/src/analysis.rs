//! The spectrum analysis of the visual feedback (#156): what a device's panel draws behind its
//! response curve (the equaliser Contour), computed off the audio path, carried by the report
//! as a few bands per device, never as samples.
//!
//! # The path, in both targets
//!
//! 1. A device that has something to analyse keeps the last frames of its output (the mono
//!    sum) in a buffer of its own, allocated by its factory, and copies them in time order on
//!    request ([`crate::device::Device::analysis_window`]): [`window_len`] frames, a power of two
//!    that grows with the sample rate so that a band always spans about the same hertz.
//! 2. The audio side, when it writes a report (`crate::host::Host::write_feedback`, about 30 times
//!    a second), copies the windows of at most [`MAX_SPECTRA`] devices into the **window region**,
//!    under a seqlock, each with its track slot, device slot, the serial of its instance
//!    (`crate::engine::DeviceEntry::serial`) and the sample rate. With more devices than that, a
//!    round robin takes the next ones at the next report.
//! 3. The prepare side (`crate::host::Prepare::flush`, every few milliseconds in both targets:
//!    the helper's control thread, the web prepare Worker) reads a window region it has not read
//!    yet, runs an [`Analyzer`] on each window (a Hann window, a radix-2 FFT, then [`BANDS`]
//!    bands spaced evenly in log frequency from [`LOW_HZ`] to [`HIGH_HZ`], each the highest power
//!    of its bins in dBFS) and publishes them in the **spectrum region**, under its own seqlock.
//! 4. The audio side copies the spectrum region into the next report (one report later than its
//!    window): only these bands cross the page ↔ helper link.
//!
//! A band reads 0 dB for a full-scale sine at its centre. Values are clamped to
//! [[`FLOOR_DB`], [`CEIL_DB`]]; a band at or above the Nyquist frequency reads [`FLOOR_DB`]
//! (the report carries the sample rate, so a reader knows which bands exist).
//!
//! The seqlock of each region: the writer makes word 0 odd, writes the body, then makes it even
//! (release); a reader takes an even word 0 (acquire), the body, then the same word 0 again, or
//! leaves the region for its next turn. Neither side ever waits for the other.

use core::sync::atomic::{AtomicU32, Ordering, fence};

/// Bands of a spectrum.
pub const BANDS: usize = 96;
/// Most devices analysed in one report.
pub const MAX_SPECTRA: usize = 4;
/// The lowest band's lower edge and the highest band's upper edge, in hertz.
pub const LOW_HZ: f32 = 20.0;
pub const HIGH_HZ: f32 = 20_000.0;
/// The bounds of a band's value, in dBFS.
pub const FLOOR_DB: f32 = -140.0;
pub const CEIL_DB: f32 = 24.0;
/// The shortest and the longest window, in frames.
pub const MIN_WINDOW: usize = 2048;
pub const MAX_WINDOW: usize = 8192;

/// The frames of a window at `sample_rate`: the power of two at or above `sample_rate / 24`
/// (about 42 ms), from [`MIN_WINDOW`] to [`MAX_WINDOW`]: 2048 at 44.1 and 48 kHz, 4096 at 96 kHz,
/// 8192 at 192 kHz.
pub fn window_len(sample_rate: f32) -> usize {
    let wanted = if sample_rate.is_finite() && sample_rate > 0.0 {
        (sample_rate / 24.0) as usize
    } else {
        MIN_WINDOW
    };
    wanted.next_power_of_two().clamp(MIN_WINDOW, MAX_WINDOW)
}

/// The edges of band `i` (0 to [`BANDS`]), in hertz.
pub fn band_edge(i: usize) -> f32 {
    LOW_HZ * (HIGH_HZ / LOW_HZ).powf(i as f32 / BANDS as f32)
}

/// The centre of band `i`, in hertz (geometric).
pub fn band_centre(i: usize) -> f32 {
    LOW_HZ * (HIGH_HZ / LOW_HZ).powf((i as f32 + 0.5) / BANDS as f32)
}

/// One analysed device: where it is, the serial of its instance, the sample rate it ran at, and
/// its bands in dBFS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spectrum {
    pub track: u32,
    pub device: u32,
    pub serial: u32,
    pub sample_rate: f32,
    pub bands: [f32; BANDS],
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            track: 0,
            device: 0,
            serial: 0,
            sample_rate: 0.0,
            bands: [FLOOR_DB; BANDS],
        }
    }
}

/// Size of a [`Spectrum`] on the wire: track, device, serial, sample rate, then the bands.
pub const SPECTRUM_WIRE_BYTES: usize = 16 + 4 * BANDS;

// ---------------------------------------------------------------- the regions

/// Words of a window's header in the window region: track, device, serial, rate, frames.
const WINDOW_HEADER: usize = 5;
/// Words of one window slot.
const WINDOW_SLOT: usize = WINDOW_HEADER + MAX_WINDOW;
/// Words of the window region: seq, count, then the slots.
pub const WINDOW_REGION_WORDS: usize = 2 + MAX_SPECTRA * WINDOW_SLOT;
/// Words of one spectrum slot: track, device, serial, rate, bands.
const SPECTRUM_SLOT: usize = 4 + BANDS;
/// Words of the spectrum region: seq, count, then the slots.
pub const SPECTRUM_REGION_WORDS: usize = 2 + MAX_SPECTRA * SPECTRUM_SLOT;

/// Where a window comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub track: u32,
    pub device: u32,
    pub serial: u32,
}

/// A region of 32-bit words shared by the two sides, built off the audio path.
pub fn try_region(words: usize) -> Result<std::sync::Arc<[AtomicU32]>, crate::msg::MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(words)
        .map_err(|_| crate::msg::MsgError::OutOfMemory { bytes: words * 4 })?;
    v.resize_with(words, || AtomicU32::new(0));
    Ok(std::sync::Arc::from(v.into_boxed_slice()))
}

fn put(region: &[AtomicU32], i: usize, value: u32) {
    if let Some(w) = region.get(i) {
        w.store(value, Ordering::Relaxed);
    }
}

fn get(region: &[AtomicU32], i: usize) -> u32 {
    region.get(i).map_or(0, |w| w.load(Ordering::Relaxed))
}

/// The audio side's writer of the window region: one call per report. No allocation.
#[derive(Debug, Default)]
pub struct WindowWriter {
    seq: u32,
    count: u32,
}

impl WindowWriter {
    /// Opens a write: the region's seq becomes odd.
    pub fn begin(&mut self, region: &[AtomicU32]) {
        self.seq = self.seq.wrapping_add(1) | 1;
        self.count = 0;
        put(region, 0, self.seq);
        fence(Ordering::Release);
    }

    /// Writes one window into the next slot; `false` (nothing written) once the slots are full
    /// or for an empty window.
    pub fn push(
        &mut self,
        region: &[AtomicU32],
        source: Source,
        sample_rate: f32,
        frames: &[f32],
    ) -> bool {
        let slot = self.count as usize;
        if slot >= MAX_SPECTRA || frames.is_empty() {
            return false;
        }
        let base = 2 + slot * WINDOW_SLOT;
        let n = frames.len().min(MAX_WINDOW);
        put(region, base, source.track);
        put(region, base + 1, source.device);
        put(region, base + 2, source.serial);
        put(region, base + 3, sample_rate.to_bits());
        put(region, base + 4, n as u32);
        for (i, s) in frames[..n].iter().enumerate() {
            put(region, base + WINDOW_HEADER + i, s.to_bits());
        }
        self.count += 1;
        true
    }

    /// Closes the write: the count, then the even seq (release).
    pub fn end(&mut self, region: &[AtomicU32]) {
        put(region, 1, self.count);
        let done = self.seq.wrapping_add(1);
        if let Some(w) = region.first() {
            w.store(done, Ordering::Release);
        }
        self.seq = done;
    }
}

/// A seqlock read: the even seq the body was read under, or `None` (a write runs, or ran
/// meanwhile, or none happened yet).
fn read_under<T>(region: &[AtomicU32], body: impl FnOnce() -> T) -> Option<(u32, T)> {
    let seq = region.first()?.load(Ordering::Acquire);
    if seq == 0 || seq & 1 == 1 {
        return None;
    }
    let value = body();
    fence(Ordering::Acquire);
    let again = region.first()?.load(Ordering::Relaxed);
    (again == seq).then_some((seq, value))
}

/// The prepare side's reader of the window region and writer of the spectrum region, with its
/// [`Analyzer`] and buffers: allocated once, by [`crate::host::channel`].
#[derive(Debug)]
pub struct Analysis {
    analyzer: Analyzer,
    /// The windows read, `MAX_SPECTRA × MAX_WINDOW` frames, and their headers.
    frames: Box<[f32]>,
    sources: [(Source, f32, usize); MAX_SPECTRA],
    spectra: [Spectrum; MAX_SPECTRA],
    /// The window region's seq last analysed.
    seen: u32,
    seq: u32,
}

impl Analysis {
    pub fn try_new() -> Result<Self, crate::msg::MsgError> {
        let n = MAX_SPECTRA * MAX_WINDOW;
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(n)
            .map_err(|_| crate::msg::MsgError::OutOfMemory { bytes: n * 4 })?;
        frames.resize(n, 0.0);
        Ok(Self {
            analyzer: Analyzer::try_new()?,
            frames: frames.into_boxed_slice(),
            sources: [(
                Source {
                    track: 0,
                    device: 0,
                    serial: 0,
                },
                0.0,
                0,
            ); MAX_SPECTRA],
            spectra: [Spectrum::default(); MAX_SPECTRA],
            seen: 0,
            seq: 0,
        })
    }

    /// Analyses the window region when it holds windows not analysed yet, and publishes their
    /// spectra. `true` when it published.
    pub fn run(&mut self, windows: &[AtomicU32], spectra: &[AtomicU32]) -> bool {
        let frames = &mut self.frames;
        let sources = &mut self.sources;
        let read = read_under(windows, || {
            let count = (get(windows, 1) as usize).min(MAX_SPECTRA);
            for (slot, source) in sources.iter_mut().enumerate().take(count) {
                let base = 2 + slot * WINDOW_SLOT;
                let n = (get(windows, base + 4) as usize).min(MAX_WINDOW);
                *source = (
                    Source {
                        track: get(windows, base),
                        device: get(windows, base + 1),
                        serial: get(windows, base + 2),
                    },
                    f32::from_bits(get(windows, base + 3)),
                    n,
                );
                let out = &mut frames[slot * MAX_WINDOW..slot * MAX_WINDOW + n];
                for (i, s) in out.iter_mut().enumerate() {
                    *s = f32::from_bits(get(windows, base + WINDOW_HEADER + i));
                }
            }
            count
        });
        let Some((seq, count)) = read else {
            return false;
        };
        if seq == self.seen {
            return false;
        }
        self.seen = seq;
        for slot in 0..count {
            let (source, rate, n) = self.sources[slot];
            let spectrum = &mut self.spectra[slot];
            spectrum.track = source.track;
            spectrum.device = source.device;
            spectrum.serial = source.serial;
            spectrum.sample_rate = rate;
            let window = &self.frames[slot * MAX_WINDOW..slot * MAX_WINDOW + n];
            self.analyzer.bands(window, rate, &mut spectrum.bands);
        }
        // Publish.
        self.seq = self.seq.wrapping_add(1) | 1;
        put(spectra, 0, self.seq);
        fence(Ordering::Release);
        for (slot, s) in self.spectra.iter().enumerate().take(count) {
            let base = 2 + slot * SPECTRUM_SLOT;
            put(spectra, base, s.track);
            put(spectra, base + 1, s.device);
            put(spectra, base + 2, s.serial);
            put(spectra, base + 3, s.sample_rate.to_bits());
            for (i, b) in s.bands.iter().enumerate() {
                put(spectra, base + 4 + i, b.to_bits());
            }
        }
        put(spectra, 1, count as u32);
        self.seq = self.seq.wrapping_add(1);
        if let Some(w) = spectra.first() {
            w.store(self.seq, Ordering::Release);
        }
        true
    }
}

/// Reads the spectrum region into `out` (the audio side, before a report): the number of
/// spectra, or `None` when a write runs or ran meanwhile, or none happened yet (the caller keeps
/// what it had). No allocation.
pub fn read_spectra(region: &[AtomicU32], out: &mut [Spectrum; MAX_SPECTRA]) -> Option<usize> {
    read_under(region, || {
        let count = (get(region, 1) as usize).min(MAX_SPECTRA);
        for (slot, s) in out.iter_mut().enumerate().take(count) {
            let base = 2 + slot * SPECTRUM_SLOT;
            s.track = get(region, base);
            s.device = get(region, base + 1);
            s.serial = get(region, base + 2);
            s.sample_rate = f32::from_bits(get(region, base + 3));
            for (i, b) in s.bands.iter_mut().enumerate() {
                *b = f32::from_bits(get(region, base + 4 + i));
            }
        }
        count
    })
    .map(|(_, count)| count)
}

// ---------------------------------------------------------------- the analyser

/// A Hann window, a radix-2 FFT and the bands, for windows up to [`MAX_WINDOW`] frames; its
/// tables and scratch are allocated once.
#[derive(Debug)]
pub struct Analyzer {
    /// `cos` and `sin` of `-2 pi k / MAX_WINDOW` for `k` below `MAX_WINDOW / 2`.
    twiddle: Box<[(f32, f32)]>,
    re: Box<[f32]>,
    im: Box<[f32]>,
}

fn try_vec<T: Clone>(n: usize, value: T) -> Result<Box<[T]>, crate::msg::MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n)
        .map_err(|_| crate::msg::MsgError::OutOfMemory {
            bytes: n.saturating_mul(size_of::<T>()),
        })?;
    v.resize(n, value);
    Ok(v.into_boxed_slice())
}

impl Analyzer {
    pub fn try_new() -> Result<Self, crate::msg::MsgError> {
        let mut twiddle = try_vec(MAX_WINDOW / 2, (0.0f32, 0.0f32))?;
        for (k, t) in twiddle.iter_mut().enumerate() {
            let a = -core::f64::consts::TAU * k as f64 / MAX_WINDOW as f64;
            *t = (a.cos() as f32, a.sin() as f32);
        }
        Ok(Self {
            twiddle,
            re: try_vec(MAX_WINDOW, 0.0)?,
            im: try_vec(MAX_WINDOW, 0.0)?,
        })
    }

    /// The power of each bin `0..=n/2` of `frames` (its length a power of two, at most
    /// [`MAX_WINDOW`]; a longer one is cut, a shorter one is zero-padded to a power of two),
    /// through a Hann window, normalised so that a full-scale sine at a bin's centre reads 1.
    /// Returns the bins written to the start of `self.re`.
    pub fn power(&mut self, frames: &[f32]) -> &[f32] {
        let n = frames
            .len()
            .clamp(2, MAX_WINDOW)
            .next_power_of_two()
            .min(MAX_WINDOW);
        let samples = frames.iter().copied().chain(core::iter::repeat(0.0));
        for (i, ((re, im), x)) in self.re[..n]
            .iter_mut()
            .zip(&mut self.im[..n])
            .zip(samples)
            .enumerate()
        {
            let x = if x.is_finite() { x } else { 0.0 };
            let w = 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n as f32).cos();
            *re = x * w;
            *im = 0.0;
        }
        self.fft(n);
        // A full-scale sine through a Hann window reads n / 4 at its bin.
        let scale = 4.0 / n as f32;
        for k in 0..=n / 2 {
            let (r, i) = (self.re[k] * scale, self.im[k] * scale);
            self.re[k] = r * r + i * i;
        }
        &self.re[..=n / 2]
    }

    /// In-place iterative radix-2 FFT of the first `n` values of `re` and `im`.
    fn fft(&mut self, n: usize) {
        let (re, im) = (&mut self.re[..n], &mut self.im[..n]);
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut size = 2;
        while size <= n {
            let half = size / 2;
            let step = MAX_WINDOW / size;
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (c, s) = self.twiddle[k * step];
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * c - im[b] * s;
                    let ti = re[b] * s + im[b] * c;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            size *= 2;
        }
    }

    /// The [`BANDS`] bands of `frames` at `sample_rate`, in dBFS, into `out`: each band the
    /// highest bin power whose frequency falls in it; a band narrower than a bin takes the
    /// power interpolated at its centre; a band at or above Nyquist reads [`FLOOR_DB`].
    pub fn bands(&mut self, frames: &[f32], sample_rate: f32, out: &mut [f32; BANDS]) {
        if frames.is_empty() || !(sample_rate.is_finite() && sample_rate > 0.0) {
            *out = [FLOOR_DB; BANDS];
            return;
        }
        let n = frames
            .len()
            .clamp(2, MAX_WINDOW)
            .next_power_of_two()
            .min(MAX_WINDOW);
        let bin_hz = sample_rate / n as f32;
        let nyquist = sample_rate / 2.0;
        let power = self.power(frames);
        let last = power.len() - 1;
        let db = |p: f32| {
            if p > 0.0 {
                (10.0 * p.log10()).clamp(FLOOR_DB, CEIL_DB)
            } else {
                FLOOR_DB
            }
        };
        for (i, o) in out.iter_mut().enumerate() {
            let (lo, hi) = (band_edge(i), band_edge(i + 1));
            if lo >= nyquist {
                *o = FLOOR_DB;
                continue;
            }
            let first = (lo / bin_hz).ceil() as usize;
            let end = ((hi / bin_hz).ceil() as usize).min(last + 1);
            let p = if first < end {
                power[first..end].iter().fold(0.0f32, |m, p| m.max(*p))
            } else {
                let x = (band_centre(i) / bin_hz).min(last as f32);
                let k = (x.floor() as usize).min(last);
                let k1 = (k + 1).min(last);
                let t = x - k as f32;
                power[k] * (1.0 - t) + power[k1] * t
            };
            *o = db(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spectra() -> [Spectrum; MAX_SPECTRA] {
        [Spectrum::default(); MAX_SPECTRA]
    }

    #[test]
    fn a_region_never_written_odd_or_torn_is_not_read() {
        let region = try_region(SPECTRUM_REGION_WORDS).unwrap();
        // Never written: seq 0.
        assert_eq!(read_spectra(&region, &mut spectra()), None);
        // A write under way: odd.
        region[0].store(3, Ordering::Relaxed);
        assert_eq!(read_spectra(&region, &mut spectra()), None);
        // Stable and even: read.
        region[0].store(4, Ordering::Relaxed);
        region[1].store(1, Ordering::Relaxed);
        assert_eq!(read_spectra(&region, &mut spectra()), Some(1));
        // A write that started and ended while the body was read: the seq moved, refused.
        let torn = read_under(&region, || region[0].store(6, Ordering::Relaxed));
        assert_eq!(torn, None);
        // A count beyond the slots is cut to them.
        region[1].store(99, Ordering::Relaxed);
        assert_eq!(read_spectra(&region, &mut spectra()), Some(MAX_SPECTRA));
    }

    #[test]
    fn the_analysis_runs_once_per_new_window_write_and_never_on_an_odd_one() {
        let windows = try_region(WINDOW_REGION_WORDS).unwrap();
        let out = try_region(SPECTRUM_REGION_WORDS).unwrap();
        let mut analysis = Analysis::try_new().unwrap();
        let mut writer = WindowWriter::default();
        // Nothing written yet.
        assert!(!analysis.run(&windows, &out));
        let frames = vec![0.25f32; 2048];
        let source = Source {
            track: 2,
            device: 3,
            serial: 9,
        };
        writer.begin(&windows);
        assert!(writer.push(&windows, source, 48_000.0, &frames));
        // Mid-write (odd): not read.
        assert!(!analysis.run(&windows, &out));
        // An empty window is refused; full slots refuse more.
        assert!(!writer.push(&windows, source, 48_000.0, &[]));
        for _ in 1..MAX_SPECTRA {
            assert!(writer.push(&windows, source, 48_000.0, &frames));
        }
        assert!(!writer.push(&windows, source, 48_000.0, &frames));
        writer.end(&windows);
        assert!(analysis.run(&windows, &out));
        // The same write again: nothing new.
        assert!(!analysis.run(&windows, &out));
        let mut read = spectra();
        assert_eq!(read_spectra(&out, &mut read), Some(MAX_SPECTRA));
        assert_eq!((read[0].track, read[0].device, read[0].serial), (2, 3, 9));
        assert_eq!(read[0].sample_rate, 48_000.0);
        // A constant reads as DC: the lowest band is the loudest.
        assert!(read[0].bands[0] > read[0].bands[60]);
        // An empty write publishes no spectrum.
        writer.begin(&windows);
        writer.end(&windows);
        assert!(analysis.run(&windows, &out));
        assert_eq!(read_spectra(&out, &mut read), Some(0));
    }
}
