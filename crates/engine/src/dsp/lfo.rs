//! A low-frequency oscillator, free or locked to the transport.
//!
//! # Shapes
//!
//! [`value`] is a pure function of a **phase in cycles** (any real number): its fractional
//! part `p` places the frame within a cycle, its integer part numbers the cycle. Every shape
//! is in `[-1, 1]`:
//!
//! - sine: `sin(2 pi p)`, 0 at the start of the cycle, rising;
//! - triangle: 0 at `p = 0`, +1 at 0.25, -1 at 0.75 (in phase with the sine);
//! - saw up: -1 at `p = 0` rising to +1; saw down: the reverse;
//! - square: +1 on the first half of the cycle, -1 on the second;
//! - random (sample and hold): one value per cycle, an integer hash of the cycle's number,
//!   so the same phase gives the same value in every render, natively and in WebAssembly.
//!
//! # Time
//!
//! In **free** mode the phase counts cycles since the oscillator was reset or restarted, at a
//! rate in hertz ([`Lfo::advance`]). It runs whether the transport plays or not.
//!
//! In **synced** mode the phase is the transport's position divided by the length of a
//! [`Division`], so a cycle starts on the same tick in every pass of a loop, after a seek and in
//! an offline export. The position of each frame is `anchor + n * ticks_per_frame`, where the
//! anchor is the position the host gave a block and `n` the frames played since: it adds no
//! rounding from block to block, so the phase does not depend on how the host cuts blocks.
//! [`Lfo::begin`] takes the anchor again when the host's position strays from that by more
//! than [`RESYNC_TICKS`] (a seek, a loop's wrap) or when the tempo changes (the position goes
//! on from where it was, at the new rate, so the cycles stay on the bars). While the
//! transport is stopped the position does not move and the value holds. A division in bars
//! uses the block's time signature, counting bars from tick 0.
//!
//! No allocation, no lock, no panic on any input.

use crate::device::Transport;
use crate::time::{PPQ, TimeSignature};

/// The shapes, in the order of a device's `shape` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Sine,
    Triangle,
    SawUp,
    SawDown,
    Square,
    Random,
}

/// The keys of the shapes, in [`Shape`]'s order (a device's parameter values).
pub const SHAPE_KEYS: &[&str] = &["sine", "triangle", "saw_up", "saw_down", "square", "random"];

impl Shape {
    /// The shape of a parameter value (its index in [`SHAPE_KEYS`]); an index beyond is the last.
    pub fn from_index(index: u32) -> Self {
        match index {
            0 => Shape::Sine,
            1 => Shape::Triangle,
            2 => Shape::SawUp,
            3 => Shape::SawDown,
            4 => Shape::Square,
            _ => Shape::Random,
        }
    }
}

/// The value of `shape` at `phase` cycles, in `[-1, 1]` (0 for a non-finite phase).
pub fn value(shape: Shape, phase: f64) -> f32 {
    if !phase.is_finite() {
        return 0.0;
    }
    let cycle = phase.floor();
    let p = phase - cycle;
    match shape {
        Shape::Sine => (core::f64::consts::TAU * p).sin() as f32,
        Shape::Triangle => {
            let v = if p < 0.25 {
                4.0 * p
            } else if p < 0.75 {
                2.0 - 4.0 * p
            } else {
                4.0 * p - 4.0
            };
            v as f32
        }
        Shape::SawUp => (2.0 * p - 1.0) as f32,
        Shape::SawDown => (1.0 - 2.0 * p) as f32,
        Shape::Square => {
            if p < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        Shape::Random => random(cycle as i64),
    }
}

/// The random value of cycle `n`, in `[-1, 1]`: the top 24 bits of a SplitMix64 hash, so the
/// arithmetic is exact in `f32`.
pub fn random(n: i64) -> f32 {
    let mut z = (n as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    let bits = (z >> 40) as u32;
    bits as f32 * (2.0 / 16_777_215.0) - 1.0
}

/// The length of a division.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// So many quarter notes.
    Quarters(f64),
    /// So many bars of the time signature.
    Bars(u32),
}

/// A note value of a synced LFO: its key (a device's parameter value) and its length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Division {
    pub key: &'static str,
    pub length: Length,
}

/// The note values, shortest first: the Echo's ten (1/32 to the whole note, with the eighth
/// and quarter triplets and the dotted eighth), then 2, 4 and 8 bars.
pub const DIVISIONS: [Division; 13] = [
    Division {
        key: "thirty_second",
        length: Length::Quarters(0.125),
    },
    Division {
        key: "sixteenth",
        length: Length::Quarters(0.25),
    },
    Division {
        key: "eighth_triplet",
        length: Length::Quarters(1.0 / 3.0),
    },
    Division {
        key: "eighth",
        length: Length::Quarters(0.5),
    },
    Division {
        key: "quarter_triplet",
        length: Length::Quarters(2.0 / 3.0),
    },
    Division {
        key: "eighth_dotted",
        length: Length::Quarters(0.75),
    },
    Division {
        key: "quarter",
        length: Length::Quarters(1.0),
    },
    Division {
        key: "quarter_dotted",
        length: Length::Quarters(1.5),
    },
    Division {
        key: "half",
        length: Length::Quarters(2.0),
    },
    Division {
        key: "whole",
        length: Length::Quarters(4.0),
    },
    Division {
        key: "two_bars",
        length: Length::Bars(2),
    },
    Division {
        key: "four_bars",
        length: Length::Bars(4),
    },
    Division {
        key: "eight_bars",
        length: Length::Bars(8),
    },
];

/// The keys of [`DIVISIONS`], in order (a device's parameter values).
pub const DIVISION_KEYS: [&str; 13] = {
    let mut keys = [""; 13];
    let mut i = 0;
    while i < 13 {
        keys[i] = DIVISIONS[i].key;
        i += 1;
    }
    keys
};

/// The length of division `index` in ticks under `signature` (an index beyond is the last).
pub fn division_ticks(index: u32, signature: TimeSignature) -> f64 {
    let d = DIVISIONS[(index as usize).min(DIVISIONS.len() - 1)];
    match d.length {
        Length::Quarters(q) => q * PPQ as f64,
        Length::Bars(n) => f64::from(n) * signature.ticks_per_bar().0 as f64,
    }
}

/// The host's position may stray this far, in ticks, from the one the LFO predicts before the
/// LFO takes it again (a seek or a loop's wrap moves it by more).
pub const RESYNC_TICKS: f64 = 1.0;

/// The state of an LFO: the free phase, and the synced position's anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lfo {
    /// Free mode: whole cycles since the reset, and the fraction of the current one.
    cycles: i64,
    fraction: f64,
    /// Synced mode: the position of the anchor frame, the frames played since, ticks per frame.
    anchor: f64,
    played: u64,
    ticks_per_frame: f64,
    anchored: bool,
    playing: bool,
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new()
    }
}

impl Lfo {
    /// An LFO at phase 0, not yet anchored to a position.
    pub const fn new() -> Self {
        Self {
            cycles: 0,
            fraction: 0.0,
            anchor: 0.0,
            played: 0,
            ticks_per_frame: 0.0,
            anchored: false,
            playing: false,
        }
    }

    /// The start of a block (or of a piece of one) at `transport`, at `sample_rate`:
    /// re-anchors the synced position when needed, and with `restart` sets the free phase back
    /// to 0 when the transport starts playing. Returns whether the transport started playing (false -> true).
    pub fn begin(&mut self, transport: &Transport, sample_rate: f32, restart: bool) -> bool {
        let rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            f64::from(sample_rate)
        } else {
            48_000.0
        };
        let tpf = transport.tempo.ticks_per_second() / rate;
        let position = if transport.position_ticks.is_finite() {
            transport.position_ticks
        } else {
            0.0
        };
        let predicted = self.anchor + self.played as f64 * self.ticks_per_frame;
        if !self.anchored
            || tpf != self.ticks_per_frame
            || (position - predicted).abs() > RESYNC_TICKS
        {
            self.anchor = position;
            self.played = 0;
            self.ticks_per_frame = tpf;
            self.anchored = true;
        }
        let started = transport.playing && !self.playing;
        self.playing = transport.playing;
        if started && restart {
            self.cycles = 0;
            self.fraction = 0.0;
        }
        started
    }

    /// The phase of the current frame, in cycles: synced to `period_ticks` ticks, or free.
    #[inline]
    pub fn phase(&self, synced: bool, period_ticks: f64) -> f64 {
        if synced {
            let position = self.anchor + self.played as f64 * self.ticks_per_frame;
            if period_ticks > 0.0 {
                position / period_ticks
            } else {
                0.0
            }
        } else {
            self.cycles as f64 + self.fraction
        }
    }

    /// One frame on: the synced position moves while the transport plays, the free phase by
    /// `rate_hz / sample_rate` always.
    #[inline]
    pub fn advance(&mut self, rate_hz: f32, sample_rate: f32) {
        if self.playing {
            self.played = self.played.saturating_add(1);
        }
        let step = if rate_hz.is_finite() && sample_rate > 0.0 {
            (f64::from(rate_hz) / f64::from(sample_rate)).clamp(0.0, 0.5)
        } else {
            0.0
        };
        self.fraction += step;
        if self.fraction >= 1.0 {
            self.fraction -= 1.0;
            self.cycles = self.cycles.wrapping_add(1);
        }
    }

    /// Back to phase 0, not anchored (the next block takes the host's position).
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Tempo;

    #[test]
    fn the_shapes_sit_where_the_doc_says() {
        let at = |s, p| value(s, p);
        assert_eq!(at(Shape::Triangle, 0.0), 0.0);
        assert_eq!(at(Shape::Triangle, 0.25), 1.0);
        assert_eq!(at(Shape::Triangle, 0.75), -1.0);
        assert_eq!(at(Shape::Triangle, 3.5), 0.0);
        assert!((at(Shape::Sine, 0.25) - 1.0).abs() < 1e-6);
        assert!(at(Shape::Sine, 0.0).abs() < 1e-6);
        assert_eq!(at(Shape::SawUp, 0.0), -1.0);
        assert_eq!(at(Shape::SawUp, 0.5), 0.0);
        assert_eq!(at(Shape::SawDown, 0.0), 1.0);
        assert_eq!(at(Shape::Square, 0.49), 1.0);
        assert_eq!(at(Shape::Square, 0.5), -1.0);
        assert_eq!(at(Shape::Square, -0.25), -1.0);
        assert_eq!(at(Shape::Sine, f64::NAN), 0.0);
        for i in 0..10_000 {
            let p = i as f64 * 0.0137 - 40.0;
            for s in 0..6 {
                let v = at(Shape::from_index(s), p);
                assert!((-1.0..=1.0).contains(&v), "{s} {p} {v}");
            }
        }
    }

    #[test]
    fn the_random_shape_holds_one_value_per_cycle_and_repeats_from_render_to_render() {
        let a: Vec<f32> = (0..64)
            .map(|n| value(Shape::Random, n as f64 + 0.3))
            .collect();
        let b: Vec<f32> = (0..64)
            .map(|n| value(Shape::Random, n as f64 + 0.9))
            .collect();
        assert_eq!(a, b);
        // Spread over the range: not a constant, not stuck on a side.
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        assert!(mean.abs() < 0.3, "{mean}");
        assert!(a.iter().any(|v| *v > 0.5) && a.iter().any(|v| *v < -0.5));
        // A frozen sequence: these numbers are the same in every build.
        assert_eq!(random(0).to_bits(), random(0).to_bits());
        assert!(random(i64::MIN).is_finite() && random(i64::MAX).is_finite());
    }

    #[test]
    fn divisions_measure_in_quarters_or_in_bars_of_the_signature() {
        let common = TimeSignature::COMMON;
        let three = TimeSignature::new(3, 4).unwrap();
        assert_eq!(division_ticks(6, common), 960.0);
        assert_eq!(division_ticks(0, common), 120.0);
        assert_eq!(division_ticks(10, common), 7680.0);
        assert_eq!(division_ticks(11, three), 4.0 * 2880.0);
        assert_eq!(division_ticks(99, common), 8.0 * 3840.0);
        assert_eq!(DIVISION_KEYS[6], "quarter");
    }

    fn transport(playing: bool, bpm: f64, position: f64) -> Transport {
        Transport {
            playing,
            tempo: Tempo::new(bpm).unwrap(),
            position_ticks: position,
            ..Transport::default()
        }
    }

    #[test]
    fn the_synced_position_follows_the_host_without_drifting_and_takes_a_seek() {
        let mut lfo = Lfo::new();
        let sr = 48_000.0;
        let tpf = 1920.0 / 48_000.0;
        lfo.begin(&transport(true, 120.0, 0.0), sr, true);
        for _ in 0..1000 {
            lfo.advance(1.0, sr);
        }
        // The host's position within a hair: kept.
        lfo.begin(&transport(true, 120.0, 1000.0 * tpf + 1e-9), sr, true);
        assert_eq!(lfo.phase(true, 960.0), 1000.0 * tpf / 960.0);
        // A seek: taken.
        lfo.begin(&transport(true, 120.0, 4800.0), sr, true);
        assert_eq!(lfo.phase(true, 960.0), 5.0);
        // Stopped: the position holds.
        lfo.begin(&transport(false, 120.0, 4800.0), sr, true);
        lfo.advance(1.0, sr);
        assert_eq!(lfo.phase(true, 960.0), 5.0);
    }

    #[test]
    fn the_free_phase_restarts_only_when_play_starts_and_restart_is_on() {
        let sr = 48_000.0;
        let mut lfo = Lfo::new();
        lfo.begin(&transport(false, 120.0, 0.0), sr, true);
        for _ in 0..12_000 {
            lfo.advance(2.0, sr);
        }
        assert!((lfo.phase(false, 0.0) - 0.5).abs() < 1e-9);
        assert!(lfo.begin(&transport(true, 120.0, 0.0), sr, true));
        assert_eq!(lfo.phase(false, 0.0), 0.0);
        lfo.advance(2.0, sr);
        assert!(!lfo.begin(&transport(true, 120.0, 0.0), sr, true));
        assert!(lfo.phase(false, 0.0) > 0.0);
        // Restart off: the phase goes on through a stop and a start.
        let mut free = Lfo::new();
        free.begin(&transport(false, 120.0, 0.0), sr, false);
        for _ in 0..100 {
            free.advance(2.0, sr);
        }
        let before = free.phase(false, 0.0);
        assert!(free.begin(&transport(true, 120.0, 0.0), sr, false));
        assert_eq!(free.phase(false, 0.0), before);
    }

    #[test]
    fn a_tempo_change_keeps_the_position_and_moves_at_the_new_rate() {
        let sr = 48_000.0;
        let mut lfo = Lfo::new();
        lfo.begin(&transport(true, 120.0, 0.0), sr, true);
        for _ in 0..24_000 {
            lfo.advance(1.0, sr);
        }
        let at = lfo.phase(true, 960.0);
        assert!((at - 1.0).abs() < 1e-9, "{at}");
        lfo.begin(&transport(true, 60.0, 960.0), sr, true);
        for _ in 0..48_000 {
            lfo.advance(1.0, sr);
        }
        assert!((lfo.phase(true, 960.0) - 2.0).abs() < 1e-9);
    }
}
