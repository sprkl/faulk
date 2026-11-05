//! The compressor, `faulk.compressor` (DV-5): offline renders through the device contract,
//! every `process` under the allocation detector. Asserts on the signal: the static curve
//! against the ratio (a steady sine at several levels, thresholds, ratios and makeups), the
//! attack and release time constants, the bound on the output (no overshoot beyond the
//! makeup gain), no NaN and no denormal with every parameter swept to its bounds, the
//! parameter safety, the block-size invariance and the reset. The contract's conformance run
//! on every device of the registry is in `osc_synth.rs`; the native/WASM parity is
//! `crates/engine-wasm/tests/compressor_parity.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::compressor::{
    self, ATTACK, AUTO_FAST_SECONDS, Compressor, DESCRIPTOR, DETECTION, FADE_SECONDS, KNEE,
    KNEE_DB, LOOKAHEAD, MAKEUP, MAKEUP_MAX_DB, MIX, RATIO, RELEASE, RELEASE_MODE, SETTLE_SECONDS,
    SIDECHAIN_FREQ, SIDECHAIN_HPF, THRESHOLD, gain_reduction_db,
};
use engine::msg::ParamId;

const SR: f32 = 48_000.0;

/// A compressor with these values, the ramps snapped (so a test starts at its values).
fn with(params: &[(u32, f32)]) -> Compressor {
    let mut c = Compressor::new(SR).unwrap();
    for (id, value) in params {
        c.set_param(ParamId(*id), *value);
    }
    c.reset();
    c
}

/// Runs `input` (left, right) through the device in blocks of `size`, `events` applying at
/// their (absolute) frame.
fn run(
    device: &mut Compressor,
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
            sample_rate: SR,
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

fn mono(device: &mut Compressor, input: &[f32]) -> Vec<f32> {
    run(device, (input, input), 128, &[]).0
}

fn sine(len: usize, hz: f32, amplitude: f32) -> Vec<f32> {
    (0..len)
        .map(|n| amplitude * (core::f32::consts::TAU * hz * n as f32 / SR).sin())
        .collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

fn db(x: f32) -> f32 {
    20.0 * x.log10()
}

fn undb(x: f32) -> f32 {
    10f32.powf(x / 20.0)
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

// ---------------------------------------------------------------- the static curve

/// The analytic output level of a sine of peak `input_db` through the curve.
fn expected_db(input_db: f32, threshold: f32, ratio: f32, makeup: f32) -> f32 {
    input_db - gain_reduction_db(input_db, threshold, 1.0 - 1.0 / ratio, KNEE_DB) + makeup
}

#[test]
fn the_curve_matches_the_textbook_formula() {
    // Independent of the device's helper: far below the knee nothing, far above the straight
    // line `T + over / ratio`, and the knee joins them continuously.
    let (t, ratio) = (-20.0f32, 4.0f32);
    let curve = |x: f32| x - gain_reduction_db(x, t, 1.0 - 1.0 / ratio, KNEE_DB);
    assert_eq!(curve(-40.0), -40.0);
    assert_eq!(curve(t - KNEE_DB / 2.0), t - KNEE_DB / 2.0);
    assert!((curve(0.0) - (t + 20.0 / ratio)).abs() < 1e-5);
    assert!((curve(t + KNEE_DB / 2.0) - (t + KNEE_DB / 2.0 / ratio)).abs() < 1e-5);
    // The knee is monotonic and continuous.
    let mut last = curve(-40.0);
    let mut x = -40.0;
    while x < 0.0 {
        let y = curve(x);
        assert!(y >= last - 1e-6 && y - last < 0.1, "x {x}");
        last = y;
        x += 0.05;
    }
}

#[test]
fn a_steady_sine_comes_out_where_the_ratio_puts_it() {
    // Tolerance: 0.15 dB. The detector follows the peak; the long release keeps the
    // reduction from drooping between two peaks of the 1 kHz sine.
    let tolerance = 0.15;
    let mut worst = 0f32;
    let mut cases = 0;
    for threshold in [-30.0f32, -18.0, -6.0] {
        for ratio in [1.5f32, 2.0, 4.0, 10.0, 20.0] {
            for makeup in [0.0f32, 6.0] {
                for input_db in [-60.0f32, -40.0, -30.0, -24.0, -18.0, -12.0, -6.0, -3.0, 0.0] {
                    let mut c = with(&[
                        (THRESHOLD, threshold),
                        (RATIO, ratio),
                        (ATTACK, 0.001),
                        (RELEASE, 0.5),
                        (MAKEUP, makeup),
                    ]);
                    let out = mono(&mut c, &sine(SR as usize, 1000.0, undb(input_db)));
                    check(&out);
                    let measured = db(peak(&out[out.len() - 4_800..]));
                    let wanted = expected_db(input_db, threshold, ratio, makeup);
                    let error = (measured - wanted).abs();
                    worst = worst.max(error);
                    cases += 1;
                    assert!(
                        error < tolerance,
                        "T {threshold} R {ratio} makeup {makeup} in {input_db}: {measured} dB, wanted {wanted}"
                    );
                    // Above the knee, the ratio itself, without the helper: every 1 dB over
                    // the threshold comes out 1/ratio dB over it.
                    if input_db >= threshold + KNEE_DB / 2.0 {
                        let line = threshold + (input_db - threshold) / ratio + makeup;
                        assert!(
                            (measured - line).abs() < tolerance,
                            "T {threshold} R {ratio} in {input_db}: {measured} dB, line {line}"
                        );
                    }
                }
            }
        }
    }
    println!("static curve: {cases} cases, worst error {worst:.3} dB");
}

#[test]
fn below_the_knee_and_at_ratio_one_the_signal_passes_untouched() {
    let input = noise(4_800, 7);
    let quiet: Vec<f32> = input.iter().map(|s| s * undb(-50.0)).collect();
    // -50 dBFS peak is below the knee of a -40 dB threshold (-43 dB).
    let mut c = with(&[(THRESHOLD, -40.0), (RATIO, 20.0)]);
    assert_eq!(mono(&mut c, &quiet), quiet);
    // Ratio 1 turns nothing down, at any level.
    let mut c = with(&[(THRESHOLD, -60.0), (RATIO, 1.0)]);
    assert_eq!(mono(&mut c, &input), input);
}

#[test]
fn the_makeup_adds_its_decibels() {
    let input = sine(4_800, 1000.0, undb(-50.0));
    let mut c = with(&[(THRESHOLD, 0.0), (MAKEUP, 12.0)]);
    let out = mono(&mut c, &input);
    let gain = db(peak(&out) / peak(&input));
    assert!((gain - 12.0).abs() < 0.01, "{gain}");
}

#[test]
fn both_channels_share_one_gain() {
    // A loud left and a quiet right: the right is turned down by the left's level.
    let loud = sine(24_000, 1000.0, 1.0);
    let quiet = sine(24_000, 1000.0, 0.01);
    let mut c = with(&[(THRESHOLD, -20.0), (RATIO, 10.0), (ATTACK, 0.001)]);
    let (l, r) = run(&mut c, (&loud, &quiet), 128, &[]);
    let gain_l = peak(&l[12_000..]) / 1.0;
    let gain_r = peak(&r[12_000..]) / 0.01;
    assert!(
        gain_l < 0.5 && (gain_l - gain_r).abs() < 0.002,
        "{gain_l} {gain_r}"
    );
}

// ---------------------------------------------------------------- the time constants

/// The gain in decibels (negative: reduction) at each frame of a constant input.
fn gain_db(out: &[f32], input: f32) -> Vec<f32> {
    out.iter().map(|s| db(s / input)).collect()
}

#[test]
fn the_attack_is_a_time_constant() {
    // A constant 0.5 (-6 dB) over a -30 dB threshold at 4:1: 18 dB of reduction in the end.
    let reduction = 18.0f32;
    for attack in [0.0001f32, 0.001, 0.005, 0.02, 0.1] {
        let mut c = with(&[
            (THRESHOLD, -30.0),
            (RATIO, 4.0),
            (ATTACK, attack),
            (RELEASE, 2.0),
        ]);
        let n = (SR * attack * 12.0) as usize + 100;
        let out = mono(&mut c, &vec![0.5; n]);
        let g = gain_db(&out, 0.5);
        let last = -g[g.len() - 1];
        assert!((last - reduction).abs() < 0.05, "ends at {last} dB");
        let frame = g
            .iter()
            .position(|v| -v >= reduction * (1.0 - (-1.0f32).exp()))
            .expect("reached");
        let measured = (frame + 1) as f32 / SR;
        let error = (measured - attack).abs();
        println!("attack {attack}: 63% after {measured:.6} s");
        assert!(error <= attack * 0.03 + 1.0 / SR, "{attack}: {measured}");
    }
}

#[test]
fn the_release_is_a_time_constant() {
    // Exact whatever the attack, a slow one included (the attack stage only filters a rise).
    let reduction = 18.0f32;
    for attack in [0.0001f32, 0.01, 0.2] {
        for release in [0.01f32, 0.05, 0.3, 1.0] {
            let params = [
                (THRESHOLD, -30.0),
                (RATIO, 4.0),
                (ATTACK, attack),
                (RELEASE, release),
            ];
            // Settle on the full reduction, then fall to a tiny constant (digital silence
            // has no measurable gain).
            let settle = (SR * attack * 12.0) as usize + 4_800;
            let mut input = vec![0.5; settle];
            input.extend(vec![1e-4; (SR * release * 6.0) as usize]);
            let out = mono(&mut with(&params), &input);
            let settled = -db(out[settle - 1] / 0.5);
            assert!((settled - reduction).abs() < 0.05, "{settled}");
            let g: Vec<f32> = out[settle..].iter().map(|s| -db(s / 1e-4)).collect();
            let frame = g
                .iter()
                .position(|v| *v <= reduction * (-1.0f32).exp())
                .expect("reached");
            let measured = (frame + 1) as f32 / SR;
            println!("attack {attack} release {release}: down to 37% after {measured:.5} s");
            assert!(
                (measured - release).abs() <= release * 0.03 + 1.0 / SR,
                "{release}: {measured}"
            );
            assert!(*g.last().unwrap() < 0.1);
        }
    }
}

#[test]
fn a_short_release_on_a_low_note_modulates_the_gain_with_the_wave() {
    // The known limit of a peak detector: the hold droops between two peaks of a low note,
    // so the gain breathes with the wave (a distortion). Measured as the swing of the gain
    // (out / in, in decibels) over the loud part of the last cycles of a 50 Hz sine, and
    // bounded; a long release keeps it small, and a 1 kHz note is unaffected.
    let swing = |hz: f32, release: f32| {
        let mut c = with(&[
            (THRESHOLD, -30.0),
            (RATIO, 4.0),
            (ATTACK, 0.001),
            (RELEASE, release),
        ]);
        let input = sine(48_000, hz, 0.5);
        let out = mono(&mut c, &input);
        let from = 48_000 - (SR / hz) as usize * 4;
        let gains: Vec<f32> = (from..48_000)
            .filter(|&n| input[n].abs() > 0.25)
            .map(|n| db(out[n] / input[n]))
            .collect();
        let max = gains.iter().cloned().fold(f32::MIN, f32::max);
        let min = gains.iter().cloned().fold(f32::MAX, f32::min);
        max - min
    };
    let (fast, default, slow, high) = (
        swing(50.0, 0.01),
        swing(50.0, 0.15),
        swing(50.0, 1.0),
        swing(1000.0, 0.01),
    );
    println!(
        "gain swing over the loud part of a cycle: 50 Hz at 10 ms release {fast:.2} dB, at 150 ms {default:.2} dB, at 1 s {slow:.2} dB; 1 kHz at 10 ms {high:.2} dB"
    );
    assert!(fast > default && default > slow, "{fast} {default} {slow}");
    assert!(fast < 6.0 && default < 1.5 && slow < 0.5 && high < 0.5);
}

// ---------------------------------------------------------------- the bound

#[test]
fn the_output_never_exceeds_the_input_times_the_makeup() {
    // A step from silence to a full-scale sine, 30+ dB over the threshold, with the fastest
    // and the slowest attack: the sample-by-sample bound, and the first cycle above the steady
    // level, measured (no look-ahead: the rising edge passes before the attack closes).
    let makeup = 6.0;
    let bound = undb(makeup) * 1.000_01;
    for attack in [0.0001f32, 0.001, 0.01, 0.2] {
        let mut input = vec![0.0; 2_000];
        input.extend(sine(48_000, 1000.0, 1.0));
        let mut c = with(&[
            (THRESHOLD, -40.0),
            (RATIO, 20.0),
            (ATTACK, attack),
            (RELEASE, 0.1),
            (MAKEUP, makeup),
        ]);
        let out = mono(&mut c, &input);
        check(&out);
        for (i, (o, x)) in out.iter().zip(&input).enumerate() {
            assert!(o.abs() <= x.abs() * bound, "frame {i}: {o} for {x}");
        }
        let steady = peak(&out[out.len() - 4_800..]);
        let first_ms = peak(&out[2_000..2_048]);
        let first_cycle_db = db(first_ms / steady);
        println!(
            "attack {attack}: first millisecond peaks {first_cycle_db:.1} dB over the steady level"
        );
        // The ceiling of the first cycle is the makeup'd input itself.
        assert!(first_ms <= undb(makeup) * 1.000_01);
        // With the fastest attack the first cycle (a rising sine, which the gain follows
        // within a few samples) stays 10 dB under the full-scale input.
        if attack <= 0.0001 {
            assert!(first_ms < undb(-10.0), "{first_ms}");
        }
    }
}

#[test]
fn a_full_scale_burst_never_gets_louder_than_the_makeup_allows() {
    let mut c = with(&[
        (THRESHOLD, -60.0),
        (RATIO, 20.0),
        (ATTACK, 0.2),
        (MAKEUP, 24.0),
    ]);
    let input = noise(48_000, 99);
    let out = mono(&mut c, &input);
    check(&out);
    assert!(peak(&out) <= undb(MAKEUP_MAX_DB as f32) * 1.000_01);
}

// ---------------------------------------------------------------- safety

#[test]
fn every_parameter_at_its_bounds_stays_finite_and_bounded() {
    let bounds: Vec<(u32, [f32; 2])> = DESCRIPTOR
        .params
        .iter()
        .map(|p| (p.id.0, [p.min as f32, p.max as f32]))
        .collect();
    assert_eq!(bounds.len(), 12);
    let defaults: Vec<f32> = DESCRIPTOR.params.iter().map(|p| p.default as f32).collect();
    let mut garbage = noise(2_000, 5);
    garbage[10] = f32::NAN;
    garbage[11] = f32::INFINITY;
    garbage[12] = f32::NEG_INFINITY;
    garbage[13] = 1e30;
    garbage[14] = -1e30;
    garbage[15] = 1e-40;
    garbage[16] = 1e6;
    garbage[17] = -1e6;
    let loud: Vec<f32> = noise(2_000, 17).iter().map(|s| s * 1e6).collect();
    let mut impulses = vec![0.0; 2_000];
    impulses[100] = 1.0;
    impulses[700] = -1.0;
    let signals: [(&str, Vec<f32>, f32); 5] = [
        ("noise", noise(2_000, 3), 1.0),
        ("silence", vec![0.0; 2_000], 0.0),
        ("impulses", impulses, 1.0),
        ("garbage", garbage, 1e6),
        ("loud", loud, 1e6),
    ];
    let mut cases = 0;
    // 2^12 corners is too many to render each: every parameter at its minimum, its maximum
    // and its default in turn (the others at a bound or their default, from a hash), 300
    // mixed corners, all at the minimum and all at the maximum.
    let mut corners: Vec<Vec<(u32, f32)>> = Vec::new();
    let mut seed = 0x9e37_79b9u32;
    for k in 0..300u32 {
        corners.push(
            bounds
                .iter()
                .zip(&defaults)
                .map(|((id, b), d)| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let pick = if k < 3 { k } else { seed % 3 };
                    (*id, [b[0], b[1], *d][pick as usize])
                })
                .collect(),
        );
    }
    for i in 0..bounds.len() {
        for pick in 0..3 {
            corners.push(
                bounds
                    .iter()
                    .zip(&defaults)
                    .enumerate()
                    .map(|(j, ((id, b), d))| {
                        let v = [b[0], b[1], *d];
                        (
                            *id,
                            if i == j {
                                v[pick]
                            } else {
                                v[(i + j + pick) % 3]
                            },
                        )
                    })
                    .collect(),
            );
        }
    }
    for params in &corners {
        for (name, input, amplitude) in &signals {
            let mut c = with(params);
            let out = mono(&mut c, input);
            check(&out);
            let limit = amplitude * undb(MAKEUP_MAX_DB as f32) * 1.000_1;
            assert!(peak(&out) <= limit, "{name} at {params:?}: {}", peak(&out));
            cases += 1;
        }
    }
    println!("{cases} sweep cases");
}

#[test]
fn parameters_moved_to_their_bounds_mid_stream_stay_bounded() {
    let mut c = Compressor::new(SR).unwrap();
    let input = noise(48_000, 21);
    let mut events = Vec::new();
    for (k, at) in (0..48_000).step_by(997).enumerate() {
        for p in DESCRIPTOR.params {
            let value = if (k + p.id.0 as usize).is_multiple_of(2) {
                p.min
            } else {
                p.max
            };
            events.push((at, param(p.id.0, value as f32)));
        }
    }
    let (l, r) = run(&mut c, (&input, &input), 211, &events);
    check(&l);
    check(&r);
    assert!(peak(&l) <= undb(MAKEUP_MAX_DB as f32) * 1.000_1);
}

#[test]
fn bad_values_and_ids_are_ignored_or_clamped() {
    let input = sine(9_600, 440.0, 0.8);
    let render = |params: &[(u32, f32)]| {
        let mut c = with(params);
        let kept = format!("{c:?}");
        // A value the device refuses leaves it as it was.
        c.set_param(ParamId(RATIO), f32::NAN);
        c.set_param(ParamId(THRESHOLD), f32::INFINITY);
        c.set_param(ParamId(99), 1.0);
        c.set_param(ParamId(u32::MAX), 1.0);
        assert_eq!(kept, format!("{c:?}"));
        mono(&mut c, &input)
    };
    // Out of range is clamped to the bound.
    assert_eq!(
        render(&[
            (RATIO, 1000.0),
            (THRESHOLD, 10.0),
            (MAKEUP, 99.0),
            (ATTACK, 0.0),
            (RELEASE, 99.0)
        ]),
        render(&[
            (RATIO, 20.0),
            (THRESHOLD, 0.0),
            (MAKEUP, 24.0),
            (ATTACK, 0.0001),
            (RELEASE, 2.0)
        ]),
    );
}

#[test]
fn the_render_does_not_depend_on_the_block_size() {
    let input = noise(9_600, 1);
    let events = [
        (1_000, param(THRESHOLD, -40.0)),
        (3_333, param(RATIO, 12.0)),
        (3_333, param(ATTACK, 0.0005)),
        (6_000, param(MAKEUP, 9.0)),
        (7_001, param(RELEASE, 0.03)),
    ];
    let render = |size: usize| {
        let mut c = with(&[]);
        run(&mut c, (&input, &input), size, &events)
    };
    let whole = render(9_600);
    for size in [1, 37, 128, 1_000] {
        assert_eq!(render(size), whole, "block size {size}");
    }
}

#[test]
fn a_parameter_change_moves_the_gain_without_a_jump() {
    // The threshold falls by 30 dB in the middle of a steady sine: the 20 ms ramp keeps the
    // gain from stepping (the attack still has to close it).
    let input = sine(48_000, 100.0, 0.5);
    let mut c = with(&[(THRESHOLD, 0.0), (RATIO, 20.0), (ATTACK, 0.05)]);
    let out = run(
        &mut c,
        (&input, &input),
        128,
        &[(24_000, param(THRESHOLD, -30.0))],
    )
    .0;
    let mut worst = 0f32;
    for n in 1..out.len() {
        let (a, b) = (out[n - 1], out[n]);
        let step = (b - a).abs() - (input[n] - input[n - 1]).abs();
        worst = worst.max(step);
    }
    assert!(worst < 0.01, "{worst}");
}

#[test]
fn reset_opens_the_gain_and_keeps_the_parameters() {
    let params = [
        (THRESHOLD, -40.0),
        (RATIO, 20.0),
        (ATTACK, 0.001),
        (RELEASE, 2.0),
        (MAKEUP, 3.0),
    ];
    let loud = sine(96_000, 1000.0, 1.0);
    let probe = sine(4_800, 1000.0, undb(-20.0));
    let mut used = with(&params);
    mono(&mut used, &loud);
    // Still turned down 2 s release: the gain has not recovered.
    let before = mono(&mut used, &probe);
    used.reset();
    let after = mono(&mut used, &probe);
    let fresh = mono(&mut with(&params), &probe);
    assert!(peak(&before) < 0.5 * peak(&fresh), "the gain was closed");
    assert_eq!(after, fresh, "a reset device renders as a new one");
    // The parameters survived: the same probe is compressed, not passed through.
    assert!(peak(&after) < peak(&probe) * undb(3.0) * 0.9);
}

#[test]
fn the_descriptor_is_valid() {
    assert_eq!(DESCRIPTOR.validate(), Ok(()));
    assert_eq!(DESCRIPTOR.name, "Compressor");
    assert_eq!(DESCRIPTOR.kind, "faulk.compressor");
    let keys: Vec<_> = DESCRIPTOR.params.iter().map(|p| p.key).collect();
    assert_eq!(
        keys,
        [
            "threshold",
            "ratio",
            "knee",
            "attack",
            "release_mode",
            "release",
            "detection",
            "lookahead",
            "sidechain_hpf",
            "sidechain_freq",
            "makeup",
            "mix"
        ]
    );
    // The MVP's ids did not move.
    for (key, id) in [
        ("threshold", THRESHOLD),
        ("ratio", RATIO),
        ("attack", ATTACK),
        ("release", RELEASE),
        ("makeup", MAKEUP),
    ] {
        assert_eq!(
            DESCRIPTOR
                .params
                .iter()
                .find(|p| p.key == key)
                .map(|p| p.id.0),
            Some(id),
            "{key}"
        );
    }
    let built = compressor::factory(&engine::device::DeviceSetup::default()).unwrap();
    assert_eq!(built.descriptor().kind, "faulk.compressor");
    // Each default is the one the device is built with.
    let mut c = Compressor::new(SR).unwrap();
    let defaults: Vec<_> = DESCRIPTOR
        .params
        .iter()
        .map(|p| (p.id.0, p.default as f32))
        .collect();
    let mut explicit = with(&defaults);
    let input = sine(4_800, 300.0, 0.9);
    assert_eq!(mono(&mut c, &input), mono(&mut explicit, &input));
}

// ---------------------------------------------------------------- the cost

/// `cargo test -p engine --test it compressor::cpu_cost --config "profile.dev.opt-level=3" -- --ignored --nocapture`
/// (the allocation detector does not build in release; debug assertions stay on, so this is an upper bound)
#[test]
#[ignore = "timing, run by hand in release"]
fn cpu_cost() {
    // Process-only timing: blocks of 128 frames of loud noise (the gain computer working),
    // no allocation in the loop. Best of 50 runs of one second of audio.
    let source = noise(48_000, 4);
    let mut left = source.clone();
    let mut right = source.clone();
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    for (name, params) in [
        ("default", vec![]),
        ("heavy", vec![(THRESHOLD, -40.0), (RATIO, 20.0)]),
        (
            "every #157 stage",
            vec![
                (THRESHOLD, -40.0),
                (RATIO, 20.0),
                (DETECTION, 1.0),
                (SIDECHAIN_HPF, 1.0),
                (RELEASE_MODE, 1.0),
                (LOOKAHEAD, 0.01),
                (MIX, 0.5),
            ],
        ),
    ] {
        let mut c = with(&params);
        let ctx = BlockCtx {
            sample_rate: SR,
            frames: 128,
            transport: Transport::default(),
        };
        let mut best = f64::MAX;
        for _ in 0..50 {
            left.copy_from_slice(&source);
            right.copy_from_slice(&source);
            let started = std::time::Instant::now();
            for (l, r) in left.chunks_mut(128).zip(right.chunks_mut(128)) {
                let mut io = AudioIo {
                    left: l,
                    right: r,
                    events_out: &mut unused,
                };
                let ctx = BlockCtx {
                    frames: io.left.len() as u32,
                    ..ctx
                };
                c.process(&ctx, &[], &mut io);
            }
            best = best.min(started.elapsed().as_secs_f64());
        }
        std::hint::black_box((&left, &right));
        let per_frame = best / 48_000.0 * 1e9;
        let block = per_frame * 128.0 / 1e3;
        println!(
            "compressor ({name}): {per_frame:.1} ns per frame, {block:.2} us per 128-frame block ({:.3}% of its 2.67 ms budget)",
            block / 2_666.7 * 100.0
        );
    }
}

// ---------------------------------------------------------------- #157: the MVP's device

#[path = "reference/old_compressor.rs"]
mod old_compressor;

/// The five parameters of the MVP, by id, with values from the musical to the bounds.
fn mvp_script(len: usize) -> Vec<(usize, Event)> {
    let mut seed = 0x1234_5677u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    let mut events = Vec::new();
    for at in (0..len).step_by(1_733) {
        let id = (next() * 5.0) as u32 % 5;
        let p = DESCRIPTOR.param(ParamId(id)).unwrap();
        let t = next();
        // A bound one time in four, else a value inside (geometric when the bounds are positive).
        let value = if t < 0.125 {
            p.min
        } else if t < 0.25 {
            p.max
        } else if p.min > 0.0 {
            p.min * (p.max / p.min).powf(f64::from(next()))
        } else {
            p.min + (p.max - p.min) * f64::from(next())
        };
        events.push((at, param(id, value as f32)));
    }
    events
}

#[test]
fn at_the_new_defaults_the_device_renders_the_mvp_s_samples_bit_for_bit() {
    // Noise at three levels, bursts of a low sine (a kick), garbage samples: the gain
    // computer at work on every path of the MVP's device.
    let len = 96_000;
    let mut left: Vec<f32> = noise(len, 11)
        .iter()
        .enumerate()
        .map(|(n, s)| s * [0.05, 0.5, 1.0][(n / 12_000) % 3])
        .collect();
    let mut right = noise(len, 12);
    for (n, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
        if (n % 9_000) < 2_400 {
            let kick = (core::f32::consts::TAU * 55.0 * n as f32 / SR).sin() * 0.9;
            *l += kick;
            *r = *r * 0.2 + kick;
        }
    }
    left[5_000] = f32::NAN;
    right[5_001] = f32::INFINITY;
    left[7_000] = 1e-30;
    let script = mvp_script(len);
    for size in [1usize, 37, 128, 500] {
        let mut new = Compressor::new(SR).unwrap();
        let mut old = old_compressor::Compressor::new(SR);
        // A first half, a reset, the second half: the reset is the MVP's too.
        let half = len / 2;
        let mut got = (Vec::new(), Vec::new());
        let mut want = (Vec::new(), Vec::new());
        for (from, to) in [(0, half), (half, len)] {
            let events: Vec<(usize, Event)> = script
                .iter()
                .filter(|(at, _)| (from..to).contains(at))
                .map(|(at, e)| (at - from, *e))
                .collect();
            let (l, r) = run(&mut new, (&left[from..to], &right[from..to]), size, &events);
            got.0.extend(l);
            got.1.extend(r);
            // The reference, event by event at the same frames.
            let (mut l, mut r) = (left[from..to].to_vec(), right[from..to].to_vec());
            let mut cursor = 0;
            for (at, e) in &events {
                old.render(&mut l[cursor..*at], &mut r[cursor..*at]);
                cursor = *at;
                if let Event::ParamValue { id, value, .. } = e {
                    old.set_param(id.0, *value);
                }
            }
            let end = l.len();
            old.render(&mut l[cursor..end], &mut r[cursor..end]);
            want.0.extend(l);
            want.1.extend(r);
            assert_eq!(new.gain_reduction_db(), old.gain_reduction_db());
            new.reset();
            old.reset();
        }
        let differ = got
            .0
            .iter()
            .zip(&want.0)
            .chain(got.1.iter().zip(&want.1))
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        assert_eq!(differ, 0, "block size {size}: {differ} samples differ");
        assert!(peak(&want.0) > 0.01, "the reference sounds");
    }
}

// ---------------------------------------------------------------- #157: the look-ahead

/// A sine of `hz` at `amplitude` that starts at `onset` (silence before), `len` frames.
fn burst(len: usize, onset: usize, hz: f32, amplitude: f32) -> Vec<f32> {
    (0..len)
        .map(|n| {
            if n < onset {
                0.0
            } else {
                amplitude * (core::f32::consts::TAU * hz * (n - onset) as f32 / SR).sin()
            }
        })
        .collect()
}

#[test]
fn a_5_ms_lookahead_turns_a_hit_down_before_it_passes() {
    // A 0 dBFS tone starts at once: 24 dB over a -24 dB threshold, at 20:1 with a hard knee
    // the static curve puts it at -22.8 dBFS.
    let (threshold, ratio) = (-24.0f32, 20.0f32);
    let curve = threshold + 24.0 / ratio;
    let input = burst(24_000, 4_800, 1_000.0, 1.0);
    let out_peak = |lookahead: f32, attack: f32| {
        let mut c = with(&[
            (THRESHOLD, threshold),
            (RATIO, ratio),
            (KNEE, 0.0),
            (ATTACK, attack),
            (RELEASE, 0.1),
            (LOOKAHEAD, lookahead),
        ]);
        assert_eq!(c.latency_frames(), (lookahead * SR).round() as u32);
        let out = mono(&mut c, &input);
        check(&out);
        db(peak(&out))
    };
    let caught = out_peak(0.005, 0.001);
    let missed = out_peak(0.0, 0.001);
    let slow = out_peak(0.005, 0.01);
    println!(
        "hit 24 dB over a -24 dB threshold, 20:1, curve {curve:.2} dBFS: lookahead 5 ms + attack 1 ms: peak {caught:.2} dBFS ({:+.2} dB over the curve, {:+.2} dB over the threshold); no lookahead: {missed:.2} dBFS ({:+.2} dB over the curve); lookahead 5 ms + attack 10 ms: {slow:.2} dBFS ({:+.2} dB)",
        caught - curve,
        caught - threshold,
        missed - curve,
        slow - curve
    );
    // The margin over the curve: the attack has covered 1 - exp(-5) of the way when the hit
    // comes out (0.15 dB short of 22.8 dB).
    assert!(caught <= curve + 0.25, "{caught}");
    assert!(caught <= threshold + 1.5, "{caught}");
    assert!(missed > curve + 6.0, "{missed}");
}

#[test]
fn with_a_lookahead_the_release_waits_for_the_loud_part_to_leave() {
    // A loud tone stops dead; with 10 ms of look-ahead and a 10 ms release, the last 10 ms
    // of the tone come out as turned down as the rest (the window keeps the reduction until
    // the tone has left the output).
    let len = 24_000;
    let stop = 14_400;
    let input: Vec<f32> = burst(len, 0, 500.0, 1.0)
        .iter()
        .enumerate()
        .map(|(n, s)| if n < stop { *s } else { 0.0 })
        .collect();
    let mut c = with(&[
        (THRESHOLD, -24.0),
        (RATIO, 20.0),
        (KNEE, 0.0),
        (ATTACK, 0.001),
        (RELEASE, 0.01),
        (LOOKAHEAD, 0.01),
    ]);
    let out = mono(&mut c, &input);
    let l = 480;
    // The tone's last 10 ms at the output against 10 ms in its middle.
    let tail = db(peak(&out[stop + l - 480..stop + l]));
    let body = db(peak(&out[stop / 2..stop / 2 + 480]));
    println!("end of the tone {tail:.2} dBFS, body {body:.2} dBFS");
    assert!((tail - body).abs() < 0.2, "tail {tail} body {body}");
    // After the tone, silence (the delayed zeros).
    assert_eq!(peak(&out[stop + l..]), 0.0);
}

/// The delay, in frames, of an impulse through a compressor at `rate` with this look-ahead
/// (well under the threshold: the gain is 1), and its declared latency.
fn impulse_latency(rate: f32, lookahead: f32) -> (usize, u32) {
    let mut c = Compressor::new(rate).unwrap();
    c.set_param(ParamId(LOOKAHEAD), lookahead);
    c.reset();
    let declared = c.latency_frames();
    let at = 10;
    let len = 4_096;
    let mut left = vec![0.0f32; len];
    left[at] = 0.01;
    let mut right = left.clone();
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    for (l, r) in left.chunks_mut(128).zip(right.chunks_mut(128)) {
        let ctx = BlockCtx {
            sample_rate: rate,
            frames: l.len() as u32,
            transport: Transport::default(),
        };
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: l,
                right: r,
                events_out: &mut unused,
            };
            c.process(&ctx, &[], &mut io);
        });
    }
    let found = left.iter().position(|s| *s != 0.0).unwrap();
    assert_eq!(left[found], 0.01, "the gain is 1 under the threshold");
    assert_eq!(left, right);
    (found - at, declared)
}

#[test]
fn the_declared_latency_is_the_measured_one() {
    for rate in [44_100.0f32, 48_000.0, 96_000.0, 192_000.0] {
        let (measured, declared) = impulse_latency(rate, 0.005);
        // The parameter is an f32: 5 ms at 44.1 kHz is 220.49 frames, so 220.
        let frames = (f64::from(0.005f32) * f64::from(rate)).round() as u32;
        assert_eq!(declared, frames, "{rate}");
        assert!(
            (f64::from(declared) / f64::from(rate) - 0.005).abs() <= 0.5 / f64::from(rate) + 1e-9
        );
        assert_eq!(measured, declared as usize, "{rate}");
        println!("5 ms at {rate} Hz: declared {declared} frames, measured {measured}");
        for lookahead in [0.0f32, 0.0001, 0.001, 0.0033, 0.01] {
            let (measured, declared) = impulse_latency(rate, lookahead);
            assert_eq!(measured, declared as usize, "{rate} {lookahead}");
            assert_eq!(
                declared,
                (f64::from(lookahead) * f64::from(rate)).round() as u32,
                "{rate} {lookahead}"
            );
        }
    }
    assert_eq!(impulse_latency(48_000.0, 0.005), (240, 240));
}

#[test]
fn a_lookahead_change_while_playing_fades_out_waits_and_fades_back_in() {
    // A steady tone under the threshold (gain 1): the output is the input, then the input
    // 240 frames late, with one fade out, one silence of the new latency, one fade in, for a
    // whole drag of the control (a value every 64 frames for 23 values).
    let len = 48_000;
    // -26 dBFS: under the knee of the default -18 dB threshold (it starts at -21 dB).
    let input = sine(len, 300.0, 0.05);
    let mut c = with(&[]);
    let start = 9_600;
    let mut events = Vec::new();
    for k in 0..22 {
        events.push((
            start + 64 * k,
            param(LOOKAHEAD, 0.000_2 * k as f32 + 0.000_6),
        ));
    }
    let last = start + 64 * 22;
    events.push((last, param(LOOKAHEAD, 0.005)));
    let (out, _) = run(&mut c, (&input, &input), 128, &events);
    check(&out);
    assert_eq!(c.latency_frames(), 240);
    let settle = (SETTLE_SECONDS * SR) as usize;
    let fade = (FADE_SECONDS * SR) as usize;
    // Untouched until the drag has rested for the settle time (the old latency, 0).
    let first = (0..last + settle).find(|&n| out[n] != input[n]);
    assert_eq!(
        first,
        None,
        "{:?}",
        first.map(|n| (n, out[n], input[n], last, settle))
    );
    // Never louder than the input.
    assert!(peak(&out) <= 0.05 + 1e-6);
    // One silent run, as long as the new latency.
    let mut runs = Vec::new();
    let mut zeros = 0;
    for s in &out[last..] {
        if *s == 0.0 {
            zeros += 1;
        } else {
            if zeros > 20 {
                runs.push(zeros);
            }
            zeros = 0;
        }
    }
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!((240..=245).contains(&runs[0]), "{runs:?}");
    // Afterwards, the input 240 frames late, bit for bit.
    let after = last + settle + 2 * fade + 240 + 16;
    for n in after..len {
        assert_eq!(out[n], input[n - 240], "frame {n}");
    }
}

#[test]
fn a_reset_hides_what_the_line_held() {
    let mut c = with(&[(LOOKAHEAD, 0.01)]);
    mono(&mut c, &noise(4_800, 3));
    c.reset();
    let out = mono(&mut c, &[0.0; 960]);
    assert_eq!(peak(&out), 0.0);
    assert_eq!(c.latency_frames(), 480);
}

// ---------------------------------------------------------------- #157: the knee

/// The static curve of the plan, written again: the output level of a level `x` (dB).
fn knee_curve(x: f32, t: f32, ratio: f32, w: f32) -> f32 {
    let over = x - t;
    let slope = 1.0 - 1.0 / ratio;
    let gr = if 2.0 * over <= -w {
        0.0
    } else if 2.0 * over >= w {
        slope * over
    } else {
        slope * (over + w / 2.0).powi(2) / (2.0 * w)
    };
    x - gr
}

#[test]
fn the_static_curve_with_a_12_db_knee_is_the_formula() {
    // A constant level (DC) reads exactly what it is: the settled output is the curve.
    let (t, ratio) = (-30.0f32, 4.0f32);
    let mut worst = 0f32;
    let mut cases = 0;
    for w in [12.0f32, 0.0, 6.0, 24.0] {
        let mut x = t - 18.0;
        while x <= t + 18.0 {
            let mut c = with(&[
                (THRESHOLD, t),
                (RATIO, ratio),
                (KNEE, w),
                (ATTACK, 0.001),
                (RELEASE, 0.01),
            ]);
            let out = mono(&mut c, &vec![undb(x); 4_800]);
            let measured = db(*out.last().unwrap());
            let wanted = knee_curve(x, t, ratio, w);
            let error = (measured - wanted).abs();
            worst = worst.max(error);
            cases += 1;
            assert!(
                error < 0.01,
                "knee {w} at {x}: {measured} dB, wanted {wanted}"
            );
            x += 0.5;
        }
    }
    // With a 12 dB knee, at the threshold the reduction is slope * 6² / 24 = 1.125 dB at 4:1.
    let at_t = knee_curve(t, t, ratio, 12.0);
    assert!((t - at_t - 1.125).abs() < 1e-4, "{at_t}");
    println!("static curve, knees 12/0/6/24 dB: {cases} levels, worst error {worst:.5} dB");
}

// ---------------------------------------------------------------- #157: the mix

#[test]
fn at_mix_0_the_output_is_the_input_delayed_by_the_latency() {
    let input_l: Vec<f32> = noise(24_000, 5).iter().map(|s| s * 0.9).collect();
    let input_r = noise(24_000, 6);
    for lookahead in [0.0f32, 0.005] {
        let mut c = with(&[
            (THRESHOLD, -40.0),
            (RATIO, 20.0),
            (MAKEUP, 12.0),
            (MIX, 0.0),
            (LOOKAHEAD, lookahead),
        ]);
        let latency = c.latency_frames() as usize;
        let (l, r) = run(&mut c, (&input_l, &input_r), 128, &[]);
        assert_eq!(c.gain_reduction_db(), 0.0, "nothing heard is turned down");
        for n in 0..l.len() {
            let (wl, wr) = if n >= latency {
                (input_l[n - latency], input_r[n - latency])
            } else {
                (0.0, 0.0)
            };
            assert_eq!(
                (l[n].to_bits(), r[n].to_bits()),
                (wl.to_bits(), wr.to_bits()),
                "{n}"
            );
        }
    }
}

#[test]
fn a_mix_between_blends_the_dry_and_the_compressed_sound() {
    let input = noise(24_000, 9);
    let params = |mix: f32| {
        [
            (THRESHOLD, -30.0),
            (RATIO, 8.0),
            (MAKEUP, 6.0),
            (LOOKAHEAD, 0.002),
            (MIX, mix),
        ]
    };
    let wet = mono(&mut with(&params(1.0)), &input);
    let dry = mono(&mut with(&params(0.0)), &input);
    for m in [0.3f32, 0.5] {
        let out = mono(&mut with(&params(m)), &input);
        let worst = (0..out.len())
            .map(|n| (out[n] - (dry[n] * (1.0 - m) + wet[n] * m)).abs())
            .fold(0f32, f32::max);
        assert!(worst < 1e-6, "{m}: {worst}");
    }
}

#[test]
fn the_reduction_shown_follows_the_mix() {
    // A steady tone 20 dB over the threshold at 20:1: 19 dB on the compressed path.
    let input = sine(48_000, 1_000.0, undb(-10.0));
    let shown = |mix: f32| {
        let mut c = with(&[
            (THRESHOLD, -30.0),
            (RATIO, 20.0),
            (KNEE, 0.0),
            (ATTACK, 0.001),
            (RELEASE, 0.5),
            (MIX, mix),
        ]);
        mono(&mut c, &input);
        c.gain_reduction_db()
    };
    let full = shown(1.0);
    assert!((full - 19.0).abs() < 0.1, "{full}");
    let half = shown(0.5);
    let wanted = -20.0 * (0.5 + 0.5 * undb(-full)).log10();
    assert!((half - wanted).abs() < 0.01, "{half} {wanted}");
    assert_eq!(shown(0.0), 0.0);
    println!("reduction shown at mix 1 / 0.5 / 0: {full:.2} / {half:.2} / 0 dB");
}

// ---------------------------------------------------------------- #157: the detection

/// The largest reduction over the last `tail` frames of `input`, read frame by frame.
fn settled_reduction(params: &[(u32, f32)], input: &[f32], tail: usize) -> f32 {
    let mut c = with(params);
    let split = input.len() - tail;
    mono(&mut c, &input[..split]);
    let mut most = 0f32;
    for s in &input[split..] {
        run(&mut c, (&[*s], &[*s]), 1, &[]);
        most = most.max(c.gain_reduction_db());
    }
    most
}

#[test]
fn rms_detection_reads_a_sine_3_db_under_its_peak_and_a_square_at_it() {
    let (t, ratio) = (-30.0f32, 4.0f32);
    let slope = 1.0 - 1.0 / ratio;
    let base = [
        (THRESHOLD, t),
        (RATIO, ratio),
        (KNEE, 0.0),
        (ATTACK, 0.001),
        (RELEASE, 1.0),
    ];
    let mut rms = base.to_vec();
    rms.push((DETECTION, 1.0));
    let mut report = Vec::new();
    for hz in [200.0f32, 1_000.0, 5_000.0] {
        let input = sine(48_000, hz, undb(-6.0));
        let peak_gr = settled_reduction(&base, &input, 4_800);
        let rms_gr = settled_reduction(&rms, &input, 4_800);
        let want_peak = slope * (-6.0 - t);
        let want_rms = slope * (-6.0 - 3.0103 - t);
        report.push(format!(
            "{hz} Hz: peak {peak_gr:.3} dB (formula {want_peak:.3}), rms {rms_gr:.3} dB (formula {want_rms:.3})"
        ));
        assert!((peak_gr - want_peak).abs() < 0.05, "{hz}: {peak_gr}");
        // The one-pole's ripple at twice the frequency lifts the held reduction (measured:
        // printed below), most at the lowest frequency.
        let tolerance = if hz < 500.0 { 0.2 } else { 0.05 };
        assert!((rms_gr - want_rms).abs() < tolerance, "{hz}: {rms_gr}");
    }
    let square: Vec<f32> = sine(48_000, 1_000.0, 1.0)
        .iter()
        .map(|s| if *s >= 0.0 { 0.5 } else { -0.5 })
        .collect();
    let square_gr = settled_reduction(&rms, &square, 4_800);
    assert!(
        (square_gr - slope * (-6.0206 - t)).abs() < 0.05,
        "{square_gr}"
    );
    println!(
        "{}\nsquare -6 dB, rms: {square_gr:.3} dB",
        report.join("\n")
    );
}

#[test]
fn the_sidechain_high_pass_stops_the_lows_from_driving_the_gain() {
    // A 0 dBFS 40 Hz sine over a -30 dB threshold at 20:1.
    let (t, ratio) = (-30.0f32, 20.0f32);
    let slope = 1.0 - 1.0 / ratio;
    // Two seconds, a 0.3 s release: the high-pass's start (its transient) has long been
    // released when the reduction is read.
    let input = sine(96_000, 40.0, 1.0);
    let base = vec![
        (THRESHOLD, t),
        (RATIO, ratio),
        (KNEE, 0.0),
        (ATTACK, 0.001),
        (RELEASE, 0.3),
    ];
    let off = settled_reduction(&base, &input, 2_400);
    assert!((off - slope * (0.0 - t)).abs() < 0.1, "{off}");
    for hz in [100.0f32, 200.0] {
        let mut on = base.clone();
        on.extend([(SIDECHAIN_HPF, 1.0), (SIDECHAIN_FREQ, hz)]);
        let measured = settled_reduction(&on, &input, 2_400);
        // The 2nd-order Butterworth high-pass at 40 Hz, `w = 40 / hz`.
        let w = 40.0 / hz;
        let attenuation = 20.0 * (w * w / ((1.0 - w * w).powi(2) + 2.0 * w * w).sqrt()).log10();
        let wanted = (slope * (attenuation - t)).max(0.0);
        println!(
            "high-pass {hz} Hz: 40 Hz reads {attenuation:.2} dB, reduction {measured:.2} dB (formula {wanted:.2}), off {off:.2} dB"
        );
        assert!(
            (measured - wanted).abs() < 0.3,
            "{hz}: {measured} vs {wanted}"
        );
    }
    // Off, the frequency changes nothing.
    let a = mono(
        &mut with(&[(THRESHOLD, -30.0), (SIDECHAIN_FREQ, 500.0)]),
        &input,
    );
    let b = mono(
        &mut with(&[(THRESHOLD, -30.0), (SIDECHAIN_FREQ, 20.0)]),
        &input,
    );
    assert_eq!(a, b);
    // The sound itself is not filtered: under the threshold the output is the input.
    let quiet = sine(4_800, 40.0, undb(-50.0));
    let mut c = with(&[
        (THRESHOLD, -30.0),
        (SIDECHAIN_HPF, 1.0),
        (SIDECHAIN_FREQ, 500.0),
    ]);
    assert_eq!(mono(&mut c, &quiet), quiet);
}

// ---------------------------------------------------------------- #157: the auto release

/// Seconds until the reduction falls under 1 dB after a 0 dBFS tone of `secs` stops.
fn recovery(auto: bool, secs: f32) -> f32 {
    let mut c = with(&[
        (THRESHOLD, -30.0),
        (RATIO, 4.0),
        (ATTACK, 0.001),
        (RELEASE, 0.3),
        (RELEASE_MODE, if auto { 1.0 } else { 0.0 }),
    ]);
    mono(&mut c, &sine((secs * SR) as usize, 1_000.0, 1.0));
    let quiet = [0.0f32; 128];
    for k in 0..2_000 {
        mono(&mut c, &quiet);
        if c.gain_reduction_db() < 1.0 {
            return (k + 1) as f32 * 128.0 / SR;
        }
    }
    f32::INFINITY
}

#[test]
fn the_auto_release_comes_back_fast_after_a_hit_and_slowly_after_a_long_compression() {
    let (auto_short, auto_long) = (recovery(true, 0.02), recovery(true, 2.0));
    let (manual_short, manual_long) = (recovery(false, 0.02), recovery(false, 2.0));
    println!(
        "back under 1 dB: auto {:.0} ms after 20 ms, {:.0} ms after 2 s; manual (300 ms) {:.0} ms / {:.0} ms",
        auto_short * 1e3,
        auto_long * 1e3,
        manual_short * 1e3,
        manual_long * 1e3
    );
    assert!(auto_short < 0.3 * auto_long, "{auto_short} {auto_long}");
    // About the fast time constant: from 22.5 dB to 1 dB is ln(22.5) ≈ 3.1 of them, plus a
    // little for the slow mean.
    assert!(auto_short < 5.0 * AUTO_FAST_SECONDS, "{auto_short}");
    // Manual is the same after a hit and after a long compression.
    assert!(
        (manual_short - manual_long).abs() < 0.01,
        "{manual_short} {manual_long}"
    );
}

// ---------------------------------------------------------------- #157: safety

#[test]
fn the_new_parameters_do_not_depend_on_the_block_size() {
    let input = noise(19_200, 13);
    let events = [
        (1_000, param(KNEE, 18.0)),
        (2_000, param(DETECTION, 1.0)),
        (3_333, param(SIDECHAIN_HPF, 1.0)),
        (4_000, param(SIDECHAIN_FREQ, 300.0)),
        (5_000, param(LOOKAHEAD, 0.004)),
        (6_000, param(MIX, 0.4)),
        (7_001, param(RELEASE_MODE, 1.0)),
        (12_000, param(LOOKAHEAD, 0.0)),
    ];
    let render = |size: usize| {
        let mut c = with(&[(THRESHOLD, -30.0), (RATIO, 6.0)]);
        run(&mut c, (&input, &input), size, &events)
    };
    let whole = render(19_200);
    for size in [1, 37, 128, 1_000] {
        assert_eq!(render(size), whole, "block size {size}");
    }
}

#[test]
fn a_quiet_tail_leaves_no_denormal_in_the_states() {
    // RMS detection, the high-pass and the auto release on, a loud tone then a long tail
    // fading under 1e-30: the output and every float the device holds stay normal or 0.
    let mut c = with(&[
        (DETECTION, 1.0),
        (SIDECHAIN_HPF, 1.0),
        (RELEASE_MODE, 1.0),
        (LOOKAHEAD, 0.003),
        (MIX, 0.5),
    ]);
    mono(&mut c, &sine(9_600, 200.0, 1.0));
    let tail: Vec<f32> = (0..96_000)
        .map(|n| 1e-3 * (-(n as f32) / 800.0).exp() * if n % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    let out = mono(&mut c, &tail);
    check(&out);
    let state = format!("{c:?}");
    let mut floats = 0;
    for word in
        state.split(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == 'e' || ch == '-'))
    {
        // Every number of the print: a subnormal may print without a dot (`1e-40`).
        if let Ok(v) = word.parse::<f32>() {
            assert!(v == 0.0 || v.abs() >= f32::MIN_POSITIVE, "{v} in the state");
            floats += 1;
        }
    }
    assert!(floats > 10, "{floats}");
    // The tail is under the threshold: the auto release's slow mean lets the gain back up.
    assert!(c.gain_reduction_db() < 0.5, "{}", c.gain_reduction_db());
}

#[test]
fn a_rate_above_the_cap_keeps_the_mvp_s_coefficients() {
    // The MVP's device accepted any finite rate from 8 kHz: so does this one (only the line
    // is sized at the cap).
    let rate = 384_000.0;
    let input = noise(9_600, 2);
    let mut new = Compressor::new(rate).unwrap();
    let mut old = old_compressor::Compressor::new(rate);
    for (id, value) in [(THRESHOLD, -40.0f32), (RATIO, 10.0)] {
        new.set_param(ParamId(id), value);
        old.set_param(id, value);
    }
    new.reset();
    old.reset();
    let (mut l, mut r) = (input.clone(), input.clone());
    old.render(&mut l, &mut r);
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let (mut nl, mut nr) = (input.clone(), input.clone());
    let ctx = BlockCtx {
        sample_rate: rate,
        frames: nl.len() as u32,
        transport: Transport::default(),
    };
    assert_no_alloc(|| {
        new.process(
            &ctx,
            &[],
            &mut AudioIo {
                left: &mut nl,
                right: &mut nr,
                events_out: &mut unused,
            },
        );
    });
    assert_eq!(nl, l);
    // The longest look-ahead is the line's.
    new.set_param(ParamId(LOOKAHEAD), 0.01);
    new.reset();
    assert!(new.latency_frames() <= 2_046, "{}", new.latency_frames());
}
