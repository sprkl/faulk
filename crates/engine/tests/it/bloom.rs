//! Bloom, `faulk.bloom` (#696): offline renders through the device contract, every `process`
//! under the allocation detector. Asserts on the signal: the depth and rate of the vibrato
//! measured on a sine, the synced period and its place on the bar, the three voices of the
//! ensemble 120 degrees apart, the stereo decorrelation, the transparency at mix 0, no click
//! when a setting moves, no NaN, denormal or runaway at any setting, the block-size invariance
//! and the reset. The contract's conformance run on every device of the registry is in
//! `osc_synth.rs`; the native/WASM parity is `crates/engine-wasm/tests/bloom_parity.rs`.
//!
//! **Measuring the delay.** Vibrato, stereo phase 0: both channels read the same delay `d(n)`.
//! With a sine on the left and the cosine of the same tone on the right, the output is
//! `sin(w (n - d(n)) + c)` and `cos(w (n - d(n)) + c)` (`c` the constant phase of the tone
//! filter), so `atan2(left, right)`, unwrapped, gives `d(n)` at every frame, up to a constant.

use assert_no_alloc::assert_no_alloc;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceRole, DeviceSetup, Event, EventBuffer, MAX_SAMPLE_RATE,
    Transport, builtin,
};
use engine::devices::bloom::{
    self, Bloom, CHORUS, DELAY, DEPTH, DESCRIPTOR, DIVISION, ENSEMBLE, FEEDBACK, MAX_DELAY_SECONDS,
    MAX_SWEEP_SECONDS, MIX, MODE, RATE, SHAPE, STEREO_PHASE, SYNC, TONE, TRIANGLE, VIBRATO,
};
use engine::msg::ParamId;
use engine::time::Tempo;

const SR: f32 = 48_000.0;
/// Ticks per frame at 120 BPM and 48 kHz (960 per quarter note).
const TPF_120: f64 = 0.04;

fn bloom_with(params: &[(u32, f32)]) -> Bloom {
    let mut b = Bloom::new(SR).expect("the lines are allocated");
    for (id, value) in params {
        b.set_param(ParamId(*id), *value);
    }
    b.reset();
    b
}

/// A transport at 120 BPM, stopped.
fn stopped() -> Transport {
    Transport {
        tempo: Tempo::new(120.0).unwrap(),
        ..Transport::default()
    }
}

/// A transport at `bpm`, playing at `ticks`.
fn playing_at(bpm: f64, ticks: f64) -> Transport {
    Transport {
        playing: true,
        tempo: Tempo::new(bpm).unwrap(),
        position_ticks: ticks,
        ..Transport::default()
    }
}

/// Runs `left` and `right` through the device in blocks of `size`, the transport of each block
/// given by `transport_of(first frame)`, `events` applying at their frame.
fn run(
    device: &mut Bloom,
    left: &[f32],
    right: &[f32],
    size: usize,
    transport_of: impl Fn(usize) -> Transport,
    events: &[(usize, Event)],
) -> (Vec<f32>, Vec<f32>) {
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
    let mut start = 0;
    while start < left.len() {
        let end = (start + size).min(left.len());
        let mut l = left[start..end].to_vec();
        let mut r = right[start..end].to_vec();
        let local: Vec<Event> = events
            .iter()
            .filter(|(at, _)| (start..end).contains(at))
            .map(|(at, e)| e.at((at - start) as u32))
            .collect();
        let c = BlockCtx {
            sample_rate: SR,
            frames: (end - start) as u32,
            transport: transport_of(start),
        };
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut l,
                right: &mut r,
                events_out: &mut unused,
            };
            device.process(&c, &local, &mut io);
        });
        out_l.extend(l);
        out_r.extend(r);
        start = end;
    }
    (out_l, out_r)
}

/// Runs a mono input, transport stopped, blocks of 256.
fn run_mono(device: &mut Bloom, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
    run(device, input, input, 256, |_| stopped(), &[])
}

fn param(at: usize, id: u32, value: f32) -> (usize, Event) {
    (
        at,
        Event::ParamValue {
            offset: 0,
            id: ParamId(id),
            value,
        },
    )
}

fn sine(len: usize, hz: f64, amplitude: f32, cosine: bool) -> Vec<f32> {
    (0..len)
        .map(|n| {
            let x = core::f64::consts::TAU * hz * n as f64 / f64::from(SR);
            amplitude * if cosine { x.cos() } else { x.sin() } as f32
        })
        .collect()
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

/// A pad-like tone: three detuned saws (a chord of A3 and E4, with a slow beating).
fn pad(len: usize) -> Vec<f32> {
    let saw = |hz: f64, n: usize| {
        let p = hz * n as f64 / f64::from(SR);
        (2.0 * (p - p.floor()) - 1.0) as f32
    };
    (0..len)
        .map(|n| 0.15 * (saw(220.0, n) + saw(220.9, n) + saw(329.6, n)))
        .collect()
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

fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len() as f64;
    let (ma, mb) = (
        a.iter().map(|x| f64::from(*x)).sum::<f64>() / n,
        b.iter().map(|x| f64::from(*x)).sum::<f64>() / n,
    );
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (f64::from(*x) - ma, f64::from(*y) - mb);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    ab / (aa * bb).sqrt()
}

/// The delay read at every frame, in frames, up to a constant: the vibrato of a sine (left)
/// and a cosine (right) of `hz` (module doc).
fn delay_of(left: &[f32], right: &[f32], hz: f64) -> Vec<f64> {
    let w = core::f64::consts::TAU * hz / f64::from(SR);
    let mut out = Vec::with_capacity(left.len());
    let mut unwrapped = 0.0f64;
    let mut previous = 0.0f64;
    for (n, (l, r)) in left.iter().zip(right).enumerate() {
        let theta = f64::from(*l).atan2(f64::from(*r));
        if n == 0 {
            unwrapped = theta;
        } else {
            let mut d = theta - previous;
            while d > core::f64::consts::PI {
                d -= core::f64::consts::TAU;
            }
            while d < -core::f64::consts::PI {
                d += core::f64::consts::TAU;
            }
            unwrapped += d;
        }
        previous = theta;
        out.push(n as f64 - unwrapped / w);
    }
    out
}

/// The frames where `x` crosses `level` going up, with linear interpolation between frames.
fn rising(x: &[f64], level: f64) -> Vec<f64> {
    x.windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < level && w[1] >= level)
        .map(|(i, w)| i as f64 + (level - w[0]) / (w[1] - w[0]))
        .collect()
}

fn min_max(x: &[f64]) -> (f64, f64) {
    x.iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)))
}

/// A vibrato on a 440 Hz sine/cosine pair, stereo phase 0: the measured delay.
fn vibrato_delay(
    params: &[(u32, f32)],
    len: usize,
    transport_of: impl Fn(usize) -> Transport,
) -> Vec<f64> {
    let mut all = vec![(MODE, VIBRATO), (STEREO_PHASE, 0.0)];
    all.extend_from_slice(params);
    let mut b = bloom_with(&all);
    let (l, r) = run(
        &mut b,
        &sine(len, 440.0, 0.5, false),
        &sine(len, 440.0, 0.5, true),
        256,
        transport_of,
        &[],
    );
    check(&l);
    check(&r);
    delay_of(&l, &r, 440.0)
}

// ---------------------------------------------------------------- depth, rate and pitch

#[test]
fn a_vibrato_of_5_hz_at_half_depth_sweeps_5_ms_and_bends_440_hz_symmetrically() {
    // 1.5 s; the first 0.1 s (the line filling) is left out.
    let len = 72_000;
    let d = vibrato_delay(
        &[(SYNC, 0.0), (RATE, 5.0), (DEPTH, 50.0), (DELAY, 0.001)],
        len,
        |_| stopped(),
    );
    let steady = &d[4_800..];
    let (lo, hi) = min_max(steady);
    let excursion = hi - lo;
    // depth 50 % of 10 ms = 5 ms peak to peak = 240 frames.
    let expected = 0.5 * f64::from(MAX_SWEEP_SECONDS * SR);
    println!("excursion {excursion:.3} frames (expected {expected})");
    assert!(
        (excursion - expected).abs() < 0.01 * expected,
        "{excursion}"
    );
    // The period: 9,600 frames between rising crossings of the centre.
    let ups = rising(steady, (lo + hi) / 2.0);
    assert!(ups.len() >= 6, "{ups:?}");
    for w in ups.windows(2) {
        assert!(
            (w[1] - w[0] - 9_600.0).abs() < 1.0,
            "period {}",
            w[1] - w[0]
        );
    }
    // The pitch: 440 Hz x (1 - d'(n)); the frequency swings pi x 5 Hz x 5 ms = 7.854 % either
    // side, i.e. 474.56 Hz and 405.44 Hz (+130.9 and -141.6 cents), symmetric in hertz.
    let f: Vec<f64> = steady
        .windows(2)
        .map(|w| 440.0 * (1.0 - (w[1] - w[0])))
        .collect();
    // A light average over 9 frames: the interpolation's phase error ripples at the tone.
    let smooth: Vec<f64> = f.windows(9).map(|w| w.iter().sum::<f64>() / 9.0).collect();
    let (f_lo, f_hi) = min_max(&smooth);
    let swing = 440.0 * core::f64::consts::PI * 5.0 * 0.005;
    let cents = |hz: f64| 1200.0 * (hz / 440.0).log2();
    println!(
        "frequency {f_lo:.2} .. {f_hi:.2} Hz ({:+.1} / {:+.1} cents), expected 440 -/+ {swing:.2}",
        cents(f_hi),
        cents(f_lo)
    );
    assert!((f_hi - 440.0 - swing).abs() < 0.02 * swing, "{f_hi}");
    assert!((440.0 - f_lo - swing).abs() < 0.02 * swing, "{f_lo}");
    // Symmetric in hertz.
    assert!(((f_hi - 440.0) - (440.0 - f_lo)).abs() < 0.01 * swing);
}

#[test]
fn depth_scales_the_sweep_and_rate_the_period() {
    for (rate, depth) in [(0.5f32, 100.0f32), (2.0, 20.0), (10.0, 100.0)] {
        let period = f64::from(SR / rate);
        let len = 4_800 + (2.5 * period) as usize;
        let d = vibrato_delay(&[(RATE, rate), (DEPTH, depth), (DELAY, 0.005)], len, |_| {
            stopped()
        });
        let steady = &d[4_800..];
        let (lo, hi) = min_max(steady);
        let expected = f64::from(depth / 100.0 * MAX_SWEEP_SECONDS * SR);
        assert!(
            (hi - lo - expected).abs() < 0.01 * expected,
            "rate {rate} depth {depth}: {} vs {expected}",
            hi - lo
        );
        let ups = rising(steady, (lo + hi) / 2.0);
        for w in ups.windows(2) {
            assert!(
                (w[1] - w[0] - period).abs() < 1.0,
                "rate {rate}: {}",
                w[1] - w[0]
            );
        }
    }
}

#[test]
fn the_shortest_delay_is_the_delay_setting_and_depth_0_is_a_plain_delay() {
    let mut b = bloom_with(&[(MODE, VIBRATO), (DEPTH, 0.0), (DELAY, 0.004)]);
    let mut x = vec![0.0; 4_000];
    x[1_000] = 1.0;
    let (l, r) = run_mono(&mut b, &x);
    // 4 ms = 192 frames; the 20 Hz tone filter takes 0.13 % of the impulse.
    let at = l.iter().enumerate().fold(
        0,
        |best, (i, s)| if s.abs() > l[best].abs() { i } else { best },
    );
    assert_eq!(at, 1_192);
    assert!((l[1_192] - 1.0).abs() < 0.005, "{}", l[1_192]);
    // 4 ms in f32 is a hair above 192 frames: the frame before takes a trace of the impulse.
    assert!(l[..1_192].iter().all(|s| s.abs() < 1e-4));
    assert_eq!(l, r);
}

#[test]
fn the_triangle_moves_the_delay_at_a_steady_speed() {
    let len = 4_800 + 3 * 48_000;
    let d = vibrato_delay(
        &[(RATE, 1.0), (DEPTH, 100.0), (SHAPE, TRIANGLE)],
        len,
        |_| stopped(),
    );
    let steady = &d[4_800..];
    // Slope of a 1 Hz triangle over 10 ms peak to peak: 2 x 480 frames per second = 0.02
    // frames per frame, away from the turns.
    let slopes: Vec<f64> = steady.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let mut sorted = slopes.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    assert!((median - 0.02).abs() < 0.001, "{median}");
}

// ---------------------------------------------------------------- tempo sync

#[test]
fn synced_on_a_whole_note_at_120_bpm_the_period_is_2_s_on_the_bar() {
    // 3 bars and a bit, playing from tick 0: position = frame x 0.04.
    let len = 3 * 96_000 + 4_800;
    let d = vibrato_delay(
        &[(SYNC, 1.0), (DIVISION, 9.0), (DEPTH, 60.0)],
        len,
        |start| playing_at(120.0, start as f64 * TPF_120),
    );
    let steady = &d[4_800..];
    let (lo, hi) = min_max(steady);
    // The sine rises through its centre at the start of every cycle: at every bar.
    let ups: Vec<f64> = rising(steady, (lo + hi) / 2.0)
        .iter()
        .map(|u| u + 4_800.0)
        .collect();
    println!("rising centre crossings: {ups:?}");
    assert_eq!(ups.len(), 3, "{ups:?}");
    for (k, u) in ups.iter().enumerate() {
        let bar = (k + 1) as f64 * 96_000.0;
        assert!((u - bar).abs() < 1.0, "crossing {u} vs bar {bar}");
    }
    for w in ups.windows(2) {
        assert!(
            (w[1] - w[0] - 96_000.0).abs() < 1.0,
            "period {}",
            w[1] - w[0]
        );
    }
}

#[test]
fn the_synced_phase_follows_the_position_not_the_block_size() {
    let len = 60_000;
    let input = noise(len, 7);
    let params = [
        (SYNC, 1.0),
        (DIVISION, 3.0),
        (DEPTH, 80.0),
        (MODE, ENSEMBLE),
    ];
    let mut renders = Vec::new();
    for size in [1usize, 97, 2_048] {
        let mut b = bloom_with(&params);
        renders.push(run(
            &mut b,
            &input,
            &input,
            size,
            |start| playing_at(120.0, 3_840.0 + start as f64 * TPF_120),
            &[],
        ));
    }
    assert_eq!(renders[0], renders[1]);
    assert_eq!(renders[1], renders[2]);
}

#[test]
fn a_seek_takes_the_new_position_and_glides_there() {
    let len = 48_000;
    let params = [(SYNC, 1.0), (DIVISION, 6.0), (DEPTH, 100.0)];
    // Seek at frame 24,064 from tick 962.56 to tick 1,920 + 480 (a quarter: half a cycle on).
    let seek = 24_064;
    let jumped = vibrato_delay(&params, len, |start| {
        let ticks = if start < seek {
            start as f64 * TPF_120
        } else {
            2_400.0 + (start - seek) as f64 * TPF_120
        };
        playing_at(120.0, ticks)
    });
    // The same frames on a transport that was always there.
    let straight = vibrato_delay(&params, len, |start| {
        playing_at(120.0, 2_400.0 + (start as f64 - seek as f64) * TPF_120)
    });
    // No jump in the delay: it glides at most MAX_SLEW frames per frame.
    let step = jumped
        .windows(2)
        .skip(4_800)
        .fold(0f64, |m, w| m.max((w[1] - w[0]).abs()));
    assert!(step <= bloom::MAX_SLEW + 0.01, "{step}");
    // Once the glide is over, the delay is the one the new position asks for (the measure's
    // constant is the same in both).
    for n in (seek + 4_800..len).step_by(97) {
        assert!(
            (jumped[n] - straight[n]).abs() < 0.01,
            "frame {n}: {} vs {}",
            jumped[n],
            straight[n]
        );
    }
}

#[test]
fn a_loop_on_whole_cycles_keeps_the_movement_continuous() {
    // A loop of one bar (3,840 ticks), whole-note cycles: the wrap moves the position by a
    // whole cycle, so the delay does not glide.
    let bar = 96_000;
    let len = 2 * bar + 10_000;
    let d = vibrato_delay(
        &[(SYNC, 1.0), (DIVISION, 9.0), (DEPTH, 100.0)],
        len,
        |start| playing_at(120.0, (start % bar) as f64 * TPF_120),
    );
    // 256 does not divide 96,000: the host gives the position of each block's first frame,
    // within the loop.
    let step = d
        .windows(2)
        .skip(4_800)
        .fold(0f64, |m, w| m.max((w[1] - w[0]).abs()));
    // Fastest movement of a 0.5 Hz sine over 480 frames: pi x 0.5 x 480 / 48,000 = 0.0157.
    assert!(step < 0.02, "{step}");
}

#[test]
fn while_stopped_a_synced_bloom_keeps_moving_at_the_tempo_rate() {
    let len = 4_800 + 2 * 96_000 + 1_000;
    let d = vibrato_delay(&[(SYNC, 1.0), (DIVISION, 9.0), (DEPTH, 60.0)], len, |_| {
        stopped()
    });
    let steady = &d[4_800..];
    let (lo, hi) = min_max(steady);
    assert!(hi - lo > 0.59 * 480.0, "it moves: {}", hi - lo);
    let ups = rising(steady, (lo + hi) / 2.0);
    for w in ups.windows(2) {
        assert!((w[1] - w[0] - 96_000.0).abs() < 1.0, "{}", w[1] - w[0]);
    }
}

#[test]
fn playback_starts_the_movement_as_a_render_from_that_point_does() {
    // Silence while stopped (the movement runs), then playback from tick 7,680 with a tone:
    // from the start of playback, the output is the one of a Bloom built at that moment.
    for sync in [0.0, 1.0] {
        let params = [
            (SYNC, sync),
            (RATE, 3.0),
            (DIVISION, 6.0),
            (DEPTH, 70.0),
            (MODE, ENSEMBLE),
            (FEEDBACK, 40.0),
        ];
        let wait = 30_080; // a whole number of blocks of 128
        let len = 40_000;
        let tone = pad(len);
        let mut input = vec![0.0; wait];
        input.extend_from_slice(&tone);
        let mut later = bloom_with(&params);
        let (l, r) = run(
            &mut later,
            &input,
            &input,
            128,
            |start| {
                if start < wait {
                    Transport {
                        position_ticks: 7_680.0,
                        ..stopped()
                    }
                } else {
                    playing_at(120.0, 7_680.0 + (start - wait) as f64 * TPF_120)
                }
            },
            &[],
        );
        let mut fresh = bloom_with(&params);
        let (fl, fr) = run(
            &mut fresh,
            &tone,
            &tone,
            128,
            |start| playing_at(120.0, 7_680.0 + start as f64 * TPF_120),
            &[],
        );
        assert_eq!(&l[wait..], &fl[..], "sync {sync}");
        assert_eq!(&r[wait..], &fr[..], "sync {sync}");
    }
}

// ---------------------------------------------------------------- the ensemble and the stereo

/// The delays of the echoes of one impulse: the signed centroid of each group of samples.
fn echoes(window: &[f32], from: usize) -> Vec<f64> {
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for (i, s) in window.iter().enumerate() {
        if s.abs() > 0.05 {
            match groups.last_mut() {
                Some((_, end)) if i <= *end + 3 => *end = i,
                _ => groups.push((i, i)),
            }
        }
    }
    groups
        .iter()
        .map(|(a, b)| {
            let (lo, hi) = (a.saturating_sub(2), (b + 3).min(window.len()));
            let (mut sw, mut swn) = (0.0f64, 0.0f64);
            for (i, s) in window.iter().enumerate().take(hi).skip(lo) {
                sw += f64::from(*s);
                swn += f64::from(*s) * i as f64;
            }
            from as f64 + swn / sw
        })
        .collect()
}

#[test]
fn the_ensemble_reads_three_voices_a_third_of_a_cycle_apart() {
    // An impulse every 50 ms, depth 100 % from 5 ms: each impulse comes back as three echoes.
    // For three equal sines 120 degrees apart, at every instant the deviations from the centre
    // sum to 0 and their squares to 3/2 a^2 (a: half the sweep). Neither holds otherwise. Each
    // echo shows its voice at its own arrival time (up to 8.5 ms apart): at 0.1 Hz the phase
    // moves 0.3 degrees in between, hence the tolerances. 5 s: half a cycle, a whole period of
    // the squares of an unbalanced triple.
    let gap = 2_400;
    let len = 100 * gap;
    let mut x = vec![0.0; len];
    for k in 0..len / gap {
        x[k * gap] = 1.0;
    }
    let mut b = bloom_with(&[
        (MODE, ENSEMBLE),
        (MIX, 1.0),
        (DEPTH, 100.0),
        (DELAY, 0.005),
        (RATE, 0.1),
        (STEREO_PHASE, 0.0),
    ]);
    let (l, r) = run_mono(&mut b, &x);
    assert_eq!(l, r, "stereo phase 0: both channels alike");
    let centre = f64::from(0.005 * SR) + 0.5 * f64::from(MAX_SWEEP_SECONDS * SR);
    let a = 0.5 * f64::from(MAX_SWEEP_SECONDS * SR);
    let mut checked = 0;
    for k in 0..len / gap - 1 {
        let from = k * gap + 200;
        let found = echoes(&l[from..from + 700], 200);
        if found.len() != 3 {
            continue; // two voices too close to tell apart
        }
        let dev: Vec<f64> = found.iter().map(|d| d - centre).collect();
        let sum: f64 = dev.iter().sum();
        let squares: f64 = dev.iter().map(|d| d * d).sum();
        assert!(sum.abs() < 0.5, "impulse {k}: {found:?} sum {sum}");
        assert!(
            (squares / (1.5 * a * a) - 1.0).abs() < 0.01,
            "impulse {k}: {found:?} {squares}"
        );
        checked += 1;
    }
    println!("{checked} impulses with three distinct echoes");
    assert!(checked >= 60, "{checked}");
}

#[test]
fn the_ensemble_with_opposed_channels_is_wide_and_a_chorus_at_0_degrees_is_mono() {
    let len = 2 * 48_000 + 4_800;
    let input = pad(len);
    let corr = |params: &[(u32, f32)]| {
        let mut b = bloom_with(params);
        let (l, r) = run_mono(&mut b, &input);
        check(&l);
        correlation(&l[4_800..], &r[4_800..])
    };
    let wide = corr(&[(MODE, ENSEMBLE), (STEREO_PHASE, 180.0), (MIX, 1.0)]);
    let default_mix = corr(&[(MODE, ENSEMBLE), (STEREO_PHASE, 180.0)]);
    let chorus_90 = corr(&[(MODE, CHORUS), (MIX, 1.0)]);
    let mono = corr(&[(MODE, CHORUS), (STEREO_PHASE, 0.0), (MIX, 1.0)]);
    let dry = correlation(&input[4_800..], &input[4_800..]);
    println!(
        "left/right correlation: ensemble 180 wet {wide:.3}, ensemble 180 mix 0.5 {default_mix:.3}, chorus 90 wet {chorus_90:.3}, chorus 0 {mono:.6}, dry {dry:.3}"
    );
    assert!(wide < 0.5, "{wide}");
    assert!(default_mix < 0.9, "{default_mix}");
    assert!(chorus_90 < 0.9, "{chorus_90}");
    assert!((mono - 1.0).abs() < 1e-9, "{mono}");
}

// ---------------------------------------------------------------- transparency

#[test]
fn a_mix_of_0_is_the_dry_signal_bit_for_bit() {
    let input = noise(20_000, 3);
    for mode in [CHORUS, ENSEMBLE] {
        let mut b = bloom_with(&[
            (MODE, mode),
            (MIX, 0.0),
            (FEEDBACK, 90.0),
            (DEPTH, 100.0),
            (RATE, 10.0),
        ]);
        let other = noise(20_000, 11);
        let (l, r) = run(&mut b, &input, &other, 129, |_| stopped(), &[]);
        assert_eq!(l, input, "mode {mode}");
        assert_eq!(r, other, "mode {mode}");
    }
}

#[test]
fn vibrato_has_no_dry_sound_whatever_the_mix() {
    let mut x = vec![0.0; 3_000];
    x[100] = 1.0;
    for mix in [0.0, 0.5, 1.0] {
        let mut b = bloom_with(&[(MODE, VIBRATO), (MIX, mix), (DEPTH, 0.0), (DELAY, 0.002)]);
        let (l, _) = run_mono(&mut b, &x);
        assert_eq!(l[100], 0.0, "mix {mix}");
        assert!((l[196] - 1.0).abs() < 0.005, "mix {mix}: {}", l[196]);
    }
}

// ---------------------------------------------------------------- no click

#[test]
fn moving_any_setting_mid_tone_never_clicks() {
    // A 220 Hz sine at 0.5: its largest step is 0.0144; a chorus that bends it by up to 30 %
    // stays under 0.02. A click (a jump of the read position or of a gain) is far above.
    let len = 48_000 * 3;
    let input = sine(len, 220.0, 0.5, false);
    let events = [
        param(5_000, RATE, 7.0),
        param(10_000, DEPTH, 100.0),
        param(15_000, DELAY, 0.002),
        param(20_000, SHAPE, TRIANGLE),
        param(25_000, STEREO_PHASE, 180.0),
        param(30_000, MODE, ENSEMBLE),
        param(40_000, MODE, VIBRATO),
        param(50_000, SYNC, 1.0),
        param(60_000, DIVISION, 0.0),
        param(70_000, DIVISION, 9.0),
        param(80_000, MODE, CHORUS),
        param(85_000, FEEDBACK, -90.0),
        param(90_000, TONE, 2_000.0),
        param(95_000, MIX, 1.0),
        param(100_000, DELAY, 0.030),
        param(110_000, SHAPE, 0.0),
        param(115_000, SYNC, 0.0),
        param(120_000, RATE, 0.05),
        param(125_000, DEPTH, 0.0),
    ];
    let mut b = bloom_with(&[]);
    let (l, r) = run(
        &mut b,
        &input,
        &input,
        256,
        |start| playing_at(120.0, start as f64 * TPF_120),
        &events,
    );
    check(&l);
    check(&r);
    let (sl, sr) = (max_step(&l[2_000..]), max_step(&r[2_000..]));
    println!("largest step: left {sl:.4}, right {sr:.4}");
    assert!(sl < 0.03 && sr < 0.03, "{sl} {sr}");
}

// ---------------------------------------------------------------- bounds, NaN, denormals

#[test]
fn every_parameter_at_its_bounds_and_default_stays_finite_and_bounded() {
    let len = 12_000;
    let hot = sine(len, 3_000.0, 1.0, false);
    let input = noise(len, 99);
    for p in DESCRIPTOR.params {
        for value in [p.min, p.max, p.default] {
            for base in [0.0f64, 1.0] {
                // Every other parameter at its min or at its max, this one at `value`.
                let params: Vec<(u32, f32)> = DESCRIPTOR
                    .params
                    .iter()
                    .map(|q| {
                        let v = if q.id == p.id {
                            value
                        } else if base == 0.0 {
                            q.min
                        } else {
                            q.max
                        };
                        (q.id.0, v as f32)
                    })
                    .collect();
                for x in [&input, &hot] {
                    let mut b = bloom_with(&params);
                    let (l, r) = run(
                        &mut b,
                        x,
                        x,
                        333,
                        |s| playing_at(120.0, s as f64 * TPF_120),
                        &[],
                    );
                    check(&l);
                    check(&r);
                    assert!(
                        peak(&l) < 20.0 && peak(&r) < 20.0,
                        "{} = {value}: {} {}",
                        p.key,
                        peak(&l),
                        peak(&r)
                    );
                }
            }
        }
    }
}

#[test]
fn a_hot_feedback_at_full_depth_and_rate_dies_out() {
    for feedback in [90.0f32, -90.0] {
        let mut b = bloom_with(&[
            (FEEDBACK, feedback),
            (DEPTH, 100.0),
            (RATE, 10.0),
            (DELAY, 0.001),
            (MIX, 1.0),
            (MODE, ENSEMBLE),
        ]);
        let mut x = noise(24_000, 5);
        x.extend(vec![0.0; 96_000]);
        let (l, r) = run_mono(&mut b, &x);
        check(&l);
        check(&r);
        let tail = peak(&l[96_000..]).max(peak(&r[96_000..]));
        println!("feedback {feedback}: peak {} tail {tail:e}", peak(&l));
        assert!(peak(&l) < 10.0);
        assert!(tail < 1e-4, "{tail}");
    }
}

#[test]
fn a_broken_input_gives_a_clean_output() {
    let mut x = noise(10_000, 1);
    for (i, bad) in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1e30,
        -1e30,
        1e-30,
        f32::MIN_POSITIVE / 4.0,
    ]
    .into_iter()
    .enumerate()
    {
        x[1_000 + i * 700] = bad;
    }
    let mut b = bloom_with(&[(FEEDBACK, 90.0), (MIX, 1.0)]);
    let (l, r) = run_mono(&mut b, &x);
    check(&l);
    check(&r);
    assert!(peak(&l) <= 3e6, "{}", peak(&l));
    // A long silence after a tiny sound produces no denormal.
    let mut b = bloom_with(&[(FEEDBACK, -90.0)]);
    let mut x = vec![1e-12f32; 2_000];
    x.extend(vec![0.0; 100_000]);
    let (l, r) = run_mono(&mut b, &x);
    check(&l);
    check(&r);
}

// ---------------------------------------------------------------- the device

#[test]
fn reset_empties_the_lines_and_keeps_the_values() {
    let params = [
        (MODE, ENSEMBLE),
        (DEPTH, 70.0),
        (FEEDBACK, 50.0),
        (MIX, 1.0),
    ];
    let input = pad(20_000);
    let mut fresh = bloom_with(&params);
    let reference = run_mono(&mut fresh, &input);
    let mut used = bloom_with(&params);
    run_mono(&mut used, &noise(30_000, 2));
    used.reset();
    assert_eq!(run_mono(&mut used, &input), reference);
}

#[test]
fn unknown_parameters_and_bad_values_are_ignored() {
    let input = noise(5_000, 4);
    let mut b = bloom_with(&[]);
    b.set_param(ParamId(99), 1.0);
    b.set_param(ParamId(DEPTH), f32::NAN);
    b.set_param(ParamId(MIX), f32::INFINITY);
    b.reset();
    let mut plain = bloom_with(&[]);
    assert_eq!(run_mono(&mut b, &input), run_mono(&mut plain, &input));
}

#[test]
fn the_descriptor_is_a_valid_effect_and_its_defaults_are_the_new_device() {
    assert_eq!(DESCRIPTOR.validate(), Ok(()));
    assert_eq!(DESCRIPTOR.role, DeviceRole::Effect);
    assert_eq!((DESCRIPTOR.kind, DESCRIPTOR.name), ("faulk.bloom", "Bloom"));
    assert_eq!(
        builtin(DESCRIPTOR.kind_id).map(|d| d.kind),
        Some("faulk.bloom")
    );
    let keys: Vec<_> = DESCRIPTOR.params.iter().map(|p| p.key).collect();
    assert_eq!(
        keys,
        [
            "mode",
            "sync",
            "rate",
            "division",
            "depth",
            "delay",
            "shape",
            "stereo_phase",
            "feedback",
            "tone",
            "mix"
        ]
    );
    // The note values are the Echo's.
    let division = DESCRIPTOR.param(ParamId(DIVISION)).unwrap();
    let echo = engine::devices::echo::DESCRIPTOR
        .param(ParamId(engine::devices::echo::DIVISION))
        .unwrap();
    assert_eq!(division.values, echo.values);
    let input = noise(5_000, 8);
    let mut fresh = Bloom::new(SR).unwrap();
    let mut set_to_defaults = Bloom::new(SR).unwrap();
    for p in DESCRIPTOR.params {
        set_to_defaults.set_param(p.id, p.default as f32);
    }
    set_to_defaults.reset();
    assert_eq!(
        run_mono(&mut fresh, &input),
        run_mono(&mut set_to_defaults, &input)
    );
}

#[test]
fn the_declared_memory_covers_the_lines_at_the_highest_rate() {
    let b = Bloom::new(MAX_SAMPLE_RATE).unwrap();
    let lines = 2 * ((MAX_DELAY_SECONDS + MAX_SWEEP_SECONDS) * MAX_SAMPLE_RATE) as u64 * 4;
    assert!(DESCRIPTOR.memory_bytes >= lines + size_of_val(&b) as u64);
    // A higher rate than declared does not take more and still runs.
    let device: Box<dyn Device> = (bloom::factory)(&DeviceSetup {
        sample_rate: 384_000.0,
        ..DeviceSetup::default()
    })
    .unwrap();
    assert_eq!(device.descriptor().kind, "faulk.bloom");
}

#[test]
fn the_lines_are_allocated_by_the_factory_and_process_allocates_nothing() {
    let input = noise(2_048, 6);
    for size in [1usize, 33, 2_048] {
        let mut b = (bloom::factory)(&DeviceSetup::default()).unwrap();
        let mut unused = EventBuffer::try_with_capacity(1).unwrap();
        let mut left = input.clone();
        let mut right = input.clone();
        let c = BlockCtx {
            sample_rate: SR,
            frames: size as u32,
            transport: playing_at(100.0, 0.0),
        };
        let events = [Event::ParamValue {
            offset: 0,
            id: ParamId(MODE),
            value: ENSEMBLE,
        }];
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left[..size],
                right: &mut right[..size],
                events_out: &mut unused,
            };
            b.process(&c, &events, &mut io);
            b.set_param(ParamId(DELAY), 0.02);
            b.set_param(ParamId(SYNC), 1.0);
            b.reset();
            let mut io = AudioIo {
                left: &mut left[..size],
                right: &mut right[..size],
                events_out: &mut unused,
            };
            b.process(&c, &[], &mut io);
        });
    }
}

#[test]
fn a_synced_cycle_faster_than_10_hz_keeps_the_delay_under_the_speed_limit() {
    // 1/32 at 120 BPM is 16 Hz, at 999 BPM 133 Hz: the sweep narrows, the cycle stays on the beat.
    for bpm in [120.0, 999.0] {
        let len = 30_000;
        let d = vibrato_delay(
            &[(SYNC, 1.0), (DIVISION, 0.0), (DEPTH, 100.0)],
            len,
            |start| playing_at(bpm, start as f64 * TPF_120 * bpm / 120.0),
        );
        let steady = &d[4_800..];
        let speed = steady
            .windows(2)
            .fold(0f64, |m, w| m.max((w[1] - w[0]).abs()));
        let (lo, hi) = min_max(steady);
        let cycle = 60.0 / bpm * 0.125 * f64::from(SR);
        let expected = (bloom::MAX_SWEEP_SPEED * cycle / core::f64::consts::PI)
            .min(f64::from(MAX_SWEEP_SECONDS * SR));
        println!(
            "{bpm} BPM: speed {speed:.4} frames/frame, sweep {:.2} frames (expected {expected:.2})",
            hi - lo
        );
        assert!(speed < bloom::MAX_SWEEP_SPEED * 1.05, "{bpm}: {speed}");
        assert!(
            (hi - lo - expected).abs() < 0.03 * expected,
            "{bpm}: {} vs {expected}",
            hi - lo
        );
        let ups = rising(steady, (lo + hi) / 2.0);
        assert!(ups.len() > 3, "{ups:?}");
        for w in ups.windows(2) {
            assert!((w[1] - w[0] - cycle).abs() < 1.0, "{bpm}: {}", w[1] - w[0]);
        }
    }
}

#[test]
fn a_mix_ramped_to_0_lands_on_the_dry_signal_exactly() {
    // The gains follow the ramp to its target: after the 20 ms ramp, the input bit for bit.
    let input = noise(10_000, 21);
    let mut b = bloom_with(&[(MIX, 0.5), (FEEDBACK, 50.0)]);
    let (l, _) = run(
        &mut b,
        &input,
        &input,
        64,
        |_| stopped(),
        &[param(1_000, MIX, 0.0)],
    );
    assert_eq!(&l[1_000 + 960..], &input[1_000 + 960..]);
    assert_ne!(
        &l[1_000 + 900..1_000 + 959],
        &input[1_000 + 900..1_000 + 959]
    );
}

#[test]
fn playback_started_while_a_sound_rings_glides_instead_of_clicking() {
    // A 220 Hz tone through a deep chorus, stopped, then playback starts: the free phase starts
    // again at 0 but the voices glide there (the lines hold sound), so the output does not step.
    // At 2.5 Hz the phase is a quarter cycle on when playback starts: a jump of 240 frames.
    let len = 48_000;
    let input = sine(len, 220.0, 0.5, false);
    let start = 24_064;
    let mut b = bloom_with(&[(RATE, 2.5), (DEPTH, 100.0), (MIX, 1.0)]);
    let (l, r) = run(
        &mut b,
        &input,
        &input,
        128,
        |s| {
            if s < start {
                stopped()
            } else {
                playing_at(120.0, (s - start) as f64 * TPF_120)
            }
        },
        &[],
    );
    let step = max_step(&l[2_000..]).max(max_step(&r[2_000..]));
    // Sine at 220 Hz bent by at most 50 % while it glides: under 0.025.
    assert!(step < 0.025, "{step}");
}

#[test]
fn a_tempo_change_and_a_stop_keep_a_synced_movement_continuous() {
    // Playing at 120 BPM, then 90 BPM from frame 48,000 (the position goes on from where it
    // was), then stopped from frame 96,000 (the LFO goes on from its phase at the tempo's rate).
    let len = 150_000;
    let (change, stop) = (48_128, 96_000);
    let at_change = change as f64 * TPF_120;
    let tpf_90 = TPF_120 * 0.75;
    let d = vibrato_delay(
        &[(SYNC, 1.0), (DIVISION, 6.0), (DEPTH, 100.0)],
        len,
        |start| {
            if start < change {
                playing_at(120.0, start as f64 * TPF_120)
            } else if start < stop {
                playing_at(90.0, at_change + (start - change) as f64 * tpf_90)
            } else {
                Transport {
                    tempo: Tempo::new(90.0).unwrap(),
                    position_ticks: at_change + (stop - change) as f64 * tpf_90,
                    ..Transport::default()
                }
            }
        },
    );
    // Fastest legitimate movement: a quarter at 120 BPM (2 Hz) over 480 frames, pi x 2 x 480 /
    // 48,000 = 0.063 frames per frame; a jump would be glided at 0.5.
    let step = d
        .windows(2)
        .skip(4_800)
        .fold(0f64, |m, w| m.max((w[1] - w[0]).abs()));
    assert!(step < 0.07, "{step}");
    // After the stop it keeps moving.
    let (lo, hi) = min_max(&d[stop + 10_000..]);
    assert!(hi - lo > 400.0, "{}", hi - lo);
}

#[test]
fn above_the_declared_rate_the_delays_stay_within_the_lines() {
    // At 384 kHz the lines hold 20 ms: a 30 ms delay and a full sweep are kept within them.
    let rate = 384_000.0;
    let mut b = Bloom::new(rate).unwrap();
    for (id, v) in [
        (DELAY, 0.03),
        (DEPTH, 100.0),
        (MIX, 1.0),
        (MODE, ENSEMBLE),
        (FEEDBACK, 90.0),
    ] {
        b.set_param(ParamId(id), v);
    }
    b.reset();
    let input = noise(40_000, 12);
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let (mut l, mut r) = (input.clone(), input.clone());
    for (cl, cr) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
        let c = BlockCtx {
            sample_rate: rate,
            frames: cl.len() as u32,
            transport: stopped(),
        };
        let mut io = AudioIo {
            left: cl,
            right: cr,
            events_out: &mut unused,
        };
        b.process(&c, &[], &mut io);
    }
    check(&l);
    check(&r);
    assert!(peak(&l) < 20.0);
}
