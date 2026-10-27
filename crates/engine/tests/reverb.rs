//! The reverb, `faulk.reverb` (DV-4): offline renders through the device contract, every
//! `process` under the allocation detector. Asserts on the signal: the reverberation time
//! against `size` (measured on the energy decay of an impulse), a tail that decays to silence,
//! the damping, the pre-delay, the level of the tail across sizes, a bounded output with every
//! parameter swept to its bounds, no NaN and no denormal, the block-size invariance and the
//! reset. The contract's conformance run on every device of the registry is in `osc_synth.rs`;
//! the native/WASM parity is `crates/engine-wasm/tests/reverb_parity.rs`.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::reverb::{
    DAMPING, DESCRIPTOR, FADE, FADE_SECONDS, GLIDE, JUMP, MIX, PREDELAY, RT60_MAX, RT60_MIN,
    Reverb, SIZE, TIME_CHANGE,
};
use engine::msg::ParamId;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const SR: f32 = 48_000.0;

fn with(params: &[(u32, f32)]) -> Reverb {
    at_rate(SR, params)
}

fn at_rate(rate: f32, params: &[(u32, f32)]) -> Reverb {
    let mut r = Reverb::new(rate).expect("the lines are allocated");
    for (id, value) in params {
        r.set_param(ParamId(*id), *value);
    }
    r.reset();
    r
}

fn ctx(frames: usize) -> BlockCtx {
    BlockCtx {
        sample_rate: SR,
        frames: frames as u32,
        transport: Transport::default(),
    }
}

/// Runs `input` (left, right) through the device in blocks of `size`, `events` applying at
/// their frame.
fn run(
    device: &mut Reverb,
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
        let c = ctx(end - start);
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
    // Every render is checked: no NaN, no infinity, no denormal.
    check(&out_l);
    check(&out_r);
    (out_l, out_r)
}

fn mono(device: &mut Reverb, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
    run(device, (input, input), 256, &[])
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

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|s| f64::from(*s) * f64::from(*s)).sum()
}

/// Deterministic noise in [-1, 1].
fn noise(len: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

/// The reverberation time of `x`, from the energy decay (Schroeder's backward integration):
/// the time the decay takes from -5 dB to -35 dB, doubled (T30).
fn rt60(x: &[f32], rate: f32) -> f32 {
    let mut edc = vec![0f64; x.len()];
    let mut acc = 0.0;
    for i in (0..x.len()).rev() {
        acc += f64::from(x[i]) * f64::from(x[i]);
        edc[i] = acc;
    }
    let total = edc[0];
    let at = |db: f64| {
        let level = total * 10f64.powf(db / 10.0);
        edc.iter().position(|e| *e < level).unwrap() as f32 / rate
    };
    2.0 * (at(-35.0) - at(-5.0))
}

fn nominal(size: f32) -> f32 {
    RT60_MIN * (RT60_MAX / RT60_MIN).powf(size)
}

fn ev(id: u32, value: f32) -> Event {
    Event::ParamValue {
        offset: 0,
        id: ParamId(id),
        value,
    }
}

#[test]
fn the_tail_decays_as_the_size_says() {
    for size in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let want = nominal(size);
        // Fully wet and bright: the reverberation time is the nominal one.
        let mut r = with(&[(SIZE, size), (DAMPING, 0.0), (PREDELAY, 0.0), (MIX, 1.0)]);
        let len = (SR * (want * 2.5 + 0.5)) as usize;
        let (l, rr) = mono(&mut r, &impulse(len, 0));
        check(&l);
        check(&rr);
        let measured = (rt60(&l, SR) + rt60(&rr, SR)) / 2.0;
        println!("size {size}: nominal RT60 {want:.2} s, measured {measured:.2} s");
        assert!(
            (measured / want - 1.0).abs() < 0.2,
            "size {size}: {measured} s for {want} s"
        );
        // Silence after two reverberation times (-100 dB): the tail decays for good.
        let late = (SR * (want * 2.0 + 0.2)) as usize;
        assert!(peak(&l[late..]) < 1e-5, "size {size}: {}", peak(&l[late..]));
        assert!(peak(&rr[late..]) < 1e-5);
    }
}

#[test]
fn damping_shortens_the_tail_and_darkens_it() {
    let len = (SR * 6.0) as usize;
    let mut times = Vec::new();
    for damping in [0.0f32, 0.5, 1.0] {
        let mut r = with(&[(SIZE, 0.7), (DAMPING, damping), (PREDELAY, 0.0), (MIX, 1.0)]);
        let (l, _) = mono(&mut r, &impulse(len, 0));
        let t = rt60(&l, SR);
        println!("damping {damping}: RT60 {t:.2} s");
        times.push(t);
    }
    assert!(
        times[0] > times[1] && times[1] > times[2],
        "more damping, shorter tail: {times:?}"
    );
}

#[test]
fn the_energy_of_the_tail_is_the_same_at_every_size() {
    let mut levels = Vec::new();
    for size in [0.0f32, 0.5, 1.0] {
        let mut r = with(&[(SIZE, size), (DAMPING, 0.0), (PREDELAY, 0.0), (MIX, 1.0)]);
        let (l, _) = mono(&mut r, &impulse((SR * 20.0) as usize, 0));
        levels.push(10.0 * energy(&l).log10());
    }
    println!("energy of the impulse response (dB): {levels:?}");
    let spread = levels.iter().cloned().fold(f64::MIN, f64::max)
        - levels.iter().cloned().fold(f64::MAX, f64::min);
    assert!(spread < 3.0, "{levels:?}");
}

#[test]
fn white_noise_comes_out_at_the_level_it_went_in() {
    for size in [0.0f32, 0.5, 1.0] {
        let mut r = with(&[(SIZE, size), (DAMPING, 0.0), (PREDELAY, 0.0), (MIX, 1.0)]);
        let x = noise((SR * 30.0) as usize, 7);
        let (l, _) = mono(&mut r, &x);
        let tail = (SR * 15.0) as usize;
        let gain = (energy(&l[tail..]) / energy(&x[tail..])).sqrt();
        println!("size {size}: noise gain {gain:.2}");
        assert!((0.5..2.0).contains(&gain), "size {size}: {gain}");
    }
}

#[test]
fn the_pre_delay_moves_the_start_of_the_tail() {
    let onset = |predelay: f32| {
        let mut r = with(&[
            (SIZE, 0.5),
            (DAMPING, 0.0),
            (PREDELAY, predelay),
            (MIX, 1.0),
        ]);
        let (l, rr) = mono(&mut r, &impulse(40_000, 0));
        let first = |x: &[f32]| x.iter().position(|s| s.abs() > 1e-9).unwrap();
        first(&l).min(first(&rr))
    };
    let (a, b) = (onset(0.0), onset(0.1));
    println!("first sample of the tail: {a} frames, {b} frames with 100 ms of pre-delay");
    assert_eq!(b - a, 4_800, "{a} {b}");
    // The shortest comb: nothing comes out before it has been traversed once.
    assert!(a >= 1_100, "{a}");
}

#[test]
fn mix_zero_is_the_dry_signal_and_the_dry_path_is_untouched() {
    let x = noise(10_000, 3);
    let mut r = with(&[(MIX, 0.0)]);
    let (l, rr) = mono(&mut r, &x);
    assert_eq!(l, x);
    assert_eq!(rr, x);
    // Fully wet: no dry signal at the start (the network answers after its shortest line).
    let mut r = with(&[(MIX, 1.0), (PREDELAY, 0.0)]);
    let (l, _) = mono(&mut r, &x);
    assert!(peak(&l[..1_000]) == 0.0);
}

#[test]
fn the_output_stays_bounded_with_every_parameter_at_its_bounds() {
    let corners = |p: &[ParamId]| -> Vec<Vec<f32>> {
        (0..1u32 << p.len())
            .map(|m| {
                p.iter()
                    .enumerate()
                    .map(|(i, id)| {
                        let d = DESCRIPTOR.param(*id).unwrap();
                        if m >> i & 1 == 1 { d.max } else { d.min }
                    })
                    .map(|v| v as f32)
                    .collect()
            })
            .collect()
    };
    let ids: Vec<ParamId> = DESCRIPTOR.params.iter().map(|p| p.id).collect();
    let mut worst = 0f32;
    for corner in corners(&ids) {
        let params: Vec<(u32, f32)> = ids.iter().map(|i| i.0).zip(corner).collect();
        for input in [
            noise((SR * 4.0) as usize, 11),
            // Full scale, constant: the worst case of a low-frequency feedback loop.
            vec![1.0; (SR * 4.0) as usize],
            // Full-scale square at 100 Hz.
            (0..(SR * 4.0) as usize)
                .map(|n| if (n / 240) % 2 == 0 { 1.0 } else { -1.0 })
                .collect(),
        ] {
            let mut r = with(&params);
            let (l, rr) = mono(&mut r, &input);
            check(&l);
            check(&rr);
            worst = worst.max(peak(&l)).max(peak(&rr));
        }
    }
    println!("largest output over every corner, for a full-scale input: {worst}");
    assert!(worst < 4.0, "{worst}");
}

#[test]
fn parameters_swept_while_playing_never_blow_up_nor_click_to_nan() {
    let x = noise((SR * 6.0) as usize, 5);
    let mut events = Vec::new();
    for k in 0..60usize {
        let at = k * 4_800 + 17;
        let v = if k % 2 == 0 { 1.0 } else { 0.0 };
        events.push((at, ev(SIZE, v)));
        events.push((at + 1, ev(DAMPING, 1.0 - v)));
        events.push((at + 2, ev(PREDELAY, v * 0.2)));
        events.push((at + 3, ev(MIX, v)));
        events.push((at + 4, ev(SIZE, f32::NAN)));
        events.push((at + 5, ev(MIX, f32::INFINITY)));
    }
    let mut r = with(&[]);
    let (l, rr) = run(&mut r, (&x, &x), 333, &events);
    check(&l);
    check(&rr);
    assert!(peak(&l) < 4.0 && peak(&rr) < 4.0);
}

#[test]
fn hostile_input_is_contained() {
    let mut x = noise(20_000, 9);
    x[10] = f32::NAN;
    x[11] = f32::INFINITY;
    x[12] = f32::NEG_INFINITY;
    x[13] = 1e30;
    x[14] = 1e-30;
    let mut r = with(&[(MIX, 1.0), (SIZE, 1.0)]);
    let (l, rr) = mono(&mut r, &x);
    check(&l);
    check(&rr);
    // A clamped 1e6 spike is bounded by the filters, not amplified without limit.
    assert!(peak(&l) < 1e8, "{}", peak(&l));
}

#[test]
fn the_result_does_not_depend_on_the_block_size() {
    let x = noise((SR * 2.0) as usize, 21);
    let events = vec![
        (1_000, ev(SIZE, 0.9)),
        (20_000, ev(DAMPING, 0.2)),
        (30_001, ev(PREDELAY, 0.13)),
        (50_000, ev(MIX, 0.8)),
        (70_000, ev(SIZE, 0.1)),
    ];
    let reference = run(&mut with(&[]), (&x, &x), 4_096, &events);
    for size in [1usize, 37, 128, 1_000] {
        let got = run(&mut with(&[]), (&x, &x), size, &events);
        assert!(got == reference, "block size {size}");
    }
}

#[test]
fn reset_silences_the_tail_and_keeps_the_parameters() {
    let x = noise((SR * 1.0) as usize, 4);
    let mut r = with(&[(SIZE, 1.0), (MIX, 1.0), (PREDELAY, 0.05)]);
    let (l, _) = mono(&mut r, &x);
    assert!(peak(&l) > 0.05, "the reverb rings");
    r.reset();
    let (l, rr) = mono(&mut r, &vec![0.0; (SR * 3.0) as usize]);
    assert!(l.iter().chain(&rr).all(|s| *s == 0.0), "nothing is left");
    // The parameters are kept: the same impulse gives the same tail as a fresh device.
    let fresh = mono(
        &mut with(&[(SIZE, 1.0), (MIX, 1.0), (PREDELAY, 0.05)]),
        &impulse(50_000, 0),
    );
    let after = mono(&mut r, &impulse(50_000, 0));
    assert!(after == fresh, "same sound after the reset as a new device");
}

#[test]
fn reset_in_the_middle_of_a_block_sequence_leaves_no_old_tail() {
    // The reset is called as the engine does on the audio thread, between two blocks, with
    // the lines full of sound: the next block must not contain any of it.
    let mut r = with(&[(SIZE, 0.6), (MIX, 1.0), (PREDELAY, 0.1)]);
    let x = noise(48_000, 13);
    let _ = mono(&mut r, &x);
    assert_no_alloc(|| r.reset());
    let (l, rr) = mono(&mut r, &vec![0.0; 40_000]);
    assert!(l.iter().chain(&rr).all(|s| *s == 0.0));
}

#[test]
fn the_defaults_are_the_descriptor_values() {
    let d = |k| DESCRIPTOR.param(ParamId(k)).unwrap().default;
    assert_eq!(
        (d(SIZE), d(DAMPING), d(PREDELAY), d(MIX)),
        (0.5, 0.5, 0.01, 0.25)
    );
    // A default device with a hit: some tail, not the dry signal alone.
    let mut r = Reverb::new(SR).unwrap();
    let (l, _) = mono(&mut r, &impulse(48_000, 0));
    assert!(peak(&l[3_000..]) > 1e-4);
}

#[test]
fn the_cutoffs_do_not_depend_on_the_sample_rate() {
    // The same settings give the same reverberation time at every rate, damped or not.
    for damping in [0.0f32, 1.0] {
        let measured: Vec<f32> = [44_100.0f32, 48_000.0, 96_000.0, 192_000.0]
            .iter()
            .map(|rate| {
                let mut r = at_rate(
                    *rate,
                    &[(SIZE, 0.7), (DAMPING, damping), (PREDELAY, 0.0), (MIX, 1.0)],
                );
                let x = impulse((*rate * 6.0) as usize, 0);
                let (l, _) = run(&mut r, (&x, &x), 256, &[]);
                rt60(&l, *rate)
            })
            .collect();
        println!("damping {damping}: RT60 at 44.1, 48, 96, 192 kHz: {measured:?}");
        // Undamped, the time is the nominal one at every rate. Damped, the measure covers the
        // whole band up to Nyquist: a higher rate adds high frequencies that die faster, so the
        // time shortens a little (it is 1.35 s against 1.83 s at 192 kHz), but nowhere near the
        // factor of 4 that a filter left unscaled gives.
        let tolerance = if damping == 0.0 { 0.03 } else { 0.35 };
        for t in &measured {
            assert!((t / measured[1] - 1.0).abs() < tolerance, "{measured:?}");
        }
    }
    // A 50 Hz sine passes the DC blocker at every rate (it is far above its 8 Hz cutoff).
    for rate in [48_000.0f32, 192_000.0] {
        let mut r = at_rate(rate, &[(MIX, 1.0), (SIZE, 0.0), (PREDELAY, 0.0)]);
        let n = (rate * 2.0) as usize;
        let x: Vec<f32> = (0..n)
            .map(|k| (core::f32::consts::TAU * 50.0 * k as f32 / rate).sin())
            .collect();
        let (l, _) = run(&mut r, (&x, &x), 256, &[]);
        let gain = (energy(&l[n / 2..]) / energy(&x[n / 2..])).sqrt();
        println!("50 Hz at {rate}: gain {gain:.2}");
        assert!(gain > 0.3, "{rate}: {gain}");
    }
}

#[test]
fn the_pre_delay_glides_instead_of_jumping() {
    // From 0 to 0.2 s with no reset: the delay in use moves by at most 0.25 frame per frame,
    // so an impulse right after the change is read almost without delay, where a jump would
    // hold it back by 9 600 frames.
    let onset = |input_at: usize| {
        let mut r = with(&[(SIZE, 0.5), (DAMPING, 0.0), (PREDELAY, 0.0), (MIX, 1.0)]);
        r.set_param(ParamId(PREDELAY), 0.2);
        let (l, _) = mono(&mut r, &impulse(input_at + 40_000, input_at));
        l.iter().position(|s| s.abs() > 1e-9).unwrap() - input_at
    };
    let early = onset(100);
    println!("impulse 100 frames after the change: first wet sample {early} frames later");
    assert!(early < 2_500, "{early}");
    // Once it has arrived (9 600 / 0.25 = 38 400 frames), the pre-delay is the full 0.2 s.
    let settled = onset(40_000);
    let plain = {
        let mut r = with(&[(SIZE, 0.5), (DAMPING, 0.0), (PREDELAY, 0.2), (MIX, 1.0)]);
        let (l, _) = mono(&mut r, &impulse(40_000, 0));
        l.iter().position(|s| s.abs() > 1e-9).unwrap()
    };
    assert_eq!(settled, plain);
}

// Issue #353: the pre-delay's `time_change` setting (Glide, Fade, Jump), shared with the Echo.

fn sine(len: usize, hz: f32) -> Vec<f32> {
    (0..len)
        .map(|n| 0.5 * (core::f32::consts::TAU * hz * n as f32 / SR).sin())
        .collect()
}

/// Power of `x` at `hz` (one DFT bin).
fn power_at(x: &[f32], hz: f32) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, s) in x.iter().enumerate() {
        let phase = f64::from(core::f32::consts::TAU) * f64::from(hz) * n as f64 / f64::from(SR);
        re += f64::from(*s) * phase.cos();
        im -= f64::from(*s) * phase.sin();
    }
    (re * re + im * im) / (x.len() as f64).powi(2)
}

/// Largest step between two neighbouring samples.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0, |m, w| m.max((w[1] - w[0]).abs()))
}

/// A bright, short, fully wet room set to `mode`, 10 ms of pre-delay.
fn room(mode: f32) -> Reverb {
    with(&[
        (SIZE, 0.0),
        (DAMPING, 0.0),
        (MIX, 1.0),
        (PREDELAY, 0.01),
        (TIME_CHANGE, mode),
    ])
}

/// A 1 kHz sine whose pre-delay moves to `to` seconds at frame `at`; the left output.
fn moved(mode: f32, to: f32, at: usize) -> Vec<f32> {
    let mut r = room(mode);
    let x = sine(at + 36_000, 1_000.0);
    run(&mut r, (&x, &x), 200, &[(at, ev(PREDELAY, to))]).0
}

#[test]
fn the_time_change_setting_is_appended_and_defaults_to_glide() {
    let p = DESCRIPTOR.param(ParamId(TIME_CHANGE)).unwrap();
    assert_eq!((TIME_CHANGE, p.key), (4, "time_change"));
    assert_eq!(
        (p.default, p.min, p.max, p.step),
        (GLIDE as f64, 0.0, 2.0, 1.0)
    );
    assert_eq!(p.values, ["glide", "fade", "jump"]);
    // Declared like the Echo's: the response is the device's own, not a ramp.
    assert_eq!(p.smoothing_seconds, 0.0);
    // The other ids did not move.
    let ids: Vec<u32> = DESCRIPTOR.params.iter().map(|p| p.id.0).collect();
    assert_eq!(ids, [0, 1, 2, 3, 4]);
    assert_eq!((FADE, JUMP), (1.0, 2.0));
}

#[test]
fn glide_bends_the_pitch_and_fade_and_jump_keep_it() {
    // 1 kHz in, the pre-delay 10 ms to 50 ms at frame 24 000. Under Glide it moves 0.25 frame
    // per frame, so the input is read at three quarters of its speed for 7 680 frames; the
    // window is inside that, after the shortest comb (1 116 frames) and some of its tail.
    let at = 24_000;
    let window = |y: &[f32]| y[at + 4_500..at + 7_000].to_vec();
    let band = |y: &[f32]| (power_at(&window(y), 1_000.0), power_at(&window(y), 750.0));
    let (g1k, g750) = band(&moved(GLIDE, 0.05, at));
    let (f1k, f750) = band(&moved(FADE, 0.05, at));
    let (j1k, j750) = band(&moved(JUMP, 0.05, at));
    println!(
        "power at 1 kHz / 750 Hz: glide {g1k:.2e} / {g750:.2e}, fade {f1k:.2e} / {f750:.2e}, jump {j1k:.2e} / {j750:.2e}"
    );
    assert!(g750 > 3.0 * g1k, "glide: {g750} at 750 Hz, {g1k} at 1 kHz");
    assert!(f1k > 100.0 * f750, "fade: {f1k} at 1 kHz, {f750} at 750 Hz");
    assert!(j1k > 100.0 * j750, "jump: {j1k} at 1 kHz, {j750} at 750 Hz");
}

#[test]
fn fade_does_not_click_and_jump_does() {
    // A move of half a period (24 frames at 1 kHz): the phase of the pre-delayed sine
    // flips. The largest step between two samples, against the same sine with
    // no move at all.
    let at = 24_000;
    let to = 0.01 + 24.0 / SR;
    let steady = {
        let mut r = room(0.0);
        let x = sine(at + 36_000, 1_000.0);
        run(&mut r, (&x, &x), 200, &[]).0
    };
    let reference = max_step(&steady[at..]);
    let fade = max_step(&moved(FADE, to, at)[at..]);
    let jump = max_step(&moved(JUMP, to, at)[at..]);
    println!(
        "largest step: steady {reference:.4}, fade {fade:.4} ({:.2}x), jump {jump:.4} ({:.2}x)",
        fade / reference,
        jump / reference
    );
    assert!(fade < reference * 1.15, "fade {fade}, steady {reference}");
    assert!(jump > reference * 1.5, "jump {jump}, steady {reference}");
}

/// The first non-zero sample of the left output for an impulse at `imp`, the pre-delay set to
/// `to` at frame `ev_at` (the pre-delay starts at 0 and the setting is `mode`).
fn onset(mode: f32, to: f32, ev_at: usize, imp: usize) -> usize {
    let mut r = with(&[
        (SIZE, 0.0),
        (DAMPING, 0.0),
        (MIX, 1.0),
        (PREDELAY, 0.0),
        (TIME_CHANGE, mode),
    ]);
    let x = impulse(imp + 20_000, imp);
    let (l, _) = run(&mut r, (&x, &x), 256, &[(ev_at, ev(PREDELAY, to))]);
    l.iter().position(|s| s.abs() > 1e-9).unwrap()
}

/// The same, for a pre-delay that was `to` from the start.
fn plain_onset(to: f32, imp: usize) -> usize {
    let mut r = with(&[(SIZE, 0.0), (DAMPING, 0.0), (MIX, 1.0), (PREDELAY, to)]);
    let x = impulse(imp + 20_000, imp);
    let (l, _) = run(&mut r, (&x, &x), 256, &[]);
    l.iter().position(|s| s.abs() > 1e-9).unwrap()
}

#[test]
fn jump_applies_the_new_pre_delay_at_the_exact_frame() {
    // An impulse at frame 3 000 (in the middle of a block). The change at 3 000 is applied
    // before that frame is rendered: the impulse is read at the new delay. At 3 001 it is
    // applied after it: the impulse is read at the old one.
    let imp = 3_000;
    let new = plain_onset(0.1, imp);
    let old = plain_onset(0.0, imp);
    assert_eq!(new - old, 4_800);
    assert_eq!(onset(JUMP, 0.1, imp, imp), new);
    assert_eq!(onset(JUMP, 0.1, imp + 1, imp), old);
    // Glide does not: it is still near the old delay.
    assert!(onset(GLIDE, 0.1, imp, imp) < old + 300);
}

#[test]
fn fade_lands_on_the_exact_pre_delay_and_a_later_change_waits() {
    let fade_frames = (FADE_SECONDS * SR).round() as usize;
    assert_eq!(fade_frames, 2_400);
    let ev_at = 1_000;
    // Once the fade is over, an impulse is read at the exact new delay.
    let imp = ev_at + fade_frames + 1;
    assert_eq!(onset(FADE, 0.1, ev_at, imp), plain_onset(0.1, imp));
    // A change during the fade waits for it, then fades toward the latest target: after both
    // fades, the delay is the second target.
    let mut r = with(&[
        (SIZE, 0.0),
        (DAMPING, 0.0),
        (MIX, 1.0),
        (PREDELAY, 0.0),
        (TIME_CHANGE, FADE),
    ]);
    let imp = ev_at + 2 * fade_frames + 100;
    let x = impulse(imp + 20_000, imp);
    let events = [
        (ev_at, ev(PREDELAY, 0.1)),
        (ev_at + 500, ev(PREDELAY, 0.05)),
    ];
    let (l, _) = run(&mut r, (&x, &x), 256, &events);
    let got = l.iter().position(|s| s.abs() > 1e-9).unwrap();
    assert_eq!(got, plain_onset(0.05, imp));
}

#[test]
fn jump_cuts_a_running_fade_and_leaving_fade_lets_it_finish() {
    let ev_at = 1_000;
    let imp = ev_at + 1_000;
    // Fade to 0.1 s starts at 1 000; the setting moves to Jump at 1 500: the delay lands on
    // the target at once, so an impulse read afterwards has the exact new delay.
    let run_with = |second: f32| {
        let mut r = with(&[
            (SIZE, 0.0),
            (DAMPING, 0.0),
            (MIX, 1.0),
            (PREDELAY, 0.0),
            (TIME_CHANGE, FADE),
        ]);
        let x = impulse(imp + 20_000, imp);
        let events = [
            (ev_at, ev(PREDELAY, 0.1)),
            (ev_at + 500, ev(TIME_CHANGE, second)),
        ];
        let (l, _) = run(&mut r, (&x, &x), 256, &events);
        l.iter().position(|s| s.abs() > 1e-9).unwrap()
    };
    assert_eq!(run_with(JUMP), plain_onset(0.1, imp));
    // Back to Glide during the fade: the fade finishes, the delay is the target after it.
    let imp_late = ev_at + 2_400 + 10;
    let mut r = with(&[
        (SIZE, 0.0),
        (DAMPING, 0.0),
        (MIX, 1.0),
        (PREDELAY, 0.0),
        (TIME_CHANGE, FADE),
    ]);
    let x = impulse(imp_late + 20_000, imp_late);
    let events = [
        (ev_at, ev(PREDELAY, 0.1)),
        (ev_at + 500, ev(TIME_CHANGE, GLIDE)),
    ];
    let (l, _) = run(&mut r, (&x, &x), 256, &events);
    let got = l.iter().position(|s| s.abs() > 1e-9).unwrap();
    assert_eq!(got, plain_onset(0.1, imp_late));
}

#[test]
fn every_setting_stays_finite_and_bounded_under_a_stream_of_changes() {
    let x = noise(SR as usize * 3, 21);
    for mode in [GLIDE, FADE, JUMP] {
        let mut r = room(mode);
        // A new pre-delay every 700 frames (several arrive during a fade), the setting
        // itself switched now and then; `run` checks for NaN and denormals and runs under the
        // allocation detector.
        let mut events = Vec::new();
        let mut seed = 5u32;
        for k in 0..200 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let at = 100 + k * 700;
            events.push((
                at,
                ev(PREDELAY, (seed >> 8) as f32 / (1u32 << 24) as f32 * 0.2),
            ));
            if k % 37 == 0 {
                events.push((at + 1, ev(TIME_CHANGE, (k % 3) as f32)));
            }
        }
        let (l, rr) = run(&mut r, (&x, &x), 333, &events);
        assert!(peak(&l).max(peak(&rr)) < 20.0, "{mode}");
    }
}

#[test]
fn reset_ends_a_fade_and_starts_at_the_target() {
    let x = noise(10_000, 8);
    let build = || {
        with(&[
            (SIZE, 0.4),
            (MIX, 1.0),
            (PREDELAY, 0.05),
            (TIME_CHANGE, FADE),
        ])
    };
    let mut r = build();
    // A fade is running (it starts at frame 100 and lasts 2 400 frames) and the line is full
    // when the reset comes, at frame 1 000.
    let _ = run(
        &mut r,
        (&x[..1_000], &x[..1_000]),
        256,
        &[(100, ev(PREDELAY, 0.15))],
    );
    assert_no_alloc(|| r.reset());
    let after = run(&mut r, (&x, &x), 256, &[]);
    // The parameter values are kept (the target is 0.15 s now): the same as a new device
    // built with it.
    let mut fresh = with(&[
        (SIZE, 0.4),
        (MIX, 1.0),
        (PREDELAY, 0.15),
        (TIME_CHANGE, FADE),
    ]);
    assert!(
        after == run(&mut fresh, (&x, &x), 256, &[]),
        "same sound as a new device"
    );
}

#[test]
fn glide_is_the_default_setting() {
    let x = noise(20_000, 2);
    let events = [(500, ev(PREDELAY, 0.15))];
    let mut a = Reverb::new(SR).unwrap();
    let mut b = Reverb::new(SR).unwrap();
    b.set_param(ParamId(TIME_CHANGE), GLIDE);
    a.reset();
    b.reset();
    assert!(run(&mut a, (&x, &x), 256, &events) == run(&mut b, (&x, &x), 256, &events));
}
