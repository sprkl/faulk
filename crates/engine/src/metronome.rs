//! The metronome (TR-2): a synthesized click on every beat, louder on the first beat of
//! the bar, mixed into the engine's output on the audio path.
//!
//! # Where the clicks fall
//!
//! [`Clicker::render`] pulls the [`Segment`]s of the clock ([`Transport::next_segment`])
//! and, for each, places a click on every beat tick `t` with `from <= t < to`. A beat is
//! `TimeSignature::ticks_per_beat` (in compound meters, the denominator's note value:
//! 6/8 has six eighth-note beats), the grid is anchored at tick 0 like `bar_position`,
//! and the accent is the first beat of the bar (`t` a multiple of the bar). The onset is
//! [`Segment::offset_of`]`(t)`: the sample that contains the beat's fractional position
//! (TR-1's rule), the same sample at any block size. A loop is already cut in segments at
//! its wrap, so the loop start is clicked at the first sample of its segment.
//!
//! # The sound
//!
//! A cosine burst with an exponential decay, 20 ms: 1.5 kHz on the accent, 1 kHz on the
//! other beats, peaks -12 and -15 dBFS at `volume` 1.0. The cosine starts at its peak, so the
//! first sample of a click is not zero and its onset is the exact sample. `volume` is a
//! linear gain (0 to 4: a non-finite or negative value is silence), read at each onset: a
//! click keeps its level, so a change never makes a zipper noise. Both the sound and the
//! default level are a sound choice for the founder to judge by ear.
//!
//! # Output
//!
//! The click is added to both channels of the master output, after the graph: it does
//! not go through a track (so no track meter hears it), nor through the master gain and
//! mute (it has its own level). A disabled metronome writes nothing and drops a click in
//! progress; a stopped clock starts no click, and a click in progress rings out.
//!
//! Nothing here allocates, locks, logs or panics.

use crate::engine::Metronome;
use crate::time::{SampleRate, Tick, TimeSignature};
use crate::transport::Transport;

/// Length of a click, in seconds.
pub const CLICK_SECONDS: f64 = 0.020;
/// Decay time constant of a click, in seconds.
const DECAY_SECONDS: f64 = 0.003;
/// Frequency of the accent and of the other beats, in hertz.
const ACCENT_HZ: f64 = 1500.0;
const BEAT_HZ: f64 = 1000.0;
/// Peak of the accent and of the other beats at `volume` 1.0.
pub const ACCENT_PEAK: f64 = 0.251_188_643_150_958; // -12 dBFS
pub const BEAT_PEAK: f64 = 0.177_827_941_003_892_3; // -15 dBFS
/// Highest `volume` taken into account.
pub const MAX_VOLUME: f32 = 4.0;
/// Most count-in bars taken into account (`project::limits::MAX_COUNT_IN_BARS`).
pub const MAX_COUNT_IN_BARS: u8 = 16;

/// The linear gain of a `volume` setting: 0 when negative or not finite, at most
/// [`MAX_VOLUME`].
pub fn level(volume: f32) -> f32 {
    if volume.is_finite() && volume > 0.0 {
        volume.min(MAX_VOLUME)
    } else {
        0.0
    }
}

/// Frames of a click at `rate` (at least 1).
fn click_frames(rate: SampleRate) -> u32 {
    ((CLICK_SECONDS * f64::from(rate.hz())).round() as u32).max(1)
}

/// The click in progress and its synthesis, owned by the audio side.
#[derive(Clone, Debug)]
pub struct Clicker {
    rate: f64,
    len: u32,
    /// Frames played of the click in progress; `len` or more: none.
    age: u32,
    /// Angular step (radians per frame) and peak gain of the click in progress.
    step: f64,
    gain: f64,
}

impl Clicker {
    /// An idle clicker at `rate`.
    pub fn new(rate: SampleRate) -> Self {
        let len = click_frames(rate);
        Clicker {
            rate: f64::from(rate.hz()),
            len,
            age: len,
            step: 0.0,
            gain: 0.0,
        }
    }

    /// Changes the sample rate; a click in progress is dropped.
    pub fn set_sample_rate(&mut self, rate: SampleRate) {
        self.rate = f64::from(rate.hz());
        self.len = click_frames(rate);
        self.age = self.len;
    }

    /// Whether a click is in progress.
    pub fn is_ringing(&self) -> bool {
        self.age < self.len
    }

    /// Drops the click in progress.
    pub fn silence(&mut self) {
        self.age = self.len;
    }

    fn start(&mut self, accent: bool, volume: f32) {
        let (hz, peak) = if accent {
            (ACCENT_HZ, ACCENT_PEAK)
        } else {
            (BEAT_HZ, BEAT_PEAK)
        };
        self.step = core::f64::consts::TAU * hz / self.rate;
        self.gain = peak * f64::from(volume);
        self.age = 0;
    }

    /// Adds the click in progress to the frames `from..to` of the block.
    fn tail(&mut self, left: &mut [f32], right: &mut [f32], from: usize, to: usize) {
        let to = to.min(left.len()).min(right.len());
        let mut i = from;
        while i < to && self.age < self.len {
            let n = f64::from(self.age);
            let value = ((self.step * n).cos()
                * (-n / (DECAY_SECONDS * self.rate)).exp()
                * self.gain) as f32;
            // No denormal leaves the metronome (the graph's own floor is behind us).
            let value = if value.abs() < 1e-20 { 0.0 } else { value };
            left[i] += value;
            right[i] += value;
            self.age += 1;
            i += 1;
        }
    }

    /// Adds the metronome to the block: pulls the segments of `clock` (a block must be
    /// open), places the clicks and renders them over `left` and `right`.
    pub fn render(
        &mut self,
        settings: &Metronome,
        signature: TimeSignature,
        clock: &mut Transport,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let frames = left.len().min(right.len());
        let volume = level(settings.volume);
        if !settings.enabled {
            self.silence();
            return;
        }
        let beat = signature.ticks_per_beat().0.max(1);
        let bar = signature.ticks_per_bar().0.max(1);
        let mut cursor = 0;
        while let Some(segment) = clock.next_segment() {
            // The first beat tick at or after `from`.
            let rest = segment.from.0.rem_euclid(beat);
            let mut tick = if rest == 0 {
                segment.from.0
            } else {
                segment.from.0.saturating_add(beat - rest)
            };
            while tick < segment.to.0 {
                let at = segment.offset_of(Tick(tick)).max(cursor);
                self.tail(left, right, cursor, at);
                cursor = at;
                if volume > 0.0 {
                    self.start(tick.rem_euclid(bar) == 0, volume);
                }
                match tick.checked_add(beat) {
                    Some(next) => tick = next,
                    None => break,
                }
            }
        }
        self.tail(left, right, cursor, frames);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_is_clamped() {
        assert_eq!(level(f32::NAN), 0.0);
        assert_eq!(level(-1.0), 0.0);
        assert_eq!(level(f32::INFINITY), 0.0);
        assert_eq!(level(0.5), 0.5);
        assert_eq!(level(100.0), MAX_VOLUME);
    }

    #[test]
    fn a_click_starts_on_a_nonzero_sample_and_ends() {
        let rate = SampleRate::HZ_48000;
        let mut c = Clicker::new(rate);
        assert!(!c.is_ringing());
        c.start(true, 1.0);
        let n = c.len as usize;
        let (mut l, mut r) = (vec![0.0f32; n + 10], vec![0.0f32; n + 10]);
        c.tail(&mut l, &mut r, 0, n + 10);
        assert!((f64::from(l[0]) - ACCENT_PEAK).abs() < 1e-6);
        assert_eq!(l, r);
        assert!(l[n..].iter().all(|&s| s == 0.0));
        assert!(l[n - 1].abs() < 0.01);
        assert!(!c.is_ringing());
    }
}
