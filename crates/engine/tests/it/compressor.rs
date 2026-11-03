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
    self, ATTACK, Compressor, DESCRIPTOR, KNEE_DB, MAKEUP, MAKEUP_MAX_DB, RATIO, RELEASE,
    THRESHOLD, gain_reduction_db,
};
use engine::msg::ParamId;

const SR: f32 = 48_000.0;

/// A compressor with these values, the ramps snapped (so a test starts at its values).
fn with(params: &[(u32, f32)]) -> Compressor {
    let mut c = Compressor::new(SR);
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
    input_db - gain_reduction_db(input_db, threshold, 1.0 - 1.0 / ratio) + makeup
}

#[test]
fn the_curve_matches_the_textbook_formula() {
    // Independent of the device's helper: far below the knee nothing, far above the straight
    // line `T + over / ratio`, and the knee joins them continuously.
    let (t, ratio) = (-20.0f32, 4.0f32);
    let curve = |x: f32| x - gain_reduction_db(x, t, 1.0 - 1.0 / ratio);
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
    assert_eq!(bounds.len(), 5);
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
    for corner in 0..32u32 {
        let params: Vec<(u32, f32)> = bounds
            .iter()
            .enumerate()
            .map(|(i, (id, b))| (*id, b[((corner >> i) & 1) as usize]))
            .collect();
        for (name, input, amplitude) in &signals {
            let mut c = with(&params);
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
    let mut c = Compressor::new(SR);
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
    assert_eq!(keys, ["threshold", "ratio", "attack", "release", "makeup"]);
    let built = compressor::factory(&engine::device::DeviceSetup::default()).unwrap();
    assert_eq!(built.descriptor().kind, "faulk.compressor");
    // Each default is the one the device is built with.
    let mut c = Compressor::new(SR);
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
