//! The Filter's movement (#686): the notch, the LFO (free and synced to the transport), the
//! envelope follower, the drive and their combination with the cutoff, through the device
//! contract, every `process` under the allocation detector. The cutoff's trajectory is read
//! frame by frame from `Filter::effective_cutoffs` (blocks of one frame; the render does not
//! depend on the block size, which a test below checks bit for bit).

#[path = "reference/movement_filter.rs"]
mod movement_filter;

use assert_no_alloc::assert_no_alloc;
use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::filter::{
    CUTOFF, CUTOFF_NYQUIST_SHARE, DESCRIPTOR, DRIVE, ENV_AMOUNT, ENV_ATTACK, ENV_RELEASE, Filter,
    LFO_DEPTH, LFO_DIVISION, LFO_PHASE, LFO_RATE, LFO_RESTART, LFO_SHAPE, LFO_STEREO, LFO_SYNC,
    LOW_PASS, MIX, MODE, NOTCH, RESONANCE,
};
use engine::dsp::lfo::{self, DIVISION_KEYS, SHAPE_KEYS, Shape};
use engine::msg::ParamId;
use engine::time::{Tempo, TimeSignature};

const SR: f32 = 48_000.0;
/// Ticks per frame at 120 BPM and 48 kHz.
const TPF: f64 = 1920.0 / 48_000.0;

fn shape(key: &str) -> f32 {
    SHAPE_KEYS.iter().position(|k| *k == key).unwrap() as f32
}

fn division(key: &str) -> f32 {
    DIVISION_KEYS.iter().position(|k| *k == key).unwrap() as f32
}

fn with(params: &[(u32, f32)]) -> Filter {
    let mut f = Filter::new(SR);
    for (id, value) in params {
        f.set_param(ParamId(*id), *value);
    }
    f.reset();
    f
}

fn param(id: u32, value: f32) -> Event {
    Event::ParamValue {
        offset: 0,
        id: ParamId(id),
        value,
    }
}

fn transport(playing: bool, bpm: f64, position: f64) -> Transport {
    Transport {
        playing,
        tempo: Tempo::new(bpm).unwrap(),
        signature: TimeSignature::COMMON,
        position_ticks: position,
        ..Transport::default()
    }
}

/// Renders `input` in blocks of `size` frames; `at(frame)` is the transport of the block
/// starting at `frame`; `events` are `(frame, event)`. With `trace`, the effective cutoffs
/// after each block (blocks of one frame give one per frame).
fn render(
    f: &mut Filter,
    input: (&[f32], &[f32]),
    size: usize,
    events: &[(usize, Event)],
    at: &dyn Fn(usize) -> Transport,
    mut trace: Option<&mut Vec<[f32; 2]>>,
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
            .filter(|(e, _)| (start..end).contains(e))
            .map(|(e, ev)| ev.at((e - start) as u32))
            .collect();
        let ctx = BlockCtx {
            sample_rate: SR,
            frames: (end - start) as u32,
            transport: at(start),
        };
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left,
                right: &mut right,
                events_out: &mut unused,
            };
            f.process(&ctx, &local, &mut io);
        });
        if let Some(t) = trace.as_deref_mut() {
            t.push(f.effective_cutoffs());
        }
        out_l.extend(left);
        out_r.extend(right);
        start = end;
    }
    (out_l, out_r)
}

/// Playing from tick 0 at 120 BPM, positions as the host gives them.
fn playing(frame: usize) -> Transport {
    transport(true, 120.0, frame as f64 * TPF)
}

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

fn sine(len: usize, hz: f32, amplitude: f32) -> Vec<f32> {
    (0..len)
        .map(|n| amplitude * (core::f32::consts::TAU * hz * n as f32 / SR).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

fn check(x: &[f32]) {
    for (i, s) in x.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is {s}");
        assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
    }
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// The movement of a trace, in octaves from `centre`.
fn octaves(trace: &[[f32; 2]], channel: usize, centre: f32) -> Vec<f32> {
    trace.iter().map(|c| (c[channel] / centre).log2()).collect()
}

// ---------------------------------------------------------------- the notch

#[test]
fn the_notch_digs_a_hole_at_the_cutoff_and_lets_the_rest_through() {
    let gain = |hz: f32| {
        let mut f = with(&[(MODE, NOTCH as f32), (CUTOFF, 1000.0)]);
        let input = sine(SR as usize, hz, 0.25);
        let out = render(&mut f, (&input, &input), 128, &[], &playing, None).0;
        let half = out.len() / 2;
        20.0 * (rms(&out[half..]) / rms(&input[half..])).log10()
    };
    let at = gain(1000.0);
    assert!(at <= -30.0, "at the cutoff: {at} dB");
    for hz in [100.0, 8000.0] {
        let g = gain(hz);
        assert!(g.abs() < 0.5, "{hz} Hz: {g} dB");
    }
}

// ---------------------------------------------------------------- the synced LFO

/// The triangle at a position, as the module doc defines it.
fn triangle(position: f64, period: f64, phase: f64) -> f32 {
    lfo::value(Shape::Triangle, position / period + phase)
}

#[test]
fn a_synced_quarter_triangle_at_120_bpm_has_a_period_of_half_a_second_to_the_frame() {
    let mut f = with(&[
        (LFO_SYNC, 1.0),
        (LFO_SHAPE, shape("triangle")),
        (LFO_DIVISION, division("quarter")),
        (LFO_DEPTH, 1.5),
        (CUTOFF, 1000.0),
    ]);
    let input = vec![0.0; 4 * 24_000 + 1];
    let mut trace = Vec::new();
    render(&mut f, (&input, &input), 1, &[], &playing, Some(&mut trace));
    let m = octaves(&trace, 0, 1000.0);
    // The peaks of each cycle, frame by frame.
    let peaks: Vec<usize> = (0..4)
        .map(|c| {
            let range = c * 24_000..(c + 1) * 24_000;
            range
                .clone()
                .max_by(|a, b| m[*a].total_cmp(&m[*b]))
                .unwrap()
        })
        .collect();
    assert_eq!(peaks[0], 6_000, "a quarter of the cycle after the beat");
    for w in peaks.windows(2) {
        assert_eq!(w[1] - w[0], 24_000, "{peaks:?}");
    }
    assert!((m[6_000] - 1.5).abs() < 1e-4, "{}", m[6_000]);
    assert!((m[18_000] + 1.5).abs() < 1e-4, "{}", m[18_000]);
}

#[test]
fn the_synced_lfo_sits_on_the_transport_position_through_loops_seeks_and_tempo_changes() {
    // The position the host gives each block: one bar at 120 BPM looped twice, a seek to
    // tick 5000, then 90 BPM from there.
    let bar = 3840.0;
    let loop_frames = (bar / TPF) as usize; // 96 000
    let position = |frame: usize| -> (f64, f64) {
        if frame < 2 * loop_frames {
            ((frame % loop_frames) as f64 * TPF, 120.0)
        } else if frame < 2 * loop_frames + 20_000 {
            (5000.0 + (frame - 2 * loop_frames) as f64 * TPF, 120.0)
        } else {
            let tpf90 = 90.0 * 960.0 / 60.0 / 48_000.0;
            (
                5000.0 + 20_000.0 * TPF + (frame - 2 * loop_frames - 20_000) as f64 * tpf90,
                90.0,
            )
        }
    };
    let at = |frame: usize| {
        let (p, bpm) = position(frame);
        transport(true, bpm, p)
    };
    let mut f = with(&[
        (LFO_SYNC, 1.0),
        (LFO_SHAPE, shape("triangle")),
        (LFO_DIVISION, division("quarter")),
        (LFO_PHASE, 90.0),
        (LFO_DEPTH, 1.0),
        (CUTOFF, 1000.0),
    ]);
    let total = 2 * loop_frames + 120_000;
    let input = vec![0.0; total];
    let mut trace = Vec::new();
    render(&mut f, (&input, &input), 1, &[], &at, Some(&mut trace));
    let m = octaves(&trace, 0, 1000.0);
    // Every frame is the triangle at its position (after a jump, once the slew limit has
    // caught up: 2 octaves per ms, so within 2 ms).
    let catch_up = 96;
    let jumps = [loop_frames, 2 * loop_frames];
    for (frame, value) in m.iter().enumerate() {
        if jumps.iter().any(|j| (*j..*j + catch_up).contains(&frame)) {
            continue;
        }
        let expected = triangle(position(frame).0, 960.0, 0.25);
        assert!(
            (value - expected).abs() < 1e-3,
            "frame {frame}: {value} vs {expected}"
        );
    }
    // The same place of the bar on each pass of the loop.
    for k in 0..loop_frames {
        assert_eq!(trace[k], trace[k + loop_frames], "frame {k} of the loop");
    }
    // At 90 BPM a cycle is still a quarter note: the peaks fall on the beats (phase 90 degrees).
    let from = 2 * loop_frames + 20_000;
    let beat90 = 32_000; // frames of a quarter note at 90 BPM
    let first = (from..from + beat90)
        .max_by(|a, b| m[*a].total_cmp(&m[*b]))
        .unwrap();
    let ticks = position(first).0;
    let off_beat = ticks - (ticks / 960.0).round() * 960.0;
    assert!(off_beat.abs() < 0.1, "peak at tick {ticks}");
    let second = (first + beat90 / 2..first + 3 * beat90 / 2)
        .max_by(|a, b| m[*a].total_cmp(&m[*b]))
        .unwrap();
    assert_eq!(second - first, beat90);
}

#[test]
fn stopped_the_synced_lfo_holds_its_value_at_the_position() {
    let mut f = with(&[(LFO_SYNC, 1.0), (LFO_DEPTH, 2.0), (CUTOFF, 1000.0)]);
    let input = vec![0.0; 10_000];
    let mut trace = Vec::new();
    render(
        &mut f,
        (&input, &input),
        64,
        &[],
        &|_| transport(false, 120.0, 300.0),
        Some(&mut trace),
    );
    let expected = 1000.0 * 2f32.powf(2.0 * triangle(300.0, 960.0, 0.0));
    for c in &trace {
        assert!((c[0] - expected).abs() < 0.05, "{c:?} vs {expected}");
    }
}

// ---------------------------------------------------------------- the free LFO

#[test]
fn a_free_lfo_with_restart_renders_the_same_from_every_start_and_without_it_runs_on() {
    let play_frames = 48_000;
    let input = noise(play_frames, 3);
    // Silence while stopped (a different length each time), then the same sound from the start.
    let run = |restart: f32, stopped: usize| {
        let mut f = with(&[
            (LFO_SYNC, 0.0),
            (LFO_RATE, 2.0),
            (LFO_SHAPE, shape("square")),
            (LFO_DEPTH, 2.0),
            (LFO_RESTART, restart),
            (CUTOFF, 800.0),
        ]);
        let mut l = vec![0.0; stopped];
        l.extend_from_slice(&input);
        let at = move |frame: usize| {
            if frame < stopped {
                transport(false, 120.0, 0.0)
            } else {
                transport(true, 120.0, (frame - stopped) as f64 * TPF)
            }
        };
        let mut trace = Vec::new();
        let (out, _) = render(&mut f, (&l, &l), 1, &[], &at, Some(&mut trace));
        (out[stopped..].to_vec(), trace[stopped..].to_vec())
    };
    let (a, ta) = run(1.0, 7_001);
    let (b, tb) = run(1.0, 19_733);
    assert_eq!(a, b, "restart on: the same render from each start");
    assert_eq!(ta, tb);
    // A period of 0.5 s: the square is high for the first quarter second from the start.
    let m = octaves(&ta, 0, 800.0);
    assert!((m[10] - 2.0).abs() < 1e-4 && (m[11_990] - 2.0).abs() < 1e-4);
    assert!((m[12_200] + 2.0).abs() < 1e-4, "{}", m[12_200]);
    let (c, _) = run(0.0, 7_001);
    let (d, _) = run(0.0, 19_733);
    assert_ne!(c, d, "restart off: the phase runs on through the stop");
}

#[test]
fn the_free_lfo_runs_at_its_rate() {
    let mut f = with(&[
        (LFO_SYNC, 0.0),
        (LFO_RATE, 4.0),
        (LFO_SHAPE, shape("saw_up")),
        (LFO_DEPTH, 1.0),
    ]);
    let input = vec![0.0; 48_000];
    let mut trace = Vec::new();
    render(
        &mut f,
        (&input, &input),
        1,
        &[],
        &|_| transport(false, 120.0, 0.0),
        Some(&mut trace),
    );
    let m = octaves(&trace, 0, 1000.0);
    // The saw drops once per cycle (the slew limit turns the drop into a 1 ms ramp).
    let drops: Vec<usize> = (1..m.len()).filter(|i| m[*i] < m[i - 1] - 0.01).collect();
    let starts: Vec<usize> = drops
        .iter()
        .copied()
        .filter(|i| !drops.contains(&(i - 1)))
        .collect();
    assert_eq!(starts.len(), 3, "{starts:?}");
    for w in starts.windows(2) {
        assert_eq!(w[1] - w[0], 12_000);
    }
}

// ---------------------------------------------------------------- stereo, random

#[test]
fn a_stereo_phase_of_180_degrees_opens_the_channels_in_opposition() {
    let mut f = with(&[
        (LFO_SYNC, 1.0),
        (LFO_SHAPE, shape("sine")),
        (LFO_DIVISION, division("half")),
        (LFO_STEREO, 180.0),
        (LFO_DEPTH, 2.0),
        (CUTOFF, 1000.0),
    ]);
    let n = 96_000;
    let input = noise(n, 9);
    let mut trace = Vec::new();
    let (l, r) = render(&mut f, (&input, &input), 1, &[], &playing, Some(&mut trace));
    let (ml, mr) = (octaves(&trace, 0, 1000.0), octaves(&trace, 1, 1000.0));
    for i in 0..n {
        assert!(
            (ml[i] + mr[i]).abs() < 1e-3,
            "frame {i}: {} {}",
            ml[i],
            mr[i]
        );
    }
    // Measured on the noise: when the left channel is bright the right one is dull. The rms of
    // each 20 ms window through the low-pass follows its cutoff.
    let w = 960;
    let (el, er): (Vec<f32>, Vec<f32>) = (0..n / w)
        .map(|k| (rms(&l[k * w..(k + 1) * w]), rms(&r[k * w..(k + 1) * w])))
        .unzip();
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let (al, ar) = (mean(&el), mean(&er));
    let cov: f32 = el.iter().zip(&er).map(|(a, b)| (a - al) * (b - ar)).sum();
    let var = |v: &[f32], m: f32| v.iter().map(|a| (a - m) * (a - m)).sum::<f32>();
    let corr = cov / (var(&el, al) * var(&er, ar)).sqrt();
    assert!(
        corr < -0.8,
        "correlation of the two channels' levels: {corr}"
    );
}

#[test]
fn the_random_shape_holds_a_step_per_eighth_and_repeats_in_every_render() {
    let run = || {
        let mut f = with(&[
            (LFO_SYNC, 1.0),
            (LFO_SHAPE, shape("random")),
            (LFO_DIVISION, division("eighth")),
            (LFO_DEPTH, 2.0),
        ]);
        let input = noise(96_000, 11);
        let mut trace = Vec::new();
        let out = render(&mut f, (&input, &input), 1, &[], &playing, Some(&mut trace));
        (out, trace)
    };
    let (a, ta) = run();
    let (b, tb) = run();
    assert_eq!(a, b);
    assert_eq!(ta, tb);
    // An eighth at 120 BPM is 12 000 frames: one value from 2 ms after its start to its end.
    let m = octaves(&ta, 0, 1000.0);
    let steps: Vec<f32> = (0..8)
        .map(|k| {
            let from = k * 12_000 + 96;
            let v = m[from];
            for (i, x) in m[from..(k + 1) * 12_000].iter().enumerate() {
                assert!((x - v).abs() < 1e-5, "step {k}, frame {}", from + i);
            }
            v
        })
        .collect();
    for (k, v) in steps.iter().enumerate() {
        let expected = 2.0 * lfo::random(k as i64);
        assert!((v - expected).abs() < 1e-4, "step {k}: {v} vs {expected}");
    }
    assert!(
        steps.windows(2).any(|w| (w[0] - w[1]).abs() > 0.1),
        "{steps:?}"
    );
}

// ---------------------------------------------------------------- the envelope follower

#[test]
fn the_envelope_follower_opens_the_cutoff_with_the_set_attack_and_release() {
    // Binary noise in bursts: random signs at a constant magnitude, so the rectified level is
    // exactly the burst's level (0.5) and the follower's curve is the textbook one.
    let level = 0.5;
    let on = 24_000;
    let off = 48_000;
    let signs = noise(on, 17);
    let mut input = Vec::new();
    for _ in 0..2 {
        input.extend(signs.iter().map(|s| if *s >= 0.0 { level } else { -level }));
        input.extend(std::iter::repeat_n(0.0, off));
    }
    let mut f = with(&[
        (ENV_AMOUNT, 3.0),
        (ENV_ATTACK, 0.005),
        (ENV_RELEASE, 0.15),
        (CUTOFF, 300.0),
    ]);
    let mut trace = Vec::new();
    render(
        &mut f,
        (&input, &input),
        1,
        &[],
        &|_| transport(false, 120.0, 0.0),
        Some(&mut trace),
    );
    let env: Vec<f32> = octaves(&trace, 0, 300.0).iter().map(|o| o / 3.0).collect();
    let block = 128;
    for burst in 0..2 {
        let start = burst * (on + off);
        let rise = (start..start + on)
            .find(|i| env[*i] >= 0.632 * level)
            .unwrap()
            - start;
        assert!(
            rise.abs_diff(240) <= block,
            "attack: {rise} frames, 240 expected"
        );
        assert!((env[start + on - 1] - level).abs() < 1e-3);
        let end = start + on;
        let fall = (end..end + off).find(|i| env[*i] <= 0.368 * level).unwrap() - end;
        assert!(
            fall.abs_diff(7_200) <= block,
            "release: {fall} frames, 7200 expected"
        );
    }
    // A negative amount closes the filter as the sound gets louder.
    let mut g = with(&[(ENV_AMOUNT, -2.0), (CUTOFF, 4000.0)]);
    let mut t = Vec::new();
    render(
        &mut g,
        (&input, &input),
        1,
        &[],
        &|_| transport(false, 120.0, 0.0),
        Some(&mut t),
    );
    assert!(t[on - 1][0] < 4000.0 * 2f32.powf(-0.9), "{:?}", t[on - 1]);
}

// ---------------------------------------------------------------- the combination

#[test]
fn a_cutoff_lane_moves_the_centre_and_the_lfo_swings_around_it() {
    // A lane from 200 Hz to 8 kHz over 4 bars (8 s at 120 BPM), sent as the automation does:
    // a value every 128 frames, each ramped by the parameter's smoothing; a one-octave
    // triangle in quarters on top.
    let frames = 8 * 48_000;
    let events: Vec<(usize, Event)> = (0..frames / 128)
        .map(|k| {
            let t = (k * 128) as f32 / frames as f32;
            (k * 128, param(CUTOFF, 200.0 * 40f32.powf(t)))
        })
        .collect();
    let mut f = with(&[
        (CUTOFF, 200.0),
        (LFO_SYNC, 1.0),
        (LFO_SHAPE, shape("triangle")),
        (LFO_DIVISION, division("quarter")),
        (LFO_DEPTH, 1.0),
    ]);
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let mut checked = 0;
    // The centre as the test computes it from the lane: each value is reached by the
    // contract's 20 ms linear ramp (960 frames) from where the centre was.
    let ramp = 960.0f64;
    let (mut from, mut to, mut since) = (200.0f64, 200.0f64, ramp);
    for frame in 0..frames {
        // The lane's value lands at offset 0 of the frame.
        let local: Vec<Event> = events
            .iter()
            .filter(|(e, _)| *e == frame)
            .map(|(_, e)| *e)
            .collect();
        for e in &local {
            if let Event::ParamValue { value, .. } = *e {
                from += (to - from) * (since / ramp).min(1.0);
                to = f64::from(value);
                since = 0.0;
            }
        }
        let centre = from + (to - from) * (since / ramp).min(1.0);
        since += 1.0;
        let ctx = BlockCtx {
            sample_rate: SR,
            frames: 1,
            transport: playing(frame),
        };
        let (mut l, mut r) = ([0.0f32], [0.0f32]);
        assert_no_alloc(|| {
            f.process(
                &ctx,
                &local,
                &mut AudioIo {
                    left: &mut l,
                    right: &mut r,
                    events_out: &mut unused,
                },
            )
        });
        let expected = (centre * 2f64.powf(f64::from(triangle(frame as f64 * TPF, 960.0, 0.0))))
            .clamp(20.0, f64::from(CUTOFF_NYQUIST_SHARE * SR)) as f32;
        let got = f.effective_cutoffs()[0];
        assert!(
            (got / expected - 1.0).abs() < 1e-4,
            "frame {frame}: {got} vs {expected} (centre {centre})"
        );
        checked += 1;
    }
    assert_eq!(checked, frames);
}

#[test]
fn the_effective_cutoff_stays_within_20_hz_and_0_45_of_the_sample_rate() {
    for (centre, amount, depth, top) in [
        (15_000.0, 5.0, 5.0, true),
        (30.0, -5.0, 5.0, false),
        (20_000.0, 0.0, 5.0, true),
    ] {
        let mut f = with(&[
            (CUTOFF, centre),
            (ENV_AMOUNT, amount),
            (ENV_ATTACK, 0.001),
            (LFO_SYNC, 0.0),
            (LFO_RATE, 20.0),
            (LFO_SHAPE, shape("square")),
            (LFO_DEPTH, depth),
        ]);
        let input: Vec<f32> = (0..48_000)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let mut trace = Vec::new();
        let (l, _) = render(&mut f, (&input, &input), 1, &[], &playing, Some(&mut trace));
        check(&l);
        let (lo, hi) = trace.iter().fold((f32::MAX, 0f32), |(lo, hi), c| {
            (lo.min(c[0]).min(c[1]), hi.max(c[0]).max(c[1]))
        });
        assert!(lo >= 20.0 && hi <= CUTOFF_NYQUIST_SHARE * SR, "{lo} {hi}");
        if top {
            assert_eq!(hi, CUTOFF_NYQUIST_SHARE * SR);
        } else {
            assert_eq!(lo, 20.0);
        }
    }
}

// ---------------------------------------------------------------- drive and bounds

#[test]
fn full_drive_and_full_resonance_on_a_full_scale_square_stay_bounded_for_60_seconds() {
    let n = 60 * SR as usize;
    let input: Vec<f32> = (0..n)
        .map(|i| if (i / 240) % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    for moving in [false, true] {
        let mut params = vec![(DRIVE, 1.0), (RESONANCE, 1.0), (CUTOFF, 100.0)];
        if moving {
            params.extend([
                (LFO_DEPTH, 5.0),
                (LFO_SHAPE, shape("random")),
                (LFO_DIVISION, division("sixteenth")),
                (ENV_AMOUNT, 5.0),
            ]);
        }
        let mut f = with(&params);
        let (l, r) = render(&mut f, (&input, &input), 512, &[], &playing, None);
        check(&l);
        check(&r);
        let p = peak(&l).max(peak(&r));
        assert!(p < 36.0 * 0.26, "moving {moving}: peak {p}");
    }
}

#[test]
fn drive_saturates_loud_sound_lifts_quiet_sound_and_is_exactly_off_at_zero() {
    let measure = |drive: f32, amplitude: f32| {
        let mut f = with(&[(DRIVE, drive), (CUTOFF, 20_000.0), (RESONANCE, 0.0)]);
        let input = sine(48_000, 200.0, amplitude);
        let out = render(&mut f, (&input, &input), 128, &[], &playing, None).0;
        (rms(&out[24_000..]) / rms(&input[24_000..]), out)
    };
    let (quiet, _) = measure(1.0, 0.001);
    assert!(
        (20.0 * quiet.log10() - 12.3).abs() < 0.5,
        "{}",
        20.0 * quiet.log10()
    );
    let (loud, out) = measure(1.0, 1.0);
    assert!(loud < 0.4, "{loud}");
    assert!(peak(&out[24_000..]) < 0.27);
    // At drive 0 the stage is skipped: the same as a filter that never had a drive.
    let (_, a) = measure(0.0, 0.7);
    let mut b = with(&[(CUTOFF, 20_000.0), (RESONANCE, 0.0)]);
    let input = sine(48_000, 200.0, 0.7);
    let reference = render(&mut b, (&input, &input), 128, &[], &playing, None).0;
    assert_eq!(a, reference);
    // A vanishing drive is none too (its gain would overflow).
    let (_, tiny) = measure(1e-30, 0.7);
    assert_eq!(tiny, reference);
}

// ---------------------------------------------------------------- no clicks, blocks

#[test]
fn jumps_of_the_movement_parameters_do_not_click() {
    // A 150 Hz sine through a low-pass at 2 kHz: each parameter of the movement jumps at once.
    let input = sine(96_000, 150.0, 0.5);
    let own = 0.5 * core::f32::consts::TAU * 150.0 / SR;
    let base = [
        (CUTOFF, 2000.0),
        (RESONANCE, 0.3),
        (LFO_SYNC, 1.0),
        (LFO_DIVISION, division("quarter")),
        (LFO_SHAPE, shape("sine")),
        (LFO_DEPTH, 0.5),
    ];
    let jumps = [
        (LFO_PHASE, 180.0),
        (LFO_STEREO, 180.0),
        (LFO_DIVISION, division("thirty_second")),
        (LFO_SHAPE, shape("square")),
        (LFO_DEPTH, 3.0),
        (ENV_AMOUNT, 4.0),
        (DRIVE, 1.0),
        (LFO_SYNC, 0.0),
        (MODE, NOTCH as f32),
    ];
    for (id, value) in jumps {
        let mut f = with(&base);
        let at = 30_000;
        let (l, r) = render(
            &mut f,
            (&input, &input),
            128,
            &[(at, param(id, value))],
            &playing,
            None,
        );
        let step = max_step(&l[at - 200..at + 4_000]).max(max_step(&r[at - 200..at + 4_000]));
        let limit = if id == DRIVE { own * 6.0 } else { own * 3.0 };
        assert!(
            step < limit,
            "parameter {id} -> {value}: step {step} (own {own})"
        );
    }
    // A square LFO's edges are ramps too.
    let mut f = with(&[
        (CUTOFF, 2000.0),
        (LFO_SHAPE, shape("square")),
        (LFO_DIVISION, division("sixteenth")),
        (LFO_DEPTH, 2.0),
    ]);
    let (l, _) = render(&mut f, (&input, &input), 128, &[], &playing, None);
    assert!(
        max_step(&l[1_000..]) < own * 3.0,
        "{}",
        max_step(&l[1_000..])
    );
}

#[test]
fn with_movement_the_render_does_not_depend_on_the_block_size() {
    let input = noise(60_000, 23);
    let events = [
        (0, param(LFO_DEPTH, 2.0)),
        (0, param(LFO_STEREO, 90.0)),
        (0, param(ENV_AMOUNT, 1.5)),
        (0, param(DRIVE, 0.4)),
        (0, param(RESONANCE, 0.8)),
        (7_000, param(LFO_SHAPE, shape("random"))),
        (13_013, param(LFO_SYNC, 0.0)),
        (13_013, param(LFO_RATE, 7.0)),
        (21_000, param(CUTOFF, 3000.0)),
        (30_000, param(MODE, NOTCH as f32)),
        (41_111, param(LFO_SYNC, 1.0)),
        (41_111, param(LFO_DIVISION, division("eighth_triplet"))),
        (50_000, param(MIX, 0.6)),
    ];
    let reference = render(
        &mut with(&[]),
        (&input, &input),
        128,
        &events,
        &playing,
        None,
    );
    for size in [33, 1024, 1, 4096] {
        let out = render(
            &mut with(&[]),
            (&input, &input),
            size,
            &events,
            &playing,
            None,
        );
        assert_eq!(out, reference, "block size {size}");
    }
}

#[test]
fn reset_sets_the_lfo_and_the_envelope_back_to_their_start() {
    let params = [
        (LFO_SYNC, 0.0),
        (LFO_RATE, 3.0),
        (LFO_SHAPE, shape("random")),
        (LFO_RESTART, 0.0),
        (LFO_DEPTH, 2.0),
        (ENV_AMOUNT, 2.0),
    ];
    let input = noise(30_000, 31);
    let still = |_: usize| transport(false, 120.0, 0.0);
    let fresh = render(&mut with(&params), (&input, &input), 128, &[], &still, None);
    let mut f = with(&params);
    render(&mut f, (&input, &input), 128, &[], &still, None);
    f.reset();
    let again = render(&mut f, (&input, &input), 128, &[], &still, None);
    assert_eq!(again, fresh);
}

#[test]
fn the_readout_is_the_left_channels_effective_cutoff() {
    let mut f = with(&[(LFO_DEPTH, 1.0), (LFO_STEREO, 90.0)]);
    let input = vec![0.0; 7_000];
    render(&mut f, (&input, &input), 128, &[], &playing, None);
    let (id, value) = f.readout().unwrap();
    assert_eq!(id, ParamId(CUTOFF));
    assert_eq!(value, f.effective_cutoffs()[0]);
    assert_ne!(f.effective_cutoffs()[0], f.effective_cutoffs()[1]);
    // Without movement it is the centre.
    let mut g = with(&[(CUTOFF, 440.0)]);
    render(&mut g, (&input, &input), 128, &[], &playing, None);
    assert_eq!(g.readout(), Some((ParamId(CUTOFF), 440.0)));
}

#[test]
fn the_descriptor_lists_the_movement_with_neutral_defaults() {
    let p = |id: u32| DESCRIPTOR.param(ParamId(id)).unwrap();
    assert_eq!(p(DRIVE).default, 0.0);
    assert_eq!(p(LFO_DEPTH).default, 0.0);
    assert_eq!(p(ENV_AMOUNT).default, 0.0);
    assert_eq!(p(LFO_SHAPE).values, SHAPE_KEYS);
    assert_eq!(p(LFO_DIVISION).values, DIVISION_KEYS);
    assert_eq!(p(LFO_DIVISION).values.len(), 13);
    assert_eq!(p(MODE).values[NOTCH as usize], "notch");
    assert_eq!(p(MODE).values[LOW_PASS as usize], "lowpass");
    for d in DESCRIPTOR.params {
        if !d.values.is_empty() {
            assert_eq!(d.max, (d.values.len() - 1) as f64, "{}", d.key);
            assert_eq!(d.step, 1.0, "{}", d.key);
        }
        assert!(d.min <= d.default && d.default <= d.max, "{}", d.key);
    }
    for id in [LFO_RATE, ENV_ATTACK, ENV_RELEASE] {
        assert!(p(id).min > 0.0);
    }
}

/// The bits of a scripted render of `f` (both channels, then the readout after each block):
/// every movement parameter moves, synced and free, and the transport plays, stops, seeks and
/// changes tempo.
fn scripted(f: &mut dyn Device, sr: f32, size: usize) -> Vec<u32> {
    let len = (sr * 3.0) as usize;
    let input_l = noise(len, 7);
    let input_r = noise(len, 11);
    f.set_param(ParamId(RESONANCE), 0.6);
    f.set_param(ParamId(CUTOFF), 900.0);
    f.reset();
    let tpf = 1920.0 / f64::from(sr);
    let script: Vec<(usize, u32, f32)> = vec![
        (0, LFO_DEPTH, 2.0),
        (0, ENV_AMOUNT, 1.5),
        (0, DRIVE, 0.3),
        (len / 20, LFO_SHAPE, shape("square")),
        (len / 10, LFO_STEREO, 90.0),
        (len / 8, LFO_DIVISION, division("eighth")),
        (len / 6, LFO_PHASE, 45.0),
        (len / 5, LFO_SYNC, 0.0),
        (len / 5 + 3, LFO_RATE, 7.0),
        (len / 4, LFO_SHAPE, shape("random")),
        (len / 3, ENV_ATTACK, 0.001),
        (len / 3 + 17, ENV_RELEASE, 0.6),
        (len / 2, LFO_RESTART, 0.0),
        (len / 2 + 5, MODE, NOTCH as f32),
        (len * 3 / 5, LFO_SYNC, 1.0),
        (len * 3 / 5, LFO_DIVISION, division("two_bars")),
        (len * 2 / 3, LFO_SHAPE, shape("saw_down")),
        (len * 3 / 4, ENV_AMOUNT, -3.0),
        (len * 4 / 5, LFO_DEPTH, 5.0),
        (len * 9 / 10, CUTOFF, 4000.0),
    ];
    let at = |frame: usize| -> Transport {
        let third = len / 3;
        if frame < third {
            transport(true, 120.0, frame as f64 * tpf)
        } else if frame < third + len / 10 {
            transport(false, 120.0, third as f64 * tpf)
        } else {
            // A seek to bar 3, then playing on at 97 BPM.
            let from = third + len / 10;
            let ticks = (frame - from) as f64 * 97.0 / 60.0 * 960.0 / f64::from(sr);
            transport(true, 97.0, 7680.0 + ticks)
        }
    };
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let mut bits = Vec::new();
    let mut start = 0;
    while start < len {
        let end = (start + size).min(len);
        let mut left = input_l[start..end].to_vec();
        let mut right = input_r[start..end].to_vec();
        let local: Vec<Event> = script
            .iter()
            .filter(|(e, _, _)| (start..end).contains(e))
            .map(|(e, id, v)| param(*id, *v).at((e - start) as u32))
            .collect();
        let ctx = BlockCtx {
            sample_rate: sr,
            frames: (end - start) as u32,
            transport: at(start),
        };
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left,
                right: &mut right,
                events_out: &mut unused,
            };
            f.process(&ctx, &local, &mut io);
        });
        bits.extend(left.iter().chain(right.iter()).map(|s| s.to_bits()));
        if let Some((_, v)) = f.readout() {
            bits.push(v.to_bits());
        }
        start = end;
    }
    bits
}

#[test]
fn the_movement_renders_the_same_samples_as_before_the_shared_block() {
    for (sr, size) in [
        (44_100.0, 128),
        (48_000.0, 37),
        (96_000.0, 512),
        (48_000.0, 1),
    ] {
        let mut now = Filter::new(sr);
        let mut before = movement_filter::Filter::new(sr);
        let a = scripted(&mut now, sr, size);
        let b = scripted(&mut before, sr, size);
        assert_eq!(a.len(), b.len());
        let first = a.iter().zip(&b).position(|(x, y)| x != y);
        assert_eq!(first, None, "{sr} Hz, blocks of {size}: first difference");
    }
}
