//! The echo, `faulk.echo` (DV-3): offline renders through the device contract, every
//! `process` under the allocation detector. Asserts on the signal: the echo at the exact
//! number of frames (free and synced, every note value), the repeats, the bound on the
//! feedback, the dry path, a tempo change and a change of mode without a discontinuity, no
//! NaN and no denormal, the block-size invariance and the reset. The contract's conformance
//! run on every device of the registry is in `osc_synth.rs`; the native/WASM parity is
//! `crates/engine-wasm/tests/echo_parity.rs`.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceRole, DeviceSetup, Event, EventBuffer, MAX_SAMPLE_RATE,
    Transport, builtin,
};
use engine::devices::echo::{
    self, DESCRIPTOR, DIVISION, DIVISION_BEATS, Echo, FEEDBACK, FEEDBACK_MAX, MAX_DELAY_SECONDS,
    MIX, MODE, TIME,
};
use engine::msg::ParamId;
use engine::time::Tempo;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const SR: f32 = 48_000.0;

fn echo_at(rate: f32) -> Echo {
    Echo::new(rate).expect("the lines are allocated")
}

fn set(device: &mut Echo, id: u32, value: f32) {
    device.set_param(ParamId(id), value);
}

/// An echo with these values, the ramps snapped (so a test starts at its values).
fn with(rate: f32, params: &[(u32, f32)]) -> Echo {
    let mut e = echo_at(rate);
    for (id, value) in params {
        set(&mut e, *id, *value);
    }
    e.reset();
    e
}

fn ctx(rate: f32, frames: usize, bpm: f64) -> BlockCtx {
    BlockCtx {
        sample_rate: rate,
        frames: frames as u32,
        transport: Transport {
            tempo: Tempo::new(bpm).unwrap(),
            ..Transport::default()
        },
    }
}

/// Runs `input` (the same on both channels) through the device in blocks of `size`, the
/// tempo of each block given by `bpm_of(first frame)`, `events` applying at their frame.
fn run(
    device: &mut Echo,
    rate: f32,
    input: &[f32],
    size: usize,
    bpm_of: impl Fn(usize) -> f64,
    events: &[(usize, Event)],
) -> (Vec<f32>, Vec<f32>) {
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
    let mut start = 0;
    while start < input.len() {
        let end = (start + size).min(input.len());
        let mut left = input[start..end].to_vec();
        let mut right = left.clone();
        let local: Vec<Event> = events
            .iter()
            .filter(|(at, _)| (start..end).contains(at))
            .map(|(at, e)| e.at((at - start) as u32))
            .collect();
        let c = ctx(rate, end - start, bpm_of(start));
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left,
                right: &mut right,
                events_out: &mut unused,
            };
            device.process(&c, &local, &mut io);
        });
        out_l.extend(left);
        out_r.extend(right);
        start = end;
    }
    (out_l, out_r)
}

fn run_at(device: &mut Echo, input: &[f32], bpm: f64) -> Vec<f32> {
    run(device, SR, input, 256, |_| bpm, &[]).0
}

fn impulse(len: usize, at: usize) -> Vec<f32> {
    let mut x = vec![0.0; len];
    x[at] = 1.0;
    x
}

fn check(x: &[f32]) {
    for (i, s) in x.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is {s}");
        assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
    }
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

/// The frame of the largest magnitude.
fn argmax(x: &[f32]) -> usize {
    let mut best = 0;
    for (i, s) in x.iter().enumerate() {
        if s.abs() > x[best].abs() {
            best = i;
        }
    }
    best
}

fn sine(len: usize, hz: f32, amplitude: f32) -> Vec<f32> {
    (0..len)
        .map(|n| amplitude * (core::f32::consts::TAU * hz * n as f32 / SR).sin())
        .collect()
}

/// Deterministic noise in [-1, 1].
fn noise(len: usize) -> Vec<f32> {
    let mut s = 0x1234_5678u32;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

// ---------------------------------------------------------------- the echo is where it belongs

#[test]
fn a_free_echo_comes_back_after_exactly_its_time() {
    for rate in [44_100.0f32, 48_000.0, 96_000.0] {
        for seconds in [0.001f32, 0.25, 0.5, 1.0, 2.0, 4.0] {
            let frames = (f64::from(seconds) * f64::from(rate)).round() as usize;
            // Only times that fall on a whole frame are exact; the others are tested below.
            if (f64::from(seconds) * f64::from(rate) - frames as f64).abs() > 1e-6 {
                continue;
            }
            let mut e = with(
                rate,
                &[(MODE, 0.0), (TIME, seconds), (MIX, 1.0), (FEEDBACK, 0.0)],
            );
            let input = impulse(frames + 1_000, 100);
            let (l, r) = run(&mut e, rate, &input, 256, |_| 120.0, &[]);
            assert_eq!(l, r);
            assert_eq!(argmax(&l), 100 + frames, "{seconds} s at {rate} Hz");
            assert!((l[100 + frames] - 1.0).abs() < 1e-5, "{}", l[100 + frames]);
            let elsewhere = l
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != 100 + frames)
                .fold(0f32, |m, (_, s)| m.max(s.abs()));
            assert!(elsewhere < 1e-6, "a single echo: {elsewhere}");
        }
    }
}

#[test]
fn a_time_between_two_frames_is_shared_by_them() {
    // 0.0123 s at 48 kHz is 590.4 frames.
    let mut e = with(
        SR,
        &[(MODE, 0.0), (TIME, 0.0123), (MIX, 1.0), (FEEDBACK, 0.0)],
    );
    let out = run_at(&mut e, &impulse(2_000, 100), 120.0);
    assert!((out[690] - 0.6).abs() < 1e-3, "{}", out[690]);
    assert!((out[691] - 0.4).abs() < 1e-3, "{}", out[691]);
}

#[test]
fn a_synced_echo_follows_the_note_value_and_the_tempo() {
    for (bpm, division) in [
        (120.0f64, 6usize),
        (120.0, 3),
        (120.0, 5),
        (100.0, 6),
        (90.0, 6),
        (240.0, 7),
    ] {
        let seconds = DIVISION_BEATS[division] * 60.0 / bpm;
        let expected = (seconds * f64::from(SR)).round() as usize;
        assert!(
            (seconds * f64::from(SR) - expected as f64).abs() < 1e-6,
            "the case is a whole frame"
        );
        let mut e = with(
            SR,
            &[
                (MODE, 1.0),
                (DIVISION, division as f32),
                (MIX, 1.0),
                (FEEDBACK, 0.0),
            ],
        );
        // The device is built before the tempo is known (120): the first block starts at it.
        let out = run_at(&mut e, &impulse(expected + 1_000, 100), bpm);
        assert_eq!(
            argmax(&out),
            100 + expected,
            "{bpm} BPM, division {division}"
        );
        assert!((out[100 + expected] - 1.0).abs() < 1e-5);
    }
}

#[test]
fn every_note_value_is_the_right_fraction_of_a_beat() {
    // At 60 BPM a beat is one second, and 48 kHz makes every value a whole frame (the
    // triplets to within half a frame).
    for (division, beats) in DIVISION_BEATS.iter().enumerate() {
        if beats * 60.0 > f64::from(MAX_DELAY_SECONDS) * 60.0 {
            continue;
        }
        let expected = beats * f64::from(SR);
        let mut e = with(
            SR,
            &[
                (MODE, 1.0),
                (DIVISION, division as f32),
                (MIX, 1.0),
                (FEEDBACK, 0.0),
            ],
        );
        let len = expected as usize + 1_000;
        let out = run_at(&mut e, &impulse(len, 0), 60.0);
        let at = argmax(&out);
        assert!(
            (at as f64 - expected).abs() <= 1.0,
            "division {division}: {at} vs {expected}"
        );
    }
    assert_eq!(
        DIVISION_BEATS.len(),
        DESCRIPTOR.params[DIVISION as usize].values.len()
    );
}

#[test]
fn a_time_beyond_the_lines_is_shortened_to_the_longest_delay() {
    // A whole note at 20 BPM is 12 s: it is held to 4 s, which also wraps the line.
    let mut e = with(
        SR,
        &[(MODE, 1.0), (DIVISION, 9.0), (MIX, 1.0), (FEEDBACK, 0.0)],
    );
    let max = (MAX_DELAY_SECONDS * SR) as usize;
    let out = run_at(&mut e, &impulse(max + 5_000, 100), 20.0);
    assert_eq!(argmax(&out), 100 + max);
    assert!((out[100 + max] - 1.0).abs() < 1e-5);
    // The same at the fastest tempo is the shortest note: no panic, an echo at 7.5 ms.
    let mut e = with(
        SR,
        &[(MODE, 1.0), (DIVISION, 0.0), (MIX, 1.0), (FEEDBACK, 0.0)],
    );
    let out = run_at(&mut e, &impulse(2_000, 0), 999.0);
    assert!((argmax(&out) as i64 - 360).abs() <= 1);
}

// ---------------------------------------------------------------- feedback, mix, dry

#[test]
fn the_repeats_fall_by_the_feedback() {
    let mut e = with(
        SR,
        &[(MODE, 0.0), (TIME, 0.25), (MIX, 1.0), (FEEDBACK, 0.5)],
    );
    let d = 12_000;
    let out = run_at(&mut e, &impulse(d * 6, 0), 120.0);
    for k in 1..=5usize {
        let expected = 0.5f32.powi(k as i32 - 1);
        assert!(
            (out[k * d] - expected).abs() < 1e-5,
            "repeat {k}: {} vs {expected}",
            out[k * d]
        );
    }
}

#[test]
fn the_feedback_never_runs_away() {
    // The descriptor's bound is below 1, and the device holds it whatever it is asked.
    assert!(f64::from(FEEDBACK_MAX) < 1.0);
    assert_eq!(
        DESCRIPTOR.params[FEEDBACK as usize].max,
        f64::from(FEEDBACK_MAX)
    );
    let mut e = with(SR, &[(MODE, 0.0), (TIME, 0.01), (MIX, 1.0)]);
    set(&mut e, FEEDBACK, 1.0e9);
    set(&mut e, FEEDBACK, f32::NAN);
    set(&mut e, FEEDBACK, f32::INFINITY);
    e.reset();
    // Full-scale noise for a minute: the output stays below the geometric bound.
    let input = noise(60 * SR as usize);
    let out = run_at(&mut e, &input, 120.0);
    check(&out);
    let bound = 1.0 / (1.0 - FEEDBACK_MAX);
    assert!(
        peak(&out) <= bound + 1e-3,
        "peak {} over {bound}",
        peak(&out)
    );
    // After the input stops the tail dies.
    let tail = run_at(&mut e, &vec![0.0; 20 * SR as usize], 120.0);
    check(&tail);
    assert_eq!(peak(&tail[tail.len() - 1_000..]), 0.0);
}

#[test]
fn a_mix_of_zero_is_the_dry_signal_and_a_mix_of_one_has_none() {
    let input = sine(8_000, 440.0, 0.5);
    let mut dry = with(SR, &[(MIX, 0.0)]);
    assert_eq!(run_at(&mut dry, &input, 120.0), input);
    let mut wet = with(SR, &[(MODE, 0.0), (TIME, 0.1), (MIX, 1.0), (FEEDBACK, 0.0)]);
    let out = run_at(&mut wet, &input, 120.0);
    assert!(
        out[..4_800].iter().all(|s| *s == 0.0),
        "no dry before the first echo"
    );
    for n in 4_800..8_000 {
        assert!((out[n] - input[n - 4_800]).abs() < 1e-5, "frame {n}");
    }
}

#[test]
fn the_two_channels_are_independent() {
    let mut e = with(SR, &[(MODE, 0.0), (TIME, 0.1), (MIX, 1.0), (FEEDBACK, 0.5)]);
    let mut left = impulse(10_000, 0);
    let mut right = vec![0.0; 10_000];
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let c = ctx(SR, 10_000, 120.0);
    let mut io = AudioIo {
        left: &mut left,
        right: &mut right,
        events_out: &mut unused,
    };
    e.process(&c, &[], &mut io);
    assert_eq!(argmax(&left), 4_800);
    assert!(
        right.iter().all(|s| *s == 0.0),
        "nothing crosses to the right"
    );
}

// ---------------------------------------------------------------- no discontinuity

/// A sine of amplitude 0.5 at 440 Hz steps at most 2 pi f / sr * 0.5 = 0.029 per frame; the
/// echo, played faster by up to 25% while the delay glides, and mixed with the dry signal,
/// stays under this.
const STEP_BOUND: f32 = 0.08;

fn steady_echo() -> Echo {
    with(
        SR,
        &[(MODE, 1.0), (DIVISION, 6.0), (MIX, 0.5), (FEEDBACK, 0.5)],
    )
}

#[test]
fn a_tempo_change_does_not_crackle() {
    let input = sine(6 * SR as usize, 440.0, 0.5);
    // A steady echo, the reference: the bound holds without any change.
    let mut e = steady_echo();
    let still = run(&mut e, SR, &input, 128, |_| 120.0, &[]).0;
    assert!(max_step(&still) < STEP_BOUND, "{}", max_step(&still));

    for (from, to) in [
        (120.0, 60.0),
        (120.0, 240.0),
        (120.0, 121.0),
        (60.0, 999.0),
        (999.0, 20.0),
    ] {
        let mut e = steady_echo();
        let out = run(
            &mut e,
            SR,
            &input,
            128,
            |at| if at < 3 * SR as usize { from } else { to },
            &[],
        )
        .0;
        check(&out);
        assert!(
            max_step(&out) < STEP_BOUND,
            "{from} to {to} BPM: step {}",
            max_step(&out)
        );
    }

    // The measure sees a discontinuity when there is one: cutting the line at the change.
    let mut e = steady_echo();
    let half = 3 * SR as usize + 27;
    let mut out = run(&mut e, SR, &input[..half], 128, |_| 120.0, &[]).0;
    e.reset();
    out.extend(run(&mut e, SR, &input[half..], 128, |_| 60.0, &[]).0);
    assert!(max_step(&out) > 2.0 * STEP_BOUND, "{}", max_step(&out));
}

#[test]
fn a_tempo_sweep_block_by_block_does_not_crackle() {
    let input = sine(8 * SR as usize, 440.0, 0.5);
    let mut e = steady_echo();
    // The tempo moves a little at every block, up then down.
    let out = run(
        &mut e,
        SR,
        &input,
        64,
        |at| 80.0 + 60.0 * ((at as f64 / f64::from(SR)) * 1.5).sin(),
        &[],
    )
    .0;
    check(&out);
    assert!(max_step(&out) < STEP_BOUND, "{}", max_step(&out));
}

#[test]
fn a_new_time_or_mode_glides_without_a_jump() {
    let input = sine(5 * SR as usize, 440.0, 0.5);
    let at = |s: f32| (s * SR) as usize;
    let mut e = with(SR, &[(MODE, 0.0), (TIME, 0.5), (MIX, 0.5), (FEEDBACK, 0.5)]);
    let events = [
        (
            at(1.0),
            Event::ParamValue {
                offset: 0,
                id: ParamId(TIME),
                value: 0.05,
            },
        ),
        (
            at(2.0),
            Event::ParamValue {
                offset: 0,
                id: ParamId(TIME),
                value: 1.9,
            },
        ),
        (
            at(3.0),
            Event::ParamValue {
                offset: 0,
                id: ParamId(MODE),
                value: 1.0,
            },
        ),
        (
            at(4.0),
            Event::ParamValue {
                offset: 0,
                id: ParamId(DIVISION),
                value: 2.0,
            },
        ),
    ];
    let out = run(&mut e, SR, &input, 128, |_| 120.0, &events).0;
    check(&out);
    assert!(max_step(&out) < STEP_BOUND, "{}", max_step(&out));
}

#[test]
fn after_a_tempo_change_the_echo_settles_on_the_new_time() {
    // Quarter note: 0.5 s at 120 BPM, then 1 s at 60.
    let mut e = with(
        SR,
        &[(MODE, 1.0), (DIVISION, 6.0), (MIX, 1.0), (FEEDBACK, 0.0)],
    );
    let mut input = vec![0.0; 6 * SR as usize];
    input[4 * SR as usize] = 1.0;
    let out = run(
        &mut e,
        SR,
        &input,
        128,
        |at| if at < SR as usize { 120.0 } else { 60.0 },
        &[],
    )
    .0;
    assert_eq!(argmax(&out), 4 * SR as usize + 48_000);
}

// ---------------------------------------------------------------- robustness

#[test]
fn a_broken_input_gives_a_clean_output() {
    let mut e = with(
        SR,
        &[(MODE, 0.0), (TIME, 0.01), (MIX, 0.5), (FEEDBACK, 0.9)],
    );
    let mut input = noise(20_000);
    for (i, bad) in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1e30,
        -1e30,
        f32::MIN_POSITIVE / 2.0,
    ]
    .iter()
    .enumerate()
    {
        for k in 0..50 {
            input[1_000 + i * 997 + k] = *bad;
        }
    }
    let out = run_at(&mut e, &input, 120.0);
    check(&out);
    let tail = run_at(&mut e, &vec![0.0; 30 * SR as usize], 120.0);
    check(&tail);
    assert_eq!(peak(&tail[tail.len() - 1_000..]), 0.0, "the tail ended");
}

#[test]
fn a_long_decay_produces_no_denormal() {
    let mut e = with(
        SR,
        &[(MODE, 0.0), (TIME, 0.002), (MIX, 1.0), (FEEDBACK, 0.95)],
    );
    let out = run_at(&mut e, &impulse(40 * SR as usize, 0), 120.0);
    check(&out);
    assert!(out.iter().any(|s| *s != 0.0));
}

#[test]
fn the_block_size_changes_nothing() {
    let input = noise(30_000);
    let render = |size: usize| {
        let mut e = steady_echo();
        run(&mut e, SR, &input, size, |_| 133.0, &[]).0
    };
    let whole = render(30_000);
    for size in [1, 7, 128, 2_048] {
        assert_eq!(render(size), whole, "blocks of {size}");
    }
}

#[test]
fn reset_empties_the_lines_and_keeps_the_values() {
    let mut e = with(
        SR,
        &[(MODE, 0.0), (TIME, 0.25), (MIX, 1.0), (FEEDBACK, 0.9)],
    );
    run_at(&mut e, &noise(10_000), 120.0);
    e.reset();
    let out = run_at(&mut e, &vec![0.0; 40_000], 120.0);
    assert!(out.iter().all(|s| *s == 0.0), "no tail after a reset");
    // The values are kept: the echo is still free, 0.25 s, fully wet.
    let out = run_at(&mut e, &impulse(20_000, 0), 120.0);
    assert_eq!(argmax(&out), 12_000);
}

#[test]
fn a_long_glide_lands_on_the_exact_frame() {
    // A delay above 65 536 frames: a f32 step of MIN_SLEW would be lost, the echo would stop early.
    for rate in [48_000.0_f32, 192_000.0] {
        let mut e = with(
            rate,
            &[(MODE, 0.0), (TIME, 0.5), (MIX, 1.0), (FEEDBACK, 0.0)],
        );
        run_at(&mut e, &vec![0.0; 256], 120.0);
        set(&mut e, TIME, 1.5);
        let at = (5.5 * rate) as usize;
        let delay = (1.5 * rate) as usize;
        let out = run(
            &mut e,
            rate,
            &impulse(at + delay + 100, at),
            256,
            |_| 120.0,
            &[],
        )
        .0;
        assert_eq!(argmax(&out), at + delay, "rate {rate}");
    }
}

#[test]
fn unknown_parameters_and_bad_values_are_ignored() {
    let mut e = echo_at(SR);
    let before = format!("{e:?}");
    e.set_param(ParamId(99), 1.0);
    e.set_param(ParamId(u32::MAX), 1.0);
    for id in 0..5 {
        e.set_param(ParamId(id), f32::NAN);
        e.set_param(ParamId(id), f32::NEG_INFINITY);
    }
    // Infinity is not finite either; the ramps did not start, the values did not move.
    assert_eq!(before, format!("{e:?}"));
}

// ---------------------------------------------------------------- the descriptor and the memory

#[test]
fn the_descriptor_is_a_valid_effect_with_a_safe_range() {
    assert_eq!(DESCRIPTOR.validate(), Ok(()));
    assert_eq!(DESCRIPTOR.role, DeviceRole::Effect);
    assert_eq!((DESCRIPTOR.kind, DESCRIPTOR.name), ("faulk.echo", "Echo"));
    assert_eq!(
        builtin(DESCRIPTOR.kind_id).map(|d| d.kind),
        Some("faulk.echo")
    );
    let keys: Vec<_> = DESCRIPTOR.params.iter().map(|p| p.key).collect();
    assert_eq!(keys, ["mode", "time", "division", "feedback", "mix"]);
    // Defaults: the values `Echo::new` starts with (a device built and one set to every
    // default behave alike).
    let input = noise(5_000);
    let mut fresh = echo_at(SR);
    fresh.reset();
    let mut set_to_defaults = echo_at(SR);
    for p in DESCRIPTOR.params {
        set(&mut set_to_defaults, p.id.0, p.default as f32);
    }
    set_to_defaults.reset();
    assert_eq!(
        run_at(&mut fresh, &input, 120.0),
        run_at(&mut set_to_defaults, &input, 120.0)
    );
}

#[test]
fn the_declared_memory_covers_the_lines_at_the_highest_rate() {
    let e = echo_at(MAX_SAMPLE_RATE);
    let lines = 2 * (MAX_DELAY_SECONDS * MAX_SAMPLE_RATE) as u64 * 4;
    assert!(DESCRIPTOR.memory_bytes >= lines + size_of_val(&e) as u64);
    // A higher rate than declared does not take more: the lines stop at the declared size,
    // the times stay right.
    let mut beyond = with(
        384_000.0,
        &[(MODE, 0.0), (TIME, 0.25), (MIX, 1.0), (FEEDBACK, 0.0)],
    );
    let out = run(
        &mut beyond,
        384_000.0,
        &impulse(100_000, 0),
        512,
        |_| 120.0,
        &[],
    )
    .0;
    assert_eq!(argmax(&out), 96_000);
    let device: Box<dyn Device> = (echo::factory)(&DeviceSetup {
        sample_rate: 384_000.0,
        ..DeviceSetup::default()
    })
    .unwrap();
    assert_eq!(device.descriptor().kind, "faulk.echo");
}

#[test]
fn the_lines_are_allocated_by_the_factory_and_process_allocates_nothing() {
    // `run` holds every `process` under the allocation detector: events, a tempo change
    // and a time change in the loop, blocks of every size.
    let input = noise(5 * SR as usize);
    let events = [
        (
            500,
            Event::ParamValue {
                offset: 0,
                id: ParamId(MIX),
                value: 0.9,
            },
        ),
        (
            9_000,
            Event::ParamValue {
                offset: 0,
                id: ParamId(TIME),
                value: 0.02,
            },
        ),
        (
            9_000,
            Event::ParamValue {
                offset: 0,
                id: ParamId(MODE),
                value: 0.0,
            },
        ),
        (
            20_000,
            Event::ParamValue {
                offset: 0,
                id: ParamId(DIVISION),
                value: 0.0,
            },
        ),
    ];
    for size in [1usize, 33, 2_048] {
        let mut e = (echo::factory)(&DeviceSetup::default()).unwrap();
        let mut unused = EventBuffer::try_with_capacity(1).unwrap();
        let mut left = input[..2_048].to_vec();
        let mut right = left.clone();
        let c = ctx(SR, size.min(2_048), 100.0);
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left[..size.min(2_048)],
                right: &mut right[..size.min(2_048)],
                events_out: &mut unused,
            };
            e.process(&c, &[], &mut io);
            e.set_param(ParamId(TIME), 0.5);
            e.reset();
        });
    }
    let mut e = echo_at(SR);
    let out = run(
        &mut e,
        SR,
        &input,
        97,
        |at| 90.0 + (at / 4_800) as f64,
        &events,
    )
    .0;
    check(&out);
}
