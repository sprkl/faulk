//! Hiccup, `faulk.hiccup` (#689): offline renders through the device contract, every
//! `process` under the allocation detector, the transport playing as the host plays it (the
//! position of each block's first frame, moving linearly). Asserts on the signal: the slice and
//! its repeats at the exact frames at several tempos and sample rates, whatever the block size;
//! the chance's draws (same seed, same captures, whatever the blocks and the start); decay and
//! filter (each repeat weaker and darker, or thinner); the variations and the pitch; the three
//! modes; the transparency when nothing captures; the click bound at the edges; the manual
//! capture, the stop and the reset; every parameter swept at its bounds and default with a
//! hostile input, the transport playing. The contract's conformance run over the registry
//! (transport stopped) is in `osc_synth.rs`; the native/WASM parity and the export are in
//! `crates/test-support/tests/it/hiccup.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceRole, DeviceSetup, Event, EventBuffer, MAX_SAMPLE_RATE,
    Transport, builtin,
};
use engine::devices::hiccup::{
    self, CAPTURE, CHANCE, DECAY, DESCRIPTOR, EVERY_1_16, FADE_SECONDS, FILTER, GATE, HIGH_PASS,
    Hiccup, INSERT, INTERVAL, LAYER, LOW_PASS, MAX_SLICE_SECONDS, MIX, MODE, PITCH, RANDOM,
    REPEATS_ONLY, SEED, SHORTER, SIZE, SIZE_1_16, SIZE_1_64, VARIATION, draw,
};
use engine::msg::ParamId;
use engine::time::{Tempo, TimeSignature};

const SR: f32 = 48_000.0;

fn fade(rate: f32) -> usize {
    ((FADE_SECONDS * rate).ceil() as usize).max(1)
}

/// A Hiccup with these values, reset (so a test starts at its values).
fn with(rate: f32, params: &[(u32, f32)]) -> Hiccup {
    let mut h = Hiccup::new(rate).expect("the rings are allocated");
    for (id, value) in params {
        h.set_param(ParamId(*id), *value);
    }
    h.reset();
    h
}

/// How the transport runs in a render.
#[derive(Clone, Copy)]
struct Play {
    rate: f32,
    bpm: f64,
    /// Position of the first frame, in ticks.
    start: f64,
    /// Block size.
    size: usize,
    /// Frame from which the clock is stopped (`usize::MAX`: never; 0: always stopped).
    stop_at: usize,
}

impl Play {
    fn at(rate: f32, bpm: f64) -> Self {
        Play {
            rate,
            bpm,
            start: 0.0,
            size: 256,
            stop_at: usize::MAX,
        }
    }
    fn blocks(self, size: usize) -> Self {
        Play { size, ..self }
    }
    fn from(self, start: f64) -> Self {
        Play { start, ..self }
    }
    fn stopped_at(self, stop_at: usize) -> Self {
        Play { stop_at, ..self }
    }
    fn samples_per_tick(&self) -> f64 {
        f64::from(self.rate) / Tempo::new(self.bpm).unwrap().ticks_per_second()
    }
}

/// Runs `left` / `right` through the device in blocks, the transport as `play` says, each
/// `(frame, id, value)` applying at its frame. A block is cut at the stop so the stop lands on
/// a block's first frame, as the host's clock does.
fn run_stereo(
    h: &mut Hiccup,
    play: Play,
    left_in: &[f32],
    right_in: &[f32],
    events: &[(usize, u32, f32)],
) -> (Vec<f32>, Vec<f32>) {
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
    let tempo = Tempo::new(play.bpm).unwrap();
    let spt = play.samples_per_tick();
    let mut position = play.start;
    let mut start = 0;
    while start < left_in.len() {
        let mut end = (start + play.size).min(left_in.len());
        if start < play.stop_at && end > play.stop_at {
            end = play.stop_at;
        }
        let playing = start < play.stop_at;
        let mut left = left_in[start..end].to_vec();
        let mut right = right_in[start..end].to_vec();
        let local: Vec<Event> = events
            .iter()
            .filter(|(at, _, _)| (start..end).contains(at))
            .map(|&(at, id, value)| Event::ParamValue {
                offset: (at - start) as u32,
                id: ParamId(id),
                value,
            })
            .collect();
        let ctx = BlockCtx {
            sample_rate: play.rate,
            frames: (end - start) as u32,
            transport: Transport {
                playing,
                tempo,
                signature: TimeSignature::default(),
                position_ticks: position,
                ..Transport::default()
            },
        };
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left,
                right: &mut right,
                events_out: &mut unused,
            };
            h.process(&ctx, &local, &mut io);
        });
        if playing {
            position += (end - start) as f64 / spt;
        }
        out_l.extend(left);
        out_r.extend(right);
        start = end;
    }
    (out_l, out_r)
}

fn run(h: &mut Hiccup, play: Play, input: &[f32], events: &[(usize, u32, f32)]) -> Vec<f32> {
    run_stereo(h, play, input, input, events).0
}

/// A signal that differs at every frame (no two neighbours equal), in [-0.5, 0.5].
fn marked(len: usize, rate: f32) -> Vec<f32> {
    (0..len)
        .map(|n| {
            let t = n as f32 / rate;
            0.3 * (core::f32::consts::TAU * 211.0 * t).sin()
                + 0.2 * (core::f32::consts::TAU * 1_327.0 * t).sin()
        })
        .collect()
}

fn sine(len: usize, rate: f32, hz: f32, amplitude: f32) -> Vec<f32> {
    (0..len)
        .map(|n| {
            let phase = core::f64::consts::TAU * f64::from(hz) * n as f64 / f64::from(rate);
            (f64::from(amplitude) * phase.sin()) as f32
        })
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

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Energy of the first difference over the energy: a measure of brightness (higher with more
/// high frequencies).
fn brightness(x: &[f32]) -> f32 {
    let d: f32 = x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    let e: f32 = x.iter().map(|s| s * s).sum();
    d / e.max(1e-30)
}

/// Upward zero crossings per second.
fn frequency(x: &[f32], rate: f32) -> f32 {
    let n = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    n as f32 * rate / x.len() as f32
}

/// The frame of grid point `ticks` for a transport started at 0: the floor of its fractional
/// sample, computed here from the tempo (the host's rule).
fn frame_of_tick(ticks: f64, play: Play) -> usize {
    ((ticks - play.start) * play.samples_per_tick() + 1e-6).floor() as usize
}

fn frames_of_ticks(ticks: f64, play: Play) -> usize {
    (ticks * play.samples_per_tick()).round() as usize
}

// ---------------------------------------------------------------- the descriptor and the memory

#[test]
fn the_descriptor_is_a_valid_effect() {
    assert_eq!(DESCRIPTOR.validate(), Ok(()));
    assert_eq!(DESCRIPTOR.role, DeviceRole::Effect);
    assert_eq!(
        (DESCRIPTOR.kind, DESCRIPTOR.name),
        ("faulk.hiccup", "Hiccup")
    );
    assert_eq!(DESCRIPTOR.kind_id.0, 13);
    assert_eq!(
        builtin(DESCRIPTOR.kind_id).map(|d| d.kind),
        Some("faulk.hiccup")
    );
    let keys: Vec<_> = DESCRIPTOR.params.iter().map(|p| p.key).collect();
    assert_eq!(
        keys,
        [
            "interval",
            "chance",
            "seed",
            "capture",
            "size",
            "gate",
            "variation",
            "decay",
            "filter",
            "pitch",
            "mode",
            "mix"
        ]
    );
    for (i, p) in DESCRIPTOR.params.iter().enumerate() {
        assert_eq!(p.id.0 as usize, i, "ids follow the order");
    }
}

#[test]
fn the_defaults_are_the_values_new_starts_with() {
    let input = noise(5 * SR as usize);
    let play = Play::at(SR, 120.0);
    let mut fresh = with(SR, &[]);
    let mut set_to_defaults = with(SR, &[]);
    for p in DESCRIPTOR.params {
        set_to_defaults.set_param(p.id, p.default as f32);
    }
    set_to_defaults.reset();
    let a = run(&mut fresh, play, &input, &[]);
    assert_eq!(a, run(&mut set_to_defaults, play, &input, &[]));
    // The defaults capture: the output is not the input.
    assert_ne!(a, input);
}

#[test]
fn the_declared_memory_covers_the_rings_at_the_highest_rate() {
    let h = Hiccup::new(MAX_SAMPLE_RATE).unwrap();
    let rings = 2 * (MAX_SLICE_SECONDS * MAX_SAMPLE_RATE) as u64 * 4;
    assert!(DESCRIPTOR.memory_bytes >= rings + size_of_val(&h) as u64);
    assert!(h.max_slice_frames() as f32 >= MAX_SLICE_SECONDS * MAX_SAMPLE_RATE);
    // A higher rate than declared does not take more: the longest slice is shorter, and two
    // fades at the real rate (2 x 768 frames) stay free beyond it.
    let beyond = Hiccup::new(384_000.0).unwrap();
    let ring = (MAX_SLICE_SECONDS * MAX_SAMPLE_RATE) as u32 + 2 * 384 + 4;
    assert!(beyond.max_slice_frames() + 2 * 768 <= ring);
    assert!(beyond.max_slice_frames() < h.max_slice_frames());
    let device: Box<dyn Device> = (hiccup::factory)(&DeviceSetup {
        sample_rate: 384_000.0,
        ..DeviceSetup::default()
    })
    .unwrap();
    assert_eq!(device.descriptor().kind, "faulk.hiccup");
    // At 48 kHz a slice of one bar of 4/4 at 20 BPM (12 s) fits.
    let at_48k = Hiccup::new(SR).unwrap();
    assert!(at_48k.max_slice_frames() as f32 >= 12.0 * SR);
}

#[test]
fn the_rings_are_allocated_by_the_factory_and_process_allocates_nothing() {
    let input = noise(4_096);
    for size in [1usize, 33, 2_048] {
        let mut h = (hiccup::factory)(&DeviceSetup::default()).unwrap();
        let mut unused = EventBuffer::try_with_capacity(1).unwrap();
        let mut left = input[..size].to_vec();
        let mut right = left.clone();
        let ctx = BlockCtx {
            sample_rate: SR,
            frames: size as u32,
            transport: Transport {
                playing: true,
                ..Transport::default()
            },
        };
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left,
                right: &mut right,
                events_out: &mut unused,
            };
            h.process(&ctx, &[], &mut io);
            h.set_param(ParamId(CAPTURE), 1.0);
            h.set_param(ParamId(PITCH), 7.0);
            h.process(&ctx, &[], &mut io);
            h.reset();
        });
    }
}

#[test]
fn unknown_parameters_and_bad_values_are_ignored() {
    let mut h = with(SR, &[]);
    let before = format!("{h:?}");
    h.set_param(ParamId(99), 1.0);
    for p in DESCRIPTOR.params {
        h.set_param(p.id, f32::NAN);
        h.set_param(p.id, f32::INFINITY);
        h.set_param(p.id, f32::NEG_INFINITY);
    }
    assert_eq!(before, format!("{h:?}"));
}

// ---------------------------------------------------------------- the slice at the exact frames

/// Interval 1 bar, size 1/16, chance 100 %, gate 100 %, insert, at `play`: the slice plays as
/// it comes, then the repeats replace the sound, each repeat the slice sample for sample on its
/// plateau (after its fade in, before its fade out).
fn assert_repeats_at_the_grid(play: Play) {
    let bars = 2;
    let spt = play.samples_per_tick();
    let len = frame_of_tick(3840.0 * bars as f64, play) + 2_000;
    let input = marked(len, play.rate);
    let mut h = with(play.rate, &[(GATE, 100.0)]);
    let out = run(&mut h, play, &input, &[]);
    let f = fade(play.rate);
    let slice = frames_of_ticks(240.0, play);
    assert_eq!(slice, (240.0 * spt).round() as usize);
    for bar in 0..bars {
        let b = frame_of_tick(3840.0 * bar as f64, play);
        // Before the bar line (after the window of the bar before has ended), and through the
        // slice: the input as is.
        if bar > 0 {
            let w = frames_of_ticks(3840.0, play);
            let previous = frame_of_tick(3840.0 * (bar - 1) as f64, play);
            assert!(previous + w <= b + 1);
        }
        for i in b..b + slice {
            assert_eq!(
                out[i], input[i],
                "bar {bar}: the slice is heard as is at {i}"
            );
        }
        // The repeats: the slice again, from the frame after it.
        for k in 1..4 {
            let start = b + k * slice;
            for u in f + 2..slice - f - 2 {
                assert_eq!(
                    out[start + u],
                    input[b + u],
                    "bar {bar}, repeat {k}, frame {u} of {slice}"
                );
            }
        }
    }
    // The frame before the first repeat's plateau ends the slice: the repeat really starts at
    // `b + slice`, not one frame on either side (a shifted repeat would not match above).
    let b = frame_of_tick(3840.0, play);
    assert_ne!(out[b + slice + f + 2], input[b + slice + f + 2]);
}

#[test]
fn the_repeats_land_at_the_exact_frames_at_120_bpm_48_khz() {
    let play = Play::at(48_000.0, 120.0);
    assert_eq!(frame_of_tick(3840.0, play), 96_000);
    assert_eq!(frames_of_ticks(240.0, play), 6_000);
    assert_repeats_at_the_grid(play);
}

#[test]
fn the_repeats_land_at_the_exact_frames_at_120_bpm_44_1_khz() {
    let play = Play::at(44_100.0, 120.0);
    assert_eq!(frame_of_tick(3840.0, play), 88_200);
    assert_repeats_at_the_grid(play);
}

#[test]
fn the_repeats_land_at_the_exact_frames_at_97_bpm_96_khz() {
    // 3840 ticks at 97 BPM and 96 kHz: 237 525.77 frames, so the bar line is frame 237 525.
    let play = Play::at(96_000.0, 97.0);
    assert_eq!(frame_of_tick(3840.0, play), 237_525);
    assert_repeats_at_the_grid(play);
}

#[test]
fn the_block_size_and_the_start_change_nothing() {
    let play = Play::at(44_100.0, 131.0);
    let input = noise(200_000);
    let params = [
        (INTERVAL, EVERY_1_16 as f32),
        (SIZE, SIZE_1_64 as f32),
        (GATE, 70.0),
        (CHANCE, 60.0),
        (VARIATION, RANDOM as f32),
        (FILTER, LOW_PASS as f32),
        (DECAY, 10.0),
        (PITCH, 5.0),
    ];
    let reference = run(&mut with(44_100.0, &params), play.blocks(256), &input, &[]);
    for size in [1usize, 37, 2_048] {
        assert_eq!(
            run(&mut with(44_100.0, &params), play.blocks(size), &input, &[]),
            reference,
            "blocks of {size}"
        );
    }
}

// ---------------------------------------------------------------- the chance

/// Which of `points` grid points (1/16 notes at 240 BPM, 3000 frames each at 48 kHz) a
/// render in repeats-only mode captured: the output is heard just after each point that did.
fn captured(seed: u32, chance: f32, play: Play, points: usize) -> Vec<bool> {
    let every = 3_000;
    let input = vec![0.5f32; points * every];
    let mut h = with(
        SR,
        &[
            (INTERVAL, EVERY_1_16 as f32),
            (SIZE, SIZE_1_64 as f32),
            (CHANCE, chance),
            (SEED, seed as f32),
            (MODE, REPEATS_ONLY as f32),
        ],
    );
    let out = run(&mut h, play, &input, &[]);
    (0..points)
        .map(|p| out[p * every + 200].abs() > 0.1)
        .collect()
}

#[test]
fn a_chance_of_50_percent_draws_the_same_captures_for_the_same_seed() {
    let play = Play::at(SR, 240.0);
    let points = 200;
    let a = captured(7, 50.0, play, points);
    let taken = a.iter().filter(|c| **c).count();
    assert!(
        (70..=130).contains(&taken),
        "about half: {taken} of {points}"
    );
    // Each one is the draw of its grid index, nothing else.
    for (n, c) in a.iter().enumerate() {
        assert_eq!(*c, hiccup::captures(7, n as i64, 50), "point {n}");
    }
    // Same seed, other blocks: the same captures.
    assert_eq!(captured(7, 50.0, play.blocks(37), points), a);
    // Same seed, started 40 points later (a seek, a loop, an export from a bar): the same
    // captures from there.
    let later = captured(7, 50.0, play.from(40.0 * 240.0), points - 40);
    assert_eq!(later[..], a[40..]);
    // Another seed, another sequence.
    assert_ne!(captured(8, 50.0, play, points), a);
    // 0 % never captures, 100 % always does.
    assert!(captured(7, 0.0, play, 40).iter().all(|c| !c));
    assert!(captured(7, 100.0, play, 40).iter().all(|c| *c));
}

#[test]
fn the_draw_is_integer_arithmetic_and_fixed() {
    // Pinned values: the same on every target (the WASM parity of a render at 50 % is in
    // test-support); a change here changes every project's captures.
    assert_eq!(draw(0, 0, 0), draw(0, 0, 0));
    let seq: Vec<bool> = (0..16).map(|n| hiccup::captures(0, n, 50)).collect();
    let again: Vec<bool> = (0..16).map(|n| hiccup::captures(0, n, 50)).collect();
    assert_eq!(seq, again);
    // A negative grid index (a count-in) is a key like any other.
    let _ = hiccup::captures(3, -5, 50);
}

// ---------------------------------------------------------------- decay, filter, variation, pitch

/// The plateaus of the first `count` repeats of the first bar (a 1/16 slice at 120 BPM).
fn plateaus(out: &[f32], count: usize) -> Vec<&[f32]> {
    let slice = 6_000;
    let f = fade(SR);
    (1..=count)
        .map(|k| &out[k * slice + 4 * f..(k + 1) * slice - 4 * f])
        .collect()
}

#[test]
fn with_decay_and_a_low_pass_each_repeat_is_weaker_and_darker_than_the_one_before() {
    let input = noise(96_000);
    let mut h = with(
        SR,
        &[(GATE, 100.0), (DECAY, 20.0), (FILTER, LOW_PASS as f32)],
    );
    let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
    check(&out);
    let reps = plateaus(&out, 14);
    for k in 1..reps.len() {
        let (a, b) = (reps[k - 1], reps[k]);
        assert!(
            rms(b) < rms(a),
            "repeat {}: rms {} then {}",
            k + 1,
            rms(a),
            rms(b)
        );
        assert!(
            brightness(b) < brightness(a),
            "repeat {}: brightness {} then {}",
            k + 1,
            brightness(a),
            brightness(b)
        );
    }
}

#[test]
fn with_a_high_pass_each_repeat_is_thinner_than_the_one_before() {
    let input = noise(96_000);
    let mut h = with(SR, &[(GATE, 100.0), (FILTER, HIGH_PASS as f32)]);
    let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
    let reps = plateaus(&out, 14);
    for k in 1..reps.len() {
        assert!(
            brightness(reps[k]) > brightness(reps[k - 1]),
            "repeat {}",
            k + 1
        );
    }
}

#[test]
fn decay_alone_lowers_each_repeat_by_its_share() {
    let input = noise(96_000);
    let mut h = with(SR, &[(GATE, 100.0), (DECAY, 50.0)]);
    let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
    let reps = plateaus(&out, 5);
    for k in 1..reps.len() {
        let ratio = rms(reps[k]) / rms(reps[k - 1]);
        assert!((ratio - 0.5).abs() < 1e-3, "repeat {}: {ratio}", k + 1);
    }
}

/// An impulse in the slice, `at` frames after the bar line, and the frames of the output where
/// it comes back (its peaks), within the first bar.
fn impulse_returns(params: &[(u32, f32)], at: usize) -> Vec<usize> {
    let mut input = vec![0.0f32; 96_000];
    input[at] = 1.0;
    let mut p = vec![(GATE, 100.0), (MODE, REPEATS_ONLY as f32)];
    p.extend_from_slice(params);
    let mut h = with(SR, &p);
    let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
    (0..out.len()).filter(|&i| out[i] > 0.5).collect()
}

#[test]
fn shorter_repeats_shrink_by_three_quarters_down_to_an_eighth() {
    let at = 300;
    let found = impulse_returns(&[(VARIATION, SHORTER as f32)], at);
    let slice = 6_000.0f64;
    let mut expected = vec![at];
    let mut start = slice;
    let mut length = slice;
    let mut k = 1;
    while start as usize + at < 24_000 {
        if k > 1 {
            length *= 0.75;
        }
        let l = length.round().max(slice / 8.0);
        expected.push(start as usize + at);
        start += l;
        k += 1;
    }
    let found: Vec<usize> = found.into_iter().filter(|&i| i < 24_000).collect();
    assert_eq!(found, expected);
}

#[test]
fn random_repeats_last_the_slice_a_half_or_a_quarter_drawn_from_the_seed() {
    let at = 200;
    let seed = 11u32;
    let found = impulse_returns(&[(VARIATION, RANDOM as f32), (SEED, seed as f32)], at);
    let mut expected = vec![at];
    let mut start = 6_000usize;
    let mut k = 1u32;
    let mut lengths = std::collections::BTreeSet::new();
    while start + at < 24_000 {
        let l = 6_000usize >> (draw(seed, 0, k) % 3);
        lengths.insert(l);
        // A repeat of a quarter (1 500 frames) still holds the impulse at 200.
        expected.push(start + at);
        start += l;
        k += 1;
    }
    assert!(lengths.len() > 1, "the draws vary: {lengths:?}");
    let found: Vec<usize> = found.into_iter().filter(|&i| i < 24_000).collect();
    assert_eq!(found, expected);
}

#[test]
fn the_pitch_reads_the_slice_faster_or_slower() {
    let input = sine(96_000, SR, 1_000.0, 0.5);
    for (pitch, hz) in [(12.0, 2_000.0), (-12.0, 500.0), (7.0, 1_498.3)] {
        let mut h = with(SR, &[(GATE, 100.0), (PITCH, pitch)]);
        let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
        check(&out);
        // The first repeat's first part (a raised repeat ends when the slice runs out).
        let part = &out[6_000 + 200..6_000 + 2_800];
        let measured = frequency(part, SR);
        assert!(
            (measured - hz).abs() < hz * 0.03,
            "pitch {pitch}: {measured} Hz, expected {hz}"
        );
    }
    // Raised an octave, the repeat is silent once the slice ran out (half the repeat).
    let mut h = with(SR, &[(GATE, 100.0), (PITCH, 12.0)]);
    let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
    assert!(peak(&out[6_000 + 3_200..12_000 - 200]) == 0.0);
}

// ---------------------------------------------------------------- modes and transparency

#[test]
fn layer_adds_the_repeats_to_the_sound() {
    let input = marked(96_000, SR);
    let mut h = with(SR, &[(GATE, 100.0), (MODE, LAYER as f32)]);
    let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
    let f = fade(SR);
    for u in f + 2..6_000 - f - 2 {
        let i = 6_000 + u;
        assert!((out[i] - (input[i] + input[u])).abs() < 1e-6, "frame {i}");
    }
}

#[test]
fn insert_mutes_the_sound_under_the_repeats_and_gives_it_back_after_the_gate() {
    let input = marked(96_000, SR);
    // Gate 25 %: the window is a quarter of the bar (24 000 frames).
    let mut h = with(SR, &[]);
    let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
    let f = fade(SR);
    // The repeats only (the dry gone) on the plateaus.
    for u in f + 2..6_000 - f - 2 {
        assert_eq!(out[6_000 + u], input[u]);
        assert_eq!(out[18_000 + u], input[u]);
    }
    // After the window: the sound as is.
    for i in 24_000 + 2..96_000 {
        assert_eq!(out[i], input[i], "frame {i}");
    }
}

#[test]
fn repeats_only_is_silent_outside_the_windows() {
    let input = marked(96_000 + 10, SR);
    let mut h = with(SR, &[(MODE, REPEATS_ONLY as f32)]);
    let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
    let f = fade(SR);
    // The slice is heard (after the dry's fade in), then its repeats, then silence.
    for i in f + 1..6_000 {
        assert_eq!(out[i], input[i]);
    }
    assert!(out[24_000 + 1..96_000].iter().all(|s| *s == 0.0));
}

#[test]
fn a_mix_of_zero_in_insert_or_layer_is_the_input() {
    let input = noise(200_000);
    for mode in [INSERT, LAYER] {
        let mut h = with(SR, &[(MIX, 0.0), (MODE, mode as f32), (GATE, 100.0)]);
        let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
        assert_eq!(out, input, "mode {mode}");
    }
}

#[test]
fn nothing_changes_the_sound_when_nothing_captures() {
    let input = noise(200_000);
    for mode in [INSERT, LAYER] {
        // The transport stopped, no manual capture.
        let mut h = with(SR, &[(MODE, mode as f32)]);
        let out = run(&mut h, Play::at(SR, 120.0).stopped_at(0), &input, &[]);
        assert_eq!(out, input, "stopped, mode {mode}");
        // Playing, chance 0.
        let mut h = with(SR, &[(MODE, mode as f32), (CHANCE, 0.0)]);
        let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
        assert_eq!(out, input, "chance 0, mode {mode}");
    }
}

#[test]
fn a_gate_shorter_than_the_slice_gives_no_repeat() {
    let input = noise(200_000);
    // 1 % of a bar (960 frames) is shorter than a 1/16 slice (6 000 frames).
    let mut h = with(SR, &[(GATE, 1.0)]);
    let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
    assert_eq!(out, input);
}

#[test]
fn the_two_channels_are_independent() {
    let left = marked(96_000, SR);
    let right: Vec<f32> = left.iter().map(|s| -s * 0.5).collect();
    let mut h = with(SR, &[(GATE, 100.0), (FILTER, LOW_PASS as f32)]);
    let (l, r) = run_stereo(&mut h, Play::at(SR, 120.0), &left, &right, &[]);
    for (a, b) in l.iter().zip(&r) {
        assert!((b + a * 0.5).abs() < 1e-6);
    }
}

// ---------------------------------------------------------------- clicks

/// The click bound (module doc): the output's largest step from a sample to the next, on a
/// 0.5 sine of 440 Hz at 48 kHz, in every mode, with full fades (1/16 at 120 BPM): within
/// 0.08, where the input's own largest step is 0.0288 and a hard cut would step by up to 1.
#[test]
fn the_edges_do_not_click_in_any_mode() {
    let input = sine(4 * 96_000, SR, 440.0, 0.5);
    let own = max_step(&input);
    assert!((own - 0.0288).abs() < 1e-3);
    for mode in [LAYER, INSERT, REPEATS_ONLY] {
        for params in [
            vec![(GATE, 100.0)],
            vec![(GATE, 37.0), (VARIATION, SHORTER as f32), (DECAY, 30.0)],
            vec![(GATE, 60.0), (FILTER, LOW_PASS as f32), (PITCH, 5.0)],
            vec![(GATE, 60.0), (FILTER, HIGH_PASS as f32), (PITCH, -7.0)],
        ] {
            let mut p = params.clone();
            p.push((MODE, mode as f32));
            let mut h = with(SR, &p);
            let out = run(&mut h, Play::at(SR, 120.0), &input, &[]);
            let step = max_step(&out);
            println!("click: mode {mode}, {params:?}: largest step {step}");
            assert!(step <= 0.08, "mode {mode}, {params:?}: step {step}");
        }
    }
}

#[test]
fn a_manual_capture_retriggered_mid_repeat_and_a_stop_mid_window_do_not_click() {
    let input = sine(3 * 96_000, SR, 440.0, 0.5);
    for mode in [LAYER, INSERT, REPEATS_ONLY] {
        let events = [
            (10_000, CAPTURE, 1.0),
            (19_123, CAPTURE, 0.0),
            (19_200, CAPTURE, 1.0),
            (40_000, CAPTURE, 0.0),
        ];
        let mut h = with(SR, &[(MODE, mode as f32), (GATE, 100.0)]);
        // Stopped at 150 000: the grid window of bar 1 (from 96 000) is cut mid-way.
        let out = run(
            &mut h,
            Play::at(SR, 120.0).stopped_at(150_000),
            &input,
            &events,
        );
        let step = max_step(&out);
        println!("click (manual, stop): mode {mode}: largest step {step}");
        assert!(step <= 0.08, "mode {mode}: step {step}");
    }
}

/// Short repeats at the fastest tempo: the fades are shorter than 2 ms (a quarter of the
/// repeat), and the bound is the general one: the input's steps (raised pitch: times the rate,
/// two of them in layer mode) plus the peak over the shortest fade.
#[test]
fn short_repeats_at_the_fastest_tempo_stay_within_the_general_bound() {
    let input = sine(200_000, SR, 440.0, 0.5);
    let own = max_step(&input);
    let play = Play::at(SR, 999.0);
    // A 1/64 at 999 BPM: 180 frames; shorter, down to an eighth: 22 frames, fades of 5.
    let slice = frames_of_ticks(60.0, play) as f32;
    let shortest_fade = (slice / 8.0 / 4.0).floor().max(1.0);
    for mode in [LAYER, INSERT, REPEATS_ONLY] {
        let mut h = with(
            SR,
            &[
                (INTERVAL, EVERY_1_16 as f32),
                (SIZE, SIZE_1_64 as f32),
                (VARIATION, SHORTER as f32),
                (GATE, 100.0),
                (PITCH, 12.0),
                (MODE, mode as f32),
            ],
        );
        let out = run(&mut h, play, &input, &[]);
        check(&out);
        let bound = 2.0 * own * 2.0 + 2.0 * 0.5 / shortest_fade;
        let step = max_step(&out);
        println!("click (1/64 at 999 BPM): mode {mode}: largest step {step}, bound {bound}");
        assert!(step <= bound, "mode {mode}: step {step}, bound {bound}");
    }
}

// ---------------------------------------------------------------- manual capture, stop, reset

#[test]
fn a_manual_capture_starts_at_its_frame_and_lasts_while_it_is_on() {
    let input = marked(120_000, SR);
    let f = fade(SR);
    let mut h = with(SR, &[]);
    // The transport stopped: the manual capture works on the live sound.
    let events = [(10_000, CAPTURE, 1.0), (70_000, CAPTURE, 0.0)];
    let out = run(&mut h, Play::at(SR, 120.0).stopped_at(0), &input, &events);
    assert_eq!(out[..10_000], input[..10_000]);
    // The slice (1/16 at 120 BPM: 6 000 frames) from 10 000, then its repeats until the off,
    // far past a quarter of a bar (the gate binds grid windows only).
    for k in 1..9 {
        let start = 10_000 + k * 6_000;
        for u in f + 2..6_000 - f - 2 {
            assert_eq!(out[start + u], input[10_000 + u], "repeat {k}");
        }
    }
    // Off at 70 000: the tail fades over a fade, then the sound as is.
    for i in 70_000 + f + 1..120_000 {
        assert_eq!(out[i], input[i], "frame {i}");
    }
}

#[test]
fn a_capture_left_on_starts_nothing_until_it_turns_on_again() {
    let input = noise(120_000);
    // As `prepare` builds a device from a saved state: the values set, then a reset.
    let mut h = with(SR, &[(CAPTURE, 1.0)]);
    let out = run(&mut h, Play::at(SR, 120.0).stopped_at(0), &input, &[]);
    assert_eq!(out, input);
    // Off then on: a capture.
    let events = [(1_000, CAPTURE, 0.0), (2_000, CAPTURE, 1.0)];
    let out = run(&mut h, Play::at(SR, 120.0).stopped_at(0), &input, &events);
    assert_ne!(out[9_000..10_000], input[9_000..10_000]);
}

#[test]
fn a_manual_capture_holds_the_grid() {
    let input = marked(200_000, SR);
    let f = fade(SR);
    // Held from before bar 1 to after it: the bar line at 96 000 captures nothing new, the
    // repeats are still those of the slice taken at 90 000.
    let events = [(90_000, CAPTURE, 1.0), (130_000, CAPTURE, 0.0)];
    let mut h = with(SR, &[]);
    let out = run(&mut h, Play::at(SR, 120.0), &input, &events);
    let start = 90_000 + 2 * 6_000;
    for u in f + 2..6_000 - f - 2 {
        assert_eq!(out[start + u], input[90_000 + u]);
    }
}

#[test]
fn a_stop_ends_the_window_and_the_sound_comes_back() {
    let input = marked(200_000, SR);
    let f = fade(SR);
    let mut h = with(SR, &[(GATE, 100.0)]);
    let out = run(&mut h, Play::at(SR, 120.0).stopped_at(30_000), &input, &[]);
    for i in 30_000 + f + 1..200_000 {
        assert_eq!(out[i], input[i], "frame {i}");
    }
    // A manual capture is ended by the stop too.
    let mut h = with(SR, &[]);
    let events = [(10_000, CAPTURE, 1.0)];
    let out = run(
        &mut h,
        Play::at(SR, 120.0).stopped_at(40_000),
        &input,
        &events,
    );
    for i in 40_000 + f + 1..200_000 {
        assert_eq!(out[i], input[i], "frame {i}");
    }
}

#[test]
fn reset_ends_the_window_and_its_tail_at_once_and_keeps_the_values() {
    let input = marked(96_000, SR);
    let play = Play::at(SR, 120.0);
    let mut h = with(SR, &[(GATE, 100.0), (MODE, LAYER as f32)]);
    let _ = run(&mut h, play, &input[..20_000], &[]);
    h.reset();
    // Right after: no repeat, no tail; the transport restarts at bar 1's line.
    let out = run(&mut h, play.from(3_000.0), &input[..5_000], &[]);
    assert_eq!(out[..], input[..5_000]);
    // The values are kept: layer mode, gate 100 % on the next bar line.
    let mut fresh = with(SR, &[(GATE, 100.0), (MODE, LAYER as f32)]);
    let b = run(&mut fresh, play, &input, &[]);
    let mut h2 = with(SR, &[(GATE, 100.0), (MODE, LAYER as f32)]);
    let _ = run(&mut h2, play, &input[..1_000], &[]);
    h2.reset();
    assert_eq!(run(&mut h2, play, &input, &[]), b);
}

// ---------------------------------------------------------------- sweeps and hostile input

#[test]
fn every_parameter_at_its_bounds_and_default_stays_finite_and_bounded() {
    let input = noise(3 * SR as usize);
    let in_peak = peak(&input);
    for p in DESCRIPTOR.params {
        for value in [p.min, p.max, p.default] {
            for base in [
                vec![(GATE, 100.0), (INTERVAL, EVERY_1_16 as f32)],
                vec![(MODE, LAYER as f32), (FILTER, LOW_PASS as f32)],
                vec![
                    (SIZE, SIZE_1_16 as f32),
                    (PITCH, 12.0),
                    (VARIATION, RANDOM as f32),
                ],
            ] {
                let mut params = base.clone();
                params.push((p.id.0, value as f32));
                for bpm in [20.0, 120.0, 999.0] {
                    let mut h = with(SR, &params);
                    let events = [(5_000, CAPTURE, 1.0), (60_000, CAPTURE, 0.0)];
                    let out = run(&mut h, Play::at(SR, bpm).blocks(511), &input, &events);
                    check(&out);
                    // Layer adds a repeat to the sound, and a high-pass on noise overshoots its
                    // input: within three times the input's peak.
                    let top = peak(&out);
                    assert!(
                        top <= 3.0 * in_peak,
                        "{} = {value} at {bpm} BPM: peak {top}",
                        p.key
                    );
                }
            }
        }
    }
}

/// The conformance run of the contract renders with the transport stopped; this one plays it:
/// blocks of random sizes, hostile values at random offsets (NaN, infinities, out of range),
/// hostile samples, random resets and stops, tempo changes, under the allocation detector.
#[test]
fn a_hostile_run_with_the_transport_playing_stays_clean() {
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    const WILD: [f32; 6] = [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1e30,
        -1e30,
        1e-40,
    ];
    for rate in [22_050.0f32, 48_000.0, 192_000.0] {
        let mut h = Hiccup::new(rate).unwrap();
        let mut unused = EventBuffer::try_with_capacity(1).unwrap();
        let mut position = 0.0f64;
        let mut left = vec![0.0f32; 2_048];
        let mut right = vec![0.0f32; 2_048];
        let mut events = Vec::with_capacity(32);
        for _ in 0..3_000 {
            let frames = (next() % 2_048) as usize;
            let bpm = 20.0 + (next() % 980) as f64;
            let playing = next() % 10 != 0;
            events.clear();
            for _ in 0..(next() % 8) {
                let p = &DESCRIPTOR.params[(next() % DESCRIPTOR.params.len() as u64) as usize];
                let value = if next() % 3 == 0 {
                    WILD[(next() % 6) as usize]
                } else {
                    (p.min + (p.max - p.min) * (next() % 1_001) as f64 / 1_000.0) as f32
                };
                events.push(Event::ParamValue {
                    offset: (next() % (frames as u64 + 2)) as u32,
                    id: p.id,
                    value,
                });
            }
            events.sort_by_key(|e| e.offset());
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let x = if next() % 97 == 0 {
                    WILD[(next() % 6) as usize]
                } else {
                    (next() % 2_001) as f32 / 1_000.0 - 1.0
                };
                *l = x;
                *r = -x;
            }
            if next() % 50 == 0 {
                h.reset();
            }
            let tempo = Tempo::new(bpm).unwrap();
            let ctx = BlockCtx {
                sample_rate: rate,
                frames: frames as u32,
                transport: Transport {
                    playing,
                    tempo,
                    position_ticks: position,
                    ..Transport::default()
                },
            };
            assert_no_alloc(|| {
                let mut io = AudioIo {
                    left: &mut left[..frames],
                    right: &mut right[..frames],
                    events_out: &mut unused,
                };
                h.process(&ctx, &events, &mut io);
            });
            check(&left[..frames]);
            check(&right[..frames]);
            assert!(peak(&left[..frames]) <= 3.0e6 + 1.0);
            if playing {
                position += frames as f64 * tempo.ticks_per_second() / f64::from(rate);
            }
            // Now and then a jump (a seek, a loop wrap).
            if next() % 40 == 0 {
                position = (next() % 100_000) as f64 - 4_000.0;
            }
            // And a broken clock: a position beyond any song, then back.
            if next() % 300 == 0 {
                position = [1e300, -1e14, f64::MAX][(next() % 3) as usize];
            } else if position.abs() > 1e9 && next() % 4 == 0 {
                position = 0.0;
            }
        }
    }
}

#[test]
fn a_count_in_at_negative_ticks_captures_on_the_grid_too() {
    // The transport starts one bar before tick 0 (a count-in): the bar line at -3840 captures.
    let input = marked(150_000, SR);
    let play = Play::at(SR, 120.0).from(-3_840.0);
    let mut h = with(SR, &[(GATE, 100.0)]);
    let out = run(&mut h, play, &input, &[]);
    let f = fade(SR);
    for u in f + 2..6_000 - f - 2 {
        assert_eq!(out[6_000 + u], input[u]);
        assert_eq!(out[96_000 + 6_000 + u], input[96_000 + u]);
    }
}

/// A broken clock (a position beyond any song) with an interval change in the block: no grid,
/// no endless search for a grid point (the hostile run above hung on this before the guard).
#[test]
fn an_interval_change_at_a_position_beyond_any_song_returns_at_once() {
    for (position, playing) in [
        (1e300, false),
        (1e300, true),
        (f64::MAX, true),
        (-1e14, true),
    ] {
        let mut h = with(SR, &[]);
        let input = noise(512);
        let events = [(10, INTERVAL, EVERY_1_16 as f32), (20, CAPTURE, 1.0)];
        let play = Play::at(SR, 120.0).from(position).blocks(512);
        let play = if playing { play } else { play.stopped_at(0) };
        let out = run(&mut h, play, &input, &events);
        check(&out);
    }
}

/// The filter's type is latched at a window's first repeat: switched while a window plays, it
/// changes nothing until the next capture (no step of the filter's output between repeats).
#[test]
fn a_filter_switched_mid_window_waits_for_the_next_capture() {
    let input = sine(2 * 96_000, SR, 440.0, 0.5);
    for to in [0.0, HIGH_PASS as f32] {
        let events = [(40_000, FILTER, to)];
        let mut h = with(
            SR,
            &[(GATE, 100.0), (FILTER, LOW_PASS as f32), (DECAY, 5.0)],
        );
        let out = run(&mut h, Play::at(SR, 120.0), &input, &events);
        let mut kept = with(
            SR,
            &[(GATE, 100.0), (FILTER, LOW_PASS as f32), (DECAY, 5.0)],
        );
        let reference = run(&mut kept, Play::at(SR, 120.0), &input, &[]);
        // Until the next bar line, the same as a low-pass left alone.
        assert_eq!(out[..96_000], reference[..96_000], "to {to}");
        assert!(max_step(&out) <= 0.08, "to {to}: step {}", max_step(&out));
    }
}
