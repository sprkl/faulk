//! The output protection (#402): an always-on, invisible safety limiter on the last stage of
//! the engine's output, so that nothing the engine plays or exports goes above [`CEILING_DB`].
//!
//! # Where it sits
//!
//! [`crate::engine::Engine::render`] applies it to each rendered slice as the very last step:
//! after the master strip and its lanes, the metronome and the all-sound-off fade, and after
//! the output's meter. So the master meter keeps showing the level **before** the protection
//! (an overload stays visible), while the speakers, the helper's audio callback and the
//! offline render (the WAV export: `Offline` renders through the same `Engine::render`) all
//! get the protected signal. There is no command, no parameter and no way to bypass it.
//!
//! # The law
//!
//! The detector is stereo-linked (the larger of the two channels' magnitudes), so the stereo
//! image does not move when the limiter works. The gain is `1.0` at rest. A sample whose
//! magnitude would go above the ceiling brings the gain down to exactly the value that
//! puts it on the ceiling, **on that very sample**: the attack is immediate and there is no
//! look-ahead, so the protection adds **no latency** (0 frames, in both modes). Between
//! overs the gain comes back to `1.0` along an exponential with a time constant of
//! [`RELEASE_SECONDS`], and snaps to exactly `1.0` once it is within [`SNAP`] of it. A final
//! clamp to the ceiling covers the rounding of the multiplication, so the guarantee is
//! strict: no output sample has a magnitude above [`CEILING`].
//!
//! A signal that stays below the ceiling is returned untouched, bit for bit (the gain is
//! exactly `1.0` and the sample is not multiplied). A signal that was limited and comes back
//! below it differs from the input by at most [`SNAP`] of its magnitude while the gain
//! snaps, except that a normal magnitude the gain would push into the subnormal range is
//! flushed to exactly 0.0 (the limiter never creates a denormal). A sample that is not finite
//! becomes silence (the limiter is the last defence against a device that blew up), and a
//! subnormal input is below any ceiling, so it passes untouched at any gain.
//!
//! The immediate attack trades some distortion on an over for zero latency: this is a safety
//! net that works on rare overs, not a mastering tool.
//!
//! Nothing here allocates, locks, logs or panics: three `f32` fields (the gain deficit, the
//! release coefficient and the peak gain reduction for the export's report) and one `u64` counter
//! of the stereo frames in which it silenced a non-finite channel (for the no-NaN guard below), all on the stack, no heap.
//!
//! # Effect on downstream signal assertions
//!
//! Because the limiter is the last stage of `Engine::render`, a test that checks the render
//! output for non-finite samples or for a peak above full scale now passes by construction:
//! the limiter turns NaN and Inf into 0.0 and bounds every sample to the ceiling. Concretely,
//! `assert_finite` and the `peak() <= 1.0` part of `assert_clean` are vacuous on the engine's
//! output, so a test that wants to catch a device blowing up to NaN or running away must render
//! through that device directly (as the device tests in `crates/engine/tests/{filter,echo,
//! compressor,reverb,osc_synth}.rs` do, driving `device.process` and asserting `is_finite`), not
//! through `Engine::render` (the `test-support` device tests use `parity`/`render_native`, which
//! go through the engine, so their checks are among the vacuous ones). The denormal check is
//! **not** affected: a subnormal magnitude is
//! below any ceiling, so the limiter passes it untouched (`subnormals_pass_untouched`), and
//! `assert_no_denormals` on the render output keeps all its power.
//!
//! For the non-device paths (master strip, master chain, automation, summing) that have no direct
//! device test, the limiter counts the frames in which it silenced a non-finite channel ([`Limiter::silenced`]): an
//! offline render asserts that count is `0`, which catches a blow-up the vacuous `assert_finite`
//! would miss.

use crate::time::SampleRate;

/// The ceiling in dBFS.
pub const CEILING_DB: f32 = -0.3;

/// The ceiling as a linear magnitude: `10^(-0.3 / 20)` = 0.9660508789, rounded **down** in
/// `f32`, so that the `f64` conversion of an output sample is never above the exact value.
pub const CEILING: f32 = 0.966_050_8;

/// Time constant of the gain's return to unity, in seconds.
pub const RELEASE_SECONDS: f64 = 0.080;

/// A deficit under this is zero: it ends the release exactly.
pub const SNAP: f32 = 1.0e-6;

/// The limiter: stateful, one per engine.
#[derive(Clone, Copy, Debug)]
pub struct Limiter {
    /// How far the gain is under unity, `1 - gain`, in `[0, 1]`. The release shrinks this
    /// distance: kept as a distance, it keeps its precision down to the snap (a gain stored
    /// near `1.0` would stall one rounding step short of it).
    deficit: f32,
    /// Fraction of the deficit kept after one sample of release.
    keep: f32,
    /// The largest deficit (`1 - gain`) applied since [`Limiter::reset_max_reduction`], i.e.
    /// the peak gain reduction, in `[0, 1]`. One monotonic `max` on this audio-thread-owned field;
    /// nothing reads it from another thread. An offline export reads it, on the thread that just
    /// ran the render, to tell the user the mix was pushed into the limiter (#402). It is **not**
    /// cleared by [`Limiter::reset`] or a sample-rate change: a render reads a whole pass, and the
    /// export resets it before the pass.
    max_reduction: f32,
    /// How many stereo frames had a non-finite (NaN or infinity) sample on either channel since
    /// the last [`Limiter::reset_silenced`]: the non-finite channel is turned to 0.0, the other
    /// one is kept and limited as usual. A count of frames (once per frame, even when both
    /// channels are non-finite), not of samples. The limiter is the last line against a device or a non-device path (master strip,
    /// automation, summing) that blows up; because it clamps and silences, an engine-level
    /// `assert_finite` on the render output passes by construction, so this counter is the seam
    /// that lets an offline render still catch a blow-up (#402). Same ownership as `max_reduction`:
    /// one audio thread writes, the thread that ran the render reads it after, never concurrently.
    silenced: u64,
}

impl Limiter {
    /// A limiter at rest for `rate`.
    pub fn new(rate: SampleRate) -> Self {
        Limiter {
            deficit: 0.0,
            keep: keep_per_sample(rate),
            max_reduction: 0.0,
            silenced: 0,
        }
    }

    /// The current gain (`1.0` at rest).
    pub fn gain(&self) -> f32 {
        1.0 - self.deficit
    }

    /// The peak gain reduction since the last [`Limiter::reset_max_reduction`], in `[0, 1]`
    /// (`0.0` means the limiter never engaged). `20·log10(1 - reduction)` is the deepest
    /// attenuation in dB.
    pub fn max_reduction(&self) -> f32 {
        self.max_reduction
    }

    /// Clears the peak gain reduction back to `0.0`. An offline render calls it before a pass.
    pub fn reset_max_reduction(&mut self) {
        self.max_reduction = 0.0;
    }

    /// How many stereo frames with a non-finite sample on either channel (that channel turned
    /// to 0.0) since the last [`Limiter::reset_silenced`] (frames, not samples). `0` on a clean render; an offline test
    /// asserts it is `0` for the paths the engine-level `assert_finite` can no longer police
    /// (master strip, automation, summing).
    pub fn silenced(&self) -> u64 {
        self.silenced
    }

    /// Clears the silenced-frame counter. An offline render calls it before a pass.
    pub fn reset_silenced(&mut self) {
        self.silenced = 0;
    }

    /// Follows a new sample rate: the release keeps its duration. No allocation.
    pub fn set_sample_rate(&mut self, rate: SampleRate) {
        self.keep = keep_per_sample(rate);
    }

    /// Back to rest: gain `1.0`. The peak gain reduction is kept (`reset_max_reduction`
    /// clears it): a reset follows an all-sound-off fade, not the end of a render pass.
    pub fn reset(&mut self) {
        self.deficit = 0.0;
    }

    /// The latency this stage adds, in frames: none.
    pub const fn latency_frames() -> u32 {
        0
    }

    /// Protects one stereo frame in place.
    #[inline]
    pub fn process_frame(&mut self, l: &mut f32, r: &mut f32) {
        // A NaN or an infinity is silence; this also keeps the gain finite. Count the blow-up so
        // an offline render can still catch it (the engine-level assert_finite cannot, since the
        // limiter has already silenced it): saturating, so it never panics.
        let finite = l.is_finite() && r.is_finite();
        if !finite {
            self.silenced = self.silenced.saturating_add(1);
        }
        let a = if l.is_finite() { l.abs() } else { 0.0 };
        let b = if r.is_finite() { r.abs() } else { 0.0 };
        let peak = a.max(b);
        if self.deficit == 0.0 && peak <= CEILING && finite {
            // At rest, below the ceiling: untouched.
            return;
        }
        // The release, then the attack: the gain never lets this sample over the ceiling.
        let mut deficit = self.deficit * self.keep;
        if deficit < SNAP {
            deficit = 0.0;
        }
        if peak * (1.0 - deficit) > CEILING {
            // An absurd input (a gain under the float's resolution) leaves a deficit of
            // exactly 1: silence for that sample, never a subnormal gain.
            deficit = 1.0 - CEILING / peak;
        }
        self.deficit = deficit;
        // The deepest attenuation of this pass, for the export's "pushed into the limiter"
        // report (#402). One monotonic `max`; the field is read only after the render, on the
        // same thread that ran it (the export's `Offline`), never concurrently.
        self.max_reduction = self.max_reduction.max(deficit);
        let gain = 1.0 - deficit;
        *l = shape(*l, gain);
        *r = shape(*r, gain);
    }

    /// Protects `left` and `right` in place (their common length).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.process_frame(l, r);
        }
    }
}

/// The fraction of the deficit left after one sample of release at `rate`.
///
/// The minimum is chosen so that the release decrement is at least one `f32` ULP away from
/// zero at a deficit of 1.0 (i.e. `keep < 1.0` in `f32`). This guards against a pathological
/// sample rate so high that the release would never advance: at any rate accepted by
/// [`SampleRate`] the release reaches `SNAP` in at most `1.0 / (1.0 - MAX_KEEP) / SNAP` samples,
/// a finite number.
const MAX_KEEP: f32 = 1.0 - f32::EPSILON;

fn keep_per_sample(rate: SampleRate) -> f32 {
    let k = (-1.0 / (RELEASE_SECONDS * f64::from(rate.hz()))).exp() as f32;
    k.min(MAX_KEEP)
}

/// One sample times the gain, clamped to the ceiling (a non-finite sample is silence).
#[inline]
fn shape(x: f32, gain: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let y = if gain < 1.0 {
        // A tiny normal times a gain under one can fall into the subnormal range: flush it to
        // zero, deterministically, so the limiter never creates a denormal. A subnormal input
        // is passed untouched (as at rest), so a device that emits one stays visible.
        let g = x * gain;
        if g.abs() >= f32::MIN_POSITIVE {
            g
        } else if x.abs() < f32::MIN_POSITIVE {
            x
        } else {
            0.0
        }
    } else {
        x
    };
    y.clamp(-CEILING, CEILING)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate() -> SampleRate {
        SampleRate::HZ_48000
    }

    #[test]
    fn the_ceiling_constant_is_minus_0_3_dbfs_rounded_down() {
        let exact = 10f64.powf(f64::from(CEILING_DB) / 20.0);
        assert!(f64::from(CEILING) <= exact);
        assert!(exact - f64::from(CEILING) < 1e-7);
    }

    #[test]
    fn a_signal_below_the_ceiling_is_bit_identical() {
        let mut lim = Limiter::new(rate());
        let n = 4800;
        let src: Vec<f32> = (0..n).map(|i| 0.9 * (i as f32 * 0.05).sin()).collect();
        let (mut l, mut r) = (src.clone(), src.iter().map(|x| -x).collect::<Vec<_>>());
        let r0 = r.clone();
        lim.process(&mut l, &mut r);
        assert_eq!(l, src);
        assert_eq!(r, r0);
        assert_eq!(lim.gain(), 1.0);
    }

    #[test]
    fn a_hot_signal_never_goes_above_the_ceiling() {
        let mut lim = Limiter::new(rate());
        for amp in [1.0f32, 2.0, 10.0, 1.0e6, f32::MAX] {
            let n = 9600;
            let mut l: Vec<f32> = (0..n).map(|i| amp * (i as f32 * 0.07).sin()).collect();
            let mut r: Vec<f32> = (0..n).map(|i| amp * (i as f32 * 0.031).cos()).collect();
            lim.process(&mut l, &mut r);
            assert!(l.iter().chain(&r).all(|s| s.abs() <= CEILING), "amp {amp}");
            assert!(l.iter().chain(&r).all(|s| s.is_finite()));
        }
    }

    #[test]
    fn a_tiny_normal_never_becomes_a_subnormal_when_the_gain_is_below_one() {
        // The other channel is hot, so the gain is far below 1: a tiny normal on this one would
        // land in the subnormal range if the product were not flushed (#416).
        for k in [1.0f32, 1.5, 2.0, 3.0, 4.0, 8.0] {
            for sign in [1.0f32, -1.0] {
                let mut lim = Limiter::new(rate());
                let tiny = sign * f32::MIN_POSITIVE * k;
                let (mut l, mut r) = ([tiny; 8], [4.0f32; 8]);
                lim.process(&mut l, &mut r);
                assert!(lim.gain() < 1.0);
                for s in l.iter().chain(&r) {
                    assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "k {k}: {s:e}");
                }
            }
        }
        // Sweep the exponent of small normals over the release too.
        let mut lim = Limiter::new(rate());
        lim.process(&mut [0.0f32], &mut [8.0f32]);
        let n = 4800;
        let mut l: Vec<f32> = (0..n)
            .map(|i| f32::MIN_POSITIVE * (1.0 + (i % 64) as f32 / 8.0))
            .collect();
        let mut r = vec![0.0f32; n];
        lim.process(&mut l, &mut r);
        assert!(l.iter().all(|s| *s == 0.0 || s.abs() >= f32::MIN_POSITIVE));
    }

    #[test]
    fn a_subnormal_input_stays_visible_when_the_gain_is_below_one() {
        let mut lim = Limiter::new(rate());
        let sub = f32::MIN_POSITIVE / 4.0;
        let (mut l, mut r) = ([sub; 4], [4.0f32; 4]);
        lim.process(&mut l, &mut r);
        assert!(lim.gain() < 1.0);
        assert_eq!(l, [sub; 4]);
    }

    #[test]
    fn the_first_over_is_caught_on_its_own_sample() {
        let mut lim = Limiter::new(rate());
        let (mut l, mut r) = ([0.0, 0.0, 5.0, 0.0], [0.0; 4]);
        lim.process(&mut l, &mut r);
        assert_eq!(l[0], 0.0);
        assert!((l[2] - CEILING).abs() < 1e-6);
        assert!(l[2] <= CEILING);
        assert_eq!(Limiter::latency_frames(), 0);
    }

    #[test]
    fn the_stereo_image_is_kept() {
        let mut lim = Limiter::new(rate());
        let (mut l, mut r) = ([4.0f32], [2.0f32]);
        lim.process(&mut l, &mut r);
        assert!((l[0] / r[0] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn the_gain_comes_back_to_exactly_one() {
        let mut lim = Limiter::new(rate());
        let (mut l, mut r) = ([8.0f32], [0.0f32]);
        lim.process(&mut l, &mut r);
        assert!(lim.gain() < 0.2);
        // About 8 time constants (0.64 s) are far more than needed: the snap ends it.
        let n = (rate().hz() as usize) * 2;
        let (mut l, mut r) = (vec![0.1f32; n], vec![0.1f32; n]);
        lim.process(&mut l, &mut r);
        assert_eq!(lim.gain(), 1.0);
        assert_eq!(*l.last().unwrap(), 0.1);
        // After one time constant the gain covered 63 % of the way, give or take.
        let mut lim = Limiter::new(rate());
        lim.process(&mut [8.0f32], &mut [0.0f32]);
        let g0 = lim.gain();
        let n = (RELEASE_SECONDS * f64::from(rate().hz())) as usize;
        lim.process(&mut vec![0.0; n], &mut vec![0.0; n]);
        let expect = 1.0 - (1.0 - g0) * (-1.0f32).exp();
        assert!(
            (lim.gain() - expect).abs() < 1e-3,
            "{} vs {expect}",
            lim.gain()
        );
    }

    #[test]
    fn a_non_finite_sample_is_silence_and_the_gain_stays_finite() {
        let mut lim = Limiter::new(rate());
        let (mut l, mut r) = ([f32::NAN, f32::INFINITY, 0.5, f32::NEG_INFINITY], [0.5; 4]);
        lim.process(&mut l, &mut r);
        assert_eq!(l, [0.0, 0.0, 0.5, 0.0]);
        assert_eq!(r, [0.5; 4]);
        assert_eq!(lim.gain(), 1.0);
        // Three frames had a non-finite sample (NaN, +Inf, -Inf); the finite frame did not.
        assert_eq!(lim.silenced(), 3);
    }

    #[test]
    fn the_silenced_counter_only_grows_on_non_finite_input_and_resets() {
        let mut lim = Limiter::new(rate());
        // A clean, even hot, signal silences nothing.
        lim.process(&mut [0.5f32, 4.0, -0.2], &mut [0.5f32, 4.0, 0.1]);
        assert_eq!(lim.silenced(), 0);
        // A NaN in either channel counts the frame once.
        lim.process(&mut [f32::NAN], &mut [0.0f32]);
        lim.process(&mut [0.0f32], &mut [f32::INFINITY]);
        assert_eq!(lim.silenced(), 2);
        // reset() (after an all-sound-off) keeps the counter; reset_silenced() clears it.
        lim.reset();
        assert_eq!(lim.silenced(), 2);
        lim.reset_silenced();
        assert_eq!(lim.silenced(), 0);
    }

    #[test]
    fn subnormals_pass_untouched() {
        let mut lim = Limiter::new(rate());
        let tiny = f32::MIN_POSITIVE / 4.0;
        let (mut l, mut r) = ([tiny, -tiny], [tiny, 0.0]);
        lim.process(&mut l, &mut r);
        assert_eq!(l, [tiny, -tiny]);
    }

    #[test]
    fn a_new_rate_keeps_the_release_duration_and_a_reset_is_at_rest() {
        let release_samples = |hz| {
            let mut lim = Limiter::new(SampleRate::HZ_48000);
            lim.set_sample_rate(SampleRate::new(hz).unwrap());
            lim.process(&mut [8.0f32], &mut [0.0f32]);
            let d0 = 1.0 - lim.gain();
            let mut n = 0;
            while 1.0 - lim.gain() > d0 / std::f32::consts::E {
                lim.process(&mut [0.0f32], &mut [0.0f32]);
                n += 1;
            }
            n as f64 / f64::from(hz)
        };
        for hz in [44_100, 48_000, 96_000] {
            let t = release_samples(hz);
            assert!((t - RELEASE_SECONDS).abs() < 0.001, "{hz}: {t}");
        }
        let mut lim = Limiter::new(rate());
        lim.process(&mut [8.0f32], &mut [0.0f32]);
        assert!(lim.gain() < 1.0);
        lim.reset();
        assert_eq!(lim.gain(), 1.0);
    }

    #[test]
    fn every_rate_has_a_usable_release() {
        // Standard and extreme rates: keep is strictly under 1.0, so the release always makes
        // progress. u32::MAX is the highest rate SampleRate accepts; MAX_KEEP caps it below 1.0.
        for hz in [
            8_000u32,
            22_050,
            44_100,
            48_000,
            96_000,
            192_000,
            384_000,
            u32::MAX,
        ] {
            let lim = Limiter::new(SampleRate::new(hz).unwrap());
            assert!(
                lim.keep > 0.0 && lim.keep < 1.0,
                "{hz}: keep = {}",
                lim.keep
            );
        }
        // At any rate, an over must release to gain 1.0 (SNAP kicks in before infinite samples).
        let mut lim = Limiter::new(SampleRate::new(u32::MAX).unwrap());
        lim.process(&mut [8.0f32], &mut [0.0f32]);
        assert!(lim.gain() < 1.0, "no limiting at u32::MAX Hz");
        // The release makes progress: the deficit drops each sample.
        let d0 = 1.0 - lim.gain();
        lim.process(&mut [0.0f32], &mut [0.0f32]);
        let d1 = 1.0 - lim.gain();
        assert!(d1 < d0, "release stalled at u32::MAX Hz: {d0} → {d1}");
    }

    #[test]
    fn the_peak_gain_reduction_is_tracked_and_resettable() {
        let mut lim = Limiter::new(rate());
        // At rest: no reduction.
        assert_eq!(lim.max_reduction(), 0.0);
        lim.process(&mut [0.5f32; 16], &mut [0.5f32; 16]);
        assert_eq!(
            lim.max_reduction(),
            0.0,
            "a signal under the ceiling never engages"
        );

        // A 2x over brings the gain to CEILING/2; the reduction is that deficit, and it holds
        // at the deepest value even as the gain releases back up.
        lim.process(&mut [2.0f32], &mut [0.0f32]);
        let deepest = lim.max_reduction();
        assert!((deepest - (1.0 - CEILING / 2.0)).abs() < 1e-5, "{deepest}");
        // Release: the current gain climbs, but the tracked peak reduction does not shrink.
        lim.process(&mut vec![0.0f32; 480], &mut vec![0.0f32; 480]);
        assert!(lim.gain() > 1.0 - deepest, "the gain released");
        assert_eq!(lim.max_reduction(), deepest, "the peak reduction is held");

        // A deeper over raises it; a shallower one does not lower it.
        lim.process(&mut [100.0f32], &mut [0.0f32]);
        assert!(lim.max_reduction() > deepest);
        let after = lim.max_reduction();
        lim.process(&mut [1.2f32], &mut [0.0f32]);
        assert_eq!(
            lim.max_reduction(),
            after,
            "a shallower over does not lower the peak"
        );

        // reset() (after an all-sound-off) keeps the reduction; reset_max_reduction() clears it.
        lim.reset();
        assert_eq!(lim.max_reduction(), after, "reset keeps the reduction");
        lim.reset_max_reduction();
        assert_eq!(lim.max_reduction(), 0.0, "reset_max_reduction clears it");
    }
}
