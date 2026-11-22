//! The noise gate, `faulk.sluice` (#158): offline renders through the device contract, every
//! `process` under the allocation detector. Asserts on the signal: the depth measured under the
//! closing threshold, the hysteresis (no flutter between the two thresholds, counted), the
//! attack and release times measured on the gain (within one block of 128 frames) and without
//! a jump, the hold, the inverted mode, the detection filters, the look-ahead and the latency it
//! declares (measured on an impulse at several rates), no NaN and no denormal with every
//! parameter swept to its bounds, the block-size invariance, the reset and the external
//! detection source, alone and through the engine (a pad gated by a hi-hat track). The
//! contract's conformance run on every device of the registry is in `osc_synth.rs`; the
//! native/WASM parity is `crates/engine-wasm/tests/sluice_parity.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::testing::{TONE, tone};
use engine::device::{AudioIo, BlockCtx, Device, DeviceSetup, Event, EventBuffer, Key, Transport};
use engine::devices::sluice::{
    self, ATTACK, CLOSE, DEPTH, DEPTH_MAX_DB, DESCRIPTOR, HOLD, HPF, INVERT, LOOKAHEAD, LPF,
    RELEASE, STATE_READOUT, Sluice, THRESHOLD,
};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state};

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

/// A gate at `rate` with these values, the ramps snapped (so a test starts at its values).
fn with_rate(rate: f32, params: &[(u32, f32)]) -> Sluice {
    let mut g = Sluice::new(rate).unwrap();
    for (id, value) in params {
        g.set_param(ParamId(*id), *value);
    }
    g.reset();
    g
}

fn with(params: &[(u32, f32)]) -> Sluice {
    with_rate(SR, params)
}

/// Runs `input` (left, right) through the device in blocks of `size`, the detection listening
/// to `key` when there is one, `events` applying at their (absolute) frame. The state read
/// after each block is returned with the output.
fn run_full(
    device: &mut Sluice,
    rate: f32,
    input: (&[f32], &[f32]),
    key: Option<(&[f32], &[f32])>,
    size: usize,
    events: &[(usize, Event)],
) -> (Vec<f32>, Vec<f32>, Vec<(f32, f32)>) {
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let (mut out_l, mut out_r, mut seen) = (Vec::new(), Vec::new(), Vec::new());
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
        let key_block = key.map(|(kl, kr)| Key {
            left: &kl[start..end],
            right: &kr[start..end],
        });
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left,
                right: &mut right,
                events_out: &mut unused,
            };
            device.process_keyed(&ctx, &local, &mut io, key_block);
        });
        out_l.extend(left);
        out_r.extend(right);
        let (_, state) = device.readout().unwrap();
        seen.push((reduction_of(device), state));
        start = end;
    }
    (out_l, out_r, seen)
}

fn run(
    device: &mut Sluice,
    input: (&[f32], &[f32]),
    size: usize,
    events: &[(usize, Event)],
) -> (Vec<f32>, Vec<f32>) {
    let (l, r, _) = run_full(device, SR, input, None, size, events);
    (l, r)
}

fn mono(device: &mut Sluice, input: &[f32]) -> Vec<f32> {
    run(device, (input, input), BLOCK, &[]).0
}

/// The output of `device` with `input` as its own detection, left channel.
fn keyed(device: &mut Sluice, input: &[f32], key: &[f32], size: usize) -> Vec<f32> {
    run_full(device, SR, (input, input), Some((key, key)), size, &[]).0
}

fn sine(len: usize, hz: f32, amplitude: f32) -> Vec<f32> {
    (0..len)
        .map(|n| amplitude * (core::f32::consts::TAU * hz * n as f32 / SR).sin())
        .collect()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

fn amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn rms_db(x: &[f32]) -> f32 {
    db((x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt())
}

fn noise(len: usize, seed: u32) -> Vec<f32> {
    let mut s = seed.max(1);
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

/// The reduction the gate's readout reports, in decibels; the strip's value stays 0.
fn reduction_of(g: &Sluice) -> f32 {
    assert_eq!(
        g.gain_reduction_db(),
        0.0,
        "the strip's reduction is not the gate's"
    );
    let (_, state) = g.readout().unwrap();
    if state >= 0.0 { state } else { -1.0 - state }
}

/// Whether a readout state says the gate lets the sound through.
fn passes(state: f32) -> bool {
    state >= 0.0
}

#[test]
fn the_descriptor_is_stable_and_the_device_starts_at_its_defaults() {
    assert_eq!(DESCRIPTOR.kind, "faulk.sluice");
    assert_eq!(DESCRIPTOR.name, "Sluice");
    assert_eq!(DESCRIPTOR.kind_id.0, 18);
    let takes_source = DESCRIPTOR.sidechain;
    assert!(takes_source, "it takes an external source");
    assert!(DESCRIPTOR.validate().is_ok());
    let ids: Vec<u32> = DESCRIPTOR.params.iter().map(|p| p.id.0).collect();
    assert_eq!(ids, (0..10).collect::<Vec<_>>(), "the ids are stable");
    let keys: Vec<&str> = DESCRIPTOR.params.iter().map(|p| p.key).collect();
    assert_eq!(
        keys,
        [
            "threshold",
            "close",
            "depth",
            "attack",
            "hold",
            "release",
            "invert",
            "hpf",
            "lpf",
            "lookahead"
        ]
    );
    // The registry holds it under its kind id.
    assert_eq!(
        engine::devices::builtin(DESCRIPTOR.kind_id).map(|b| b.descriptor.kind),
        Some("faulk.sluice")
    );
    let g = Sluice::new(SR).unwrap();
    assert_eq!(g.latency(), 0);
    assert_eq!(reduction_of(&g), DEPTH_MAX_DB as f32, "closed at start");
    // The readout is not a parameter: no control is drawn for it.
    assert!(DESCRIPTOR.param(ParamId(STATE_READOUT)).is_none());
    assert_eq!(g.readout().unwrap().0, ParamId(STATE_READOUT));
}

/// A gate with no look-ahead: the depth under the closing threshold, measured. A 1 kHz sine at
/// -50 dB, under the closing threshold (-36): the gate stays closed and the output is the input
/// turned down by the depth.
#[test]
fn a_signal_under_the_closing_threshold_is_turned_down_by_the_depth() {
    let input = sine(48_000, 1000.0, amp(-50.0));
    for depth in [3.0, 6.0, 12.0, 24.0, 40.0, 60.0, 79.0] {
        let mut g = with(&[(DEPTH, depth)]);
        let out = mono(&mut g, &input);
        let tail = 24_000..48_000;
        let turned_down = rms_db(&input[tail.clone()]) - rms_db(&out[tail]);
        assert!(
            (turned_down - depth).abs() < 0.05,
            "depth {depth}: the output is {turned_down} dB under the input"
        );
        assert!((reduction_of(&g) - depth).abs() < 1e-3);
        assert!(!passes(g.readout().unwrap().1), "reported closed");
    }
    // At the largest depth a closed gate is exactly silent.
    let mut g = with(&[(DEPTH, DEPTH_MAX_DB as f32)]);
    let out = mono(&mut g, &input);
    assert!(out.iter().all(|s| *s == 0.0), "silence");
    // At depth 0 nothing is turned down: the input comes out as it went in.
    let mut g = with(&[(DEPTH, 0.0)]);
    assert_eq!(mono(&mut g, &input), input);
}

/// Open, the gate is transparent: the very samples of the input.
#[test]
fn an_open_gate_passes_the_input_unchanged() {
    let input = sine(48_000, 440.0, amp(-6.0));
    let mut g = with(&[]);
    let out = mono(&mut g, &input);
    // The attack (1 ms) finishes in the first 100 ms; after it the samples are the input's.
    assert_eq!(&out[4_800..], &input[4_800..]);
    assert!(passes(g.readout().unwrap().1));
    assert_eq!(reduction_of(&g), 0.0);
}

/// A level that swings between the two thresholds does not make the gate flutter: after it has
/// opened it stays open, and closed it stays closed; with one threshold the same level does.
#[test]
fn a_level_between_the_thresholds_makes_no_flutter() {
    let len = 96_000;
    // A 1 kHz sine whose level swings between -38 and -32 dB at 7 Hz.
    let swing: Vec<f32> = (0..len)
        .map(|n| {
            let t = n as f32 / SR;
            let level = -35.0 + 3.0 * (core::f32::consts::TAU * 7.0 * t).sin();
            amp(level) * (core::f32::consts::TAU * 1000.0 * t).sin()
        })
        .collect();
    // Count the transitions of the state read after every 8 frames.
    let transitions = |g: &mut Sluice, input: &[f32]| {
        let (_, _, seen) = run_full(g, SR, (input, input), None, 8, &[]);
        // From the second half on: the opening burst is over.
        let from = seen.len() / 2;
        seen[from..]
            .windows(2)
            .filter(|w| passes(w[0].1) != passes(w[1].1))
            .count()
    };
    // Opened by a loud start (-20 dB), then the swing between close -40 and open -30.
    let mut loud_start = sine(4_800, 1000.0, amp(-20.0));
    loud_start.extend(&swing);
    let mut g = with(&[(THRESHOLD, -30.0), (CLOSE, -40.0), (HOLD, 0.0)]);
    assert_eq!(transitions(&mut g, &loud_start), 0, "open stays open");
    assert!(passes(g.readout().unwrap().1));
    // The same swing on a gate that starts closed never opens: it never reached -30.
    let mut g = with(&[(THRESHOLD, -30.0), (CLOSE, -40.0), (HOLD, 0.0)]);
    assert_eq!(transitions(&mut g, &swing), 0, "closed stays closed");
    assert!(!passes(g.readout().unwrap().1));
    // The control: with the closing threshold at the opening one, the same swing around it
    // flutters (this is what the hysteresis prevents). The swing -38..-32 around -35.
    let mut g = with(&[(THRESHOLD, -35.0), (CLOSE, -35.0), (HOLD, 0.0)]);
    let flutter = transitions(&mut g, &loud_start);
    assert!(
        flutter >= 8,
        "one threshold flutters: {flutter} transitions"
    );
}

/// A closing threshold above the opening one counts as equal to it.
#[test]
fn a_closing_threshold_above_the_opening_one_is_taken_as_the_opening_one() {
    let input: Vec<f32> = sine(24_000, 500.0, amp(-26.0))
        .into_iter()
        .chain(sine(24_000, 500.0, amp(-34.0)))
        .collect();
    let mut a = with(&[(THRESHOLD, -30.0), (CLOSE, 0.0), (HOLD, 0.0)]);
    let mut b = with(&[(THRESHOLD, -30.0), (CLOSE, -30.0), (HOLD, 0.0)]);
    assert_eq!(mono(&mut a, &input), mono(&mut b, &input));
}

/// A key (the detection) and a steady input at 0.3: the output is 0.3 times the gain, so the
/// gain is read straight off the output.
fn gain_of(key: &[f32], params: &[(u32, f32)], size: usize) -> Vec<f32> {
    let input = vec![0.3; key.len()];
    let mut g = with(params);
    keyed(&mut g, &input, key, size)
        .iter()
        .map(|s| s / 0.3)
        .collect()
}

/// The frames the gain takes to go from one end to the other of its range around `at`.
fn ramp_frames(gain: &[f32], from: usize, rising: bool, depth_db: f32) -> usize {
    let low = amp(-depth_db);
    let (start, end) = if rising {
        let s = (from..gain.len())
            .find(|&i| gain[i] > low * 1.0001)
            .unwrap();
        let e = (s..gain.len()).find(|&i| gain[i] >= 0.9999).unwrap();
        (s, e)
    } else {
        let s = (from..gain.len()).find(|&i| gain[i] < 0.9999).unwrap();
        let e = (s..gain.len()).find(|&i| gain[i] <= low * 1.0001).unwrap();
        (s, e)
    };
    end - start + 1
}

/// The attack and the release are the times set, within one block, and the gain moves without
/// a jump. The detection is a key (a burst), the input a steady 0.3.
#[test]
fn the_attack_and_the_release_last_as_set_within_a_block_without_a_click() {
    let depth = 40.0;
    for (attack, release) in [(0.002, 0.005), (0.01, 0.02), (0.03, 0.2), (0.0001, 0.001)] {
        let len = 48_000 * 2;
        // Silence, a burst of 0.5 s at -10 dB, silence.
        let mut key = vec![0.0; len];
        for k in key.iter_mut().take(72_000).skip(12_000) {
            *k = amp(-10.0);
        }
        let gain = gain_of(
            &key,
            &[
                (DEPTH, depth),
                (ATTACK, attack),
                (RELEASE, release),
                (HOLD, 0.0),
            ],
            BLOCK,
        );
        let open = ramp_frames(&gain, 10_000, true, depth);
        let want = (attack * SR).round() as usize;
        assert!(
            open.abs_diff(want) <= BLOCK,
            "attack {attack}: {open} frames, set {want}"
        );
        let close = ramp_frames(&gain, 72_000, false, depth);
        let want = (release * SR).round() as usize;
        assert!(
            close.abs_diff(want) <= BLOCK,
            "release {release}: {close} frames, set {want}"
        );
        // No jump: a step of the gain is the depth over the time, in decibels, per frame.
        let worst = gain
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        let steepest = attack.min(release) * SR;
        let bound = 0.1152 * depth / steepest.max(1.0) * 1.01;
        assert!(
            worst <= bound.clamp(0.0, 1.0) + 1e-6,
            "a jump of {worst} (bound {bound})"
        );
    }
}

/// An attack under a frame is one step (the gain is up on the next frame): bounded, no NaN.
#[test]
fn an_attack_under_a_frame_is_one_step() {
    let mut key = vec![0.0; 4_800];
    for k in key.iter_mut().skip(2_000) {
        *k = amp(-10.0);
    }
    let gain = gain_of(&key, &[(ATTACK, 0.00001), (HOLD, 0.0)], BLOCK);
    assert!(gain[1_999] < 1e-3, "closed before the burst");
    assert!(
        gain[2_000] > 0.999,
        "open on the burst's first frame: one step"
    );
}

/// The hold keeps the gate open for `hold` after the level falls under the closing threshold.
#[test]
fn the_gate_stays_open_for_the_hold() {
    let len = 48_000 * 2;
    let mut key = vec![0.0; len];
    for k in key.iter_mut().take(12_000).skip(2_000) {
        *k = amp(-10.0);
    }
    let first_closing = |hold: f32| {
        let gain = gain_of(
            &key,
            &[
                (HOLD, hold),
                (RELEASE, 0.001),
                (ATTACK, 0.0005),
                (DEPTH, 40.0),
            ],
            BLOCK,
        );
        (12_000..len).find(|&i| gain[i] < 0.5).unwrap() - 12_000
    };
    let none = first_closing(0.0);
    // The peak follower's fall to -36 dB from -10 takes at most 8 ms * ln(20) = 24 ms.
    assert!(
        none < (0.03 * SR) as usize,
        "no hold closes within 30 ms: {none}"
    );
    for hold in [0.05f32, 0.2, 1.0] {
        let late = first_closing(hold);
        let held = (hold * SR) as usize;
        assert!(
            late >= held && late <= held + (0.03 * SR) as usize + BLOCK,
            "hold {hold}: closes {late} frames after the burst"
        );
    }
}

/// Inverted, the gate turns the sound down while the detection is above the threshold.
#[test]
fn the_inverted_gate_turns_the_sound_down_above_the_threshold() {
    let len = 48_000 * 2;
    let mut key = vec![0.0; len];
    for k in key.iter_mut().take(72_000).skip(24_000) {
        *k = amp(-10.0);
    }
    let gain = gain_of(
        &key,
        &[
            (INVERT, 1.0),
            (DEPTH, 30.0),
            (ATTACK, 0.002),
            (RELEASE, 0.01),
            (HOLD, 0.0),
        ],
        BLOCK,
    );
    assert!(gain[10_000] > 0.9999, "passes under the threshold");
    assert!(
        (db(gain[60_000]) + 30.0).abs() < 0.01,
        "turned down by the depth above it: {}",
        db(gain[60_000])
    );
    assert!(gain[95_000] > 0.9999, "passes again after it");
    // The state tells it: closed while the key is loud.
    let mut g = with(&[(INVERT, 1.0)]);
    assert!(passes(g.readout().unwrap().1), "open at rest");
    let input = vec![0.3; len];
    let (_, _, seen) = run_full(&mut g, SR, (&input, &input), Some((&key, &key)), BLOCK, &[]);
    assert!(passes(seen[10].1) && !passes(seen[300].1) && passes(seen[700].1));
}

/// The detection filters: a 50 Hz key does not open a gate that listens above 1 kHz, a 10 kHz
/// one does not open a gate that listens under 500 Hz; the filters at their off ends change
/// nothing.
#[test]
fn the_detection_filters_decide_what_the_gate_listens_to() {
    let input = vec![0.3; 48_000];
    let low = sine(48_000, 50.0, amp(-10.0));
    let high = sine(48_000, 10_000.0, amp(-10.0));
    let closed = |params: &[(u32, f32)], key: &[f32]| {
        let mut g = with(params);
        let out = keyed(&mut g, &input, key, BLOCK);
        out[24_000..].iter().all(|s| *s == 0.0)
    };
    assert!(!closed(&[], &low), "unfiltered, the lows open it");
    assert!(
        closed(&[(HPF, 1000.0)], &low),
        "a 1 kHz high-pass shuts the lows out"
    );
    assert!(!closed(&[(HPF, 1000.0)], &high), "and lets the highs in");
    assert!(!closed(&[], &high));
    assert!(
        closed(&[(LPF, 500.0)], &high),
        "a 500 Hz low-pass shuts the highs out"
    );
    assert!(!closed(&[(LPF, 500.0)], &low), "and lets the lows in");
    // At the off ends the filters are not there: the very samples of an unfiltered gate.
    let a = keyed(&mut with(&[]), &input, &low, BLOCK);
    let b = keyed(
        &mut with(&[(HPF, 20.0), (LPF, 20_000.0)]),
        &input,
        &low,
        BLOCK,
    );
    assert_eq!(a, b);
    // The sound itself is not filtered: an open gate gives the input's very samples.
    let program = sine(48_000, 50.0, amp(-6.0));
    let mut g = with(&[
        (HPF, 1000.0),
        (LPF, 5_000.0),
        (THRESHOLD, -80.0),
        (CLOSE, -80.0),
    ]);
    let loud = sine(48_000, 3_000.0, amp(-10.0));
    let out = keyed(&mut g, &program, &loud, BLOCK);
    assert_eq!(&out[24_000..], &program[24_000..]);
}

/// The look-ahead: the declared latency is the impulse's delay, at several rates; the gate is
/// already open when a hit arrives.
#[test]
fn the_declared_latency_is_the_measured_one() {
    for rate in [8_000.0f32, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for seconds in [0.0f32, 0.0005, 0.002, 0.005] {
            let mut g = with_rate(rate, &[(LOOKAHEAD, seconds), (HOLD, 2.0)]);
            let want = sluice::lookahead_frames(seconds, rate);
            assert_eq!(g.latency(), want, "{rate} Hz, {seconds} s");
            if rate == 48_000.0 && seconds == 0.005 {
                assert_eq!(want, 240);
            }
            assert_eq!(g.latency_frames(), want);
            assert!(g.latency_frames() <= g.max_latency_frames());
            // A steady 0.1 holds the gate open; an impulse of +0.5 on top at frame 5000.
            let len = (rate * 0.5) as usize;
            let mut input = vec![0.1; len];
            let at = len / 2;
            input[at] += 0.5;
            let (out, _, _) = run_full(&mut g, rate, (&input, &input), None, BLOCK, &[]);
            let peak = (0..len)
                .max_by(|a, b| out[*a].partial_cmp(&out[*b]).unwrap())
                .unwrap();
            assert_eq!(peak, at + want as usize, "{rate} Hz, {seconds} s");
            assert!((out[peak] - 0.6).abs() < 1e-6);
        }
    }
}

/// With a look-ahead the gate is open on the first sample of a hit that its attack would
/// otherwise have cut.
#[test]
fn a_look_ahead_longer_than_the_attack_lets_the_first_sample_through() {
    let mut input = vec![0.0; 9_600];
    for s in input.iter_mut().skip(4_800) {
        *s = 0.3;
    }
    let first = |lookahead: f32| {
        let mut g = with(&[(LOOKAHEAD, lookahead), (ATTACK, 0.002), (DEPTH, 40.0)]);
        let out = mono(&mut g, &input);
        let at = 4_800 + g.latency() as usize;
        out[at]
    };
    assert!(
        first(0.0) < 0.01,
        "without it the hit's first sample is cut: {}",
        first(0.0)
    );
    assert!((first(0.003) - 0.3).abs() < 1e-6, "with 3 ms it is whole");
}

/// A change of look-ahead while the gate sounds goes through the latency switch: the latency
/// stays, then changes once the gesture is over, and the output stays finite and bounded.
#[test]
fn a_change_of_look_ahead_switches_the_latency_seamlessly() {
    let mut g = with(&[(THRESHOLD, -80.0), (CLOSE, -80.0)]);
    let input = sine(48_000, 220.0, 0.5);
    let events = [(1_000, param(LOOKAHEAD, 0.004))];
    let (out, _, _) = run_full(&mut g, SR, (&input, &input), None, 64, &events);
    assert_eq!(g.latency(), (0.004 * SR) as u32, "taken after the settle");
    assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 0.5));
    // At once before any frame.
    let mut g = Sluice::new(SR).unwrap();
    g.set_param(ParamId(LOOKAHEAD), 0.002);
    assert_eq!(g.latency(), 96);
    // A change back to the one in use cancels the switch.
    let mut g = with(&[(LOOKAHEAD, 0.002)]);
    let events = [
        (100, param(LOOKAHEAD, 0.004)),
        (200, param(LOOKAHEAD, 0.002)),
    ];
    let silence = vec![0.0; 24_000];
    run_full(&mut g, SR, (&silence, &silence), None, 64, &events);
    assert_eq!(g.latency(), 96);
}

/// A rate above the one the line is sized for, and a rate that is not usable.
#[test]
fn an_extreme_sample_rate_keeps_the_look_ahead_within_the_line() {
    for rate in [384_000.0f32, 1_000_000.0] {
        let mut g = with_rate(rate, &[(LOOKAHEAD, 0.005)]);
        assert!(g.latency() <= g.max_latency_frames());
        let input = noise(2_000, 9);
        let (out, _, _) = run_full(&mut g, rate, (&input, &input), None, 100, &[]);
        assert!(out.iter().all(|s| s.is_finite()));
    }
    for rate in [f32::NAN, 0.0, 100.0, f32::INFINITY] {
        let mut g = with_rate(rate, &[(LOOKAHEAD, 0.005)]);
        // The fallback is 48 kHz.
        assert_eq!(g.latency(), 240, "{rate}");
        let input = noise(2_000, 9);
        let (out, _, _) = run_full(&mut g, 48_000.0, (&input, &input), None, 100, &[]);
        assert!(out.iter().all(|s| s.is_finite()));
    }
}

/// Every parameter at its bounds and its default, on every kind of signal: finite, no
/// denormal, never louder than the delayed input.
#[test]
fn every_parameter_at_its_bounds_stays_finite_and_bounded() {
    let bounds: Vec<(u32, [f32; 2])> = DESCRIPTOR
        .params
        .iter()
        .map(|p| (p.id.0, [p.min as f32, p.max as f32]))
        .collect();
    assert_eq!(bounds.len(), 10);
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
    let mut impulses = vec![0.0; 2_000];
    impulses[100] = 1.0;
    impulses[700] = -1.0;
    let signals: [(&str, Vec<f32>, f32); 4] = [
        ("noise", noise(2_000, 3), 1.0),
        ("silence", vec![0.0; 2_000], 0.0),
        ("impulses", impulses, 1.0),
        ("garbage", garbage, 1e6),
    ];
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
                    let v = match seed % 3 {
                        0 => b[0],
                        1 => b[1],
                        _ => *d,
                    };
                    let _ = k;
                    (*id, v)
                })
                .collect(),
        );
    }
    for (id, b) in &bounds {
        for v in [b[0], b[1], defaults[*id as usize]] {
            corners.push(vec![(*id, v)]);
        }
    }
    corners.push(bounds.iter().map(|(id, b)| (*id, b[0])).collect());
    corners.push(bounds.iter().map(|(id, b)| (*id, b[1])).collect());
    corners.push(Vec::new());
    let mut cases = 0;
    for corner in &corners {
        for (name, input, limit) in &signals {
            let mut g = with(corner);
            let l = g.latency() as usize;
            let (out, _, _) = run_full(&mut g, SR, (input, input), Some((input, input)), 100, &[]);
            let (out_own, _, _) = run_full(&mut with(corner), SR, (input, input), None, 100, &[]);
            for (label, out) in [("keyed", &out), ("own", &out_own)] {
                for (n, s) in out.iter().enumerate() {
                    assert!(s.is_finite(), "{label} {name} {corner:?}: {s} at {n}");
                    assert!(
                        *s == 0.0 || s.abs() >= f32::MIN_POSITIVE,
                        "{label} {name} {corner:?}: denormal {s} at {n}"
                    );
                    // Never louder than the delayed (sanitized) input.
                    let delayed = n.checked_sub(l).map_or(0.0, |k| {
                        let x = input[k];
                        if x.is_finite() {
                            x.clamp(-1e6, 1e6)
                        } else {
                            0.0
                        }
                    });
                    assert!(
                        s.abs() <= delayed.abs() * (1.0 + 1e-6) + 1e-30,
                        "{label} {name} {corner:?}: {s} over {delayed} at {n}"
                    );
                    assert!(s.abs() <= *limit);
                }
            }
            let reduction = reduction_of(&g);
            assert!((0.0..=DEPTH_MAX_DB as f32).contains(&reduction));
            let (id, state) = g.readout().unwrap();
            assert_eq!(id, ParamId(STATE_READOUT));
            assert!(state.is_finite());
            cases += 1;
        }
    }
    assert!(cases > 1_000, "{cases} cases");
}

/// The same samples whatever the size of the blocks.
#[test]
fn the_output_does_not_depend_on_the_size_of_the_blocks() {
    let mut input = sine(24_000, 300.0, amp(-6.0));
    input.extend(sine(24_000, 300.0, amp(-50.0)));
    input.extend(noise(24_000, 4).iter().map(|s| s * amp(-20.0)));
    let params = [
        (HPF, 200.0),
        (LPF, 8_000.0),
        (LOOKAHEAD, 0.003),
        (HOLD, 0.02),
        (DEPTH, 30.0),
    ];
    let reference = run(&mut with(&params), (&input, &input), BLOCK, &[]);
    for size in [1, 7, 64, 1000, 4096, 100_000] {
        assert_eq!(
            run(&mut with(&params), (&input, &input), size, &[]),
            reference,
            "block size {size}"
        );
    }
}

/// A parameter event lands on its frame, inside the block.
#[test]
fn a_parameter_event_applies_at_its_frame() {
    let input = sine(8_192, 1000.0, amp(-6.0));
    let events = [(3_000, param(INVERT, 1.0))];
    let whole = run(&mut with(&[]), (&input, &input), 8_192, &events);
    let split = run(&mut with(&[]), (&input, &input), 3_000, &events);
    assert_eq!(whole, split);
    let plain = run(&mut with(&[]), (&input, &input), 8_192, &[]);
    assert_eq!(&whole.0[..3_000], &plain.0[..3_000]);
    assert_ne!(
        &whole.0[3_100..],
        &plain.0[3_100..],
        "the inversion took effect"
    );
}

/// A reset closes the gate, rests the detector and hides the line: the same output as a
/// fresh device.
#[test]
fn a_reset_gives_a_fresh_gate() {
    let loud = sine(24_000, 500.0, amp(-6.0));
    let params = [(LOOKAHEAD, 0.002), (HPF, 300.0)];
    let mut g = with(&params);
    mono(&mut g, &loud);
    assert!(passes(g.readout().unwrap().1));
    g.reset();
    assert_eq!(reduction_of(&g), DEPTH_MAX_DB as f32);
    assert!(!passes(g.readout().unwrap().1));
    let mut fresh = with(&params);
    let probe = noise(10_000, 8);
    assert_eq!(mono(&mut g, &probe), mono(&mut fresh, &probe));
    // Inverted it opens at a reset.
    let mut g = with(&[(INVERT, 1.0)]);
    g.reset();
    assert_eq!(reduction_of(&g), 0.0);
    assert!(passes(g.readout().unwrap().1));
}

/// The readout carries the state and the reduction, and the strip's value is the reduction.
#[test]
fn the_readout_says_open_or_closed_with_the_reduction() {
    let mut g = with(&[(DEPTH, 30.0), (HOLD, 0.0), (RELEASE, 0.5)]);
    assert_eq!(g.readout().unwrap(), (ParamId(STATE_READOUT), -31.0));
    let burst = sine(4_800, 1000.0, amp(-10.0));
    mono(&mut g, &burst);
    assert_eq!(g.readout().unwrap(), (ParamId(STATE_READOUT), 0.0));
    // Silence: the trigger lets go, the gate is closing (reported closed) while the
    // reduction is still on its way to the depth.
    let quiet = vec![0.0; 4_800];
    mono(&mut g, &quiet);
    let (_, state) = g.readout().unwrap();
    let r = reduction_of(&g);
    assert!(r > 0.0 && r < 30.0, "mid-release: {r}");
    assert_eq!(state, -1.0 - r);
}

/// The detection listens to the key: a silent key keeps a gate closed on a loud input, a loud
/// key opens it on a silent-looking input; past the key's end it is silence.
#[test]
fn the_detection_listens_to_the_other_track() {
    let input = sine(48_000, 440.0, amp(-6.0));
    let silent = vec![0.0; 48_000];
    let hat = {
        let mut k = vec![0.0; 48_000];
        for pulse in 0..4 {
            for s in &mut k[pulse * 12_000 + 2_000..pulse * 12_000 + 3_000] {
                *s = amp(-12.0);
            }
        }
        k
    };
    // Own detection: the loud input opens it.
    assert!(mono(&mut with(&[]), &input).iter().any(|s| s.abs() > 0.1));
    // A silent key: closed, silent, whatever the input.
    let out = keyed(&mut with(&[]), &input, &silent, BLOCK);
    assert!(out.iter().all(|s| *s == 0.0));
    // The hat's pulses cut the input into pieces: loud while it plays, silent between.
    let params = [(ATTACK, 0.0005), (RELEASE, 0.002), (HOLD, 0.0)];
    let out = keyed(&mut with(&params), &input, &hat, BLOCK);
    let in_pulse = rms_db(&out[2_500..3_000]);
    let between = rms_db(&out[9_000..11_000]);
    assert!(in_pulse > -10.0, "{in_pulse}");
    assert!(between < -100.0, "{between}");
    // A key shorter than the block is silence past its end.
    let mut g = with(&[]);
    let (l, r) = (vec![0.3; 256], vec![0.3; 256]);
    let (kl, kr) = (vec![amp(-6.0); 100], vec![amp(-6.0); 100]);
    let mut out_l = l.clone();
    let mut out_r = r.clone();
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let ctx = BlockCtx {
        sample_rate: SR,
        frames: 256,
        transport: Transport::default(),
    };
    assert_no_alloc(|| {
        let mut io = AudioIo {
            left: &mut out_l,
            right: &mut out_r,
            events_out: &mut unused,
        };
        g.process_keyed(
            &ctx,
            &[],
            &mut io,
            Some(Key {
                left: &kl,
                right: &kr,
            }),
        );
    });
    assert!(out_l.iter().all(|s| s.is_finite()));
}

/// The external source is not latency compensated, as the Compressor's: with a look-ahead the
/// gated sound is the delayed one while the gate follows the key at its own time, so it opens
/// the look-ahead before the sound.
#[test]
fn with_a_source_and_a_look_ahead_the_gate_opens_before_the_sound() {
    let mut input = vec![0.0; 9_600];
    for s in input.iter_mut().skip(4_800) {
        *s = 0.3;
    }
    // The key's hit and the input's are at the same frame; the output has the hit L later,
    // and whole.
    let mut g = with(&[(LOOKAHEAD, 0.003), (ATTACK, 0.002), (DEPTH, 40.0)]);
    let out = keyed(&mut g, &input, &input, BLOCK);
    let l = g.latency() as usize;
    assert_eq!(l, 144);
    assert!((out[4_800 + l] - 0.3).abs() < 1e-6);
    assert!(out[4_800 + l - 1].abs() < 1e-6, "silent before it");
}

// The same through the engine: a pad (track 0) gated by a hi-hat track (track 1).

const PAD: TrackSlot = TrackSlot(0);
const HAT: TrackSlot = TrackSlot(1);
const GATE: DeviceSlot = DeviceSlot(1);

struct Rig {
    p: Preparer,
    e: Engine,
}

impl Rig {
    fn new() -> Rig {
        let setup = DeviceSetup {
            sample_rate: SR,
            max_block_frames: BLOCK,
        };
        let (p, e) = channel(setup, engine::device::testing::factory).unwrap();
        Rig { p, e }
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p.handle(m.clone()).unwrap();
        }
        self.p.flush();
    }

    fn run(&mut self, blocks: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
        for _ in 0..blocks {
            assert_no_alloc(|| self.e.process(&mut l, &mut r));
            out.extend_from_slice(&l);
            self.p.collect();
        }
        for (i, s) in out.iter().enumerate() {
            assert!(s.is_finite(), "sample {i} is {s}");
            assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
        }
        out
    }
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, s| m.max(s.abs()))
}

#[test]
fn a_pad_is_gated_by_a_hi_hat_track_through_the_engine() {
    let mut rig = Rig::new();
    let tone_on = |track| M::DeviceInsert {
        track,
        device: DeviceSlot(0),
        kind: TONE.kind_id,
        state: encode_state(&[(tone::GAIN, 1.0)]),
    };
    rig.send(&[
        M::TrackCreate {
            track: PAD,
            kind: TrackKind::Midi,
        },
        M::TrackCreate {
            track: HAT,
            kind: TrackKind::Midi,
        },
        tone_on(PAD),
        tone_on(HAT),
        M::DeviceInsert {
            track: PAD,
            device: GATE,
            kind: DESCRIPTOR.kind_id,
            state: encode_state(&[
                (ParamId(THRESHOLD), -30.0),
                (ParamId(CLOSE), -36.0),
                (ParamId(ATTACK), 0.0005),
                (ParamId(HOLD), 0.0),
                (ParamId(RELEASE), 0.005),
            ]),
        },
        M::NoteOn {
            track: PAD,
            pitch: 60,
            velocity: 127,
            at: None,
        },
    ]);
    // The pad plays alone, listening to itself: it is loud, so the gate is open.
    let own = rig.run(100);
    assert!(peak(&own[6_000..]) > 0.1, "{}", peak(&own));
    // Listening to the hat, which is silent: the pad is silent (depth 80).
    rig.send(&[M::DeviceSidechain {
        track: PAD,
        device: GATE,
        source: Some(HAT),
    }]);
    let gated = rig.run(200);
    assert_eq!(peak(&gated[12_000..]), 0.0, "silent without the hat");
    // The hat plays: the pad comes through.
    rig.send(&[M::NoteOn {
        track: HAT,
        pitch: 72,
        velocity: 127,
        at: None,
    }]);
    let open = rig.run(100);
    assert!(peak(&open[4_000..]) > 0.1, "the pad sounds with the hat");
    // The hat stops: silent again, however loud the pad.
    rig.send(&[M::NoteOff {
        track: HAT,
        pitch: 72,
        at: None,
    }]);
    let after = rig.run(200);
    assert_eq!(peak(&after[19_200..]), 0.0);
    // A muted hat still triggers (the key is tapped before the strip), as the Compressor's.
    rig.send(&[
        M::TrackSetMix {
            track: HAT,
            gain: 1.0,
            pan: 0.0,
            mute: true,
        },
        M::NoteOn {
            track: HAT,
            pitch: 72,
            velocity: 127,
            at: None,
        },
    ]);
    let muted = rig.run(100);
    assert!(peak(&muted[4_000..]) > 0.1);
    let _ = sluice::DEPTH;
}
