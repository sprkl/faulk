//! Ripple (#688): the phaser's notches where the plan computes them (an impulse response and a
//! noise spectrum), the flanger's comb against its delay, the feedback bounded at ±95 %, the
//! synced LFO's period at 120 BPM and its position at each pass of a loop, the envelope
//! follower's response, transparency at mix 0, every parameter at its bounds, the fades of the
//! choices, through the device contract, every `process` under the allocation detector.

use assert_no_alloc::assert_no_alloc;
use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport};
use engine::devices::ripple::{
    CENTRE, DELAY, DESCRIPTOR, ENV_AMOUNT, ENV_ATTACK, ENV_RELEASE, FEEDBACK, FLANGER, LFO_DEPTH,
    LFO_DIVISION, LFO_RATE, LFO_SHAPE, LFO_STEREO, LFO_SYNC, MIX, MODE, NEGATIVE, PHASER, POLARITY,
    Ripple, SAT_LIMIT, SPREAD, STAGES,
};
use engine::dsp::lfo::{DIVISION_KEYS, SHAPE_KEYS};
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

fn stages(n: usize) -> f32 {
    [4, 6, 8, 12].iter().position(|c| *c == n).unwrap() as f32
}

/// A Ripple at `params`, its ramps snapped.
fn with(params: &[(u32, f32)]) -> Ripple {
    let mut r = Ripple::new(SR).unwrap();
    for (id, value) in params {
        r.set_param(ParamId(*id), *value);
    }
    r.reset();
    r
}

/// The settings of a static phaser: no movement, no feedback, mix 100 %.
fn still(extra: &[(u32, f32)]) -> Vec<(u32, f32)> {
    let mut p = vec![
        (LFO_DEPTH, 0.0),
        (ENV_AMOUNT, 0.0),
        (FEEDBACK, 0.0),
        (MIX, 100.0),
        (SPREAD, 0.0),
    ];
    p.extend_from_slice(extra);
    p
}

fn param(id: u32, value: f32) -> Event {
    Event::ParamValue {
        offset: 0,
        id: ParamId(id),
        value,
    }
}

fn transport(playing: bool, position: f64) -> Transport {
    Transport {
        playing,
        tempo: Tempo::new(120.0).unwrap(),
        signature: TimeSignature::COMMON,
        position_ticks: position,
        ..Transport::default()
    }
}

fn stopped(_: usize) -> Transport {
    transport(false, 0.0)
}

fn playing(frame: usize) -> Transport {
    transport(true, frame as f64 * TPF)
}

/// Renders `input` in blocks of `size` frames; `at(frame)` is the transport of the block that
/// starts at `frame`; `events` are `(frame, event)`. With `trace`, after each block, the centre
/// and the delay the left channel used.
fn render(
    r: &mut Ripple,
    input: (&[f32], &[f32]),
    size: usize,
    events: &[(usize, Event)],
    at: &dyn Fn(usize) -> Transport,
    mut trace: Option<&mut Vec<(f32, f32)>>,
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
            r.process(&ctx, &local, &mut io);
        });
        if let Some(t) = trace.as_deref_mut() {
            t.push((r.centres_used()[0], r.delays_used()[0]));
        }
        out_l.extend(left);
        out_r.extend(right);
        start = end;
    }
    (out_l, out_r)
}

fn mono(r: &mut Ripple, input: &[f32], size: usize) -> Vec<f32> {
    render(r, (input, input), size, &[], &stopped, None).0
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

fn impulse(len: usize) -> Vec<f32> {
    let mut x = vec![0.0; len];
    x[0] = 1.0;
    x
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

/// The magnitude of the DFT of `x` at `hz`.
fn dft(x: &[f32], hz: f64) -> f64 {
    let w = core::f64::consts::TAU * hz / f64::from(SR);
    let (mut re, mut im) = (0.0, 0.0);
    for (n, s) in x.iter().enumerate() {
        let a = w * n as f64;
        re += f64::from(*s) * a.cos();
        im -= f64::from(*s) * a.sin();
    }
    (re * re + im * im).sqrt()
}

/// The frequency of the smallest (or largest) DFT magnitude of `x` in `lo..hi`, on a 0.5 Hz
/// grid, and that magnitude.
fn extremum(x: &[f32], lo: f64, hi: f64, largest: bool) -> (f64, f64) {
    let mut best = (lo, if largest { -1.0 } else { f64::MAX });
    let mut f = lo;
    while f <= hi {
        let m = dft(x, f);
        if (largest && m > best.1) || (!largest && m < best.1) {
            best = (f, m);
        }
        f += 0.5;
    }
    best
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

/// The notches the plan computes for `n` stages at `centre` hertz (spread 0, no feedback):
/// `tan(pi f_k / fs) = t tan(pi (2k + 1) / (2n))`.
fn computed_notches(n: usize, centre: f64) -> Vec<f64> {
    let fs = f64::from(SR);
    let t = (core::f64::consts::PI * centre / fs).tan();
    (0..n / 2)
        .map(|k| {
            let q = (core::f64::consts::PI * (2 * k + 1) as f64 / (2 * n) as f64).tan();
            (t * q).atan() * fs / core::f64::consts::PI
        })
        .collect()
}

#[test]
fn four_phaser_stages_dig_their_notches_where_the_plan_computes_them() {
    let notches = computed_notches(4, 1000.0);
    // The plan's numbers: 414.7 Hz and about 2 398 Hz at 48 kHz.
    assert!((notches[0] - 414.7).abs() < 0.5, "{notches:?}");
    assert!((notches[1] - 2398.0).abs() < 2.0, "{notches:?}");
    let mut r = with(&still(&[
        (MODE, PHASER as f32),
        (STAGES, stages(4)),
        (CENTRE, 1000.0),
    ]));
    let ir = mono(&mut r, &impulse(16_384), 128);
    check(&ir);
    for (k, want) in notches.iter().enumerate() {
        let (f, m) = extremum(&ir, want * 0.8, want * 1.2, false);
        println!(
            "notch {k}: measured {f} Hz ({:.1} dB), computed {want:.1} Hz",
            db(m)
        );
        assert!((f - want).abs() <= want * 0.01, "notch {k}: {f} vs {want}");
        assert!(db(m) < -30.0, "notch {k} is {} dB", db(m));
    }
    // Between and away from the notches the sound passes (the chain's phase a multiple of
    // 2 pi): at the centre and near DC, unity.
    assert!(db(dft(&ir, 1000.0)).abs() < 0.1);
    assert!(db(dft(&ir, 20.0)).abs() < 0.2);

    // The same two dips in the spectrum of white noise through it (averaged periodograms of
    // Hann-windowed segments, output over input).
    let x = noise(48_000 * 8, 3);
    let mut r = with(&still(&[
        (MODE, PHASER as f32),
        (STAGES, stages(4)),
        (CENTRE, 1000.0),
    ]));
    let y = mono(&mut r, &x, 512);
    let seg = 4096;
    let hann: Vec<f32> = (0..seg)
        .map(|n| 0.5 - 0.5 * (core::f32::consts::TAU * n as f32 / seg as f32).cos())
        .collect();
    let band = |hz: f64| {
        let (mut num, mut den) = (0.0, 0.0);
        for s in (0..x.len() - seg).step_by(seg) {
            let wx: Vec<f32> = (0..seg).map(|n| x[s + n] * hann[n]).collect();
            let wy: Vec<f32> = (0..seg).map(|n| y[s + n] * hann[n]).collect();
            num += dft(&wy, hz).powi(2);
            den += dft(&wx, hz).powi(2);
        }
        10.0 * (num / den).log10()
    };
    let pass = band(1000.0);
    for want in &notches {
        let dip = band(*want);
        println!("noise: {dip:.1} dB at {want:.1} Hz, {pass:.1} dB at 1 kHz");
        assert!(dip < pass - 20.0, "{want} Hz: {dip} dB vs {pass} dB");
    }
}

#[test]
fn more_stages_dig_more_notches_and_each_count_lands_on_its_formula() {
    for n in [6, 8, 12] {
        let mut r = with(&still(&[(STAGES, stages(n)), (CENTRE, 2000.0)]));
        let ir = mono(&mut r, &impulse(16_384), 128);
        let notches = computed_notches(n, 2000.0);
        assert_eq!(notches.len(), n / 2);
        for want in notches.iter().filter(|f| **f < 20_000.0) {
            let (f, m) = extremum(&ir, want * 0.97, want * 1.03, false);
            assert!((f - want).abs() <= want * 0.01, "{n} stages: {f} vs {want}");
            assert!(db(m) < -30.0, "{n} stages, {want} Hz: {} dB", db(m));
        }
    }
}

#[test]
fn a_one_millisecond_flanger_at_90_percent_feedback_combs_every_kilohertz() {
    for (ms, spacing) in [(1.0, 1000.0), (2.0, 500.0), (5.0, 200.0)] {
        let mut r = with(&[
            (MODE, FLANGER as f32),
            (DELAY, ms / 1000.0),
            (FEEDBACK, 90.0),
            (MIX, 100.0),
            (LFO_DEPTH, 0.0),
            (ENV_AMOUNT, 0.0),
        ]);
        let ir = mono(&mut r, &impulse(32_768), 128);
        check(&ir);
        let mut peaks = Vec::new();
        for k in 1..=6 {
            let want = spacing * k as f64;
            let (f, m) = extremum(&ir, want - spacing / 4.0, want + spacing / 4.0, true);
            // (1 + (1 - g)) / (2 (1 - g)) at the peaks, g = 0.9: 5.5.
            assert!((m - 5.5).abs() < 0.1, "{ms} ms, peak {k}: {m}");
            peaks.push(f);
        }
        let measured = (peaks[5] - peaks[0]) / 5.0;
        println!("{ms} ms: peaks {peaks:?}, spacing {measured} Hz");
        assert!((measured - spacing).abs() <= 1.0, "{ms} ms: {measured}");
        // Half-way between: (1 - (1 - g)) / (2 (1 + g)) = 0.237.
        let between = dft(&ir, spacing * 1.5);
        assert!((between - 0.9 / 3.8).abs() < 0.01, "{ms} ms: {between}");
    }
}

#[test]
fn the_flanger_at_90_percent_feedback_stays_bounded_for_60_seconds() {
    let len = 60 * SR as usize;
    let inputs = [noise(len, 9), {
        // A full-scale square at the comb's first peak.
        (0..len)
            .map(|n| if (n / 24) % 2 == 0 { 1.0 } else { -1.0 })
            .collect()
    }];
    for x in &inputs {
        let mut r = with(&[
            (MODE, FLANGER as f32),
            (DELAY, 0.001),
            (FEEDBACK, 90.0),
            (MIX, 100.0),
            (LFO_DEPTH, 0.0),
        ]);
        let y = mono(&mut r, x, 2048);
        check(&y);
        // The line holds at most |x| + 0.9 * 4 (module doc), the output half of dry + that.
        let p = peak(&y);
        println!("60 s at 90 %: peak {p}");
        assert!(p <= (1.0 + 1.0 + 0.9 * SAT_LIMIT) / 2.0, "{p}");
    }
}

#[test]
fn feedback_at_plus_and_minus_95_percent_never_runs_away_even_swept_hard() {
    let len = 60 * SR as usize;
    let x = noise(len, 21);
    for mode in [PHASER, FLANGER] {
        for fb in [95.0, -95.0] {
            for n in [4, 12] {
                let mut r = with(&[
                    (MODE, mode as f32),
                    (STAGES, stages(n)),
                    (SPREAD, 3.0),
                    (FEEDBACK, fb),
                    (MIX, 100.0),
                    (LFO_SYNC, 0.0),
                    (LFO_RATE, 20.0),
                    (LFO_SHAPE, shape("square")),
                    (LFO_DEPTH, 5.0),
                    (LFO_STEREO, 180.0),
                    (ENV_AMOUNT, 5.0),
                    (ENV_ATTACK, 0.001),
                    (ENV_RELEASE, 0.01),
                ]);
                let (l, rr) = render(&mut r, (&x, &x), 1024, &[], &stopped, None);
                check(&l);
                check(&rr);
                let p = peak(&l).max(peak(&rr));
                println!("mode {mode}, feedback {fb} %, {n} stages: peak {p}");
                // The last minute is no louder than the first: no slow build-up.
                let first = peak(&l[..len / 6]);
                let last = peak(&l[len * 5 / 6..]);
                assert!(last <= first * 1.5 + 0.1, "{first} then {last}");
                assert!(p < 100.0, "{p}");
            }
        }
    }
}

#[test]
fn a_synced_half_note_lfo_at_120_bpm_has_a_period_of_one_second_to_the_frame() {
    let len = 3 * SR as usize;
    let x = vec![0.0; len];
    let mut r = with(&still(&[
        (CENTRE, 800.0),
        (LFO_SYNC, 1.0),
        (LFO_DIVISION, division("half")),
        (LFO_SHAPE, shape("sine")),
        (LFO_DEPTH, 1.0),
    ]));
    let mut trace = Vec::new();
    render(&mut r, (&x, &x), 1, &[], &playing, Some(&mut trace));
    let c: Vec<f32> = trace.iter().map(|t| t.0).collect();
    // A sine starts each cycle rising through the centre: between the frames 48 000 k - 1 and
    // 48 000 k + 1, for every cycle.
    for k in 1..3 {
        let at = k * 48_000;
        assert!(
            c[at - 1] < 800.0 && c[at + 1] > 800.0,
            "cycle {k}: {} {}",
            c[at - 1],
            c[at + 1]
        );
    }
    // Every frame is where it was one second earlier (the position is an f64 product, so the
    // f32 centre may differ in its last bit: within 1e-3 Hz).
    let worst = (0..len - 48_000)
        .map(|n| (c[n + 48_000] - c[n]).abs())
        .fold(0f32, f32::max);
    println!("one second apart: largest difference {worst:e} Hz");
    assert!(worst <= 1e-3, "{worst}");
    // It peaks a quarter cycle in: 800 * 2 at frame 12 000.
    assert!((c[12_000] - 1600.0).abs() < 0.01, "{}", c[12_000]);
}

#[test]
fn the_synced_lfo_takes_the_same_position_at_each_pass_of_a_loop() {
    // A loop of one bar (2 s at 120 BPM): the host sends the position wrapped back to 0.
    let bar = 2 * SR as usize;
    let len = 3 * bar;
    let x = noise(len, 4);
    let looped = |frame: usize| transport(true, (frame % bar) as f64 * TPF);
    let mut r = with(&[
        (MODE, FLANGER as f32),
        (DELAY, 0.003),
        (LFO_SYNC, 1.0),
        (LFO_DIVISION, division("quarter_dotted")),
        (LFO_SHAPE, shape("triangle")),
        (LFO_DEPTH, 1.5),
    ]);
    let mut trace = Vec::new();
    render(&mut r, (&x, &x), 64, &[], &looped, Some(&mut trace));
    let blocks = bar / 64;
    for pass in 1..3 {
        for b in 0..blocks {
            let (a, z) = (trace[b].1, trace[pass * blocks + b].1);
            assert!((a - z).abs() <= 1e-9, "pass {pass}, block {b}: {a} vs {z}");
        }
    }
}

#[test]
fn the_envelope_follower_raises_the_notches_with_its_attack_and_lowers_them_with_its_release() {
    let attack = 0.01;
    let release = 0.15;
    let level = 0.5f32;
    let amount = 2.0f32;
    let up = (attack * SR) as usize;
    let down = (release * SR) as usize;
    let mut x = vec![level; 20 * up];
    x.extend(vec![0.0; 2 * down]);
    let mut r = with(&still(&[
        (CENTRE, 800.0),
        (ENV_AMOUNT, amount),
        (ENV_ATTACK, attack),
        (ENV_RELEASE, release),
    ]));
    let mut trace = Vec::new();
    render(&mut r, (&x, &x), 1, &[], &stopped, Some(&mut trace));
    let centre_at = |frame: usize| trace[frame - 1].0;
    let expect = |env: f32| 800.0 * (amount * env).exp2();
    let after_attack = centre_at(up);
    let want = expect(level * (1.0 - (-1.0f32).exp()));
    println!("one attack in: {after_attack} Hz, expected {want}");
    assert!((after_attack / want - 1.0).abs() < 0.01);
    let top = centre_at(20 * up);
    assert!((top / expect(level) - 1.0).abs() < 1e-3, "{top}");
    let after_release = centre_at(20 * up + down);
    let want = expect(level * (-1.0f32).exp());
    println!("one release out: {after_release} Hz, expected {want}");
    assert!((after_release / want - 1.0).abs() < 0.01);
}

#[test]
fn at_mix_zero_the_output_is_the_input_bit_for_bit_whatever_moves() {
    let len = SR as usize;
    let l = noise(len, 5);
    let rr = noise(len, 6);
    let events: Vec<(usize, Event)> = vec![
        (1_000, param(MODE, FLANGER as f32)),
        (1_300, param(MODE, PHASER as f32)),
        (9_000, param(STAGES, stages(12))),
        (12_000, param(POLARITY, NEGATIVE as f32)),
        (20_000, param(FEEDBACK, -95.0)),
        (30_000, param(CENTRE, 18_000.0)),
        (40_000, param(MODE, FLANGER as f32)),
        (41_000, param(DELAY, 0.0001)),
    ];
    let mut r = with(&[
        (MIX, 0.0),
        (FEEDBACK, 95.0),
        (LFO_SYNC, 0.0),
        (LFO_RATE, 13.0),
        (LFO_DEPTH, 5.0),
        (ENV_AMOUNT, -5.0),
        (SPREAD, 4.0),
    ]);
    let (ol, or) = render(&mut r, (&l, &rr), 97, &events, &playing, None);
    for i in 0..len {
        assert_eq!(ol[i].to_bits(), l[i].to_bits(), "left {i}");
        assert_eq!(or[i].to_bits(), rr[i].to_bits(), "right {i}");
    }
}

#[test]
fn every_parameter_at_its_bounds_and_default_stays_finite_bounded_and_free_of_denormals() {
    let len = SR as usize / 2;
    let inputs = [noise(len, 8), vec![0.0; len], {
        let mut v = vec![0.0; len];
        v[0] = 1.0;
        v
    }];
    let base: Vec<(u32, f32)> = vec![(LFO_SYNC, 0.0), (LFO_RATE, 5.0)];
    let mut settings: Vec<Vec<(u32, f32)>> = Vec::new();
    for p in DESCRIPTOR.params {
        for v in [p.min, p.max, p.default] {
            for mode in [PHASER, FLANGER] {
                let mut s = base.clone();
                s.push((MODE, mode as f32));
                s.push((p.id.0, v as f32));
                settings.push(s);
            }
        }
    }
    // The corners: every parameter at its min or max at once.
    for corner in 0..4u32 {
        settings.push(
            DESCRIPTOR
                .params
                .iter()
                .map(|p| {
                    let high = (corner >> (p.id.0 % 2)) & 1 == 1;
                    (p.id.0, (if high { p.max } else { p.min }) as f32)
                })
                .collect(),
        );
    }
    for s in &settings {
        for x in &inputs {
            let mut r = with(s);
            let (l, rr) = render(&mut r, (x, x), 256, &[], &playing, None);
            check(&l);
            check(&rr);
            let p = peak(&l).max(peak(&rr));
            assert!(p < 100.0, "{s:?}: peak {p}");
        }
    }
    // Garbage in: NaN, infinities, huge and tiny values.
    let mut r = with(&[(FEEDBACK, 95.0), (MODE, FLANGER as f32)]);
    let garbage: Vec<f32> = (0..len)
        .map(|n| match n % 5 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => -1e30,
            3 => 1e-30,
            _ => 0.3,
        })
        .collect();
    let (l, _) = render(&mut r, (&garbage, &garbage), 128, &[], &playing, None);
    check(&l);
    // Garbage values of the parameters are ignored or clamped.
    let mut r = with(&[]);
    for p in DESCRIPTOR.params {
        r.set_param(p.id, f32::NAN);
        r.set_param(p.id, f32::INFINITY);
        r.set_param(p.id, -1e30);
    }
    let (l, _) = render(&mut r, (&inputs[0], &inputs[0]), 128, &[], &playing, None);
    check(&l);
}

/// A script that changes every choice and moves the continuous parameters.
fn script(len: usize) -> Vec<(usize, Event)> {
    vec![
        (0, param(LFO_SYNC, 0.0)),
        (0, param(LFO_RATE, 3.0)),
        (len / 10, param(STAGES, stages(12))),
        (len / 10 + 50, param(STAGES, stages(6))),
        (len / 8, param(SPREAD, 2.5)),
        (len / 6, param(POLARITY, NEGATIVE as f32)),
        (len / 5, param(MODE, FLANGER as f32)),
        (len / 5 + 100, param(MODE, PHASER as f32)),
        (len / 4, param(MODE, FLANGER as f32)),
        (len / 3, param(DELAY, 0.009)),
        (len / 2, param(FEEDBACK, -80.0)),
        (len / 2, param(ENV_AMOUNT, 2.0)),
        (len * 3 / 5, param(LFO_SYNC, 1.0)),
        (len * 2 / 3, param(MIX, 40.0)),
        (len * 3 / 4, param(MODE, PHASER as f32)),
        (len * 4 / 5, param(CENTRE, 3000.0)),
    ]
}

#[test]
fn the_render_does_not_depend_on_the_block_size() {
    let len = 2 * SR as usize;
    let l = noise(len, 12);
    let rr = noise(len, 13);
    let events = script(len);
    let reference = render(&mut with(&[]), (&l, &rr), 1, &events, &playing, None);
    for size in [37, 128, 2048] {
        let got = render(&mut with(&[]), (&l, &rr), size, &events, &playing, None);
        for i in 0..len {
            assert_eq!(
                got.0[i].to_bits(),
                reference.0[i].to_bits(),
                "{size}: left {i}"
            );
            assert_eq!(
                got.1[i].to_bits(),
                reference.1[i].to_bits(),
                "{size}: right {i}"
            );
        }
    }
}

/// The largest jump between two frames of `x` in `range`.
fn max_step(x: &[f32], range: core::ops::Range<usize>) -> f32 {
    x[range.start..range.end]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn the_choices_fade_instead_of_clicking_even_when_changed_back_quickly() {
    let len = SR as usize;
    let x = sine(len, 150.0, 0.5);
    // The steepest step of the effect itself, before any change.
    let base = still(&[(FEEDBACK, 50.0), (SPREAD, 2.0), (DELAY, 0.004)]);
    let steady = {
        let mut r = with(&base);
        let y = mono(&mut r, &x, 128);
        max_step(&y, 4_800..24_000)
    };
    let changes: [&[(usize, Event)]; 3] = [
        // Phaser, flanger and back to the phaser within 3 ms.
        &[
            (24_000, param(MODE, FLANGER as f32)),
            (24_144, param(MODE, PHASER as f32)),
        ],
        // 4 stages, then 12, then 6 within 5 ms.
        &[
            (24_000, param(STAGES, stages(12))),
            (24_240, param(STAGES, stages(6))),
        ],
        &[
            (24_000, param(POLARITY, NEGATIVE as f32)),
            (24_100, param(POLARITY, 0.0)),
        ],
    ];
    for events in changes {
        let mut r = with(&base);
        let y = render(&mut r, (&x, &x), 128, events, &stopped, None).0;
        check(&y);
        let step = max_step(&y, 23_990..26_000);
        println!("steady {steady}, across the change {step}");
        assert!(
            step <= steady * 2.0 + 1e-3,
            "{events:?}: {step} vs {steady}"
        );
    }
}

#[test]
fn reset_clears_the_states_and_renders_like_a_new_device() {
    let x = noise(SR as usize / 4, 30);
    let params = [(MODE, FLANGER as f32), (FEEDBACK, 90.0), (LFO_SYNC, 0.0)];
    let mut r = with(&params);
    mono(&mut r, &x, 128);
    r.reset();
    let silence = mono(&mut r, &vec![0.0; 4096], 128);
    assert!(silence.iter().all(|s| *s == 0.0));
    r.reset();
    let again = mono(&mut r, &x, 128);
    let fresh = mono(&mut with(&params), &x, 128);
    assert_eq!(again, fresh);
}

#[test]
fn the_readout_follows_the_mode_and_the_device_declares_no_latency() {
    let x = vec![0.0; 4800];
    let mut r = with(&[(LFO_SYNC, 0.0), (LFO_RATE, 2.0), (LFO_DEPTH, 1.0)]);
    mono(&mut r, &x, 128);
    let (id, v) = r.readout().unwrap();
    assert_eq!(id, ParamId(CENTRE));
    assert_eq!(v, r.centres_used()[0]);
    assert!(v != 800.0);
    r.set_param(ParamId(MODE), FLANGER as f32);
    mono(&mut r, &x, 128);
    let (id, v) = r.readout().unwrap();
    assert_eq!(id, ParamId(DELAY));
    assert_eq!(v, r.delays_used()[0]);
    assert_eq!(r.latency_frames(), 0);
}

#[test]
fn a_stereo_phase_of_180_degrees_moves_the_channels_in_opposition() {
    let x = vec![0.0; 24_000];
    let mut r = with(&still(&[
        (LFO_SYNC, 0.0),
        (LFO_RATE, 1.0),
        (LFO_SHAPE, shape("sine")),
        (LFO_DEPTH, 1.0),
        (LFO_STEREO, 180.0),
    ]));
    mono(&mut r, &x[..12_000], 1);
    let [left, right] = r.centres_used();
    // A quarter cycle in: left at +1 octave, right at -1.
    assert!((left / 1600.0 - 1.0).abs() < 1e-3, "{left}");
    assert!((right / 400.0 - 1.0).abs() < 1e-3, "{right}");
}

#[test]
fn the_descriptor_numbers_its_parameters_and_keeps_its_defaults_in_range() {
    assert_eq!(DESCRIPTOR.kind, "faulk.ripple");
    assert_eq!(DESCRIPTOR.kind_id.0, 15);
    assert_eq!(DESCRIPTOR.name, "Ripple");
    for (i, p) in DESCRIPTOR.params.iter().enumerate() {
        assert_eq!(p.id.0 as usize, i, "{}", p.key);
        assert!(p.min <= p.default && p.default <= p.max, "{}", p.key);
        assert!(!p.description.is_empty());
        if !p.values.is_empty() {
            assert_eq!(p.max, (p.values.len() - 1) as f64, "{}", p.key);
            assert_eq!(p.step, 1.0, "{}", p.key);
        }
    }
    assert_eq!(DESCRIPTOR.params.len(), 19);
}
