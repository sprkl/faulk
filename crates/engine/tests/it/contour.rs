//! The equaliser, `faulk.contour` (#156): offline renders through the device contract, every
//! `process` under the allocation detector. Asserts on the signal:
//!
//! - the gain at the centre of each bell within 0.2 dB of its setting and its Q measured from
//!   the points at half its gain;
//! - the shelves, the cut bands at each slope (24 dB per octave: -24 dB one octave from the
//!   cutoff), the output gain, against [`Contour::response_db`];
//! - the device at its defaults (every gain at 0 dB) gives out its input unchanged;
//! - a fast automated sweep of every frequency neither clicks nor goes unstable;
//! - no NaN, no denormal, a bounded output with every parameter at its bounds and defaults;
//! - smoothing of the switches and the slopes (no step), block-size invariance, the reset.
//!
//! The conformance run on every device of the registry is in `osc_synth.rs`; the native/WASM
//! parity is `crates/engine-wasm/tests/contour_parity.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::contour::{
    BANDS, BELL_1, BELL_2, BELL_3, BandParam, Contour, DB12, DB24, DB48, DESCRIPTOR, HIGH_PASS,
    HIGH_SHELF, LOW_PASS, LOW_SHELF, OUTPUT, band_param,
};
use engine::msg::ParamId;

const SR: f32 = 48_000.0;

fn id(band: usize, what: BandParam) -> u32 {
    band_param(band, what).expect("the band has this parameter")
}

fn with_rate(sr: f32, params: &[(u32, f32)]) -> Contour {
    let mut c = Contour::new(sr);
    for (id, value) in params {
        c.set_param(ParamId(*id), *value);
    }
    c.reset();
    c
}

/// An equaliser with these values, the ramps snapped.
fn with(params: &[(u32, f32)]) -> Contour {
    with_rate(SR, params)
}

fn run_at(
    sr: f32,
    device: &mut Contour,
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
    device: &mut Contour,
    input: (&[f32], &[f32]),
    size: usize,
    events: &[(usize, Event)],
) -> (Vec<f32>, Vec<f32>) {
    run_at(SR, device, input, size, events)
}

fn mono(device: &mut Contour, input: &[f32]) -> Vec<f32> {
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
    ((x.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>()) / x.len() as f64).sqrt() as f32
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

/// Every band off, the output at 0 dB: a base the tests switch one band on over.
fn all_off() -> Vec<(u32, f32)> {
    (0..BANDS).map(|b| (id(b, BandParam::On), 0.0)).collect()
}

/// One band on with these settings, the others off.
fn only(band: usize, settings: &[(BandParam, f32)]) -> Vec<(u32, f32)> {
    let mut p = all_off();
    p.push((id(band, BandParam::On), 1.0));
    for (what, value) in settings {
        p.push((id(band, *what), *value));
    }
    p
}

/// The gain, in decibels, of a steady sine at `hz` through an equaliser with `params`,
/// measured on the second half of a render of `seconds` (once the start-up has settled).
fn measure_db(params: &[(u32, f32)], hz: f32) -> f32 {
    let mut c = with(params);
    // Long enough for the slowest settling (a narrow low band) to be gone from the second half.
    let len = (SR * if hz < 100.0 { 1.0 } else { 0.25 }) as usize;
    let input = sine(len, hz, 0.25);
    let out = mono(&mut c, &input);
    let half = out.len() / 2;
    20.0 * (rms(&out[half..]) / rms(&input[half..])).log10()
}

// ---------------------------------------------------------------- the descriptor

#[test]
fn the_descriptor_is_valid_with_stable_ids_and_one_english_name() {
    assert_eq!(DESCRIPTOR.validate(), Ok(()));
    assert_eq!(DESCRIPTOR.kind, "faulk.contour");
    assert_eq!(DESCRIPTOR.name, "Contour");
    assert_eq!(DESCRIPTOR.kind_id.0, 7);
    assert_eq!(DESCRIPTOR.params.len(), 29);
    // The ids are 0..29 in order, the keys `<band>_<what>` then `output`.
    let prefixes = ["hp", "low", "bell1", "bell2", "bell3", "high", "lp"];
    for (i, p) in DESCRIPTOR.params.iter().enumerate() {
        assert_eq!(p.id.0, i as u32);
    }
    for (b, prefix) in prefixes.iter().enumerate() {
        for what in [
            BandParam::On,
            BandParam::Freq,
            BandParam::Gain,
            BandParam::Q,
            BandParam::Slope,
        ] {
            let Some(pid) = band_param(b, what) else {
                continue;
            };
            let key = DESCRIPTOR.param(ParamId(pid)).unwrap().key;
            let suffix = match what {
                BandParam::On => "on",
                BandParam::Freq => "freq",
                BandParam::Gain => "gain",
                BandParam::Q => "q",
                BandParam::Slope => "slope",
            };
            assert_eq!(key, format!("{prefix}_{suffix}"));
        }
    }
    assert_eq!(DESCRIPTOR.param(ParamId(OUTPUT)).unwrap().key, "output");
    // The cut bands have a slope and no gain, the others a gain and no slope.
    for b in 0..BANDS {
        let cut = b == HIGH_PASS || b == LOW_PASS;
        assert_eq!(band_param(b, BandParam::Slope).is_some(), cut);
        assert_eq!(band_param(b, BandParam::Gain).is_some(), !cut);
    }
    // Every continuous parameter is smoothed, every choice is not.
    for p in DESCRIPTOR.params {
        if p.values.is_empty() {
            assert_eq!(p.smoothing_seconds, 0.02, "{}", p.key);
        } else {
            assert_eq!(p.smoothing_seconds, 0.0, "{}", p.key);
        }
    }
}

// ---------------------------------------------------------------- the response

#[test]
fn the_gain_at_the_centre_of_each_bell_is_within_0_2_db_of_its_setting() {
    let mut worst = 0f32;
    for band in [BELL_1, BELL_2, BELL_3] {
        for (hz, q) in [(100.0, 0.7), (1000.0, 1.0), (5000.0, 4.0), (12_000.0, 0.5)] {
            for gain in [-24.0, -12.0, -6.0, 3.0, 6.0, 12.0, 24.0] {
                let p = only(
                    band,
                    &[
                        (BandParam::Freq, hz),
                        (BandParam::Gain, gain),
                        (BandParam::Q, q),
                    ],
                );
                let measured = measure_db(&p, hz);
                worst = worst.max((measured - gain).abs());
                assert!(
                    (measured - gain).abs() <= 0.2,
                    "band {band} at {hz} Hz, Q {q}, {gain} dB: {measured} dB"
                );
            }
        }
    }
    println!("largest error at a bell's centre: {worst:.4} dB");
}

/// The frequency, between `lo` and `hi` (in hertz, log bisection), where the measured gain
/// crosses `target` dB; the gain is above `target` at `inside`.
fn crossing(params: &[(u32, f32)], target: f32, inside: f32, outside: f32) -> f32 {
    let (mut a, mut b) = (inside, outside);
    for _ in 0..18 {
        let mid = (a * b).sqrt();
        let g = measure_db(params, mid);
        let above = if target > 0.0 { g > target } else { g < target };
        if above {
            a = mid;
        } else {
            b = mid;
        }
    }
    (a * b).sqrt()
}

#[test]
fn each_bell_s_q_is_its_centre_over_the_bandwidth_at_half_its_gain() {
    // The analog Q at the points where the gain is half of the set gain, in decibels. The
    // bilinear warp narrows the band towards Nyquist, so the check stays well under it.
    for (band, hz, q, gain) in [
        (BELL_1, 300.0, 0.7, 12.0),
        (BELL_2, 1000.0, 2.0, -12.0),
        (BELL_3, 2000.0, 6.0, 9.0),
        (BELL_2, 500.0, 0.3, 6.0),
    ] {
        let p = only(
            band,
            &[
                (BandParam::Freq, hz),
                (BandParam::Gain, gain),
                (BandParam::Q, q),
            ],
        );
        let half = gain / 2.0;
        let lo = crossing(&p, half, hz, hz / 40.0);
        let hi = crossing(&p, half, hz, (hz * 40.0).min(0.45 * SR));
        // The bandwidth in analog frequencies: the warp of each point, exact at the centre.
        let warp = |f: f32| (core::f32::consts::PI * f / SR).tan();
        let measured = warp(hz) / (warp(hi) - warp(lo));
        println!("Q {q} at {hz} Hz, {gain} dB: {measured:.3} (points {lo:.1} Hz, {hi:.1} Hz)");
        assert!(
            (measured - q).abs() <= 0.03 * q,
            "Q {q}: measured {measured}"
        );
        // And in plain hertz, near the centre the warp is small: within 5 %.
        let plain = hz / (hi - lo);
        assert!((plain - q).abs() <= 0.05 * q, "Q {q}: plain {plain}");
    }
}

#[test]
fn the_shelves_reach_their_gain_far_from_the_corner_and_half_of_it_at_the_corner() {
    for gain in [-18.0f32, -6.0, 6.0, 18.0] {
        let low = only(
            LOW_SHELF,
            &[(BandParam::Freq, 300.0), (BandParam::Gain, gain)],
        );
        let far_below = measure_db(&low, 30.0);
        let corner = measure_db(&low, 300.0);
        let far_above = measure_db(&low, 6000.0);
        assert!(
            (far_below - gain).abs() < 0.3,
            "low shelf {gain}: {far_below}"
        );
        assert!(
            (corner - gain / 2.0).abs() < 0.2,
            "low shelf corner {gain}: {corner}"
        );
        assert!(far_above.abs() < 0.2, "low shelf above {gain}: {far_above}");
        let high = only(
            HIGH_SHELF,
            &[(BandParam::Freq, 3000.0), (BandParam::Gain, gain)],
        );
        let far_above = measure_db(&high, 18_000.0);
        let corner = measure_db(&high, 3000.0);
        let far_below = measure_db(&high, 150.0);
        assert!(
            (far_above - gain).abs() < 0.5,
            "high shelf {gain}: {far_above}"
        );
        assert!(
            (corner - gain / 2.0).abs() < 0.2,
            "high shelf corner {gain}: {corner}"
        );
        assert!(
            far_below.abs() < 0.2,
            "high shelf below {gain}: {far_below}"
        );
    }
}

#[test]
fn a_24_db_high_pass_takes_24_db_off_one_octave_under_its_cutoff() {
    let p = only(
        HIGH_PASS,
        &[(BandParam::Freq, 1000.0), (BandParam::Slope, DB24 as f32)],
    );
    let measured = measure_db(&p, 500.0);
    println!("24 dB/oct high-pass one octave below: {measured:.2} dB");
    assert!((measured + 24.0).abs() <= 2.0, "{measured} dB");
}

#[test]
fn each_slope_of_both_cut_bands_falls_as_its_order_one_octave_from_the_cutoff() {
    // A Butterworth of order n is 10 log10(1 + 2^(2n)) dB down one octave away: 12.3, 24.1, 48.2.
    for (slope, order) in [(DB12, 2), (DB24, 4), (DB48, 8)] {
        let expected = -10.0 * (1.0 + 2f32.powi(2 * order)).log10();
        for (band, cutoff, test_hz) in [(HIGH_PASS, 2000.0, 1000.0), (LOW_PASS, 1000.0, 2000.0)] {
            let p = only(
                band,
                &[(BandParam::Freq, cutoff), (BandParam::Slope, slope as f32)],
            );
            let measured = measure_db(&p, test_hz);
            println!("band {band}, order {order}: {measured:.2} dB (Butterworth {expected:.2})");
            assert!(
                (measured - expected).abs() <= 2.0,
                "{measured} vs {expected}"
            );
            // -3 dB at the cutoff, at every order (the default Q).
            let at = measure_db(&p, cutoff);
            assert!((at + 3.01).abs() < 0.3, "order {order} at the cutoff: {at}");
        }
    }
}

#[test]
fn the_measured_response_matches_the_analytic_one_with_every_band_working() {
    let p = vec![
        (id(HIGH_PASS, BandParam::On), 1.0),
        (id(HIGH_PASS, BandParam::Freq), 40.0),
        (id(LOW_SHELF, BandParam::Gain), 4.0),
        (id(BELL_1, BandParam::Gain), -6.0),
        (id(BELL_2, BandParam::Gain), 8.0),
        (id(BELL_2, BandParam::Q), 3.0),
        (id(BELL_3, BandParam::Gain), -3.0),
        (id(HIGH_SHELF, BandParam::Gain), -5.0),
        (id(LOW_PASS, BandParam::On), 1.0),
        (id(LOW_PASS, BandParam::Freq), 15_000.0),
        (id(LOW_PASS, BandParam::Slope), DB12 as f32),
        (OUTPUT, -2.0),
    ];
    let c = with(&p);
    let mut worst = 0f32;
    for hz in [
        30.0, 60.0, 120.0, 250.0, 700.0, 1000.0, 1500.0, 4000.0, 9000.0, 16_000.0,
    ] {
        let measured = measure_db(&p, hz);
        let expected = c.response_db(hz);
        worst = worst.max((measured - expected).abs());
        assert!(
            (measured - expected).abs() < 0.15,
            "{hz} Hz: {measured} dB, analytic {expected} dB"
        );
    }
    println!("largest gap to the analytic response: {worst:.4} dB");
}

#[test]
fn the_output_gain_scales_the_whole_signal() {
    for db in [-24.0f32, -6.0, 6.0, 24.0] {
        let measured = measure_db(&[(OUTPUT, db)], 1000.0);
        assert!((measured - db).abs() < 0.01, "{db}: {measured}");
    }
}

#[test]
fn at_its_defaults_every_gain_at_0_db_gives_the_input_back_unchanged() {
    let input_l = noise(SR as usize, 3);
    let input_r = sine(SR as usize, 440.0, 0.7);
    let mut c = Contour::new(SR);
    let (l, r) = run(&mut c, (&input_l, &input_r), 128, &[]);
    assert_eq!(l, input_l);
    assert_eq!(r, input_r);
    // The bands moved but at 0 dB, a Q and a slope changed: still exact.
    let mut c = with(&[
        (id(BELL_2, BandParam::Freq), 3000.0),
        (id(BELL_2, BandParam::Q), 12.0),
        (id(LOW_SHELF, BandParam::Q), 0.2),
        (id(HIGH_PASS, BandParam::Slope), DB48 as f32),
    ]);
    let (l, _) = run(&mut c, (&input_l, &input_l), 77, &[]);
    assert_eq!(l, input_l);
}

// ---------------------------------------------------------------- modulation

/// The largest step between two consecutive samples.
fn largest_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

#[test]
fn a_fast_automated_sweep_of_every_frequency_neither_clicks_nor_goes_unstable() {
    // A 300 Hz sine at -6 dBFS, every band on and working, every frequency swept 20 Hz to
    // 20 kHz in 50 ms and back (exponential, events every 64 frames), four times.
    let input = sine(SR as usize, 300.0, 0.5);
    let base = vec![
        (id(HIGH_PASS, BandParam::On), 1.0),
        (id(LOW_SHELF, BandParam::Gain), 12.0),
        (id(BELL_1, BandParam::Gain), 12.0),
        (id(BELL_1, BandParam::Q), 4.0),
        (id(BELL_2, BandParam::Gain), -12.0),
        (id(BELL_3, BandParam::Gain), 12.0),
        (id(BELL_3, BandParam::Q), 8.0),
        (id(HIGH_SHELF, BandParam::Gain), 12.0),
        (id(LOW_PASS, BandParam::On), 1.0),
    ];
    let sweep_frames = (0.05 * SR) as usize;
    let mut events = Vec::new();
    for at in (0..input.len()).step_by(64) {
        let phase = (at % (2 * sweep_frames)) as f32 / sweep_frames as f32;
        let t = if phase < 1.0 { phase } else { 2.0 - phase };
        let hz = 20.0 * 1000f32.powf(t);
        for b in 0..BANDS {
            // The cut bands sweep the other way, so they do not mute the sine at once.
            let f = if b == HIGH_PASS {
                20.0 * 1000f32.powf(t * 0.4)
            } else if b == LOW_PASS {
                20_000.0 / 1000f32.powf(t * 0.4)
            } else {
                hz
            };
            events.push((at, param(id(b, BandParam::Freq), f)));
        }
    }
    let mut c = with(&base);
    let (out, _) = run(&mut c, (&input, &input), 128, &events);
    check(&out);
    // The same settings, parked: the steepest a steady output gets. The sine's own step is
    // 2 pi 300 / 48000 * 0.5 = 0.02; the bands lift it by at most +24 dB (x 15.8) here.
    let steady_bound = 0.5 * core::f32::consts::TAU * 300.0 / SR * 15.85;
    let step = largest_step(&out);
    println!(
        "fast sweep: peak {:.3}, largest step {step:.4} (steady bound {steady_bound:.4})",
        peak(&out)
    );
    assert!(peak(&out) < 16.0, "bounded: {}", peak(&out));
    assert!(step < steady_bound, "a click: step {step} > {steady_bound}");
}

#[test]
fn a_switch_and_a_slope_change_fade_without_a_step() {
    let input = sine(SR as usize / 2, 200.0, 0.5);
    for (what, value) in [
        (id(BELL_1, BandParam::On), 0.0),
        (id(HIGH_PASS, BandParam::On), 1.0),
        (id(HIGH_PASS, BandParam::Slope), DB48 as f32),
        (id(LOW_SHELF, BandParam::On), 0.0),
    ] {
        let mut c = with(&[
            (id(BELL_1, BandParam::Freq), 200.0),
            (id(BELL_1, BandParam::Gain), 18.0),
            (id(LOW_SHELF, BandParam::Gain), -12.0),
            (id(HIGH_PASS, BandParam::Freq), 400.0),
            (
                id(HIGH_PASS, BandParam::On),
                if what == id(HIGH_PASS, BandParam::On) {
                    0.0
                } else {
                    1.0
                },
            ),
        ]);
        let at = SR as usize / 4;
        let (out, _) = run(&mut c, (&input, &input), 128, &[(at, param(what, value))]);
        check(&out);
        let before = largest_step(&out[at - 2_000..at]);
        let across = largest_step(&out[at - 1..at + 2_000]);
        let after = largest_step(&out[out.len() - 2_000..]);
        println!("param {what}: step before {before:.4}, across {across:.4}, after {after:.4}");
        // A fade never steps more than the steeper of the two steady outputs (a margin of 10 %
        // for the start-up of a band that fades in).
        assert!(
            across <= 1.1 * before.max(after),
            "param {what}: {across} > {before} / {after}"
        );
    }
}

#[test]
fn the_render_does_not_depend_on_the_block_size() {
    let input = noise(SR as usize / 2, 77);
    let events = vec![
        (1_000, param(id(BELL_2, BandParam::Gain), 12.0)),
        (5_003, param(id(BELL_2, BandParam::Freq), 6000.0)),
        (9_000, param(id(HIGH_PASS, BandParam::On), 1.0)),
        (12_345, param(id(HIGH_PASS, BandParam::Slope), DB12 as f32)),
        (15_000, param(OUTPUT, -9.0)),
    ];
    let mut a = Contour::new(SR);
    let mut b = Contour::new(SR);
    let (ra, _) = run(&mut a, (&input, &input), 128, &events);
    let (rb, _) = run(&mut b, (&input, &input), 37, &events);
    assert_eq!(ra, rb);
}

// ---------------------------------------------------------------- bounds and safety

#[test]
fn every_parameter_at_its_bounds_and_at_its_default_gives_clean_bounded_output() {
    let input = noise(SR as usize, 21);
    let mut seed = 0x1234_5678u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let (mut worst, mut transient) = (0f32, 0f32);
    for case in 0..64 {
        let mut c = Contour::new(SR);
        let events: Vec<(usize, Event)> = DESCRIPTOR
            .params
            .iter()
            .map(|p| {
                let value = match (case, next() % 3) {
                    (0, _) => p.min,
                    (1, _) => p.max,
                    (2, _) => p.default,
                    (_, 0) => p.min,
                    (_, 1) => p.max,
                    _ => p.default,
                };
                (0, param(p.id.0, value as f32))
            })
            .collect();
        let (out, _) = run(&mut c, (&input, &input), 128, &events);
        check(&out);
        // Bounded by the chain's own highest gain: five bands at +24 dB on the same frequency
        // and +24 dB of output are +144 dB, as the parameters allow. The bound is the largest
        // gain of the analytic response over a fine grid, with a margin (x 4) for the transients
        // of noise through the narrowest peaks.
        let top = (0..=4000)
            .map(|i| c.response_db(1.0 * (0.499 * SR).powf(i as f32 / 4000.0)))
            .fold(f32::MIN, f32::max);
        // Once the 20 ms ramps are over and their transients have died (the second half): the ramps pass through settings
        // in between (two bells crossing on their way), which the settled response does not
        // hold, so the whole render is only held finite and free of denormals above.
        let settled = peak(&out[out.len() / 2..]);
        let bound = 4.0 * 10f32.powf(top / 20.0);
        worst = worst.max(settled / bound);
        transient = transient.max(peak(&out));
        assert!(
            settled < bound,
            "case {case}: settled peak {settled} over the bound {bound} ({top} dB)"
        );
    }
    println!(
        "every parameter at its bounds: settled peak at most {worst:.3} of its bound, largest peak while ramping {transient:.1}"
    );
}

#[test]
fn it_stays_clean_at_every_sample_rate_with_every_frequency_at_its_top() {
    for sr in [
        8_000.0f32, 22_050.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0,
    ] {
        let mut p: Vec<(u32, f32)> = (0..BANDS)
            .map(|b| (id(b, BandParam::Freq), 20_000.0))
            .collect();
        p.push((id(HIGH_PASS, BandParam::On), 1.0));
        p.push((id(LOW_PASS, BandParam::On), 1.0));
        p.push((id(HIGH_PASS, BandParam::Q), 18.0));
        p.push((id(LOW_PASS, BandParam::Q), 18.0));
        p.push((id(BELL_2, BandParam::Gain), 24.0));
        let mut c = with_rate(sr, &p);
        let input = noise(sr as usize / 2, 3);
        let out = run_at(sr, &mut c, (&input, &input), 128, &[]).0;
        check(&out);
        assert!(peak(&out) < 1e4, "sr {sr}: peak {}", peak(&out));
    }
}

#[test]
fn an_impulse_through_the_sharpest_settings_rings_then_dies_away() {
    let mut p = Vec::new();
    for b in 0..BANDS {
        p.push((id(b, BandParam::On), 1.0));
        p.push((id(b, BandParam::Q), 18.0));
        p.push((id(b, BandParam::Freq), 60.0));
        if let Some(g) = band_param(b, BandParam::Gain) {
            p.push((g, 24.0));
        }
    }
    p.push((id(HIGH_PASS, BandParam::Freq), 20.0));
    p.push((id(HIGH_PASS, BandParam::Slope), DB48 as f32));
    p.push((id(LOW_PASS, BandParam::Slope), DB48 as f32));
    let mut c = with(&p);
    let mut input = vec![0.0; SR as usize * 12];
    input[0] = 1.0;
    let out = mono(&mut c, &input);
    check(&out);
    assert!(peak(&out) > 1e-3, "the impulse sounds");
    let tail = peak(&out[out.len() - SR as usize..]);
    println!(
        "sharpest impulse: peak {:.2}, last second {tail:e}",
        peak(&out)
    );
    assert!(tail < 1e-4, "tail {tail}");
}

#[test]
fn a_loud_burst_into_silence_ends_in_exact_zeros_without_denormals() {
    let mut c = with(&[
        (id(HIGH_PASS, BandParam::On), 1.0),
        (id(LOW_PASS, BandParam::On), 1.0),
        (id(BELL_2, BandParam::Gain), 12.0),
        (id(BELL_2, BandParam::Q), 10.0),
        (id(LOW_SHELF, BandParam::Gain), -12.0),
    ]);
    let mut input = noise(2_000, 5);
    input.extend(vec![0.0; SR as usize * 20]);
    let out = mono(&mut c, &input);
    check(&out);
    assert_eq!(peak(&out[out.len() - 100..]), 0.0);
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
    let mut c = with(&[(id(BELL_1, BandParam::Gain), 24.0), (OUTPUT, 24.0)]);
    let out = mono(&mut c, &input);
    check(&out);
    // A non-finite value or a wild one is ignored.
    c.set_param(ParamId(id(BELL_1, BandParam::Freq)), f32::NAN);
    c.set_param(ParamId(999), 1.0);
    let out = mono(&mut c, &input);
    check(&out);
}

#[test]
fn the_reset_empties_the_bands_and_snaps_the_ramps() {
    let mut c = with(&[
        (id(BELL_2, BandParam::Gain), 12.0),
        (id(BELL_2, BandParam::Q), 15.0),
    ]);
    let _ = mono(&mut c, &noise(4_000, 9));
    c.set_param(ParamId(id(BELL_2, BandParam::Gain)), -12.0);
    c.reset();
    assert_eq!(c.band(BELL_2).unwrap().gain_db, -12.0);
    // Empty: silence in, silence out at once.
    let out = mono(&mut c, &[0.0; 512]);
    assert_eq!(peak(&out), 0.0);
}

// ---------------------------------------------------------------- the panel's curve

/// The settings of the curve fixture: each a list of `(key, value)` over the defaults, and a
/// sample rate.
fn fixture_cases() -> Vec<(Vec<(&'static str, f32)>, f32)> {
    vec![
        (vec![], 48_000.0),
        (
            vec![
                ("hp_on", 1.0),
                ("hp_freq", 80.0),
                ("hp_slope", 2.0),
                ("low_gain", 6.0),
                ("low_q", 1.5),
                ("bell1_gain", -9.0),
                ("bell1_q", 3.0),
                ("bell2_gain", 12.0),
                ("bell2_freq", 2500.0),
                ("bell3_gain", -24.0),
                ("bell3_q", 18.0),
                ("high_gain", -6.0),
                ("lp_on", 1.0),
                ("lp_freq", 12_000.0),
                ("lp_slope", 0.0),
                ("lp_q", 4.0),
                ("output", -3.0),
            ],
            48_000.0,
        ),
        (
            vec![
                ("hp_on", 1.0),
                ("hp_q", 0.3),
                ("lp_on", 1.0),
                ("lp_freq", 20_000.0),
                ("lp_slope", 1.0),
                ("high_gain", 24.0),
                ("high_freq", 18_000.0),
                ("bell1_on", 0.0),
                ("bell1_gain", 24.0),
            ],
            44_100.0,
        ),
    ]
}

/// The response the panel draws (`web/src/lib/contour/response.ts`) is held to the engine's by
/// a fixture this test writes with `FAULK_BLESS=1` and otherwise compares: a change of the
/// engine's response that the fixture does not follow fails here, and the page's test fails on
/// a curve that does not follow the fixture.
#[test]
fn the_panel_s_curve_fixture_is_the_engine_s_response() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/lib/contour/response.fixture.json");
    let hz: Vec<f32> = (0..=60)
        .map(|i| 20.0 * 1000f32.powf(i as f32 / 60.0))
        .collect();
    let cases: Vec<serde_json::Value> = fixture_cases()
        .into_iter()
        .map(|(params, sr)| {
            let mut c = Contour::new(sr);
            for (key, value) in &params {
                let p = DESCRIPTOR.params.iter().find(|p| p.key == *key).unwrap();
                c.set_param(p.id, *value);
            }
            c.reset();
            let values: serde_json::Map<String, serde_json::Value> = DESCRIPTOR
                .params
                .iter()
                .map(|p| {
                    let v = params
                        .iter()
                        .find(|(k, _)| *k == p.key)
                        .map_or(p.default, |(_, v)| f64::from(*v));
                    (p.key.to_owned(), serde_json::json!(v))
                })
                .collect();
            let points: Vec<serde_json::Value> = hz
                .iter()
                .map(|f| serde_json::json!([f, (c.response_db(*f) * 1e4).round() / 1e4]))
                .collect();
            serde_json::json!({ "sample_rate": sr, "params": values, "points": points })
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::json!({ "cases": cases })).unwrap() + "\n";
    if std::env::var_os("FAULK_BLESS").is_some() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        on_disk == text,
        "{} is stale: FAULK_BLESS=1 cargo test -p engine --test it contour::the_panel",
        path.display()
    );
}
