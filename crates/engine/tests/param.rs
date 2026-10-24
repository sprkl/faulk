//! The shared parameter smoother (AM-3): the ramp's rule on its own, then through the
//! devices that use it, rendered offline under the allocation detector. Asserts on the
//! signal: no click on a -60 dB to 0 dB gain jump (spectrum), no zipper on a fast sweep (the
//! largest sample-to-sample step against a render without the sweep), the target reached in
//! the declared time whatever the block size, a zero smoothing at the exact frame, no NaN and
//! no denormal.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::testing::{TestGain, gain};
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceSetup, Event, EventBuffer, Transport, VoiceId,
};
use engine::msg::ParamId;
use engine::param::Smoother;
use engine::{Param, Synth};
use test_support::Render;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const SR: f32 = 48_000.0;

fn setup() -> DeviceSetup {
    DeviceSetup {
        sample_rate: SR,
        max_block_frames: 2048,
    }
}

fn pv(offset: u32, id: ParamId, value: f32) -> Event {
    Event::ParamValue { offset, id, value }
}

fn sine(frames: usize, hz: f32, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / SR).sin())
        .collect()
}

/// Renders `input` through `device` in blocks of `size`; `events` carry absolute frames.
/// Every `process` runs under the allocation detector.
fn render(
    device: &mut dyn Device,
    input: &[f32],
    size: usize,
    events: &[(usize, Event)],
) -> Vec<f32> {
    let mut out = Vec::with_capacity(input.len());
    let mut unused = EventBuffer::try_with_capacity(1).unwrap();
    let mut start = 0;
    while start < input.len() {
        let frames = size.min(input.len() - start);
        let local: Vec<Event> = events
            .iter()
            .filter(|(at, _)| (start..start + frames).contains(at))
            .map(|(at, e)| e.at((at - start) as u32))
            .collect();
        let mut left = input[start..start + frames].to_vec();
        let mut right = left.clone();
        let ctx = BlockCtx {
            sample_rate: SR,
            frames: frames as u32,
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
        out.extend(left);
        start += frames;
    }
    out
}

/// The same effect as [`TestGain`] without smoothing: the gain is a hard step.
fn unsmoothed(input: &[f32], steps: &[(usize, f32)], initial: f32) -> Vec<f32> {
    let mut s = Smoother::new(initial);
    let mut next = steps.iter().peekable();
    input
        .iter()
        .enumerate()
        .map(|(i, x)| {
            while let Some((_, v)) = next.next_if(|(at, _)| *at == i) {
                s.set(*v, 0);
            }
            x * s.next_value()
        })
        .collect()
}

fn max_step(signal: &[f32], from: usize, to: usize) -> f32 {
    signal[from..to]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

fn assert_clean(signal: &[f32]) {
    for (i, s) in signal.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is not finite");
        assert!(*s == 0.0 || s.abs() >= 1e-20, "sample {i} is denormal: {s}");
    }
}

/// Energy of the spectrum above `hz` of the first power-of-two run of `signal`.
fn energy_above(signal: &[f32], hz: f32) -> f32 {
    let render = Render::new(SR, signal.to_vec(), signal.to_vec());
    let spectrum = render.spectrum(0.0..signal.len() as f32 / SR);
    spectrum
        .magnitudes
        .iter()
        .enumerate()
        .filter(|(i, _)| *i as f32 * spectrum.bin_hz > hz)
        .map(|(_, m)| m * m)
        .sum()
}

// ---------------------------------------------------------------- the ramp itself

#[test]
fn the_target_is_reached_in_the_declared_frames_whatever_the_block() {
    for n in [1u32, 7, 480, 4800] {
        for size in [1u32, 16, 128, 513, 2048] {
            // Per sample.
            let mut s = Smoother::new(0.25);
            s.set(1.0, n);
            let mut reached = None;
            for i in 0..n + 4 {
                let v = s.next_value();
                if v == 1.0 && reached.is_none() {
                    reached = Some(i);
                }
            }
            assert_eq!(reached, Some(n), "per sample, n {n}");
            // Per block: the reading at the first frame of a block, so the target shows in
            // the first block that starts at or after frame n: within one block of n.
            let mut s = Smoother::new(0.25);
            s.set(1.0, n);
            let (mut at, mut shown) = (0u32, None);
            while shown.is_none() && at < n + 2 * size {
                if s.block(size) == 1.0 {
                    shown = Some(at);
                }
                at += size;
            }
            let shown = shown.expect("reached");
            assert!(
                shown >= n && shown < n + size,
                "per block, n {n}, block {size}: shown at {shown}"
            );
            assert!(s.is_still() && s.target() == 1.0);
        }
    }
}

#[test]
fn reading_in_blocks_or_per_sample_gives_the_same_ramp() {
    let mut a = Smoother::new(0.0);
    let mut b = Smoother::new(0.0);
    a.set(1.0, 1000);
    b.set(1.0, 1000);
    let mut whole = vec![0.0f32; 1500];
    a.fill(&mut whole);
    // `block` returns the value of the first frame of the block, then moves by the whole
    // block: the sequence it gives is the per-sample one, sampled at the block starts, and
    // both end on the same frame.
    let mut at = 0usize;
    for size in [1u32, 99, 300, 2, 1000, 64, 64] {
        assert_eq!(b.block(size), whole[at], "start of the block at {at}");
        at += size as usize;
    }
    assert!(at > 1000 && b.is_still(), "past the ramp's end");
    assert_eq!(b.current(), *whole.last().unwrap());
    // Mixed with per-sample reads: the same position.
    let mut c = Smoother::new(0.0);
    c.set(1.0, 1000);
    c.advance(10);
    assert_eq!(c.next_value(), whole[10]);
    assert_eq!(c.block(5), whole[11]);
    assert_eq!(c.current(), whole[16]);
}

#[test]
fn a_zero_smoothing_applies_the_value_at_the_exact_frame() {
    let mut s = Smoother::new(0.3);
    assert_eq!(s.next_value(), 0.3);
    s.set(0.9, 0);
    assert!(s.is_still());
    assert_eq!(s.current(), 0.9);
    assert_eq!(s.block(64), 0.9);
    // Through a device: the continuous `trim` of the test effect (smoothing 0, so through
    // `Smoother::set(v, 0)`), at the exact offset, in every block size and mid-block.
    for size in [1usize, 7, 64, 256] {
        let mut g = TestGain::new(&setup());
        let out = render(
            &mut g,
            &vec![1.0; 256],
            size,
            &[(70, pv(0, gain::TRIM, 0.5))],
        );
        assert!(out[..70].iter().all(|v| *v == 1.0), "block {size}");
        assert!(out[70..].iter().all(|v| *v == 0.5), "block {size}");
    }
    // And the stepped mute.
    let mut g = TestGain::new(&setup());
    let out = render(&mut g, &vec![1.0; 256], 64, &[(70, pv(0, gain::MUTE, 1.0))]);
    assert_eq!(out[69], 1.0);
    assert!(out[70..].iter().all(|v| *v == 0.0));
}

#[test]
fn a_new_target_during_a_ramp_starts_where_the_value_is() {
    let mut s = Smoother::new(0.0);
    s.set(1.0, 100);
    for _ in 0..40 {
        s.next_value();
    }
    let before = s.current();
    s.set(0.0, 100);
    assert_eq!(s.current(), before, "no jump at the new target");
    // The same target again restarts (the contract), `aim` keeps the ramp.
    let mut kept = s;
    kept.next_value();
    let mut restarted = kept;
    restarted.set(0.0, 100);
    let mut aimed = kept;
    aimed.aim(0.0, 100);
    assert_eq!(aimed, kept, "aim towards the same target changes nothing");
    assert_ne!(restarted, kept);
}

#[test]
fn a_non_finite_value_is_ignored_and_snap_ends_the_ramp() {
    let mut s = Smoother::new(0.5);
    s.set(f32::NAN, 10);
    s.set(f32::INFINITY, 10);
    s.jump(f32::NEG_INFINITY);
    assert_eq!((s.current(), s.target()), (0.5, 0.5));
    s.set(1.0, 100);
    s.next_value();
    s.snap();
    assert_eq!(s.current(), 1.0);
    assert!(s.is_still());
    // A huge frame count never overflows.
    s.set(0.0, u32::MAX);
    s.advance(u32::MAX);
    s.advance(u32::MAX);
    assert_eq!(s.current(), 0.0);
}

// ---------------------------------------------------------------- through the devices

/// A -60 dB to 0 dB gain jump on a steady tone, with and without smoothing.
fn gain_jump_renders() -> (Vec<f32>, Vec<f32>) {
    let input = sine(8192, 1000.0, 0.5);
    let at = 4096usize;
    let mut g = TestGain::new(&setup());
    // The gain starts at -60 dB (0.001).
    g.set_param(gain::GAIN, 0.001);
    g.reset();
    let smoothed = render(&mut g, &input, 128, &[(at, pv(0, gain::GAIN, 1.0))]);
    let hard = unsmoothed(&input, &[(at, 1.0)], 0.001);
    (smoothed, hard)
}

#[test]
fn a_gain_jump_from_minus_60_to_0_db_makes_no_click() {
    let (smoothed, hard) = gain_jump_renders();
    assert_clean(&smoothed);
    // The click is the broadband energy a clean tone does not have: above 3 kHz.
    let (e_smooth, e_hard) = (energy_above(&smoothed, 3000.0), energy_above(&hard, 3000.0));
    let ratio_db = 10.0 * (e_hard / e_smooth).log10();
    println!(
        "gain jump: energy above 3 kHz, hard {e_hard:e}, smoothed {e_smooth:e}, {ratio_db:.1} dB"
    );
    assert!(
        ratio_db > 40.0,
        "the smoothed jump is only {ratio_db} dB quieter"
    );
    // The same in the time domain: the largest step of the smoothed jump stays near the
    // steady tone's, while the hard one steps by the whole level.
    let steady = max_step(&sine(8192, 1000.0, 0.5), 0, 8192);
    assert!(max_step(&smoothed, 4000, 5000) < 1.1 * steady);
    assert!(max_step(&hard, 4000, 5000) > 3.0 * steady);
}

/// A gain that jumps between 0 and 1 every 128 frames on a steady tone.
fn sweep_events() -> Vec<(usize, Event)> {
    (0..300)
        .map(|k| {
            (
                1000 + k * 128,
                pv(0, gain::GAIN, if k % 2 == 0 { 0.0 } else { 1.0 }),
            )
        })
        .collect()
}

#[test]
fn a_fast_gain_sweep_does_not_zipper() {
    let input = sine(48_000, 440.0, 0.5);
    let events = sweep_events();
    let steady = max_step(&input, 2000, 40_000);
    let steps: Vec<(usize, f32)> = events
        .iter()
        .map(|(at, e)| match e {
            Event::ParamValue { value, .. } => (*at, *value),
            _ => unreachable!(),
        })
        .collect();
    let hard = unsmoothed(&input, &steps, 1.0);
    let hard_step = max_step(&hard, 2000, 40_000);
    for size in [1usize, 37, 128, 512, 2048] {
        let mut g = TestGain::new(&setup());
        let out = render(&mut g, &input, size, &events);
        assert_clean(&out);
        let step = max_step(&out, 2000, 40_000);
        println!("sweep, block {size}: step {step:.4}, steady {steady:.4}, hard {hard_step:.4}");
        assert!(
            step <= 1.25 * steady,
            "block {size}: step {step} against a steady {steady}"
        );
    }
    assert!(hard_step > 8.0 * steady, "the unsmoothed sweep zippers");
}

#[test]
fn the_synth_gain_sweep_does_not_zipper_and_stays_clean() {
    let gain_id = ParamId(Param::Gain.id());
    let sweep: Vec<(usize, Event)> = (0..250)
        .map(|k| {
            (
                12_000 + k * 128,
                pv(0, gain_id, if k % 2 == 0 { 0.0 } else { 1.0 }),
            )
        })
        .collect();
    let reference = synth_note(&[(12_000, pv(0, gain_id, 1.0))], 256);
    let swept = synth_note(&sweep, 256);
    assert_clean(&swept);
    let (r, s) = (
        max_step(&reference, 14_000, 44_000),
        max_step(&swept, 14_000, 44_000),
    );
    println!("synth gain sweep: step {s:.4}, reference {r:.4}");
    assert!(s <= 1.25 * r, "swept step {s} against {r}");
    // The ramps are cut short by the next target, so the gain only reaches part of the way:
    // the swept note stays within the reference's level.
    assert!(swept.iter().all(|v| v.abs() <= 1.0));
    // The same sweep cut in blocks of any size gives the same signal.
    for size in [1usize, 37, 2048] {
        assert_eq!(synth_note(&sweep, size), swept, "block {size}");
    }
}

/// A sine note at 440 Hz held for a second on the real synth, with `events` (gain moves)
/// at their absolute frames, rendered in blocks of `size`.
fn synth_note(events: &[(usize, Event)], size: usize) -> Vec<f32> {
    let mut synth = Synth::new(SR);
    let mut all = vec![
        (0usize, pv(0, ParamId(Param::Waveform.id()), 0.0)),
        (
            0,
            Event::NoteOn {
                offset: 0,
                voice: VoiceId::new(1).unwrap(),
                key: 69,
                velocity: 1.0,
                tune_cents: 0.0,
            },
        ),
    ];
    all.extend_from_slice(events);
    render(&mut synth, &vec![0.0; 48_000], size, &all)
}

#[test]
fn a_synth_gain_jump_from_minus_60_to_0_db_makes_no_click() {
    let gain_id = ParamId(Param::Gain.id());
    let at = 20_480usize;
    // The gain starts at -60 dB, then jumps to 0 dB; the control render holds 0 dB all the
    // way and is cut by a hard step at the same frame.
    let smoothed = synth_note(
        &[(1, pv(0, gain_id, 0.001)), (at, pv(0, gain_id, 1.0))],
        128,
    );
    let steady = synth_note(&[(1, pv(0, gain_id, 1.0))], 128);
    // The synth's gain is already settled at -60 dB long before the jump (10 ms ramp).
    let hard: Vec<f32> = steady
        .iter()
        .enumerate()
        .map(|(i, x)| x * if i < at { 0.001 } else { 1.0 })
        .collect();
    assert_clean(&smoothed);
    let window = at - 4096..at + 4096;
    let (e_smooth, e_hard) = (
        energy_above(&smoothed[window.clone()], 3000.0),
        energy_above(&hard[window], 3000.0),
    );
    let ratio_db = 10.0 * (e_hard / e_smooth).log10();
    println!(
        "synth gain jump: above 3 kHz, hard {e_hard:e}, smoothed {e_smooth:e}, {ratio_db:.1} dB"
    );
    assert!(
        ratio_db > 40.0,
        "only {ratio_db} dB quieter than a hard step"
    );
}

// ---------------------------------------------------------------- for listening

/// Writes `.qa/renders/param-smoothing.wav`: the same gain moves heard unsmoothed, then
/// through the shared smoother, each preceded by a short silence.
#[test]
fn the_listening_render_is_written() {
    let gap = vec![0.0f32; 24_000];
    let mut left = Vec::new();
    let mut section = |signal: Vec<f32>| {
        left.extend(signal);
        left.extend_from_slice(&gap);
    };
    // 1 and 2: a tone whose gain jumps between -60 dB and 0 dB every 250 ms.
    let tone = sine(96_000, 440.0, 0.4);
    let jumps: Vec<(usize, f32)> = (1..8)
        .map(|k| (k * 12_000, if k % 2 == 1 { 1.0 } else { 0.001 }))
        .collect();
    let hard = unsmoothed(&tone, &jumps, 0.001);
    let mut g = TestGain::new(&setup());
    g.set_param(gain::GAIN, 0.001);
    g.reset();
    let events: Vec<(usize, Event)> = jumps
        .iter()
        .map(|(at, v)| (*at, pv(0, gain::GAIN, *v)))
        .collect();
    let soft = render(&mut g, &tone, 128, &events);
    // 3 and 4: a gain staircase refreshed every 512 frames (a block-rate control), rising
    // then falling, the way a dragged control or a coarse automation moves it.
    let tone = sine(96_000, 220.0, 0.4);
    let stairs: Vec<(usize, f32)> = (0..180)
        .map(|k| {
            let phase = k as f32 / 90.0;
            let level = if phase < 1.0 { phase } else { 2.0 - phase };
            (k * 512, level)
        })
        .collect();
    let hard_stairs = unsmoothed(&tone, &stairs, 0.0);
    let mut g = TestGain::new(&setup());
    g.set_param(gain::GAIN, 0.0);
    g.reset();
    let events: Vec<(usize, Event)> = stairs
        .iter()
        .map(|(at, v)| (*at, pv(0, gain::GAIN, *v)))
        .collect();
    let soft_stairs = render(&mut g, &tone, 512, &events);
    for signal in [hard, soft, hard_stairs, soft_stairs] {
        assert_clean(&signal);
        section(signal);
    }
    let path = Render::new(SR, left.clone(), left).write_wav("param-smoothing");
    println!("listening render: {}", path.display());
}
