//! The track graph (MO-3): offline renders through the real `Preparer`, `Engine::apply` and
//! `Engine::render`, every audio-side call under the allocation detector. Asserts on the
//! signal (level, spectrum, silence, steps between samples, no NaN, no denormal) for
//! several tracks with different chains and mixes, the fades of a track added or removed
//! during playback (growth of the track table included), and each transition of the
//! retiring mechanism. The cost of an empty track is measured in `graph_cost.rs`.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::testing::{GAIN, TONE, TRANSPOSE, gain, transpose};
use engine::device::{AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceSetup, Event, Factory};
use engine::engine::{Engine, Preparer, channel};
use engine::graph::{FADE_SECONDS, RETIRE_RING_CAPACITY, fade_frames, mix_levels};
use engine::msg::{
    ControlMsg as M, DeviceKind, DeviceSlot, MsgError, ParamId, Put, RtMsg, Table, TrackKind,
    TrackSlot, encode_state, try_box,
};
use engine::{OSC_SYNTH, Param};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
/// Level of a test tone at velocity 127: its gain (default 0.5) × 0.25.
const TONE_LEVEL: f32 = 0.125;

/// An effect that breaks the contract: it writes NaN and infinities.
struct Broken;

static BROKEN: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.test_broken",
    kind_id: DeviceKind(0xFFFF_00AA),
    name: "Broken",
    ..GAIN
};

impl Device for Broken {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &BROKEN
    }

    fn process(&mut self, _ctx: &BlockCtx, _events: &[Event], io: &mut AudioIo<'_>) {
        for (i, (l, r)) in io.left.iter_mut().zip(io.right.iter_mut()).enumerate() {
            *l = if i % 2 == 0 { f32::NAN } else { f32::INFINITY };
            *r = f32::NEG_INFINITY;
        }
    }

    fn set_param(&mut self, _id: ParamId, _value: f32) {}

    fn reset(&mut self) {}
}

fn build_broken(_setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Broken)?)
}

fn resolver(kind: DeviceKind) -> Option<Factory> {
    if kind == BROKEN.kind_id {
        Some(build_broken)
    } else {
        // The test devices, then the registry: the synth is the shipped one (DV-1).
        engine::device::testing::factory(kind)
    }
}

// ---------------------------------------------------------------- a rig

struct Rig {
    p: Preparer,
    e: Engine,
}

impl Rig {
    fn new() -> Rig {
        Rig::with_block(BLOCK)
    }

    fn with_block(max_block_frames: usize) -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE,
            max_block_frames,
        };
        let (p, e) = channel(setup, resolver).unwrap();
        Rig { p, e }
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p.handle(m.clone()).unwrap();
        }
        self.p.flush();
    }

    /// `blocks` blocks of `BLOCK` frames; returns (left, right).
    fn run(&mut self, blocks: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
        for _ in 0..blocks {
            assert_no_alloc(|| self.e.process(&mut l, &mut r));
            left.extend_from_slice(&l);
            right.extend_from_slice(&r);
            self.p.collect();
        }
        check_samples(&left);
        check_samples(&right);
        (left, right)
    }
}

fn check_samples(x: &[f32]) {
    for (i, s) in x.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is {s}");
        assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
    }
}

fn t(slot: u32) -> TrackSlot {
    TrackSlot(slot)
}

fn create(slot: u32) -> M {
    M::TrackCreate {
        track: t(slot),
        kind: TrackKind::Midi,
    }
}

fn insert(slot: u32, device: u16, kind: DeviceKind, state: &[(ParamId, f32)]) -> M {
    M::DeviceInsert {
        track: t(slot),
        device: DeviceSlot(device),
        kind,
        state: encode_state(state),
    }
}

fn tone(slot: u32) -> M {
    insert(slot, 0, TONE.kind_id, &[])
}

fn note(slot: u32, pitch: u8) -> M {
    M::NoteOn {
        track: t(slot),
        pitch,
        velocity: 127,
    }
}

fn mix(slot: u32, gain: f32, pan: f32, mute: bool) -> M {
    M::TrackSetMix {
        track: t(slot),
        gain,
        pan,
        mute,
    }
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Largest difference between two neighbouring samples.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// Magnitude of the DFT of `x` at `freq`.
fn bin(x: &[f32], freq: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (n, s) in x.iter().enumerate() {
        let a = std::f64::consts::TAU * freq * n as f64 / f64::from(RATE);
        re += f64::from(*s) * a.cos();
        im -= f64::from(*s) * a.sin();
    }
    (re * re + im * im).sqrt() / x.len() as f64
}

fn midi_hz(pitch: u8) -> f64 {
    440.0 * ((f64::from(pitch) - 69.0) / 12.0).exp2()
}

/// Rising zero crossings per second.
fn crossings_hz(x: &[f32]) -> f64 {
    let n = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    n as f64 * f64::from(RATE) / x.len() as f64
}

// ---------------------------------------------------------------- the graph renders

const PITCHES: [u8; 8] = [48, 52, 55, 59, 62, 65, 69, 72];

fn synth_track(slot: u32, pitch: u8) -> Vec<M> {
    vec![
        create(slot),
        insert(
            slot,
            0,
            OSC_SYNTH.kind_id,
            &[
                (ParamId(Param::Waveform.id()), 0.0),
                (ParamId(Param::Gain.id()), 0.3),
            ],
        ),
        note(slot, pitch),
    ]
}

#[test]
fn eight_tracks_with_the_synth_sum_into_the_master() {
    let mut all = Rig::new();
    for (i, pitch) in PITCHES.iter().enumerate() {
        all.send(&synth_track(i as u32, *pitch));
    }
    let (left, right) = all.run(200);
    assert_eq!(left, right, "centred tracks: both channels alike");
    let tail = &left[left.len() - 9_600..];
    assert!(
        rms(tail) > 0.05,
        "the eight tracks sound: rms {}",
        rms(tail)
    );

    // Each track's fundamental is in the spectrum, well above a frequency none plays.
    let floor = bin(tail, 1_234.5);
    for pitch in PITCHES {
        let level = bin(tail, midi_hz(pitch));
        assert!(
            level > 20.0 * floor,
            "pitch {pitch}: {level} vs floor {floor}"
        );
    }

    // The graph is a sum: each track rendered alone, in its slot, adds up to the whole.
    let mut sum = vec![0.0f32; left.len()];
    for (i, pitch) in PITCHES.iter().enumerate() {
        let mut one = Rig::new();
        one.send(&synth_track(i as u32, *pitch));
        let (l, _) = one.run(200);
        for (s, x) in sum.iter_mut().zip(l) {
            *s += x;
        }
    }
    let diff = sum
        .iter()
        .zip(&left)
        .fold(0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(diff < 1e-5, "the sum differs by {diff}");
}

#[test]
fn chains_run_midi_effects_then_the_instrument_then_the_effects() {
    let reference = {
        let mut r = Rig::new();
        r.send(&[create(0), tone(0), note(0, 69)]);
        r.run(40).0
    };
    assert!((crossings_hz(&reference[2_400..]) - 440.0).abs() < 5.0);

    // An audio effect at 0.5 halves the level.
    let mut r = Rig::new();
    r.send(&[
        create(0),
        tone(0),
        insert(0, 1, GAIN.kind_id, &[(gain::GAIN, 0.5)]),
        note(0, 69),
    ]);
    let halved = r.run(40).0;
    for (a, b) in halved.iter().zip(&reference) {
        assert!((a - 0.5 * b).abs() < 1e-6);
    }

    // Disabled, it is bypassed.
    r.send(&[M::DeviceSetEnabled {
        track: t(0),
        device: DeviceSlot(1),
        enabled: false,
    }]);
    let bypassed = r.run(10).0;
    assert!(
        (peak(&bypassed) - TONE_LEVEL).abs() < 1e-3,
        "{}",
        peak(&bypassed)
    );

    // A MIDI effect before the instrument: its notes reach it, an octave up.
    let mut r = Rig::new();
    r.send(&[
        create(0),
        insert(0, 0, TRANSPOSE.kind_id, &[(transpose::SEMITONES, 12.0)]),
        insert(0, 1, TONE.kind_id, &[]),
        note(0, 69),
    ]);
    let up = r.run(40).0;
    assert!((crossings_hz(&up[2_400..]) - 880.0).abs() < 5.0);
    assert!((peak(&up[2_400..]) - TONE_LEVEL).abs() < 1e-3);
}

/// One tone track at the steady state after `blocks` blocks, with `msgs` applied first.
fn steady(msgs: &[M], blocks: usize) -> (Vec<f32>, Vec<f32>) {
    let mut r = Rig::new();
    r.send(&[create(0), tone(0), note(0, 69)]);
    r.send(msgs);
    let (l, rr) = r.run(blocks);
    (
        l[l.len() - 2_400..].to_vec(),
        rr[rr.len() - 2_400..].to_vec(),
    )
}

#[test]
fn gain_pan_mute_and_the_master_set_the_levels() {
    let (l, r) = steady(&[], 40);
    assert!((peak(&l) - TONE_LEVEL).abs() < 1e-3);
    assert_eq!(l, r);

    let (l, _) = steady(&[mix(0, 0.25, 0.0, false)], 40);
    assert!((peak(&l) - 0.25 * TONE_LEVEL).abs() < 1e-3);

    // Pan: the opposite side goes down, the near one keeps unity.
    let (l, r) = steady(&[mix(0, 1.0, -1.0, false)], 40);
    assert!((peak(&l) - TONE_LEVEL).abs() < 1e-3);
    assert_eq!(peak(&r), 0.0);
    let (l, r) = steady(&[mix(0, 1.0, 0.5, false)], 40);
    assert!((peak(&l) - 0.5 * TONE_LEVEL).abs() < 1e-3);
    assert!((peak(&r) - TONE_LEVEL).abs() < 1e-3);

    let (l, r) = steady(&[mix(0, 1.0, 0.0, true)], 40);
    assert_eq!(peak(&l) + peak(&r), 0.0, "muted");

    let (l, _) = steady(
        &[M::SetMaster {
            gain: 0.5,
            mute: false,
        }],
        40,
    );
    assert!((peak(&l) - 0.5 * TONE_LEVEL).abs() < 1e-3);
    let (l, _) = steady(
        &[M::SetMaster {
            gain: 1.0,
            mute: true,
        }],
        40,
    );
    assert_eq!(peak(&l), 0.0, "master muted");

    // The levels the law gives.
    assert_eq!(mix_levels(1.0, 0.0, true), (1.0, 1.0));
    assert_eq!(mix_levels(2.0, 1.0, true), (0.0, 2.0));
    assert_eq!(mix_levels(1.0, f32::NAN, true), (1.0, 1.0));
    assert_eq!(mix_levels(-1.0, 0.0, true), (0.0, 0.0));
    assert_eq!(mix_levels(f32::INFINITY, 0.0, true), (0.0, 0.0));
    assert_eq!(mix_levels(1.0, 9.0, true), (0.0, 1.0));
    assert_eq!(mix_levels(1.0, 0.0, false), (0.0, 0.0));
}

#[test]
fn solo_is_cumulative_and_unsolo_restores_every_track() {
    // Three tones at different pitches; the spectrum tells which ones play.
    let pitches = [57u8, 69, 81];
    let mut r = Rig::new();
    for (i, p) in pitches.iter().enumerate() {
        r.send(&[create(i as u32), tone(i as u32), note(i as u32, *p)]);
    }
    let level = |x: &[f32], p: u8| bin(x, midi_hz(p));
    let tail = |x: Vec<f32>| x[x.len() - 2_400..].to_vec();

    let all = tail(r.run(40).0);
    for p in pitches {
        assert!(level(&all, p) > 0.01, "{p} plays");
    }
    r.send(&[
        M::TrackSetSolo {
            track: t(0),
            solo: true,
        },
        M::TrackSetSolo {
            track: t(2),
            solo: true,
        },
    ]);
    let soloed = tail(r.run(40).0);
    assert!(level(&soloed, 57) > 0.01);
    assert!(level(&soloed, 81) > 0.01);
    assert!(level(&soloed, 69) < 1e-4, "the unsoloed track is silent");
    r.send(&[
        M::TrackSetSolo {
            track: t(0),
            solo: false,
        },
        M::TrackSetSolo {
            track: t(2),
            solo: false,
        },
    ]);
    let back = tail(r.run(40).0);
    for p in pitches {
        let (was, is) = (level(&all, p), level(&back, p));
        assert!(
            (was - is).abs() < 1e-4 * was.max(1.0),
            "{p}: {was} then {is}"
        );
    }
}

#[test]
fn every_change_of_a_level_ramps() {
    let n = fade_frames(RATE) as usize;
    assert_eq!(n, (FADE_SECONDS * f64::from(RATE)).round() as usize);
    let mut r = Rig::new();
    r.send(&[create(0), tone(0), note(0, 69)]);
    let steady = r.run(20).0;
    let bound = max_step(&steady[1_000..]) * 1.05 + TONE_LEVEL / n as f32 * 1.05;
    for msg in [
        mix(0, 0.1, 0.0, false),
        mix(0, 1.0, -1.0, false),
        mix(0, 1.0, 0.0, true),
        mix(0, 1.0, 0.0, false),
        M::TrackSetSolo {
            track: t(0),
            solo: true,
        },
        M::SetMaster {
            gain: 0.0,
            mute: false,
        },
        M::SetMaster {
            gain: 1.0,
            mute: false,
        },
    ] {
        r.send(std::slice::from_ref(&msg));
        let (l, rr) = r.run(6);
        assert!(max_step(&l) <= bound, "{msg:?}: left step {}", max_step(&l));
        assert!(
            max_step(&rr) <= bound,
            "{msg:?}: right step {}",
            max_step(&rr)
        );
    }
}

// ---------------------------------------------------------------- add and remove

#[test]
fn a_track_added_during_playback_fades_in() {
    let n = fade_frames(RATE) as usize;
    let mut r = Rig::new();
    r.send(&[create(0), tone(0), note(0, 57)]);
    r.run(10);
    // The new tone starts at its peak (a cosine): without the fade, a jump of its level.
    r.send(&[create(1), tone(1), note(1, 69)]);
    let mut alone = Rig::new();
    alone.send(&[create(1), tone(1), note(1, 69)]);
    let new = alone.run(4).0;
    for (k, s) in new.iter().enumerate().take(n) {
        let ramp = TONE_LEVEL * k as f32 / n as f32;
        assert!(
            s.abs() <= ramp + 1e-6,
            "frame {k}: {s} above the ramp {ramp}"
        );
    }
    assert!(
        (peak(&new[n..]) - TONE_LEVEL).abs() < 1e-3,
        "full level after the fade"
    );
    let both = r.run(4).0;
    let step = max_step(&both);
    assert!(step < 0.2 * TONE_LEVEL, "the add clicks: step {step}");
}

#[test]
fn a_track_removed_during_playback_fades_out_then_retires() {
    let n = fade_frames(RATE) as usize;
    let mut r = Rig::new();
    r.send(&[create(0), tone(0), note(0, 69)]);
    let before = r.run(20).0;
    let bound = max_step(&before[1_000..]) * 1.05 + TONE_LEVEL / n as f32;
    r.p.handle(M::TrackRemove { track: t(0) }).unwrap();
    r.p.flush();
    let (mut l, mut rr) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    assert_no_alloc(|| r.e.process(&mut l, &mut rr));
    assert_eq!(r.e.graph().retiring(), 1, "fading out");
    assert!(
        r.e.state().track(t(0)).is_none(),
        "the slot is free at once"
    );
    let mut out = l.clone();
    out.extend(r.run(4).0);
    let mut joined = before[before.len() - 64..].to_vec();
    joined.extend_from_slice(&out);
    assert!(
        max_step(&joined) <= bound,
        "the removal clicks: {}",
        max_step(&joined)
    );
    assert_eq!(peak(&out[n..]), 0.0, "silent after the fade");
    assert_eq!(r.e.graph().retiring(), 0, "retired");
    assert!(r.p.collected() >= 1);
}

#[test]
fn a_growth_of_the_track_table_leaves_the_playing_tracks_untouched() {
    let play = |r: &mut Rig| {
        for i in 0..4 {
            r.send(&[create(i), tone(i), note(i, 57 + 4 * i as u8)]);
        }
    };
    let mut plain = Rig::new();
    play(&mut plain);
    let mut grown = Rig::new();
    play(&mut grown);
    let a1 = plain.run(10).0;
    let b1 = grown.run(10).0;
    assert_eq!(a1, b1);
    // Tracks beyond the table (16 → 32 → 128 slots): an empty one, and a muted tone.
    grown.send(&[create(20), create(100), tone(100), mix(100, 1.0, 0.0, true)]);
    grown.send(&[note(100, 60)]);
    let a2 = plain.run(20).0;
    let b2 = grown.run(20).0;
    assert_eq!(grown.e.state().tracks.capacity(), 128);
    assert_eq!(a2, b2, "sample-identical across the growth");
    // A growth that brings a playing track (128 → 512 slots): the old ones go on
    // untouched, the new one fades in under the ramp from its first frame.
    let n = fade_frames(RATE) as usize;
    grown.send(&[create(300), tone(300), note(300, 81)]);
    let a3 = plain.run(10).0;
    let b3 = grown.run(10).0;
    assert_eq!(grown.e.state().tracks.capacity(), 512);
    for (k, (b, a)) in b3.iter().zip(&a3).enumerate() {
        let new = b - a;
        let ramp = TONE_LEVEL * (k.min(n) as f32 / n as f32);
        assert!(
            new.abs() <= ramp + 1e-5,
            "frame {k}: {new} above the ramp {ramp}"
        );
    }
    assert!(bin(&b3[1_000..], midi_hz(81)) > 0.01);
}

// ---------------------------------------------------------------- transitions

#[test]
fn a_track_removed_during_its_fade_in_fades_out_from_where_it_was() {
    let n = fade_frames(RATE) as usize;
    let mut r = Rig::new();
    r.send(&[create(0), tone(0), note(0, 69)]);
    let (mut l, mut rr) = (vec![0.0; 64], vec![0.0; 64]);
    assert_no_alloc(|| r.e.process(&mut l, &mut rr));
    let reached = r.e.state().track(t(0)).unwrap().graph.levels().0;
    assert!((reached - 64.0 / n as f32).abs() < 1e-6);
    r.send(&[M::TrackRemove { track: t(0) }]);
    let out = r.run(4).0;
    assert!(
        peak(&out) <= reached * TONE_LEVEL + 1e-6,
        "never above where it was"
    );
    let mut joined = l.clone();
    joined.extend_from_slice(&out);
    assert!(
        max_step(&joined) < 0.1 * TONE_LEVEL,
        "{}",
        max_step(&joined)
    );
    assert_eq!(r.e.graph().retiring(), 0);
}

#[test]
fn a_slot_created_again_during_a_fade_out_plays_both_without_a_click() {
    let mut r = Rig::new();
    r.send(&[create(0), tone(0), note(0, 69)]);
    let before = r.run(10).0;
    // Removed and created again in the same block, and a put on the occupied slot.
    r.send(&[
        M::TrackRemove { track: t(0) },
        create(0),
        tone(0),
        note(0, 57),
    ]);
    let (mut l, mut rr) = (vec![0.0; 64], vec![0.0; 64]);
    assert_no_alloc(|| r.e.process(&mut l, &mut rr));
    assert_eq!(r.e.graph().retiring(), 1);
    r.send(&[create(0), tone(0), note(0, 81)]);
    assert_no_alloc(|| r.e.process(&mut l, &mut rr));
    assert_eq!(
        r.e.graph().retiring(),
        2,
        "the put replaced a playing track"
    );
    let after = r.run(40).0;
    let mut joined = before[before.len() - 64..].to_vec();
    joined.extend_from_slice(&l);
    joined.extend_from_slice(&after);
    assert!(max_step(&joined) < 0.1, "{}", max_step(&joined));
    assert_eq!(r.e.graph().retiring(), 0);
    // 2 400 frames: whole periods of each pitch, so no leakage between their bins.
    let tail = &after[after.len() - 2_400..];
    assert!(bin(tail, midi_hz(81)) > 0.01);
    assert!(bin(tail, midi_hz(69)) < 1e-4);
    assert!(bin(tail, midi_hz(57)) < 1e-4);
}

#[test]
fn a_reset_during_playback_fades_every_track_out() {
    let mut r = Rig::new();
    for i in 0..5 {
        r.send(&[create(i), tone(i), note(i, 57 + 5 * i as u8)]);
    }
    let before = r.run(10).0;
    r.send(&[M::Reset { epoch: 1 }]);
    let (mut l, mut rr) = (vec![0.0; 64], vec![0.0; 64]);
    assert_no_alloc(|| r.e.process(&mut l, &mut rr));
    assert_eq!(r.e.graph().retiring(), 5);
    let after = r.run(6).0;
    let mut joined = before[before.len() - 64..].to_vec();
    joined.extend_from_slice(&l);
    joined.extend_from_slice(&after);
    assert!(max_step(&joined) < 0.1, "{}", max_step(&joined));
    assert_eq!(peak(&after[BLOCK * 2..]), 0.0);
    assert_eq!(r.e.graph().retiring(), 0);
}

#[test]
fn a_full_retire_ring_keeps_the_tracks_until_there_is_room() {
    let mut r = Rig::new();
    let extra = 5;
    // Puts on slot 0 without a collect: each replaces the last, empty tracks.
    for _ in 0..RETIRE_RING_CAPACITY + extra + 1 {
        let rt = r.p.prepare(create(0)).unwrap().unwrap();
        assert_no_alloc(|| r.e.apply(rt));
    }
    assert_eq!(r.e.graph().retiring(), RETIRE_RING_CAPACITY + extra);
    let (mut l, mut rr) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    assert_no_alloc(|| r.e.render(&mut l, &mut rr));
    assert_eq!(r.e.graph().retiring(), extra, "the ring is full");
    assert_no_alloc(|| r.e.render(&mut l, &mut rr));
    assert_eq!(r.e.graph().retiring(), extra, "still waiting, nothing lost");
    assert_eq!(r.p.collect(), RETIRE_RING_CAPACITY);
    assert_no_alloc(|| r.e.render(&mut l, &mut rr));
    assert_eq!(r.e.graph().retiring(), 0);
    assert_eq!(r.p.collect(), extra);
    assert_eq!(r.e.leaked(), 0);
}

#[test]
fn silent_tracks_retire_at_once() {
    let mut r = Rig::new();
    r.send(&[
        create(0),
        create(1),
        tone(1),
        note(1, 69),
        mix(1, 1.0, 0.0, true),
    ]);
    r.run(4);
    r.send(&[
        M::TrackRemove { track: t(0) },
        M::TrackRemove { track: t(1) },
    ]);
    let (mut l, mut rr) = (vec![0.0; 8], vec![0.0; 8]);
    assert_no_alloc(|| r.e.process(&mut l, &mut rr));
    assert_eq!(r.e.graph().retiring(), 0, "an empty and a muted track");
    assert_eq!(r.p.collect(), 2);
}

#[test]
fn a_track_put_beyond_the_table_is_garbage_not_a_retiring_track() {
    let mut r = Rig::new();
    let rt = RtMsg::TrackPut(Put {
        slot: 999,
        value: Box::new(engine::engine::Track {
            kind: TrackKind::Midi,
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            armed: false,
            monitor: false,
            devices: Table::try_filled(1).unwrap(),
            scenes: Table::try_filled(1).unwrap(),
            placements: Table::from_vec(Vec::new()),
            launch: None,
            back: false,
            graph: Default::default(),
            player: Default::default(),
        }),
        grown: None,
    });
    assert_no_alloc(|| r.e.apply(rt));
    assert_eq!(r.e.skipped(), 1);
    assert_eq!(r.e.graph().retiring(), 0);
    assert_eq!(r.p.collect(), 1);
}

#[test]
fn a_long_retiring_list_drops_without_recursion() {
    // Dropping a linked list of boxes recursively would overflow the test thread's stack.
    let mut r = Rig::new();
    for _ in 0..200_000 {
        let rt = r.p.prepare(create(0)).unwrap().unwrap();
        assert_no_alloc(|| r.e.apply(rt));
    }
    assert_eq!(r.e.graph().retiring(), 199_999);
    drop(r);
}

#[test]
fn a_render_longer_than_the_largest_block_is_split_and_zero_frames_is_nothing() {
    let setup = |r: &mut Rig| {
        r.send(&[
            create(0),
            tone(0),
            insert(0, 1, GAIN.kind_id, &[(gain::GAIN, 0.7)]),
            note(0, 69),
            mix(0, 0.8, 0.3, false),
        ])
    };
    let mut small = Rig::with_block(64);
    setup(&mut small);
    let mut large = Rig::with_block(2048);
    setup(&mut large);
    let (mut l, mut r) = (vec![0.0; 200], vec![0.0; 200]);
    assert_no_alloc(|| small.e.process(&mut l, &mut r));
    let (mut l2, mut r2) = (vec![0.0; 200], vec![0.0; 200]);
    assert_no_alloc(|| large.e.process(&mut l2[..64], &mut r2[..64]));
    for range in [64..128, 128..192, 192..200] {
        assert_no_alloc(|| large.e.render(&mut l2[range.clone()], &mut r2[range]));
    }
    assert_eq!(l, l2);
    assert_eq!(r, r2);
    assert!(peak(&l) > 0.0);
    assert_no_alloc(|| small.e.process(&mut [], &mut []));
}

#[test]
fn a_note_off_in_a_block_of_zero_frames_is_not_lost() {
    let mut r = Rig::new();
    r.send(&[create(0), tone(0), note(0, 69)]);
    r.run(4);
    r.send(&[M::NoteOff {
        track: t(0),
        pitch: 69,
    }]);
    // The host renders nothing in this block: the note-off waits for the next block.
    assert_no_alloc(|| r.e.process(&mut [], &mut []));
    let (mut l, mut rr) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    assert_no_alloc(|| r.e.process(&mut l, &mut rr));
    assert_eq!(peak(&l), 0.0, "the note stopped");
    // The same when the zero frames come between begin_block and render.
    r.send(&[note(0, 69)]);
    r.run(2);
    r.send(&[M::NoteOff {
        track: t(0),
        pitch: 69,
    }]);
    assert_no_alloc(|| r.e.begin_block());
    assert_no_alloc(|| r.e.render(&mut [], &mut []));
    assert_no_alloc(|| r.e.render(&mut l, &mut rr));
    assert_eq!(peak(&l), 0.0, "the note stopped");
    // And it is delivered once: a later note is not released by it again.
    r.send(&[note(0, 69)]);
    let out = r.run(4).0;
    assert!(peak(&out[BLOCK * 3..]) > 0.1 * TONE_LEVEL);
}

#[test]
fn changing_a_midi_effect_releases_the_notes_of_its_track() {
    for change in 0..3 {
        let mut r = Rig::new();
        r.send(&[
            create(0),
            insert(0, 0, TRANSPOSE.kind_id, &[(transpose::SEMITONES, 12.0)]),
            insert(0, 1, TONE.kind_id, &[]),
            note(0, 69),
        ]);
        let held = r.run(10).0;
        assert!(peak(&held[640..]) > 0.5 * TONE_LEVEL);
        let msg = match change {
            0 => M::DeviceSetEnabled {
                track: t(0),
                device: DeviceSlot(0),
                enabled: false,
            },
            1 => M::DeviceRemove {
                track: t(0),
                device: DeviceSlot(0),
            },
            _ => insert(0, 0, TRANSPOSE.kind_id, &[(transpose::SEMITONES, 7.0)]),
        };
        r.send(std::slice::from_ref(&msg));
        let after = r.run(4).0;
        assert_eq!(peak(&after), 0.0, "{msg:?}: the held note is released");
        assert!(!r.e.state().track(t(0)).unwrap().graph.releasing());
        // An effect's change releases nothing: the note plays on.
        let mut r = Rig::new();
        r.send(&[
            create(0),
            tone(0),
            insert(0, 1, GAIN.kind_id, &[]),
            note(0, 69),
        ]);
        r.run(10);
        r.send(&[M::DeviceSetEnabled {
            track: t(0),
            device: DeviceSlot(1),
            enabled: false,
        }]);
        assert!(peak(&r.run(4).0) > 0.5 * TONE_LEVEL);
    }
}

#[test]
fn a_bypassed_instrument_keeps_no_stuck_note() {
    let mut r = Rig::new();
    r.send(&[create(0), tone(0), note(0, 69)]);
    r.run(4);
    let disable = |enabled| M::DeviceSetEnabled {
        track: t(0),
        device: DeviceSlot(0),
        enabled,
    };
    // Bypassed while the key is held: its note-off never reaches the instrument.
    r.send(&[disable(false)]);
    r.run(2);
    r.send(&[M::NoteOff {
        track: t(0),
        pitch: 69,
    }]);
    r.run(2);
    r.send(&[disable(true)]);
    let after = r.run(4).0;
    assert_eq!(peak(&after), 0.0, "the held voice was released");
}

#[test]
fn nan_and_infinities_from_a_device_never_reach_the_output() {
    let mut r = Rig::new();
    r.send(&[
        create(0),
        tone(0),
        note(0, 69),
        create(1),
        tone(1),
        insert(1, 1, BROKEN.kind_id, &[]),
        note(1, 60),
    ]);
    // `run` asserts every sample is finite and not denormal.
    let (l, rr) = r.run(8);
    assert!(peak(&l) <= 1.0 && peak(&rr) <= 1.0);
    r.send(&[M::SetMaster {
        gain: 1.0,
        mute: true,
    }]);
    let (l, rr) = r.run(8);
    assert_eq!(peak(&l[BLOCK * 4..]) + peak(&rr[BLOCK * 4..]), 0.0);
}

/// Writes `.qa/renders/56-eight-tracks.wav` (16-bit stereo, 48 kHz) for listening: the
/// eight synth tracks spread across the pan, then removed one by one while they play, the
/// last ones soloed. Run with `cargo test -p engine --test graph -- --ignored listening`.
#[test]
#[ignore]
fn listening_render() {
    let mut r = Rig::new();
    for (i, pitch) in PITCHES.iter().enumerate() {
        r.send(&synth_track(i as u32, *pitch));
        r.send(&[mix(i as u32, 0.8, -1.0 + 2.0 * i as f32 / 7.0, false)]);
    }
    let (mut left, mut right) = r.run(375);
    for i in 0..4 {
        r.send(&[M::TrackRemove { track: t(i) }]);
        let (l, rr) = r.run(94);
        left.extend(l);
        right.extend(rr);
    }
    r.send(&[
        M::TrackSetSolo {
            track: t(6),
            solo: true,
        },
        M::TrackSetSolo {
            track: t(7),
            solo: true,
        },
    ]);
    let (l, rr) = r.run(375);
    left.extend(l);
    right.extend(rr);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.qa/renders");
    std::fs::create_dir_all(&dir).unwrap();
    let mut bytes = Vec::new();
    let frames = left.len() as u32;
    let data = frames * 4;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for (l, r) in left.iter().zip(&right) {
        for s in [l, r] {
            let v = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    std::fs::write(dir.join("56-eight-tracks.wav"), bytes).unwrap();
}
