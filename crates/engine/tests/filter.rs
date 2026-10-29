//! The filter, `faulk.filter` (DV-2): offline renders through the device contract, every
//! `process` under the allocation detector. Asserts on the signal: the attenuation one octave
//! (and two) from the cutoff for each mode against the analytic response, the peak at full
//! resonance, stability at maximum resonance under noise and an impulse at every sample rate,
//! no NaN and no denormal with every parameter swept to its bounds, smoothing of the cutoff
//! and of the mode (no step), block-size invariance, the mix, the reset and the descriptor.
//! The conformance run on every device of the registry is in `osc_synth.rs`; the native/WASM
//! parity is `crates/engine-wasm/tests/filter_parity.rs`.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::filter::{
    self, BAND_PASS, CUTOFF, DESCRIPTOR, Filter, HIGH_PASS, K_MIN, LOW_PASS, MIX, MODE, RESONANCE,
    damping,
};
use engine::msg::ParamId;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const SR: f32 = 48_000.0;

/// The most any bounded input of peak 1 can come out at: the L1 norm of the impulse response
/// at full resonance is about 32 for the low-pass, and one more for the high-pass.
const BOUND: f32 = 36.0;

fn with_rate(sr: f32, params: &[(u32, f32)]) -> Filter {
    let mut f = Filter::new(sr);
    for (id, value) in params {
        f.set_param(ParamId(*id), *value);
    }
    f.reset();
    f
}

/// A filter with these values, the ramps snapped (so a test starts at its values).
fn with(params: &[(u32, f32)]) -> Filter {
    with_rate(SR, params)
}

fn run_at(
    sr: f32,
    device: &mut Filter,
    input: (&[f32], &[f32]),
    size: usize,
    events: &[(usize, Event)],
) -> (Vec<f32>, Vec<f32>) {
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
    let mut start = 0;
    while start < input.0.len() {
        let end = (start + size).min(input.0.len());
        let mut left = input.0[start..end].to_vec();
        let mut right = input.1[start..end].to_vec();
        let local: Vec<Event> = events
            .iter()
            .filter(|(at, _)| (start..end).contains(at))
            .map(|(at, e)| e.at((at - start) as u32))
            .collect();
        let ctx = BlockCtx {
            sample_rate: sr,
            frames: (end - start) as u32,
            transport: Transport::default(),
        };
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left,
                right: &mut right,
                events_out: &mut unused,
            };
            device.process(&ctx, &local, &mut io);
        });
        out_l.extend(left);
        out_r.extend(right);
        start = end;
    }
    (out_l, out_r)
}

fn run(
    device: &mut Filter,
    input: (&[f32], &[f32]),
    size: usize,
    events: &[(usize, Event)],
) -> (Vec<f32>, Vec<f32>) {
    run_at(SR, device, input, size, events)
}

fn mono(device: &mut Filter, input: &[f32]) -> Vec<f32> {
    run(device, (input, input), 128, &[]).0
}

fn sine_at(sr: f32, len: usize, hz: f32, amplitude: f32) -> Vec<f32> {
    (0..len)
        .map(|n| amplitude * (core::f32::consts::TAU * hz * n as f32 / sr).sin())
        .collect()
}

fn sine(len: usize, hz: f32, amplitude: f32) -> Vec<f32> {
    sine_at(SR, len, hz, amplitude)
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

fn check(x: &[f32]) {
    for (i, s) in x.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is {s}");
        assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
    }
}

/// Deterministic noise in [-1, 1].
fn noise(len: usize, seed: u32) -> Vec<f32> {
    let mut s = seed | 1;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

fn param(id: u32, value: f32) -> Event {
    Event::ParamValue {
        offset: 0,
        id: ParamId(id),
        value,
    }
}

/// The gain, in decibels, of a steady sine at `hz` through a filter with `params` (the
/// second half of the render, once the start-up has settled).
fn gain_db(params: &[(u32, f32)], hz: f32) -> f32 {
    let mut f = with(params);
    let input = sine(SR as usize, hz, 0.25);
    let out = mono(&mut f, &input);
    let half = out.len() / 2;
    20.0 * (rms(&out[half..]) / rms(&input[half..])).log10()
}

/// The analytic gain in decibels of the second-order response with damping `k` at `w = f /
/// cutoff`.
fn analytic_db(mode: u32, k: f32, w: f32) -> f32 {
    let den = ((1.0 - w * w).powi(2) + k * k * w * w).sqrt();
    let num = match mode {
        LOW_PASS => 1.0,
        HIGH_PASS => w * w,
        _ => k * w,
    };
    20.0 * (num / den).log10()
}

// ---------------------------------------------------------------- the response

#[test]
fn one_octave_from_the_cutoff_each_mode_is_attenuated_as_the_analytic_response() {
    // No resonance (k = sqrt 2): the low-pass and the high-pass lose 12.3 dB one octave
    // from the cutoff, 24.6 dB two octaves from it.
    let k = damping(0.0);
    let cases = [
        (LOW_PASS, 2000.0, "low-pass one octave above"),
        (HIGH_PASS, 500.0, "high-pass one octave below"),
        (BAND_PASS, 2000.0, "band-pass one octave above"),
        (BAND_PASS, 500.0, "band-pass one octave below"),
        (LOW_PASS, 4000.0, "low-pass two octaves above"),
        (HIGH_PASS, 250.0, "high-pass two octaves below"),
    ];
    for (mode, hz, what) in cases {
        let measured = gain_db(
            &[
                (MODE, mode as f32),
                (CUTOFF, 1000.0),
                (RESONANCE, 0.0),
                (MIX, 1.0),
            ],
            hz,
        );
        let expected = analytic_db(mode, k, hz / 1000.0);
        println!("{what}: {measured:.2} dB (analytic {expected:.2} dB)");
        assert!(
            (measured - expected).abs() < 0.6,
            "{what}: {measured} dB, expected {expected} dB"
        );
    }
    // The 12 dB per octave of the issue, stated outright.
    let lp = gain_db(&[(MODE, 0.0), (CUTOFF, 1000.0), (RESONANCE, 0.0)], 2000.0);
    let hp = gain_db(&[(MODE, 1.0), (CUTOFF, 1000.0), (RESONANCE, 0.0)], 500.0);
    assert!((-13.0..-11.5).contains(&lp), "low-pass: {lp} dB");
    assert!((-13.0..-11.5).contains(&hp), "high-pass: {hp} dB");
}

#[test]
fn the_slope_is_12_db_per_octave_far_from_the_cutoff_at_the_default_resonance() {
    // At the default resonance (0.2) the asymptote is 12 dB per octave: two octaves apart
    // (400 Hz -> 800 Hz low-pass, 250 Hz -> 125 Hz high-pass) the loss is within 0.5 dB of 12.
    let lp = |hz| gain_db(&[(MODE, 0.0), (CUTOFF, 100.0)], hz);
    let hp = |hz| gain_db(&[(MODE, 1.0), (CUTOFF, 4000.0)], hz);
    let (a, b) = (lp(400.0), lp(800.0));
    assert!((a - b - 12.0).abs() < 0.8, "low-pass slope {}", a - b);
    let (a, b) = (hp(500.0), hp(250.0));
    assert!((a - b - 12.0).abs() < 0.8, "high-pass slope {}", a - b);
}

#[test]
fn the_default_resonance_matches_the_analytic_response_too() {
    let k = damping(0.2);
    for mode in [LOW_PASS, HIGH_PASS, BAND_PASS] {
        for hz in [250.0, 500.0, 1000.0, 2000.0, 4000.0] {
            let measured = gain_db(&[(MODE, mode as f32), (CUTOFF, 1000.0)], hz);
            let expected = analytic_db(mode, k, hz / 1000.0);
            assert!(
                (measured - expected).abs() < 0.6,
                "mode {mode} at {hz} Hz: {measured} dB, expected {expected} dB"
            );
        }
    }
}

#[test]
fn at_the_cutoff_the_band_pass_is_unity_and_the_flat_low_pass_is_minus_3_db() {
    let band = gain_db(&[(MODE, 2.0), (CUTOFF, 1000.0), (RESONANCE, 0.0)], 1000.0);
    assert!(band.abs() < 0.3, "band-pass at the cutoff: {band} dB");
    let band_peaky = gain_db(&[(MODE, 2.0), (CUTOFF, 1000.0), (RESONANCE, 1.0)], 1000.0);
    assert!(
        band_peaky.abs() < 0.3,
        "band-pass at full resonance: {band_peaky} dB"
    );
    let low = gain_db(&[(MODE, 0.0), (CUTOFF, 1000.0), (RESONANCE, 0.0)], 1000.0);
    assert!((low + 3.0).abs() < 0.3, "low-pass at the cutoff: {low} dB");
}

#[test]
fn the_low_pass_peaks_at_full_resonance_by_one_over_the_minimum_damping() {
    let peak_db = gain_db(&[(MODE, 0.0), (CUTOFF, 1000.0), (RESONANCE, 1.0)], 1000.0);
    let expected = 20.0 * (1.0 / K_MIN).log10();
    println!("peak at full resonance: {peak_db:.2} dB (expected {expected:.2} dB)");
    assert!((peak_db - expected).abs() < 1.0, "{peak_db} dB");
}

#[test]
fn the_cutoff_sits_where_it_is_set_across_the_range() {
    // The low-pass is 3 dB down at the cutoff, at several cutoffs (and the bilinear warp
    // near Nyquist stays under 1 dB at 10 kHz).
    for cutoff in [50.0, 200.0, 1000.0, 5000.0, 10_000.0] {
        let g = gain_db(&[(MODE, 0.0), (CUTOFF, cutoff), (RESONANCE, 0.0)], cutoff);
        assert!((g + 3.0).abs() < 1.0, "cutoff {cutoff}: {g} dB");
    }
}

// ---------------------------------------------------------------- stability

#[test]
fn it_is_stable_at_maximum_resonance_under_noise_for_every_mode_and_cutoff() {
    let input = noise(SR as usize * 2, 7);
    let bound = BOUND;
    let mut worst = 0f32;
    for mode in [LOW_PASS, HIGH_PASS, BAND_PASS] {
        for cutoff in [20.0, 100.0, 1000.0, 8000.0, 20_000.0] {
            let mut f = with(&[
                (MODE, mode as f32),
                (CUTOFF, cutoff),
                (RESONANCE, 1.0),
                (MIX, 1.0),
            ]);
            let out = mono(&mut f, &input);
            check(&out);
            worst = worst.max(peak(&out));
            assert!(
                peak(&out) < bound,
                "mode {mode}, cutoff {cutoff}: peak {}",
                peak(&out)
            );
        }
    }
    println!("largest peak under full-scale noise: {worst:.2} (bound {bound:.0})");
}

#[test]
fn an_impulse_at_maximum_resonance_rings_then_dies_away() {
    for mode in [LOW_PASS, HIGH_PASS, BAND_PASS] {
        for cutoff in [20.0, 60.0, 1000.0, 15_000.0, 20_000.0] {
            let mut f = with(&[(MODE, mode as f32), (CUTOFF, cutoff), (RESONANCE, 1.0)]);
            let mut input = vec![0.0; SR as usize * 6];
            input[0] = 1.0;
            let out = mono(&mut f, &input);
            check(&out);
            assert!(peak(&out) < BOUND, "peak {}", peak(&out));
            assert!(peak(&out) > 1e-4, "the impulse sounds");
            // The ring decays: the last second is a tiny fraction of the first.
            let tail = peak(&out[out.len() - SR as usize..]);
            assert!(tail < 1e-3, "mode {mode}, cutoff {cutoff}: tail {tail}");
        }
    }
}

#[test]
fn a_full_scale_square_wave_at_the_cutoff_reaches_about_the_worst_case_gain() {
    // The worst case of the bound: a square wave at the cutoff has a fundamental of 4/pi, which
    // the peak of 1 / K_MIN lifts to about 32 (and never beyond BOUND).
    let input: Vec<f32> = (0..SR as usize)
        .map(|n| if (n / 24) % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    let mut f = with(&[(MODE, 0.0), (CUTOFF, 1000.0), (RESONANCE, 1.0)]);
    let out = mono(&mut f, &input);
    check(&out);
    let worst = peak(&out[SR as usize / 2..]);
    println!("full-scale square at the cutoff, full resonance: peak {worst:.1}");
    assert!((24.0..BOUND).contains(&worst), "peak {worst}");
}

#[test]
fn it_stays_bounded_while_the_cutoff_and_resonance_are_swept_hard() {
    let input = noise(SR as usize * 2, 99);
    let mut f = with(&[(MODE, 0.0), (RESONANCE, 1.0)]);
    let mut events = Vec::new();
    for (i, at) in (0..SR as usize * 2).step_by(97).enumerate() {
        let cutoff = if i % 2 == 0 { 20.0 } else { 20_000.0 };
        events.push((at, param(CUTOFF, cutoff)));
        events.push((at, param(RESONANCE, if i % 3 == 0 { 0.0 } else { 1.0 })));
    }
    let out = run(&mut f, (&input, &input), 128, &events).0;
    check(&out);
    assert!(peak(&out) < BOUND, "peak {}", peak(&out));
}

#[test]
fn the_cutoff_stays_under_nyquist_at_every_sample_rate() {
    for sr in [
        8_000.0f32, 11_025.0, 22_050.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0,
    ] {
        for mode in [LOW_PASS, HIGH_PASS, BAND_PASS] {
            let mut f = with_rate(
                sr,
                &[(MODE, mode as f32), (CUTOFF, 20_000.0), (RESONANCE, 1.0)],
            );
            let input = noise(sr as usize, 3);
            let out = run_at(sr, &mut f, (&input, &input), 128, &[]).0;
            check(&out);
            assert!(peak(&out) < BOUND, "sr {sr}: peak {}", peak(&out));
        }
    }
}

#[test]
fn it_does_not_produce_nan_nor_denormals_from_a_loud_burst_into_silence() {
    for mode in [LOW_PASS, HIGH_PASS, BAND_PASS] {
        for resonance in [0.0, 0.5, 1.0] {
            let mut f = with(&[(MODE, mode as f32), (CUTOFF, 300.0), (RESONANCE, resonance)]);
            let mut input = noise(2_000, 5);
            input.extend(vec![0.0; SR as usize * 20]);
            let out = mono(&mut f, &input);
            check(&out);
            // A fully quiet end: the flush leaves exact zeros.
            assert_eq!(
                peak(&out[out.len() - 100..]),
                0.0,
                "mode {mode}, res {resonance}"
            );
        }
    }
}

#[test]
fn it_survives_non_finite_and_huge_input() {
    let mut input = noise(3_000, 11);
    input[10] = f32::NAN;
    input[20] = f32::INFINITY;
    input[30] = f32::NEG_INFINITY;
    input[40] = 3.0e38;
    input[50] = -3.0e38;
    input[60] = 1.0e-40;
    for mode in [LOW_PASS, HIGH_PASS, BAND_PASS] {
        let mut f = with(&[(MODE, mode as f32), (RESONANCE, 1.0)]);
        let out = mono(&mut f, &input);
        check(&out);
        assert!(peak(&out) < 1e6 * BOUND);
    }
}

#[test]
fn every_parameter_swept_to_its_bounds_gives_clean_output() {
    let input = noise(SR as usize, 21);
    for corner in 0..16u32 {
        let mut f = Filter::new(SR);
        let events: Vec<(usize, Event)> = DESCRIPTOR
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let high = (corner >> i) & 1 == 1;
                (0, param(p.id.0, (if high { p.max } else { p.min }) as f32))
            })
            .collect();
        let out = run(&mut f, (&input, &input), 64, &events).0;
        check(&out);
        assert!(peak(&out) < BOUND, "corner {corner}");
    }
}

// ---------------------------------------------------------------- smoothing

/// The largest step between two neighbouring samples.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn a_cutoff_jump_is_ramped_not_stepped() {
    // A 100 Hz sine through a low-pass: a jump of the cutoff from 200 Hz to 8 kHz in one
    // event must not put a step in the output; the new cutoff is reached after 20 ms.
    let input = sine(SR as usize, 100.0, 0.5);
    let steady = |cutoff: f32| {
        let mut f = with(&[(CUTOFF, cutoff), (RESONANCE, 0.0)]);
        mono(&mut f, &input)
    };
    let mut f = with(&[(CUTOFF, 200.0), (RESONANCE, 0.0)]);
    let at = 24_000;
    let out = run(
        &mut f,
        (&input, &input),
        128,
        &[(at, param(CUTOFF, 8000.0))],
    )
    .0;
    check(&out);
    // The sine's own slope is 0.5 * 2 pi 100 / 48000 = 0.0065 per sample. Across the ramp the
    // output sheds the phase lag of the 200 Hz filter within a few hundred samples (measured:
    // up to 0.0125 per sample); an unramped jump would shed it at once, a step of several tenths.
    let own = 0.5 * core::f32::consts::TAU * 100.0 / SR;
    let step = max_step(&out[at - 200..at + 2000]);
    assert!(step < own * 3.0, "step {step}");
    // 20 ms (960 frames) after the event, it is the steady state of the new cutoff.
    let reference = steady(8000.0);
    let from = at + 960 + 480;
    let d = out[from..from + 2000]
        .iter()
        .zip(&reference[from..from + 2000])
        .fold(0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(d < 0.01, "after the ramp: {d}");
    // And not before it: halfway through the ramp the cutoff is between the two, so the
    // output differs from the final steady state.
    let mid = at + 100;
    let early = (out[mid] - reference[mid]).abs();
    assert!(early > 1e-4, "the ramp is not instantaneous: {early}");
}

#[test]
fn the_cutoff_follows_the_contracts_linear_ramp_in_hertz() {
    // `start + (target - start) * i / n`, n = 960 frames (20 ms): halfway is the mean of the
    // two cutoffs, not their geometric mean.
    let mut f = with(&[(CUTOFF, 200.0)]);
    let input = vec![0.0; 480];
    let at = Event::ParamValue {
        offset: 0,
        id: ParamId(CUTOFF),
        value: 8000.0,
    };
    run(&mut f, (&input, &input), 480, &[(0, at)]);
    let halfway = f.cutoff_hz();
    assert!((halfway - 4100.0).abs() < 1.0, "{halfway}");
    run(&mut f, (&input, &input), 480, &[]);
    assert_eq!(f.cutoff_hz(), 8000.0);
}

#[test]
fn a_mode_change_crossfades_without_a_step() {
    // A 300 Hz sine, cutoff 300 Hz: low-pass to high-pass and to band-pass.
    let input = sine(SR as usize, 300.0, 0.5);
    for to in [1.0, 2.0] {
        let mut f = with(&[(CUTOFF, 300.0), (RESONANCE, 0.0)]);
        let at = 20_000;
        let out = run(&mut f, (&input, &input), 128, &[(at, param(MODE, to))]).0;
        check(&out);
        let own = 0.5 * core::f32::consts::TAU * 300.0 / SR;
        assert!(
            max_step(&out[at - 100..at + 1500]) < own * 2.0,
            "to {to}: step {}",
            max_step(&out[at - 100..at + 1500])
        );
    }
}

#[test]
fn the_render_does_not_depend_on_the_block_size() {
    let input = noise(20_000, 33);
    let events = [
        (500, param(CUTOFF, 4000.0)),
        (3_000, param(RESONANCE, 0.9)),
        (3_001, param(MODE, 2.0)),
        (9_000, param(MIX, 0.4)),
        (15_000, param(CUTOFF, 90.0)),
    ];
    let reference = run(&mut with(&[]), (&input, &input), 128, &events).0;
    for size in [1, 7, 64, 500, 4096] {
        let out = run(&mut with(&[]), (&input, &input), size, &events).0;
        assert_eq!(out, reference, "block size {size}");
    }
}

// ---------------------------------------------------------------- mix, reset, stereo

#[test]
fn the_mix_blends_the_dry_and_the_filtered_sound() {
    let input = noise(8_000, 41);
    let wet = mono(&mut with(&[(MIX, 1.0), (CUTOFF, 500.0)]), &input);
    let dry = mono(&mut with(&[(MIX, 0.0), (CUTOFF, 500.0)]), &input);
    let half = mono(&mut with(&[(MIX, 0.5), (CUTOFF, 500.0)]), &input);
    assert_eq!(dry, input, "mix 0 is the dry sound, bit for bit");
    assert_ne!(wet, input);
    for i in 0..input.len() {
        let expected = 0.5 * (input[i] + wet[i]);
        assert!((half[i] - expected).abs() < 1e-5, "frame {i}");
    }
}

#[test]
fn the_two_channels_are_filtered_independently() {
    let input = noise(4_000, 51);
    let silence = vec![0.0; input.len()];
    let mut f = with(&[(CUTOFF, 800.0)]);
    let (l, r) = run(&mut f, (&input, &silence), 128, &[]);
    assert!(peak(&l) > 0.1);
    assert_eq!(peak(&r), 0.0);
    let mut g = with(&[(CUTOFF, 800.0)]);
    let (l2, _) = run(&mut g, (&input, &input), 128, &[]);
    assert_eq!(l, l2);
}

#[test]
fn reset_empties_the_filter_and_keeps_the_parameter_values() {
    let mut f = with(&[(CUTOFF, 100.0), (RESONANCE, 1.0)]);
    let burst = noise(5_000, 61);
    let out = mono(&mut f, &burst);
    assert!(peak(&out) > 0.1);
    f.reset();
    let silence = vec![0.0; 2_000];
    assert_eq!(peak(&mono(&mut f, &silence)), 0.0, "no ring after reset");
    // The cutoff is still 100 Hz: a 1 kHz sine is far down, a 20 Hz one is not.
    let high = mono(&mut f, &sine(24_000, 1000.0, 0.5));
    f.reset();
    let low = mono(&mut f, &sine(24_000, 40.0, 0.5));
    assert!(rms(&high[12_000..]) < 0.02 * rms(&low[12_000..]).max(0.01));
}

#[test]
fn unknown_or_invalid_parameters_are_ignored() {
    let input = noise(4_000, 71);
    let reference = mono(&mut with(&[]), &input);
    let mut f = with(&[]);
    f.set_param(ParamId(99), 1.0);
    f.set_param(ParamId(CUTOFF), f32::NAN);
    f.set_param(ParamId(RESONANCE), f32::INFINITY);
    f.set_param(ParamId(MIX), f32::NEG_INFINITY);
    f.set_param(ParamId(MODE), f32::NAN);
    assert_eq!(mono(&mut f, &input), reference);
    // Out-of-range values clamp to the bounds.
    let mut a = with(&[]);
    a.set_param(ParamId(CUTOFF), 1.0e9);
    a.set_param(ParamId(RESONANCE), 7.0);
    a.set_param(ParamId(MODE), 9.0);
    a.reset();
    let mut b = with(&[(CUTOFF, 20_000.0), (RESONANCE, 1.0), (MODE, 2.0)]);
    assert_eq!(mono(&mut a, &input), mono(&mut b, &input));
}

// ---------------------------------------------------------------- the descriptor, the cost

#[test]
fn the_descriptor_names_the_device_and_its_defaults_are_the_constructors() {
    assert_eq!(DESCRIPTOR.kind, "faulk.filter");
    assert_eq!(DESCRIPTOR.name, "Filter");
    assert_eq!(DESCRIPTOR.params.len(), 4);
    let keys: Vec<&str> = DESCRIPTOR.params.iter().map(|p| p.key).collect();
    assert_eq!(keys, ["mode", "cutoff", "resonance", "mix"]);
    assert_eq!(DESCRIPTOR.param(ParamId(MODE)).unwrap().values.len(), 3);
    assert_eq!(filter::BUILTIN.descriptor.kind_id, DESCRIPTOR.kind_id);
    // A fresh filter and one set to the defaults render the same.
    let input = noise(4_000, 81);
    let defaults: Vec<(u32, f32)> = DESCRIPTOR
        .params
        .iter()
        .map(|p| (p.id.0, p.default as f32))
        .collect();
    assert_eq!(
        mono(&mut Filter::new(SR), &input),
        mono(&mut with(&defaults), &input)
    );
}

/// Cost of a block of 128 frames, measured (printed; run with `--release -- --nocapture`).
/// The assertion is loose: it only guards against a gross regression in any profile.
#[test]
fn the_cost_of_a_block_is_small() {
    let input = noise(128, 91);
    for (label, moving) in [("steady", false), ("cutoff sweeping", true)] {
        let mut f = with(&[(RESONANCE, 0.7)]);
        let blocks = 4_000;
        let ctx = BlockCtx {
            sample_rate: SR,
            frames: 128,
            transport: Transport::default(),
        };
        let mut unused = EventBuffer::try_with_capacity(1).unwrap();
        let mut left = input.clone();
        let mut right = input.clone();
        let start = std::time::Instant::now();
        for n in 0..blocks {
            left.copy_from_slice(&input);
            right.copy_from_slice(&input);
            let events = if moving {
                [Event::ParamValue {
                    offset: 0,
                    id: ParamId(CUTOFF),
                    value: if n % 2 == 0 { 200.0 } else { 8000.0 },
                }]
            } else {
                [Event::ParamValue {
                    offset: 0,
                    id: ParamId(MIX),
                    value: 1.0,
                }]
            };
            let mut io = AudioIo {
                left: &mut left,
                right: &mut right,
                events_out: &mut unused,
            };
            f.process(&ctx, &events, &mut io);
        }
        let per_block = start.elapsed().as_nanos() as f64 / f64::from(blocks);
        println!("{label}: {per_block:.0} ns per block of 128 frames (stereo)");
        // 128 frames last 2.67 ms; a debug build is allowed 100x the release cost.
        assert!(per_block < 1_000_000.0, "{label}: {per_block} ns");
    }
}
