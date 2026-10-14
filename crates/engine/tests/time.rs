//! The musical time contract (CT-2): units, conversions and their rounding, quantization
//! boundaries, loop regions. Property tests cover the whole input space, so no public
//! function may panic.

use assert_no_alloc::AllocDisabler;
use engine::time::{Frame, LoopRegion, PPQ, Quantize, SampleRate, Tempo, Tick, TimeSignature};
use proptest::prelude::*;

// The allocation detector is a global allocator: without it `assert_no_alloc` sees nothing.
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATES: [SampleRate; 3] = [
    SampleRate::HZ_44100,
    SampleRate::HZ_48000,
    SampleRate::HZ_96000,
];

fn tempos() -> [Tempo; 4] {
    [
        Tempo::new(20.0).unwrap(),
        Tempo::new(120.0).unwrap(),
        Tempo::new(133.33).unwrap(),
        Tempo::new(999.0).unwrap(),
    ]
}

fn sig(n: u8, d: u8) -> TimeSignature {
    TimeSignature::new(n, d).unwrap()
}

fn arb_tempo() -> impl Strategy<Value = Tempo> {
    (Tempo::MIN..=Tempo::MAX).prop_map(|bpm| Tempo::new(bpm).unwrap())
}

fn arb_rate() -> impl Strategy<Value = SampleRate> {
    prop_oneof![
        Just(SampleRate::HZ_44100),
        Just(SampleRate::HZ_48000),
        Just(SampleRate::HZ_96000),
        (1u32..=400_000).prop_map(|hz| SampleRate::new(hz).unwrap()),
    ]
}

fn arb_sig() -> impl Strategy<Value = TimeSignature> {
    (1u8..=32, 0u32..=5).prop_map(|(n, p)| TimeSignature::new(n, 1 << p).unwrap())
}

fn arb_quantize() -> impl Strategy<Value = Quantize> {
    proptest::sample::select(Quantize::ALL.to_vec())
}

// ---------------------------------------------------------------------------------------
// Units and constants
// ---------------------------------------------------------------------------------------

#[test]
fn ppq_is_960_and_divides_tuplets() {
    assert_eq!(PPQ, 960);
    assert_eq!(Tick::QUARTER, Tick(960));
    for divisor in [2, 3, 4, 5, 6, 8, 10, 12, 15, 16] {
        assert_eq!(PPQ % divisor, 0, "PPQ must divide by {divisor}");
    }
}

#[test]
fn ticks_per_beat_and_bar_follow_the_signature() {
    let cases = [
        ((4, 4), 960, 3840),
        ((3, 4), 960, 2880),
        ((6, 8), 480, 2880),
        ((7, 8), 480, 3360),
        ((5, 4), 960, 4800),
        ((1, 1), 3840, 3840),
        ((32, 32), 120, 3840),
        ((2, 2), 1920, 3840),
        ((12, 16), 240, 2880),
    ];
    for ((n, d), beat, bar) in cases {
        let s = sig(n, d);
        assert_eq!(s.ticks_per_beat(), Tick(beat), "{n}/{d} beat");
        assert_eq!(s.ticks_per_bar(), Tick(bar), "{n}/{d} bar");
    }
    assert_eq!(TimeSignature::default(), TimeSignature::COMMON);
    assert_eq!(TimeSignature::COMMON.numerator(), 4);
    assert_eq!(TimeSignature::COMMON.denominator(), 4);
}

#[test]
fn time_signature_rejects_invalid_parts() {
    for (n, d) in [(0, 4), (33, 4), (4, 0), (4, 3), (4, 6), (4, 64), (4, 12)] {
        assert!(
            TimeSignature::new(n, d).is_none(),
            "{n}/{d} must be refused"
        );
    }
    for d in [1, 2, 4, 8, 16, 32] {
        assert!(TimeSignature::new(1, d).is_some());
        assert!(TimeSignature::new(32, d).is_some());
    }
}

#[test]
fn tempo_range_and_default() {
    assert_eq!(Tempo::default().bpm(), 120.0);
    assert_eq!(Tempo::new(20.0).map(Tempo::bpm), Some(20.0));
    assert_eq!(Tempo::new(999.0).map(Tempo::bpm), Some(999.0));
    for bpm in [
        19.999,
        999.001,
        0.0,
        -120.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert!(Tempo::new(bpm).is_none(), "{bpm} must be refused");
    }
    assert_eq!(Tempo::clamped(5.0).bpm(), 20.0);
    assert_eq!(Tempo::clamped(5000.0).bpm(), 999.0);
    assert_eq!(Tempo::clamped(f64::INFINITY).bpm(), 999.0);
    assert_eq!(Tempo::clamped(f64::NEG_INFINITY).bpm(), 20.0);
    assert_eq!(Tempo::clamped(f64::NAN).bpm(), 120.0);
    assert_eq!(Tempo::clamped(140.5).bpm(), 140.5);
}

#[test]
fn sample_rate_rejects_zero() {
    assert!(SampleRate::new(0).is_none());
    assert_eq!(SampleRate::new(44_100), Some(SampleRate::HZ_44100));
    assert_eq!(SampleRate::default().hz(), 48_000);
}

#[test]
fn tick_and_frame_arithmetic_saturates() {
    assert_eq!(Tick(i64::MAX) + Tick(1), Tick(i64::MAX));
    assert_eq!(Tick(i64::MIN) - Tick(1), Tick(i64::MIN));
    assert_eq!(Tick(5) - Tick(7), Tick(-2));
    assert_eq!(Frame(u64::MAX) + Frame(1), Frame(u64::MAX));
    assert_eq!(Frame(5) - Frame(7), Frame::ZERO);
    let mut t = Tick(10);
    t += Tick(5);
    t -= Tick(3);
    assert_eq!(t, Tick(12));
    let mut f = Frame(10);
    f += Frame(5);
    f -= Frame(3);
    assert_eq!(f, Frame(12));
    assert_eq!(Tick::default(), Tick::ZERO);
    assert_eq!(Frame::default(), Frame::ZERO);
    assert_eq!(Tick::new(7).get(), 7);
    assert_eq!(Frame::new(7).get(), 7);
}

// ---------------------------------------------------------------------------------------
// Conversions and documented rounding
// ---------------------------------------------------------------------------------------

#[test]
fn documented_rounding_examples() {
    let t120 = Tempo::new(120.0).unwrap();
    // 120 BPM: a quarter note lasts 0.5 s, a tick 0.5 / 960 s.
    assert_eq!(t120.seconds_per_tick(), 0.5 / 960.0);
    assert_eq!(t120.ticks_per_second(), 1920.0);
    assert_eq!(t120.tick_to_seconds(Tick::QUARTER), 0.5);
    // 120 BPM at 48 kHz: 1920 ticks per second, 25 samples per tick, one quarter is
    // 24 000 samples.
    assert_eq!(t120.samples_per_tick(SampleRate::HZ_48000), 25.0);
    assert_eq!(
        t120.tick_to_sample(Tick::QUARTER, SampleRate::HZ_48000),
        24_000.0
    );
    assert_eq!(t120.tick_to_sample(Tick(1), SampleRate::HZ_48000), 25.0);
    // 120 BPM at 44.1 kHz: 22.96875 (735/32) samples per tick, not an integer.
    assert_eq!(t120.samples_per_tick(SampleRate::HZ_44100), 22.96875);
    assert_eq!(t120.tick_to_sample(Tick(1), SampleRate::HZ_44100), 22.96875);
    // Nearest integer sample, ties away from zero: 22.96875 -> 23, 45.9375 -> 46,
    // tick 16 = 367.5 -> 368, tick -16 = -367.5 -> -368.
    assert_eq!(
        t120.tick_to_sample_rounded(Tick(1), SampleRate::HZ_44100),
        23
    );
    assert_eq!(
        t120.tick_to_sample_rounded(Tick(2), SampleRate::HZ_44100),
        46
    );
    assert_eq!(
        t120.tick_to_sample_rounded(Tick(16), SampleRate::HZ_44100),
        368
    );
    assert_eq!(
        t120.tick_to_sample_rounded(Tick(-16), SampleRate::HZ_44100),
        -368
    );
    // Sample -> nearest tick at 120 BPM, 48 kHz (25 samples per tick): samples 0..=12
    // -> tick 0 (12/25 = 0.48), 13 -> tick 1 (0.52), 37 -> tick 1 (1.48), 38 -> tick 2.
    let s2t = |s: f64| t120.sample_to_tick(s, SampleRate::HZ_48000);
    assert_eq!(s2t(0.0), Tick(0));
    assert_eq!(s2t(12.0), Tick(0));
    assert_eq!(s2t(13.0), Tick(1));
    assert_eq!(s2t(37.0), Tick(1));
    assert_eq!(s2t(38.0), Tick(2));
    // A tie at exactly half a tick (12.5 samples) rounds away from zero.
    assert_eq!(s2t(12.5), Tick(1));
    assert_eq!(s2t(-12.5), Tick(-1));
    // Seconds.
    assert_eq!(t120.seconds_to_tick(0.5), Tick::QUARTER);
    assert_eq!(t120.seconds_to_tick(60.0), Tick(120 * 960));
    // Frames of an audio file to ticks, nearest: 1 s at 44.1 kHz = 1920 ticks; 100 frames
    // at 48 kHz = 4 ticks (exact); 112 frames = 4.48 -> 4; 113 frames = 4.52 -> 5.
    assert_eq!(
        t120.frames_to_ticks(Frame(44_100), SampleRate::HZ_44100),
        Tick(1920)
    );
    assert_eq!(
        t120.frames_to_ticks(Frame(100), SampleRate::HZ_48000),
        Tick(4)
    );
    assert_eq!(
        t120.frames_to_ticks(Frame(112), SampleRate::HZ_48000),
        Tick(4)
    );
    assert_eq!(
        t120.frames_to_ticks(Frame(113), SampleRate::HZ_48000),
        Tick(5)
    );
    assert_eq!(
        t120.audio_end_tick(Tick(1000), Frame(48_000 * 2), SampleRate::HZ_48000),
        Tick(1000 + 2 * 1920)
    );
}

#[test]
fn non_finite_and_huge_inputs_do_not_panic() {
    let t = Tempo::default();
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(t.seconds_to_tick(v), Tick::ZERO);
        assert_eq!(t.sample_to_tick(v, SampleRate::HZ_48000), Tick::ZERO);
    }
    assert_eq!(t.seconds_to_tick(1e300), Tick(i64::MAX));
    assert_eq!(t.seconds_to_tick(-1e300), Tick(i64::MIN));
    // A finite input whose product overflows f64 (1e306 * 1920 > f64::MAX) saturates too,
    // by sign, instead of falling through the non-finite rule.
    assert_eq!(t.seconds_to_tick(1e306), Tick(i64::MAX));
    assert_eq!(t.seconds_to_tick(-1e306), Tick(i64::MIN));
    assert_eq!(
        t.sample_to_tick(f64::MAX, SampleRate::new(1).unwrap()),
        Tick(i64::MAX)
    );
    assert_eq!(
        t.sample_to_tick(-f64::MAX, SampleRate::new(1).unwrap()),
        Tick(i64::MIN)
    );
    assert_eq!(
        t.tick_to_sample_rounded(Tick(i64::MAX), SampleRate::HZ_96000),
        i64::MAX
    );
    // u64::MAX frames at 44.1 kHz and 120 BPM is about 8.03e17 ticks, inside i64; at
    // 999 BPM and 1 Hz (15 984 ticks per frame) it is beyond i64 and saturates.
    assert_eq!(
        t.frames_to_ticks(Frame(u64::MAX), SampleRate::HZ_44100),
        Tick(803_123_551_508_443_008)
    );
    let fast = Tempo::new(999.0).unwrap();
    assert_eq!(
        fast.frames_to_ticks(Frame(u64::MAX), SampleRate::new(1).unwrap()),
        Tick(i64::MAX)
    );
    assert_eq!(
        t.audio_end_tick(Tick(i64::MAX), Frame(1), SampleRate::HZ_44100),
        Tick(i64::MAX)
    );
    assert!(
        t.tick_to_sample(Tick(i64::MIN), SampleRate::HZ_44100)
            .is_finite()
    );
}

#[test]
fn tick_to_sample_round_trip_over_a_bar_at_each_rate_and_tempo() {
    for tempo in tempos() {
        for rate in RATES {
            for tick in -3840..=3840 {
                let tick = Tick(tick);
                let sample = tempo.tick_to_sample(tick, rate);
                assert_eq!(
                    tempo.sample_to_tick(sample, rate),
                    tick,
                    "{} BPM at {} Hz",
                    tempo.bpm(),
                    rate.hz()
                );
                let seconds = tempo.tick_to_seconds(tick);
                assert_eq!(tempo.seconds_to_tick(seconds), tick);
            }
        }
    }
}

#[test]
fn no_drift_over_ten_minutes_at_120_bpm() {
    // 10 minutes at 120 BPM: 1200 quarters, 1 152 000 ticks, 600 s.
    let t120 = Tempo::new(120.0).unwrap();
    let end = Tick(1200 * PPQ);
    assert_eq!(t120.tick_to_seconds(end), 600.0);
    for rate in RATES {
        let expected = 600.0 * f64::from(rate.hz());
        assert_eq!(t120.tick_to_sample(end, rate), expected);
        // Walking tick by tick and converting each absolute position gives the same end.
        let mut last = 0.0;
        for tick in (0..=end.0).step_by(PPQ as usize) {
            last = t120.tick_to_sample(Tick(tick), rate);
        }
        assert_eq!(last, expected);
    }
}

#[test]
fn inexact_ratios_stay_within_a_few_ulps_and_never_accumulate() {
    // 133.33 BPM: no rate gives an integer samples-per-tick ratio, so tick_to_sample is
    // rounded, but as a pure function of the absolute tick its error stays at the ulp
    // level (here: 4 ulps of the result) whatever the position; it never grows with time.
    let tempo = Tempo::new(133.33).unwrap();
    for rate in RATES {
        for tick in [Tick(1), Tick(PPQ), Tick(1200 * PPQ), Tick(1 << 40)] {
            let got = tempo.tick_to_sample(tick, rate);
            // The true value as a rational: tick * 60 * rate * 100 / (13333 * 960), in
            // exact i128 arithmetic, then one rounding to f64 (the quotient is below
            // 2^53, so it is exact; the remainder adds less than one unit).
            let num = i128::from(tick.0) * 60 * i128::from(rate.hz()) * 100;
            let den: i128 = 13_333 * 960;
            let exact = (num / den) as f64 + (num % den) as f64 / den as f64;
            let ulp = exact.abs() * f64::EPSILON;
            assert!(
                (got - exact).abs() <= 4.0 * ulp,
                "{} Hz, tick {}: got {got}, exact {exact}",
                rate.hz(),
                tick.0
            );
        }
    }
}

#[test]
fn conversions_and_boundaries_allocate_nothing() {
    // These functions run on the audio thread (transport, launch grid): no heap use.
    let tempo = Tempo::new(133.33).unwrap();
    let sig = TimeSignature::COMMON;
    let region = LoopRegion::new(Tick(960), Tick(4800)).unwrap();
    let mut sink = 0.0;
    let mut ticks = Tick::ZERO;
    assert_no_alloc::assert_no_alloc(|| {
        for tick in 0..10_000 {
            let tick = Tick(tick);
            for rate in RATES {
                sink += tempo.tick_to_sample(tick, rate);
                sink += tempo.tick_to_sample_rounded(tick, rate) as f64;
                ticks += tempo.sample_to_tick(sink, rate);
                ticks += tempo.frames_to_ticks(Frame(tick.0 as u64), rate);
                ticks += tempo.audio_end_tick(tick, Frame(1), rate);
            }
            sink += tempo.tick_to_seconds(tick);
            ticks += tempo.seconds_to_tick(sink);
            for q in Quantize::ALL {
                ticks += q.next_boundary_at_or_after(tick, sig).unwrap_or(Tick::ZERO);
                ticks += q.next_boundary_after(tick, sig).unwrap_or(Tick::ZERO);
            }
            ticks += region.wrap(tick);
        }
    });
    assert!(sink.is_finite());
    let _ = ticks;
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn tick_sample_round_trip_is_identity(
        tempo in arb_tempo(),
        rate in arb_rate(),
        tick in -(1i64 << 40)..=(1i64 << 40),
    ) {
        let tick = Tick(tick);
        prop_assert_eq!(tempo.sample_to_tick(tempo.tick_to_sample(tick, rate), rate), tick);
        prop_assert_eq!(tempo.seconds_to_tick(tempo.tick_to_seconds(tick)), tick);
    }

    #[test]
    fn tick_sample_round_trip_at_the_three_rates(
        bpm in prop::sample::select(vec![20.0, 120.0, 133.33, 999.0]),
        rate in prop::sample::select(RATES.to_vec()),
        tick in -(1i64 << 40)..=(1i64 << 40),
    ) {
        let tempo = Tempo::new(bpm).unwrap();
        let tick = Tick(tick);
        prop_assert_eq!(tempo.sample_to_tick(tempo.tick_to_sample(tick, rate), rate), tick);
    }

    #[test]
    fn conversions_are_monotonic(
        tempo in arb_tempo(),
        rate in arb_rate(),
        a in any::<i64>(),
        b in any::<i64>(),
    ) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        prop_assert!(tempo.tick_to_sample(Tick(lo), rate) <= tempo.tick_to_sample(Tick(hi), rate));
        prop_assert!(tempo.tick_to_seconds(Tick(lo)) <= tempo.tick_to_seconds(Tick(hi)));
    }

    #[test]
    fn frames_to_ticks_is_monotonic_and_nearest(
        tempo in arb_tempo(),
        rate in arb_rate(),
        frames in 0u64..=(1 << 40),
    ) {
        let ticks = tempo.frames_to_ticks(Frame(frames), rate);
        let exact = frames as f64 * tempo.ticks_per_second() / f64::from(rate.hz());
        // Nearest tick of the f64 value: half a tick, plus the ulp error of both sides.
        prop_assert!((ticks.0 as f64 - exact).abs() <= 0.5 + exact.abs() * 4.0 * f64::EPSILON);
        prop_assert!(tempo.frames_to_ticks(Frame(frames + 1), rate) >= ticks);
    }

    #[test]
    fn every_conversion_is_total(
        bpm in any::<f64>(),
        hz in any::<u32>(),
        tick in any::<i64>(),
        frames in any::<u64>(),
        x in any::<f64>(),
    ) {
        let tempo = Tempo::clamped(bpm);
        prop_assert!(tempo.bpm().is_finite());
        let _ = Tempo::new(bpm);
        let rate = SampleRate::new(hz).unwrap_or_default();
        let _ = tempo.tick_to_seconds(Tick(tick));
        let _ = tempo.seconds_to_tick(x);
        let _ = tempo.tick_to_sample(Tick(tick), rate);
        let _ = tempo.tick_to_sample_rounded(Tick(tick), rate);
        let _ = tempo.sample_to_tick(x, rate);
        let _ = tempo.frames_to_ticks(Frame(frames), rate);
        let _ = tempo.audio_end_tick(Tick(tick), Frame(frames), rate);
        let _ = tempo.samples_per_tick(rate);
    }
}

// ---------------------------------------------------------------------------------------
// Quantization
// ---------------------------------------------------------------------------------------

#[test]
fn quantize_default_is_one_bar_and_all_lists_every_value() {
    assert_eq!(Quantize::default(), Quantize::Bar);
    assert_eq!(Quantize::ALL.len(), 7);
    let mut seen = Quantize::ALL.to_vec();
    seen.dedup();
    assert_eq!(seen.len(), 7, "ALL has no duplicate");
    // Every variant must be in ALL: a new variant breaks this exhaustive match at compile
    // time, and its arm checks that ALL lists it.
    fn listed(q: Quantize) -> bool {
        match q {
            Quantize::None
            | Quantize::Sixteenth
            | Quantize::Beat
            | Quantize::Bar
            | Quantize::Bars2
            | Quantize::Bars4
            | Quantize::Bars8 => Quantize::ALL.contains(&q),
        }
    }
    for q in [
        Quantize::None,
        Quantize::Sixteenth,
        Quantize::Beat,
        Quantize::Bar,
        Quantize::Bars2,
        Quantize::Bars4,
        Quantize::Bars8,
    ] {
        assert!(listed(q), "{q:?} is missing from Quantize::ALL");
    }
}

#[test]
fn quantize_grid_in_ticks() {
    let common = TimeSignature::COMMON;
    let expected = [
        None,
        Some(240),
        Some(960),
        Some(3840),
        Some(7680),
        Some(15_360),
        Some(30_720),
    ];
    for (q, grid) in Quantize::ALL.into_iter().zip(expected) {
        assert_eq!(q.grid(common), grid.map(Tick), "{q:?} in 4/4");
    }
    let six_eight = sig(6, 8);
    let expected = [
        None,
        Some(240),
        Some(480),
        Some(2880),
        Some(5760),
        Some(11_520),
        Some(23_040),
    ];
    for (q, grid) in Quantize::ALL.into_iter().zip(expected) {
        assert_eq!(q.grid(six_eight), grid.map(Tick), "{q:?} in 6/8");
    }
}

#[test]
fn next_boundary_for_every_quantize_value() {
    let common = TimeSignature::COMMON;
    let at = |q: Quantize, pos: i64| q.next_boundary_at_or_after(Tick(pos), common);
    let after = |q: Quantize, pos: i64| q.next_boundary_after(Tick(pos), common);
    for q in Quantize::ALL {
        let Some(grid) = q.grid(common) else {
            // No quantization: both functions give the position itself, always.
            for pos in [-5, 0, 1, 239, 240, 3840, 100_000, i64::MAX, i64::MIN] {
                assert_eq!(at(q, pos), Some(Tick(pos)));
                assert_eq!(after(q, pos), Some(Tick(pos)));
            }
            continue;
        };
        let g = grid.0;
        // Before a boundary: both give the boundary.
        assert_eq!(at(q, 1), Some(grid), "{q:?}");
        assert_eq!(after(q, 1), Some(grid), "{q:?}");
        assert_eq!(at(q, g - 1), Some(grid), "{q:?}");
        // On a boundary: at-or-after gives it, after gives the next one.
        assert_eq!(at(q, 0), Some(Tick::ZERO), "{q:?}");
        assert_eq!(after(q, 0), Some(grid), "{q:?}");
        assert_eq!(at(q, g), Some(grid), "{q:?}");
        assert_eq!(after(q, g), Some(Tick(2 * g)), "{q:?}");
        // Just after a boundary.
        assert_eq!(at(q, g + 1), Some(Tick(2 * g)), "{q:?}");
        // Far away: tick 100 000 in 4/4.
        assert_eq!(at(q, 100_000), Some(Tick((100_000 / g + 1) * g)), "{q:?}");
        // Negative positions (count-in) quantize on the same grid: -1 -> 0, -g -> -g.
        assert_eq!(at(q, -1), Some(Tick::ZERO), "{q:?}");
        assert_eq!(at(q, -g), Some(Tick(-g)), "{q:?}");
        assert_eq!(after(q, -g), Some(Tick::ZERO), "{q:?}");
        assert_eq!(at(q, -g - 1), Some(Tick(-g)), "{q:?}");
        // The start of time: the first grid point at or above i64::MIN (ceiling in i128).
        let first =
            (i128::from(i64::MIN) + i128::from(g) - 1).div_euclid(i128::from(g)) * i128::from(g);
        assert_eq!(at(q, i64::MIN), Some(Tick(first as i64)), "{q:?}");
        // The end of time: no grid point beyond the last one.
        let last = (i64::MAX / g) * g;
        assert_eq!(at(q, last), Some(Tick(last)), "{q:?}");
        assert_eq!(after(q, last), None, "{q:?}");
        assert_eq!(at(q, last + 1), None, "{q:?}");
        assert_eq!(at(q, i64::MAX), None, "{q:?}");
        assert_eq!(after(q, i64::MAX), None, "{q:?}");
    }
}

#[test]
fn beat_boundary_follows_the_signature() {
    let waltz = sig(3, 4);
    let at = |q: Quantize, pos: i64, s: TimeSignature| q.next_boundary_at_or_after(Tick(pos), s);
    assert_eq!(at(Quantize::Beat, 961, waltz), Some(Tick(1920)));
    assert_eq!(at(Quantize::Bar, 961, waltz), Some(Tick(2880)));
    assert_eq!(at(Quantize::Bars2, 2881, waltz), Some(Tick(5760)));
    // 6/8: the beat is the eighth note (480 ticks), the bar 2880 ticks.
    let six_eight = sig(6, 8);
    assert_eq!(at(Quantize::Beat, 481, six_eight), Some(Tick(960)));
    assert_eq!(at(Quantize::Bar, 481, six_eight), Some(Tick(2880)));
    assert_eq!(at(Quantize::Sixteenth, 241, six_eight), Some(Tick(480)));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn next_boundary_is_the_smallest_grid_point_at_or_after(
        q in arb_quantize(),
        sig in arb_sig(),
        pos in any::<i64>(),
    ) {
        let pos = Tick(pos);
        let at_or_after = q.next_boundary_at_or_after(pos, sig);
        let after = q.next_boundary_after(pos, sig);
        let Some(grid) = q.grid(sig) else {
            prop_assert_eq!(at_or_after, Some(pos));
            prop_assert_eq!(after, Some(pos));
            return Ok(());
        };
        let g = grid.0;
        prop_assert!(g > 0);
        let last = (i64::MAX / g) * g;
        match at_or_after {
            None => prop_assert!(pos.0 > last),
            Some(b) => {
                prop_assert!(b >= pos);
                prop_assert_eq!(b.0.rem_euclid(g), 0);
                prop_assert!(b.0 - pos.0 < g);
            }
        }
        match after {
            None => prop_assert!(pos.0 >= last),
            Some(b) => {
                prop_assert!(b > pos);
                prop_assert_eq!(b.0.rem_euclid(g), 0);
                prop_assert!(b.0 - pos.0 <= g);
                if pos.0.rem_euclid(g) == 0 {
                    prop_assert_eq!(at_or_after, Some(pos));
                    prop_assert_eq!(b.0, pos.0 + g);
                } else {
                    prop_assert_eq!(after, at_or_after);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Loop region
// ---------------------------------------------------------------------------------------

#[test]
fn loop_region_rejects_empty_or_reversed() {
    assert!(LoopRegion::new(Tick(10), Tick(10)).is_none());
    assert!(LoopRegion::new(Tick(10), Tick(9)).is_none());
    // Longer than i64::MAX ticks: refused, so that `len` is exact.
    assert!(LoopRegion::new(Tick(i64::MIN), Tick(i64::MAX)).is_none());
    assert!(LoopRegion::new(Tick(-1), Tick(i64::MAX)).is_none());
    assert!(LoopRegion::new(Tick(0), Tick(i64::MAX)).is_some());
    assert!(LoopRegion::new(Tick(i64::MIN), Tick(-1)).is_some());
    let region = LoopRegion::new(Tick(0), Tick(3840)).unwrap();
    assert_eq!(region.start(), Tick(0));
    assert_eq!(region.end(), Tick(3840));
    assert_eq!(region.len(), Tick(3840));
}

#[test]
fn loop_region_contains_and_wraps() {
    let region = LoopRegion::new(Tick(960), Tick(4800)).unwrap();
    assert!(region.contains(Tick(960)));
    assert!(region.contains(Tick(4799)));
    assert!(!region.contains(Tick(4800)));
    assert!(!region.contains(Tick(959)));
    // Inside: unchanged.
    assert_eq!(region.wrap(Tick(960)), Tick(960));
    assert_eq!(region.wrap(Tick(2000)), Tick(2000));
    // The end maps to the start; past the end wraps modulo the length (3840).
    assert_eq!(region.wrap(Tick(4800)), Tick(960));
    assert_eq!(region.wrap(Tick(4801)), Tick(961));
    assert_eq!(region.wrap(Tick(4800 + 3840 * 3 + 5)), Tick(965));
    // Before the start wraps backwards.
    assert_eq!(region.wrap(Tick(959)), Tick(4799));
    assert_eq!(region.wrap(Tick(0)), Tick(4800 - 960));
    assert_eq!(region.wrap(Tick(-1)), Tick(4800 - 961));
    // Extreme values.
    let whole = LoopRegion::new(Tick(0), Tick(i64::MAX)).unwrap();
    assert_eq!(whole.len(), Tick(i64::MAX));
    assert_eq!(whole.start() + whole.len(), whole.end());
    assert_eq!(whole.wrap(Tick(i64::MAX)), Tick(0));
    assert_eq!(whole.wrap(Tick(-1)), Tick(i64::MAX - 1));
    assert_eq!(whole.wrap(Tick(7)), Tick(7));
    let negative = LoopRegion::new(Tick(i64::MIN), Tick(-1)).unwrap();
    assert_eq!(negative.len(), Tick(i64::MAX));
    assert_eq!(negative.wrap(Tick(-1)), Tick(i64::MIN));
    assert_eq!(negative.wrap(Tick(0)), Tick(i64::MIN + 1));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn loop_wrap_lands_inside_and_is_idempotent(
        start in any::<i64>(),
        end in any::<i64>(),
        tick in any::<i64>(),
    ) {
        let Some(region) = LoopRegion::new(Tick(start), Tick(end)) else {
            let len = i128::from(end) - i128::from(start);
            prop_assert!(len <= 0 || len > i128::from(i64::MAX));
            return Ok(());
        };
        prop_assert_eq!(region.start() + region.len(), region.end());
        let wrapped = region.wrap(Tick(tick));
        prop_assert!(region.contains(wrapped));
        prop_assert_eq!(region.wrap(wrapped), wrapped);
        if region.contains(Tick(tick)) {
            prop_assert_eq!(wrapped, Tick(tick));
        }
        // The distance to the original is a multiple of the length.
        let len = i128::from(end) - i128::from(start);
        prop_assert_eq!((i128::from(tick) - i128::from(wrapped.0)).rem_euclid(len), 0);
        let _ = region.len();
    }
}

// ---------------------------------------------------------------------------------------
// Serialization (the project file)
// ---------------------------------------------------------------------------------------

#[cfg(feature = "serde")]
mod serde_tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Settings {
        tempo: Tempo,
        meter: TimeSignature,
        launch_quantize: Quantize,
        loop_region: Option<LoopRegion>,
        start: Tick,
        frames: Frame,
        rate: SampleRate,
    }

    #[test]
    fn round_trip_through_json_with_plain_numbers_and_names() {
        let settings = Settings {
            tempo: Tempo::new(133.5).unwrap(),
            meter: sig(6, 8),
            launch_quantize: Quantize::Bars2,
            loop_region: LoopRegion::new(Tick(960), Tick(4800)),
            start: Tick(-240),
            frames: Frame(44_100),
            rate: SampleRate::HZ_44100,
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            json,
            r#"{"tempo":133.5,"meter":{"numerator":6,"denominator":8},"launch_quantize":"bars_2","loop_region":{"start":960,"end":4800},"start":-240,"frames":44100,"rate":44100}"#
        );
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    }

    #[test]
    fn every_quantize_value_has_a_stable_name() {
        let names = [
            "none",
            "sixteenth",
            "beat",
            "bar",
            "bars_2",
            "bars_4",
            "bars_8",
        ];
        for (q, name) in Quantize::ALL.into_iter().zip(names) {
            assert_eq!(serde_json::to_string(&q).unwrap(), format!("\"{name}\""));
            assert_eq!(
                serde_json::from_str::<Quantize>(&format!("\"{name}\"")).unwrap(),
                q
            );
        }
        assert!(serde_json::from_str::<Quantize>("\"2 bars\"").is_err());
    }

    #[test]
    fn invalid_values_are_refused_on_read() {
        assert!(serde_json::from_str::<Tempo>("1000").is_err());
        assert!(serde_json::from_str::<Tempo>("19.9").is_err());
        assert!(serde_json::from_str::<Tempo>("null").is_err());
        assert!(serde_json::from_str::<SampleRate>("0").is_err());
        assert!(
            serde_json::from_str::<TimeSignature>(r#"{"numerator":4,"denominator":3}"#).is_err()
        );
        assert!(
            serde_json::from_str::<TimeSignature>(r#"{"numerator":0,"denominator":4}"#).is_err()
        );
        assert!(serde_json::from_str::<LoopRegion>(r#"{"start":10,"end":10}"#).is_err());
        assert!(serde_json::from_str::<LoopRegion>(r#"{"start":10,"end":9}"#).is_err());
        assert_eq!(
            serde_json::from_str::<LoopRegion>(r#"{"start":10,"end":11}"#).unwrap(),
            LoopRegion::new(Tick(10), Tick(11)).unwrap()
        );
    }
}
