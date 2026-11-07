//! The brick-wall limiter, `faulk.ceiling` (#159): offline renders through the device
//! contract, every `process` under the allocation detector. Asserts on the signal: no sample
//! above the ceiling on noise and drums pushed by 18 dB (and the reconstructed peak within
//! 0.1 dB of it with the true-peak detection), the transparency below the ceiling (the input
//! delayed by the declared latency, bit for bit), the declared latency against the one
//! measured with an impulse, the attack before the peak, both releases, the compensation, no
//! NaN and no denormal with every parameter swept to its bounds, the parameter safety, the
//! block-size invariance, a change of latency while sounding, and the reset. The contract's
//! conformance run on every device of the registry is in `osc_synth.rs`; the native/WASM
//! parity is `crates/engine-wasm/tests/ceiling_parity.rs`.
//!
//! `cargo test -p engine --test it ceiling:: -- --nocapture` prints the measured numbers and
//! writes the drum renders to `.qa/159/` (dry, pushed and clipped, pushed through Ceiling).

use assert_no_alloc::assert_no_alloc;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceSetup, Event, EventBuffer, Transport, VoiceId,
};
use engine::devices::ceiling::{
    self, CEILING, COMPENSATE, Ceiling, DESCRIPTOR, FADE_SECONDS, GAIN, LOOKAHEAD, RELEASE,
    RELEASE_MODE, SETTLE_SECONDS, TP_DELAY, TP_MIN_LOOKAHEAD, TRUE_PEAK, db_to_gain,
};
use engine::devices::drums;
use engine::msg::ParamId;
use test_support::Render;

const SR: f32 = 48_000.0;

/// A limiter at `rate` with these values, the ramps snapped (so a test starts at its values).
fn at_rate(rate: f32, params: &[(u32, f32)]) -> Ceiling {
    let mut c = Ceiling::new(rate).unwrap();
    for (id, value) in params {
        c.set_param(ParamId(*id), *value);
    }
    c.reset();
    c
}

fn with(params: &[(u32, f32)]) -> Ceiling {
    at_rate(SR, params)
}

/// Runs `input` (left, right) through the device in blocks of `size`, `events` applying at
/// their (absolute) frame.
fn run_at(
    device: &mut Ceiling,
    rate: f32,
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
            sample_rate: rate,
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
    device: &mut Ceiling,
    input: (&[f32], &[f32]),
    size: usize,
    events: &[(usize, Event)],
) -> (Vec<f32>, Vec<f32>) {
    run_at(device, SR, input, size, events)
}

fn stereo(device: &mut Ceiling, input: (&[f32], &[f32])) -> (Vec<f32>, Vec<f32>) {
    run(device, input, 128, &[])
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

/// `x` low-passed at 20 kHz (a 255-tap Blackman-windowed sinc): program material, which an
/// converter or a resampler has already band-limited, has nothing between 20 kHz and the
/// Nyquist frequency, where the true peak of a signal is not defined by any finite
/// interpolator (the reconstructed peak of white noise keeps rising with the length of the
/// interpolator: +4.8 dB over the sample peak with 16 taps, +6.4 dB with 512).
fn band_limited(x: &[f32]) -> Vec<f32> {
    const HALF: i64 = 127;
    let fc = 20_000.0 / f64::from(SR);
    let taps: Vec<f64> = (-HALF..=HALF)
        .map(|i| {
            let t = i as f64;
            let sinc = if i == 0 {
                2.0 * fc
            } else {
                (core::f64::consts::TAU * fc * t).sin() / (core::f64::consts::PI * t)
            };
            let w = 0.42
                + 0.5 * (core::f64::consts::PI * t / HALF as f64).cos()
                + 0.08 * (core::f64::consts::TAU * t / HALF as f64).cos();
            sinc * w
        })
        .collect();
    (0..x.len() as i64)
        .map(|n| {
            let mut v = 0f64;
            for (k, h) in taps.iter().enumerate() {
                let at = n + HALF - k as i64;
                if (0..x.len() as i64).contains(&at) {
                    v += h * f64::from(x[at as usize]);
                }
            }
            v as f32
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

/// Two bars of a drum loop at 120 BPM from the Drums device (kick, snare, hats, accents), at
/// its defaults: mono, rendered through the device contract.
fn drum_loop() -> Vec<f32> {
    let mut kit = drums::factory(&DeviceSetup::default()).unwrap();
    let step = (SR * 0.25) as usize; // an eighth note at 120 BPM
    const BAR: [(f32, f32, f32); 8] = [
        (0.95, 0.0, 0.7),
        (0.0, 0.0, 0.4),
        (0.0, 0.9, 0.7),
        (0.0, 0.0, 0.4),
        (0.85, 0.0, 0.7),
        (0.75, 0.0, 0.4),
        (0.0, 0.95, 0.7),
        (0.0, 0.0, 0.35),
    ];
    let mut out = Vec::new();
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let mut voice = 0u32;
    let mut next = |key: u8, velocity: f32| {
        voice += 1;
        Event::NoteOn {
            offset: 0,
            voice: VoiceId::new(voice).unwrap(),
            key,
            velocity,
            tune_cents: 0.0,
        }
    };
    for _ in 0..2 {
        for (kick, snare, hat) in BAR {
            let mut events = Vec::new();
            if kick > 0.0 {
                events.push(next(drums::KICK_KEY, kick));
            }
            if snare > 0.0 {
                events.push(next(drums::SNARE_KEY, snare));
            }
            events.push(next(drums::CLOSED_HAT_KEY, hat));
            let mut done = 0;
            while done < step {
                let frames = 128.min(step - done);
                let mut left = vec![0.0; frames];
                let mut right = vec![0.0; frames];
                let ctx = BlockCtx {
                    sample_rate: SR,
                    frames: frames as u32,
                    transport: Transport::default(),
                };
                let mut io = AudioIo {
                    left: &mut left,
                    right: &mut right,
                    events_out: &mut unused,
                };
                let now: &[Event] = if done == 0 { &events } else { &[] };
                kit.process(&ctx, now, &mut io);
                out.extend(left);
                done += frames;
            }
        }
    }
    out
}

/// The reconstructed (true) peak of `x`: 8× oversampling by a long Kaiser-windowed sinc (48
/// samples each side), independent of the device's own 4× detector.
fn true_peak(x: &[f32]) -> f32 {
    const OVER: usize = 8;
    const HALF: i64 = 48;
    let beta = 9.0f64;
    let i0 = |v: f64| {
        let (mut sum, mut term) = (1.0f64, 1.0f64);
        for k in 1..80 {
            let h = v / (2.0 * k as f64);
            term *= h * h;
            sum += term;
        }
        sum
    };
    let norm = i0(beta);
    let mut table = vec![vec![0f64; 2 * HALF as usize]; OVER];
    for (j, row) in table.iter_mut().enumerate().skip(1) {
        let frac = j as f64 / OVER as f64;
        for (i, tap) in row.iter_mut().enumerate() {
            let t = frac + (HALF - 1) as f64 - i as f64;
            let sinc = (core::f64::consts::PI * t).sin() / (core::f64::consts::PI * t);
            let u = (t / HALF as f64).clamp(-1.0, 1.0);
            *tap = sinc * i0(beta * (1.0 - u * u).sqrt()) / norm;
        }
    }
    let mut best = peak(x);
    for k in (HALF - 1)..(x.len() as i64 - HALF) {
        for row in table.iter().skip(1) {
            let mut v = 0f64;
            for (i, h) in row.iter().enumerate() {
                v += h * f64::from(x[(k - (HALF - 1)) as usize + i]);
            }
            best = best.max(v.abs() as f32);
        }
    }
    best
}

fn write(name: &str, left: &[f32], right: &[f32]) -> std::path::PathBuf {
    let render = Render::new(SR, left.to_vec(), right.to_vec());
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.qa/159");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.wav"));
    std::fs::write(&path, render.wav()).unwrap();
    path
}

// ---------------------------------------------------------------- the ceiling holds

/// Every setting the push tests go through: look-ahead 0, default and longest, both releases.
const SHAPES: [(f32, f32, f32); 5] = [
    (0.0015, 1.0, 0.1),
    (0.0, 1.0, 0.1),
    (0.005, 1.0, 0.1),
    (0.0015, 0.0, 0.001),
    (0.005, 0.0, 1.0),
];

#[test]
fn noise_and_drums_pushed_18_db_never_go_above_the_ceiling() {
    let white = noise(48_000, 7);
    let white_r = noise(48_000, 11);
    let kit = drum_loop();
    let inputs: [(&str, &[f32], &[f32]); 2] = [("noise", &white, &white_r), ("drums", &kit, &kit)];
    for (name, left, right) in inputs {
        for ceiling_db in [-12.0f32, -6.0, -0.3, 0.0] {
            for (lookahead, mode, release) in SHAPES {
                let mut c = with(&[
                    (CEILING, ceiling_db),
                    (GAIN, 18.0),
                    (LOOKAHEAD, lookahead),
                    (RELEASE_MODE, mode),
                    (RELEASE, release),
                ]);
                let (l, r) = stereo(&mut c, (left, right));
                check(&l);
                check(&r);
                let limit = db_to_gain(ceiling_db);
                let top = peak(&l).max(peak(&r));
                assert!(
                    top <= limit,
                    "{name} at {ceiling_db} dB, {lookahead} s, mode {mode}: peak {top} over {limit}"
                );
                // It really limits: the pushed signal sits on the ceiling.
                assert!(
                    db(top) > ceiling_db - 0.5,
                    "{name} at {ceiling_db} dB: peak {} dB",
                    db(top)
                );
            }
        }
    }
}

#[test]
fn with_the_true_peak_the_reconstructed_peak_stays_within_a_tenth_of_a_decibel() {
    let white = band_limited(&noise(24_000, 3));
    let white_r = band_limited(&noise(24_000, 5));
    let kit = band_limited(&drum_loop()[..48_000]);
    let kit = &kit[..];
    let inputs: [(&str, &[f32], &[f32]); 2] = [("noise", &white, &white_r), ("drums", kit, kit)];
    let mut worst = f32::MIN;
    for (name, left, right) in inputs {
        for ceiling_db in [-6.0f32, -1.0, -0.3] {
            for (lookahead, mode, release) in SHAPES {
                let mut c = with(&[
                    (CEILING, ceiling_db),
                    (GAIN, 18.0),
                    (LOOKAHEAD, lookahead),
                    (RELEASE_MODE, mode),
                    (RELEASE, release),
                    (TRUE_PEAK, 1.0),
                ]);
                let (l, r) = stereo(&mut c, (left, right));
                check(&l);
                check(&r);
                assert!(peak(&l).max(peak(&r)) <= db_to_gain(ceiling_db));
                let over = db(true_peak(&l).max(true_peak(&r))) - ceiling_db;
                println!(
                    "{name} at {ceiling_db} dB, lookahead {lookahead}, mode {mode}: true peak {over:+.3} dB over the ceiling"
                );
                worst = worst.max(over);
                assert!(
                    over <= 0.1,
                    "{name} at {ceiling_db} dB: true peak {over:+.3} dB"
                );
                assert!(
                    over > -1.0,
                    "{name} at {ceiling_db} dB: it limits ({over:+.3} dB)"
                );
            }
        }
    }
    println!("worst true peak: {worst:+.3} dB over the ceiling");
    // For the record, not asserted: the same at the defaults on the raw signals, whose
    // content up to the Nyquist frequency has no finite reconstruction (see `band_limited`).
    let raw_l = noise(24_000, 3);
    let raw_r = noise(24_000, 5);
    let raw_kit = drum_loop()[..48_000].to_vec();
    for (name, left, right) in [
        ("raw noise", &raw_l, &raw_r),
        ("raw drums", &raw_kit, &raw_kit),
    ] {
        let mut c = with(&[(GAIN, 18.0), (TRUE_PEAK, 1.0)]);
        let (l, r) = stereo(&mut c, (left, right));
        let over = db(true_peak(&l).max(true_peak(&r))) + 0.3;
        println!("{name}, defaults: true peak {over:+.3} dB over the ceiling (not band-limited)");
    }
}

#[test]
fn without_the_true_peak_the_peaks_between_samples_can_pass() {
    // The reason for the option: a sine near a quarter of the sample rate, sampled off its
    // crests, reads lower on the samples than its true peak.
    let len = 24_000;
    let tone: Vec<f32> = (0..len)
        .map(|n| (core::f32::consts::TAU * 12_000.0 * n as f32 / SR + 0.785).sin())
        .collect();
    let mut plain = with(&[(GAIN, 6.0), (CEILING, -1.0)]);
    let (l, _) = stereo(&mut plain, (&tone, &tone));
    let mut tp = with(&[(GAIN, 6.0), (CEILING, -1.0), (TRUE_PEAK, 1.0)]);
    let (lt, _) = stereo(&mut tp, (&tone, &tone));
    let (over, over_tp) = (
        db(true_peak(&l[4_800..])) + 1.0,
        db(true_peak(&lt[4_800..])) + 1.0,
    );
    println!(
        "sine at fs/4: true peak {over:+.2} dB over the ceiling without, {over_tp:+.3} dB with"
    );
    assert!(over > 0.5);
    assert!(over_tp <= 0.1);
}

// ---------------------------------------------------------------- transparency and latency

#[test]
fn below_the_ceiling_the_output_is_the_input_delayed_by_the_latency() {
    let quiet_l: Vec<f32> = noise(9_600, 21).iter().map(|s| s * 0.3).collect();
    let quiet_r: Vec<f32> = sine(9_600, 440.0, 0.5);
    for true_peak_on in [0.0, 1.0] {
        for lookahead in [0.0, 0.0015, 0.005, 0.00073] {
            let mut c = with(&[(LOOKAHEAD, lookahead), (TRUE_PEAK, true_peak_on)]);
            let latency = c.latency_frames() as usize;
            let (l, r) = stereo(&mut c, (&quiet_l, &quiet_r));
            assert!(l[..latency].iter().chain(&r[..latency]).all(|s| *s == 0.0));
            for i in latency..l.len() {
                assert_eq!(l[i].to_bits(), quiet_l[i - latency].to_bits(), "left {i}");
                assert_eq!(r[i].to_bits(), quiet_r[i - latency].to_bits(), "right {i}");
            }
            assert_eq!(c.gain_reduction_db(), 0.0);
        }
    }
}

#[test]
fn the_declared_latency_is_the_measured_one() {
    for rate in [44_100.0f32, 48_000.0, 96_000.0, 192_000.0] {
        for true_peak_on in [false, true] {
            for lookahead in [0.0f32, 0.0015, 0.005, 0.0021] {
                let mut c = at_rate(
                    rate,
                    &[
                        (LOOKAHEAD, lookahead),
                        (TRUE_PEAK, f32::from(u8::from(true_peak_on))),
                    ],
                );
                // The look-ahead rounded to the frame; with the true peak at least
                // `TP_MIN_LOOKAHEAD`, plus the detector's own delay.
                let frames = (f64::from(lookahead) * f64::from(rate)).round() as u32;
                let expected = if true_peak_on {
                    frames.max(TP_MIN_LOOKAHEAD) + TP_DELAY
                } else {
                    frames
                };
                assert_eq!(c.latency_frames(), expected);
                // An impulse well under the ceiling: it comes out whole, `latency` frames on.
                let mut impulse = vec![0.0; 4_096];
                impulse[100] = 0.5;
                let (l, _) = run_at(&mut c, rate, (&impulse, &impulse), 128, &[]);
                let at = l.iter().position(|s| *s != 0.0).unwrap();
                assert_eq!(
                    at - 100,
                    expected as usize,
                    "{rate} Hz, tp {true_peak_on}, {lookahead} s"
                );
                assert_eq!(l[at], 0.5);
                // And loud, limited: its peak still comes out at the same frame.
                let mut c = at_rate(
                    rate,
                    &[
                        (LOOKAHEAD, lookahead),
                        (TRUE_PEAK, f32::from(u8::from(true_peak_on))),
                        (GAIN, 18.0),
                    ],
                );
                let (l, _) = run_at(&mut c, rate, (&impulse, &impulse), 128, &[]);
                let loudest = l.iter().enumerate().fold((0, 0f32), |m, (i, s)| {
                    if s.abs() > m.1 { (i, s.abs()) } else { m }
                });
                assert_eq!(loudest.0 - 100, expected as usize);
            }
        }
    }
}

#[test]
fn the_gain_is_down_before_the_peak_with_the_lookahead() {
    // Silence then a full-scale burst pushed 18 dB: with 2 ms of look-ahead the gain has
    // ramped down before the burst comes out, the first loud sample is already on the ceiling.
    let mut input = vec![0.0f32; 4_800];
    for (i, s) in input.iter_mut().enumerate().skip(2_000) {
        *s = if i % 2 == 0 { 0.9 } else { -0.9 };
    }
    let mut c = with(&[(GAIN, 18.0), (LOOKAHEAD, 0.002)]);
    let latency = c.latency_frames() as usize;
    let (l, _) = stereo(&mut c, (&input, &input));
    let limit = db_to_gain(-0.3);
    let first = 2_000 + latency;
    assert!(l[..first].iter().all(|s| *s == 0.0));
    assert!(
        l[first].abs() <= limit && l[first].abs() > limit * 0.98,
        "{}",
        l[first]
    );
    assert!(c.gain_reduction_db() > 17.0);
}

#[test]
fn the_manual_release_is_a_time_constant() {
    // A loud burst then a quiet steady tone: the reduction (in dB) decays by 63% in `release`.
    let release = 0.05f32;
    let len = 48_000;
    let mut input = vec![0.1f32; len];
    for s in input.iter_mut().take(4_800) {
        *s = 1.0;
    }
    let mut c = with(&[
        (RELEASE_MODE, 0.0),
        (RELEASE, release),
        (GAIN, 12.0),
        (CEILING, -6.0),
        (LOOKAHEAD, 0.0),
    ]);
    let (l, _) = stereo(&mut c, (&input, &input));
    // The output over the input after the gain: the limiter's gain, in dB.
    let g = db_to_gain(12.0);
    let reduction = |i: usize| -db(l[i].abs() / (input[i] * g));
    let start = reduction(4_800);
    let after = reduction(4_800 + (release * SR) as usize);
    let wanted = start * (-1.0f32).exp();
    // The steady tone at 0.1 · 4 = 0.4 sits under 0.5: the reduction goes back to 0.
    assert!(
        (after - wanted).abs() < 0.05 * start,
        "start {start}, after {after}, wanted {wanted}"
    );
    assert!(reduction(len - 1) < 0.01);
}

#[test]
fn the_auto_release_is_fast_after_a_click_and_slow_after_sustained_limiting() {
    // The time the reduction takes to fall under 1 dB after a 1 ms click and after 2 s of
    // limiting, both 12 dB over the ceiling, under a quiet tone.
    let time_to_recover = |loud: usize| {
        let len = loud + 96_000;
        let mut input = vec![0.05f32; len];
        for s in input.iter_mut().take(loud) {
            *s = 1.0;
        }
        let mut c = with(&[(RELEASE_MODE, 1.0), (CEILING, -12.0), (LOOKAHEAD, 0.0)]);
        let (l, _) = stereo(&mut c, (&input, &input));
        (loud..len)
            .find(|&i| -db(l[i].abs() / input[i]) < 1.0)
            .map(|i| (i - loud) as f32 / SR)
            .unwrap()
    };
    let (click, sustained) = (time_to_recover(48), time_to_recover(96_000));
    println!("auto release: {click:.3} s after a click, {sustained:.3} s after 2 s of limiting");
    assert!(click < 0.08, "{click}");
    assert!(sustained > 4.0 * click, "{sustained} vs {click}");
}

#[test]
fn the_compensation_keeps_the_level_of_a_quiet_signal() {
    let tone = sine(9_600, 300.0, 0.05);
    let mut c = with(&[(GAIN, 12.0), (COMPENSATE, 1.0)]);
    let latency = c.latency_frames() as usize;
    let (l, _) = stereo(&mut c, (&tone, &tone));
    for i in latency..l.len() {
        assert!(
            (l[i] - tone[i - latency]).abs() <= 1e-6,
            "{i}: {} vs {}",
            l[i],
            tone[i - latency]
        );
    }
    // Loud, it limits, then turns the result down by the gain: under the ceiling by 12 dB.
    let loud = sine(9_600, 300.0, 1.0);
    let mut c = with(&[(GAIN, 12.0), (COMPENSATE, 1.0)]);
    let (l, _) = stereo(&mut c, (&loud, &loud));
    assert!(
        (db(peak(&l[2_000..])) - (-0.3 - 12.0)).abs() < 0.1,
        "{}",
        db(peak(&l[2_000..]))
    );
}

#[test]
fn both_channels_share_one_gain() {
    let loud = sine(9_600, 200.0, 1.0);
    let quiet = sine(9_600, 200.0, 0.1);
    let mut c = with(&[(GAIN, 12.0)]);
    let (l, r) = stereo(&mut c, (&loud, &quiet));
    for i in 2_000..l.len() {
        if l[i].abs() > 0.05 {
            assert!((r[i] / l[i] - 0.1).abs() < 1e-3, "{i}");
        }
    }
}

// ---------------------------------------------------------------- safety

#[test]
fn every_parameter_at_its_bounds_stays_finite_and_bounded() {
    let loud = noise(9_600, 9);
    let loud: Vec<f32> = loud.iter().map(|s| s * 16.0).collect();
    let silence = vec![0.0f32; 4_800];
    let tiny = vec![1e-30f32; 4_800];
    for p in DESCRIPTOR.params {
        for value in [p.min, p.default, p.max] {
            for others in [false, true] {
                // The others at their defaults, or each at its max.
                let mut params: Vec<(u32, f32)> = if others {
                    DESCRIPTOR
                        .params
                        .iter()
                        .map(|q| (q.id.0, q.max as f32))
                        .collect()
                } else {
                    Vec::new()
                };
                params.push((p.id.0, value as f32));
                for input in [&loud, &silence, &tiny] {
                    let mut c = with(&params);
                    let (l, r) = stereo(&mut c, (input, input));
                    check(&l);
                    check(&r);
                    let ceiling_db = params
                        .iter()
                        .rev()
                        .find(|(id, _)| *id == CEILING)
                        .map_or(-0.3, |(_, v)| *v);
                    assert!(
                        peak(&l).max(peak(&r)) <= db_to_gain(ceiling_db),
                        "{} = {value}",
                        p.key
                    );
                    let reported = c.gain_reduction_db();
                    assert!(reported.is_finite() && reported >= 0.0);
                }
            }
        }
    }
}

#[test]
fn parameters_moved_to_their_bounds_mid_stream_stay_bounded() {
    let loud: Vec<f32> = noise(48_000, 13).iter().map(|s| s * 8.0).collect();
    let mut events = Vec::new();
    for (k, p) in DESCRIPTOR.params.iter().enumerate() {
        events.push((1_000 + 3_000 * k, param(p.id.0, p.max as f32)));
        events.push((2_500 + 3_000 * k, param(p.id.0, p.min as f32)));
    }
    events.sort_by_key(|(at, _)| *at);
    let mut c = with(&[]);
    let (l, r) = run(&mut c, (&loud, &loud), 97, &events);
    check(&l);
    check(&r);
    // The lowest ceiling the moves reach is -12 dB, the highest 0 dB.
    assert!(peak(&l).max(peak(&r)) <= 1.0);
}

#[test]
fn bad_values_and_ids_are_ignored_or_clamped() {
    let tone = sine(4_800, 300.0, 0.9);
    let reference = stereo(&mut with(&[]), (&tone, &tone));
    let mut c = with(&[]);
    for id in 0..12u32 {
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            c.set_param(ParamId(id), v);
        }
    }
    c.reset();
    assert_eq!(
        stereo(&mut c, (&tone, &tone)),
        reference,
        "non-finite values are ignored"
    );
    // Out of range values are clamped: a ceiling of +20 dB is 0 dB.
    let mut high = with(&[(CEILING, 20.0), (GAIN, 18.0)]);
    let (l, _) = stereo(&mut high, (&tone, &tone));
    assert!(peak(&l) <= 1.0);
    let mut low = with(&[(LOOKAHEAD, 1.0)]);
    assert_eq!(low.latency_frames(), (0.005 * SR) as u32);
    low.set_param(ParamId(LOOKAHEAD), -1.0);
    low.reset();
    assert_eq!(low.latency_frames(), 0);
    // A non-finite input never comes out.
    let mut c = with(&[(GAIN, 12.0)]);
    let mut bad = tone.clone();
    for (i, s) in bad.iter_mut().enumerate() {
        if i % 7 == 0 {
            *s = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30][i % 4];
        }
    }
    let (l, r) = stereo(&mut c, (&bad, &bad));
    check(&l);
    check(&r);
    assert!(peak(&l) <= db_to_gain(-0.3));
}

#[test]
fn the_render_does_not_depend_on_the_block_size() {
    let input: Vec<f32> = noise(24_000, 17).iter().map(|s| s * 4.0).collect();
    let events = [
        (3_001, param(CEILING, -3.0)),
        (9_000, param(GAIN, 6.0)),
        (12_345, param(RELEASE_MODE, 0.0)),
        (15_000, param(LOOKAHEAD, 0.004)),
        (20_000, param(TRUE_PEAK, 1.0)),
    ];
    let reference = run(
        &mut with(&[(TRUE_PEAK, 0.0)]),
        (&input, &input),
        2_048,
        &events,
    );
    for size in [1, 37, 128, 500] {
        let out = run(&mut with(&[]), (&input, &input), size, &events);
        assert_eq!(out, reference, "block size {size}");
    }
}

/// The last frame of `x` before `before` that is not 0.
fn last_sound(x: &[f32], before: usize) -> usize {
    (0..before).rev().find(|&i| x[i] != 0.0).unwrap()
}

#[test]
fn a_change_of_latency_while_sounding_settles_fades_and_keeps_the_ceiling() {
    let input: Vec<f32> = noise(48_000, 19).iter().map(|s| s * 6.0).collect();
    let events = [
        (10_000, param(LOOKAHEAD, 0.005)),
        (30_000, param(TRUE_PEAK, 1.0)),
    ];
    let mut c = with(&[]);
    let (l, r) = run(&mut c, (&input, &input), 128, &events);
    check(&l);
    check(&r);
    let limit = db_to_gain(-0.3);
    assert!(peak(&l).max(peak(&r)) <= limit);
    let settle = (SETTLE_SECONDS * SR) as usize;
    let fade = (FADE_SECONDS * SR) as usize;
    // The first change: the old latency for the settle, a fade out, silence for the new
    // latency (240 frames), then a fade in.
    let out_1 = 10_000 + settle;
    assert!(
        peak(&l[10_000..out_1]) > limit * 0.9,
        "it sounds while the change settles"
    );
    let mute_1 = out_1 + fade;
    assert!(
        last_sound(&l, mute_1 + 240) <= mute_1 + 1,
        "silent for the new latency"
    );
    assert!(
        l[mute_1 + 241..mute_1 + 260].iter().any(|s| *s != 0.0),
        "then it fades in"
    );
    assert!(peak(&l[mute_1 + 240 + fade..mute_1 + 240 + fade + 2_000]) > limit * 0.9);
    // The second change: the true peak adds its delay.
    let mute_2 = 30_000 + settle + fade;
    let latency = 240 + TP_DELAY as usize;
    assert!(last_sound(&l, mute_2 + latency) <= mute_2 + 1);
    assert_eq!(c.latency_frames() as usize, latency);
    // The new latency is the measured one: a quiet impulse once the reduction let go.
    c.set_param(ParamId(GAIN), 0.0);
    c.set_param(ParamId(RELEASE_MODE), 0.0);
    c.set_param(ParamId(RELEASE), 0.001);
    let _ = stereo(&mut c, (&vec![0.0; 4_800], &vec![0.0; 4_800]));
    let mut quiet = vec![0.0f32; 2_000];
    quiet[100] = 0.25;
    let (l, _) = stereo(&mut c, (&quiet, &quiet));
    assert_eq!(l.iter().position(|s| *s != 0.0), Some(100 + latency));
    assert_eq!(l[100 + latency], 0.25);
}

#[test]
fn a_drag_of_the_lookahead_switches_once_after_it_stops() {
    // A drag: a new value every 128 frames for 0.5 s, from 1.5 ms up to 4 ms. The old
    // latency plays all along (no silence), one switch after the last value.
    let input = sine(72_000, 220.0, 0.5);
    let mut events = Vec::new();
    for k in 0..188 {
        let value = 0.0015 + 0.0025 * k as f32 / 187.0;
        events.push((4_000 + 128 * k, param(LOOKAHEAD, value)));
    }
    let last = events.last().unwrap().0;
    let mut c = with(&[]);
    let (l, _) = run(&mut c, (&input, &input), 128, &events);
    check(&l);
    // No silence while dragging: the tone sounds in every 2 ms up to the end of the settle.
    let settle = (SETTLE_SECONDS * SR) as usize;
    for window in l[200..last + settle].chunks(96) {
        assert!(peak(window) > 0.2, "a drop-out during the drag");
    }
    // Then one silence, of the new latency, and the tone again.
    let fade = (FADE_SECONDS * SR) as usize;
    let mute = last + settle + fade;
    let latency = (0.004 * SR) as usize;
    assert!(l[mute + 1..mute + latency].iter().all(|s| *s == 0.0));
    assert!(peak(&l[mute + latency + fade..mute + latency + fade + 960]) > 0.45);
    assert_eq!(c.latency_frames() as usize, latency);
    // Exactly one silence: elsewhere, the tone never stops for 1 ms.
    let silent_runs = l[200..]
        .chunks(48)
        .filter(|w| w.iter().all(|s| *s == 0.0))
        .count();
    assert!(silent_runs <= latency / 48 + 1, "{silent_runs}");
}

#[test]
fn a_change_back_cancels_the_wait_and_a_change_in_the_silence_restarts_it() {
    let input = sine(48_000, 220.0, 0.5);
    let settle = (SETTLE_SECONDS * SR) as usize;
    let fade = (FADE_SECONDS * SR) as usize;
    // Asked, then taken back before the settle ends: no silence at all.
    let events = [
        (4_000, param(LOOKAHEAD, 0.004)),
        (5_000, param(LOOKAHEAD, 0.0015)),
    ];
    let mut c = with(&[]);
    let (l, _) = run(&mut c, (&input, &input), 128, &events);
    for window in l[200..].chunks(96) {
        assert!(peak(window) > 0.2);
    }
    assert_eq!(c.latency_frames(), 72);
    // A second change during the silence of the first: the newer latency, a new silence.
    let mute = 4_000 + settle + fade;
    let events = [
        (4_000, param(LOOKAHEAD, 0.004)),
        (mute + 50, param(LOOKAHEAD, 0.002)),
    ];
    let mut c = with(&[]);
    let (l, _) = run(&mut c, (&input, &input), 128, &events);
    check(&l);
    assert_eq!(c.latency_frames(), 96);
    assert!(l[mute + 1..mute + 50 + 96].iter().all(|s| *s == 0.0));
    assert!(peak(&l[mute + 50 + 96 + fade..mute + 50 + 96 + fade + 960]) > 0.45);
    // A change during the fade in waits again, the fade in going on.
    let fade_in = mute + 192;
    let events = [
        (4_000, param(LOOKAHEAD, 0.004)),
        (fade_in + 60, param(LOOKAHEAD, 0.001)),
    ];
    let mut c = with(&[]);
    let (l, _) = run(&mut c, (&input, &input), 128, &events);
    check(&l);
    let back = fade_in + 60 + settle;
    assert!(peak(&l[back - 960..back]) > 0.45, "the fade in went on");
    assert_eq!(c.latency_frames(), 48);
}

#[test]
fn a_change_taken_back_during_the_fade_out_fades_back_in_without_a_silence() {
    let input = sine(24_000, 220.0, 0.5);
    let settle = (SETTLE_SECONDS * SR) as usize;
    let fade = (FADE_SECONDS * SR) as usize;
    let out = 4_000 + settle;
    let events = [
        (4_000, param(TRUE_PEAK, 1.0)),
        (out + fade / 2, param(TRUE_PEAK, 0.0)),
    ];
    let mut c = with(&[]);
    let (l, _) = run(&mut c, (&input, &input), 128, &events);
    check(&l);
    assert_eq!(c.latency_frames(), 72);
    // It dips (half the fade out, then back in) but is never silent for a millisecond.
    for window in l[200..].chunks(48) {
        assert!(peak(window) > 0.1, "a silence");
    }
}

#[test]
fn a_switch_to_no_latency_has_no_silence_only_the_fades() {
    let input = sine(24_000, 220.0, 0.5);
    let settle = (SETTLE_SECONDS * SR) as usize;
    let fade = (FADE_SECONDS * SR) as usize;
    let mut c = with(&[]);
    let (l, _) = run(
        &mut c,
        (&input, &input),
        128,
        &[(4_000, param(LOOKAHEAD, 0.0))],
    );
    assert_eq!(c.latency_frames(), 0);
    let bottom = 4_000 + settle + fade;
    // At most the frame where the fade reached 0 is silent, then it fades in at once.
    assert!(
        l[bottom + 2..bottom + 2 + fade]
            .iter()
            .filter(|s| **s == 0.0)
            .count()
            < 4
    );
    assert!(peak(&l[bottom + fade + 10..bottom + fade + 970]) > 0.45);
}

#[test]
fn the_compensation_follows_the_gain_each_sample_was_pushed_by() {
    // Compensate on, a quiet tone, the gain ramped from 0 to 24 dB (20 ms) with the longest
    // latency: the level stays the tone's all along.
    let tone = sine(24_000, 300.0, 0.02);
    let mut c = with(&[(COMPENSATE, 1.0), (LOOKAHEAD, 0.005), (TRUE_PEAK, 1.0)]);
    let latency = c.latency_frames() as usize;
    let (l, _) = run(&mut c, (&tone, &tone), 128, &[(6_000, param(GAIN, 24.0))]);
    let mut worst = 0f32;
    for i in latency..l.len() {
        worst = worst.max((l[i] - tone[i - latency]).abs());
    }
    println!("compensation during a 24 dB ramp: largest difference {worst:e}");
    assert!(worst < 1e-6, "{worst}");
}

#[test]
fn a_change_before_any_input_applies_at_once() {
    let mut c = Ceiling::new(SR).unwrap();
    c.set_param(ParamId(LOOKAHEAD), 0.003);
    assert_eq!(c.latency_frames(), 144);
    let tone = sine(2_000, 300.0, 0.5);
    let (l, _) = stereo(&mut c, (&tone, &tone));
    assert_eq!(l[144], tone[0]);
    // Once sounding, a change waits: the latency in use stays until the switch.
    c.set_param(ParamId(LOOKAHEAD), 0.0);
    assert_eq!(c.latency_frames(), 144);
    let tone = sine(9_600, 300.0, 0.5);
    let _ = stereo(&mut c, (&tone, &tone));
    assert_eq!(c.latency_frames(), 0);
}

#[test]
fn reset_leaves_no_tail_and_keeps_the_parameters() {
    let loud: Vec<f32> = noise(4_800, 23).iter().map(|s| s * 4.0).collect();
    let mut c = with(&[(LOOKAHEAD, 0.005), (CEILING, -6.0)]);
    let _ = stereo(&mut c, (&loud, &loud));
    assert!(c.gain_reduction_db() > 0.0);
    assert_no_alloc(|| c.reset());
    assert_eq!(c.gain_reduction_db(), 0.0);
    // Nothing of what was in the line comes out.
    let silence = vec![0.0f32; 1_000];
    let (l, r) = stereo(&mut c, (&silence, &silence));
    assert!(l.iter().chain(&r).all(|s| *s == 0.0));
    // The parameters survived: the latency and the ceiling.
    assert_eq!(c.latency_frames(), 240);
    let (l, _) = stereo(&mut c, (&loud, &loud));
    assert!(peak(&l) <= db_to_gain(-6.0) && peak(&l) > db_to_gain(-6.5));
}

#[test]
fn the_reduction_is_reported_for_the_meter() {
    let mut c = with(&[(GAIN, 12.0)]);
    let loud = sine(4_800, 100.0, 1.0);
    let _ = stereo(&mut c, (&loud, &loud));
    let reported = c.gain_reduction_db();
    assert!((reported - 12.3).abs() < 0.5, "{reported}");
    // It falls back as the auto release lets go (its slow part has a 0.5 s time constant).
    let quiet = vec![0.0f32; 96_000];
    let _ = stereo(&mut c, (&quiet, &quiet));
    assert!(c.gain_reduction_db() < 0.3, "{}", c.gain_reduction_db());
    // A reset reports none.
    c.reset();
    assert_eq!(c.gain_reduction_db(), 0.0);
}

#[test]
fn the_descriptor_is_valid() {
    assert_eq!(DESCRIPTOR.validate(), Ok(()));
    assert_eq!(DESCRIPTOR.name, "Ceiling");
    assert_eq!(DESCRIPTOR.kind, "faulk.ceiling");
    let keys: Vec<_> = DESCRIPTOR.params.iter().map(|p| p.key).collect();
    assert_eq!(
        keys,
        [
            "ceiling",
            "gain",
            "compensate",
            "lookahead",
            "release_mode",
            "release",
            "true_peak"
        ]
    );
    let built = ceiling::factory(&DeviceSetup::default()).unwrap();
    assert_eq!(built.descriptor().kind, "faulk.ceiling");
    assert_eq!(built.latency_frames(), 72);
    // Each default is the one the device is built with.
    let mut c = Ceiling::new(SR).unwrap();
    let defaults: Vec<_> = DESCRIPTOR
        .params
        .iter()
        .map(|p| (p.id.0, p.default as f32))
        .collect();
    let mut explicit = with(&defaults);
    let input = noise(9_600, 5);
    assert_eq!(
        stereo(&mut c, (&input, &input)),
        stereo(&mut explicit, (&input, &input))
    );
}

// ---------------------------------------------------------------- the renders for the ear

#[test]
fn the_drum_renders_for_the_ear() {
    let kit = drum_loop();
    let push = db_to_gain(18.0);
    let ceiling = db_to_gain(-0.3);
    let clipped: Vec<f32> = kit
        .iter()
        .map(|s| (s * push).clamp(-ceiling, ceiling))
        .collect();
    let mut c = with(&[(GAIN, 18.0)]);
    let (limited, _) = stereo(&mut c, (&kit, &kit));
    let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    println!(
        "drums dry: peak {:.2} dB rms {:.2} dB; pushed 18 dB and clipped: peak {:.2} dB rms {:.2} dB, {} clipped samples; through Ceiling: peak {:.2} dB rms {:.2} dB, true peak {:.2} dB",
        db(peak(&kit)),
        db(rms(&kit)),
        db(peak(&clipped)),
        db(rms(&clipped)),
        clipped.iter().filter(|s| s.abs() >= ceiling).count(),
        db(peak(&limited)),
        db(rms(&limited)),
        db(true_peak(&limited[..24_000])),
    );
    assert!(peak(&limited) <= ceiling);
    for (name, x) in [
        ("drums-dry", &kit),
        ("drums-18db-clipped", &clipped),
        ("drums-18db-ceiling", &limited),
    ] {
        println!("{}", write(name, x, x).display());
    }
}

/// `cargo test -p engine --test it ceiling::cpu_cost --config "profile.dev.opt-level=3" -- --ignored --nocapture`
/// (the allocation detector does not build in release; debug assertions stay on, so this is an upper bound)
#[test]
#[ignore = "timing, run by hand in release"]
fn cpu_cost() {
    let source: Vec<f32> = noise(48_000, 4).iter().map(|s| s * 4.0).collect();
    let mut left = source.clone();
    let mut right = source.clone();
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    for (name, params) in [
        ("default", vec![]),
        (
            "true peak, 5 ms",
            vec![(TRUE_PEAK, 1.0), (LOOKAHEAD, 0.005)],
        ),
    ] {
        let mut c = with(&params);
        let mut best = f64::MAX;
        for _ in 0..50 {
            left.copy_from_slice(&source);
            right.copy_from_slice(&source);
            let started = std::time::Instant::now();
            for (l, r) in left.chunks_mut(128).zip(right.chunks_mut(128)) {
                let ctx = BlockCtx {
                    sample_rate: SR,
                    frames: l.len() as u32,
                    transport: Transport::default(),
                };
                let mut io = AudioIo {
                    left: l,
                    right: r,
                    events_out: &mut unused,
                };
                c.process(&ctx, &[], &mut io);
            }
            best = best.min(started.elapsed().as_secs_f64());
        }
        std::hint::black_box((&left, &right));
        let per_frame = best / 48_000.0 * 1e9;
        let block = per_frame * 128.0 / 1e3;
        println!(
            "ceiling ({name}): {per_frame:.1} ns per frame, {block:.2} us per 128-frame block ({:.3}% of its 2.67 ms budget)",
            block / 2_666.7 * 100.0
        );
    }
}
