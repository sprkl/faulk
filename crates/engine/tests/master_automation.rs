//! The master's automation lanes (#378) at the engine's own level, from raw control messages:
//! a lane on `TrackSlot::MASTER` drives the master's fader and pan and the parameters of the
//! effects of its chain, with the sample accuracy, the fader's law, the stop and seek rule
//! and the manual gestures of a track's lane (AM-2, #388, #389). Offline renders through the
//! real `Preparer`, `Engine::apply` and `Engine::render`, every audio-side call under the
//! allocation detector. The signal is a test tone through the test gain effect on a track,
//! then through the same effect in the master's chain, so that a rendered sample can be
//! compared with the same sample of a render without lanes: `out[k] = reference[k] × level(k)`.
//!
//! At 120 BPM and 48 kHz a tick is exactly 25 frames and a bar (3 840 ticks) 96 000 frames.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::automation::Manual;
use engine::device::DeviceSetup;
use engine::device::testing::{GAIN, TONE, gain};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{
    AutomationPoint, AutomationTarget, ControlMsg as M, Curve, DeviceSlot, LaneSlot, TrackKind,
    TrackSlot, encode_state,
};
use engine::time::{Quantize, Tempo, Tick, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: u32 = 48_000;
const FPT: usize = 25;
const BAR: i64 = 3_840;
/// Frames rendered before the clock starts: beyond the fade in of a new track.
const WARM: usize = 4_096;
/// Largest difference allowed between a sample and its expected value (-120 dBFS).
const EXACT: f32 = 1e-6;
/// Largest difference allowed against the ideal curve of the fader's law (see the track
/// tests): the strip draws chords of at most 64 frames of that curve.
const CHORD: f32 = 2e-5;
const MASTER: TrackSlot = TrackSlot::MASTER;

fn gain_of(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn pt(tick: i64, value: f32, curve: Curve) -> AutomationPoint {
    AutomationPoint {
        tick: Tick(tick),
        value,
        curve,
    }
}

/// The test effect's `trim` of the master's first device: no smoothing.
const MASTER_TRIM: AutomationTarget = AutomationTarget::DeviceParam {
    device: DeviceSlot(0),
    param: gain::TRIM,
};

struct Rig {
    p: Preparer,
    e: Engine,
    block: usize,
    at: usize,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Rig {
    /// One track (a tone through the test gain effect) and the same effect in the master's
    /// chain, rendered past the track's fade in, the clock stopped.
    fn new(block: usize) -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE as f32,
            max_block_frames: block,
        };
        let (p, e) = channel(setup, engine::device::testing::factory).unwrap();
        let mut r = Rig {
            p,
            e,
            block,
            at: 0,
            left: Vec::new(),
            right: Vec::new(),
        };
        r.send(&[
            M::SetTransport {
                tempo: Tempo::new(120.0).unwrap(),
                signature: TimeSignature::COMMON,
                launch_quantize: Quantize::default(),
            },
            M::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                kind: TONE.kind_id,
                state: encode_state(&[]),
            },
            M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(1),
                kind: GAIN.kind_id,
                state: encode_state(&[]),
            },
            M::DeviceInsert {
                track: MASTER,
                device: DeviceSlot(0),
                kind: GAIN.kind_id,
                state: encode_state(&[]),
            },
            M::NoteOn {
                track: TrackSlot(0),
                pitch: 57,
                velocity: 127,
                at: None,
            },
        ]);
        r.run(WARM.div_ceil(block) * block);
        r
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p
                .handle(m.clone())
                .unwrap_or_else(|e| panic!("{m:?}: {e:?}"));
        }
        self.p.flush();
    }

    fn lane(
        &mut self,
        lane: u32,
        track: TrackSlot,
        target: AutomationTarget,
        points: &[AutomationPoint],
    ) {
        self.send(&[M::AutomationLaneUpload {
            lane: LaneSlot(lane),
            track,
            target,
            first: 0,
            total: points.len() as u32,
            points: points.to_vec(),
        }]);
    }

    fn master(&mut self, gain: f32, pan: f32) {
        self.send(&[M::SetMaster {
            gain,
            pan,
            mute: false,
        }]);
    }

    fn run(&mut self, frames: usize) {
        let (mut l, mut r) = (vec![0.0; self.block], vec![0.0; self.block]);
        let mut todo = frames;
        while todo > 0 {
            let n = todo.min(self.block);
            assert_no_alloc(|| self.e.process(&mut l[..n], &mut r[..n]));
            self.left.extend_from_slice(&l[..n]);
            self.right.extend_from_slice(&r[..n]);
            todo -= n;
            self.at += n;
            self.p.collect();
        }
    }

    fn run_to(&mut self, to: usize) {
        self.run(to - self.at);
    }

    fn play(&mut self, from: i64) -> usize {
        self.send(&[M::Play { from: Tick(from) }]);
        self.at
    }

    fn check(&self) {
        assert_eq!(self.p.rejected(), 0, "the preparer rejected a message");
        // The `is_finite` check below is vacuous on the render output since the output protection
        // (#402) silences non-finite samples; `limiter_silenced` is the real no-NaN guard — a
        // master gain lane that produced NaN at an odd block size would make it non-zero.
        assert_eq!(
            self.e.limiter_silenced(),
            0,
            "the output protection silenced a non-finite sample: the master automation produced NaN"
        );
        for (i, s) in self.left.iter().chain(&self.right).enumerate() {
            assert!(s.is_finite(), "sample {i} is {s}");
            assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
        }
    }
}

/// The same rig without lanes or gestures: what each sample is at level 1.
fn reference(block: usize, frames: usize) -> Vec<f32> {
    let mut r = Rig::new(block);
    r.play(0);
    r.run_to(frames);
    r.left
}

fn assert_levels_within(
    out: &[f32],
    reference: &[f32],
    range: core::ops::Range<usize>,
    tolerance: f32,
    level: impl Fn(usize) -> f32,
) {
    for k in range {
        let want = reference[k] * level(k);
        assert!(
            (out[k] - want).abs() <= tolerance,
            "frame {k}: {} instead of {want} (level {})",
            out[k],
            level(k)
        );
    }
}

fn assert_levels(
    out: &[f32],
    reference: &[f32],
    range: core::ops::Range<usize>,
    level: impl Fn(usize) -> f32,
) {
    assert_levels_within(out, reference, range, EXACT, level);
}

/// The fader's law, written again here from its table (position, dB).
const LAW: [(f64, f64); 6] = [
    (0.0, -96.0),
    (0.08, -60.0),
    (0.28, -30.0),
    (0.5, -12.0),
    (0.78, 0.0),
    (1.0, 6.0),
];

fn law_db(position: f64) -> f64 {
    let i = LAW.iter().position(|&(p, _)| position <= p).unwrap().max(1);
    let ((p0, d0), (p1, d1)) = (LAW[i - 1], LAW[i]);
    d0 + (position - p0) / (p1 - p0) * (d1 - d0)
}

fn law_gain(position: f64) -> f32 {
    if position <= 0.0 {
        return 0.0;
    }
    10f64.powf(law_db(position) / 20.0) as f32
}

/// Level of `out` against `reference` over the 128 frames around `k`, in dB.
fn measured_db(out: &[f32], reference: &[f32], k: usize) -> f64 {
    let sum = |s: &[f32]| {
        s[k - 64..k + 64]
            .iter()
            .map(|x| f64::from(*x).powi(2))
            .sum::<f64>()
    };
    10.0 * (sum(out) / sum(reference)).log10()
}

#[test]
fn an_eight_bar_master_fade_goes_down_like_a_fader_pulled_at_constant_speed_whatever_the_block_size()
 {
    let bar = BAR as usize * FPT;
    let len = 8 * bar;
    let mut renders = Vec::new();
    for block in [33, 128, 1024] {
        let mut r = Rig::new(block);
        // One lane on the master's volume, 0 dB to -96 dB (silence) over eight bars.
        r.lane(
            0,
            MASTER,
            AutomationTarget::MixerGain,
            &[pt(0, 0.0, Curve::Linear), pt(8 * BAR, -96.0, Curve::Linear)],
        );
        r.run_to(8_192);
        let start = r.play(0);
        r.run(len + 4_000);
        r.check();
        let reference = reference(block, r.at);
        let out = &r.left[start..];
        let reference = &reference[start..];
        let level = |k: usize| law_gain(0.78 * (1.0 - (k as f64 / len as f64).min(1.0)));
        assert_levels_within(out, reference, 0..len + 4_000, CHORD, level);
        // The level at each bar line, measured: the dB of the law at that travel.
        for i in 1..8 {
            let want = law_db(0.78 * (1.0 - i as f64 / 8.0));
            let got = measured_db(out, reference, i * bar);
            println!("block {block}, bar {i}: {got:.3} dB (law {want:.3} dB)");
            assert!(
                (got - want).abs() < 0.01,
                "bar {i}: {got} dB instead of {want} dB"
            );
        }
        // The end of the fade is silence, exactly, and stays so.
        assert!(out[len..len + 4_000].iter().all(|x| *x == 0.0));
        renders.push(out[..len + 4_000].to_vec());
    }
    for r in &renders[1..] {
        let diff = r
            .iter()
            .zip(&renders[0])
            .fold(0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(diff <= EXACT, "block sizes differ by {diff}");
    }
}

#[test]
fn a_master_lane_sounds_the_same_as_the_same_lane_on_the_only_track() {
    // One track in the mix and a unity master: a gain lane or a pan lane on the master is
    // the same signal as on the track.
    for (target, points) in [
        (
            AutomationTarget::MixerGain,
            vec![
                pt(0, -3.0, Curve::Linear),
                pt(1000, -40.0, Curve::Step),
                pt(2000, -6.0, Curve::Linear),
                pt(3000, 0.0, Curve::Step),
            ],
        ),
        (
            AutomationTarget::MixerPan,
            vec![pt(0, -0.8, Curve::Linear), pt(2000, 0.6, Curve::Step)],
        ),
    ] {
        for block in [64, 97] {
            let mut on_master = Rig::new(block);
            on_master.lane(0, MASTER, target, &points);
            let mut on_track = Rig::new(block);
            on_track.lane(0, TrackSlot(0), target, &points);
            for r in [&mut on_master, &mut on_track] {
                let start = r.play(0);
                r.run_to(start + 4_000 * FPT);
                r.check();
            }
            let worst =
                |a: &[f32], b: &[f32]| a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
            assert!(
                worst(&on_master.left, &on_track.left) <= EXACT,
                "{target:?} left, block {block}"
            );
            assert!(
                worst(&on_master.right, &on_track.right) <= EXACT,
                "{target:?} right, block {block}"
            );
            assert!(on_master.left.iter().any(|x| x.abs() > 0.01));
        }
    }
}

#[test]
fn a_master_effect_lane_lands_on_its_points_and_moves_at_the_control_frames() {
    for block in [97, 128, 1024] {
        let mut r = Rig::new(block);
        // A step at tick 1 001 (frame 25 025), then a ramp from 0.5 to 1.5.
        r.lane(
            0,
            MASTER,
            MASTER_TRIM,
            &[
                pt(100, 0.25, Curve::Step),
                pt(1001, 0.5, Curve::Linear),
                pt(1801, 1.5, Curve::Step),
            ],
        );
        let start = r.play(0);
        r.run(50_000);
        r.check();
        let reference = reference(block, r.at);
        let lane = |f: usize| -> f32 {
            let tick = f as f64 / FPT as f64;
            if tick < 1001.0 {
                0.25
            } else if tick < 1801.0 {
                (0.5 + (tick - 1001.0) / 800.0) as f32
            } else {
                1.5
            }
        };
        assert_levels(&r.left, &reference, start..start + 50_000, |k| {
            let f = k - start;
            let instant = if f >= 1801 * FPT {
                1801 * FPT
            } else if f >= 1001 * FPT {
                let control = k - k % 64 - start;
                control.max(1001 * FPT)
            } else {
                f
            };
            lane(instant)
        });
    }
}

#[test]
fn a_stop_and_every_seek_apply_the_master_lanes_value_at_the_playhead() {
    let block = 128;
    let mut r = Rig::new(block);
    let points = [
        pt(100, -20.0, Curve::Step),
        pt(1000, -6.0, Curve::Step),
        pt(2000, -12.0, Curve::Step),
    ];
    r.lane(0, MASTER, AutomationTarget::MixerGain, &points);
    // The clock has not run: the lane already applies at the playhead (tick 0).
    let before = r.at;
    r.run(20 * block);
    let start = r.play(0);
    r.run_to(start + 1500 * FPT);
    r.send(&[M::Stop]);
    let stopped = r.at;
    r.run(20 * block);
    let seeks = [(50, -20.0), (1000, -6.0), (999, -20.0), (5000, -12.0)];
    let mut spans = Vec::new();
    for (tick, db) in seeks {
        r.send(&[M::Seek { tick: Tick(tick) }]);
        spans.push((r.at, db));
        r.run(20 * block);
    }
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, before..before + 20 * block, |_| {
        gain_of(-20.0)
    });
    assert_levels(&r.left, &reference, stopped..stopped + 20 * block, |_| {
        gain_of(-6.0)
    });
    for (at, db) in spans {
        assert_levels(&r.left, &reference, at..at + 20 * block, |_| gain_of(db));
    }
    // A master effect lane applies while stopped too.
    r.lane(1, MASTER, MASTER_TRIM, &[pt(0, 0.5, Curve::Step)]);
    let effect = r.at;
    r.run(20 * block);
    r.check();
    let reference = self::reference(block, r.at);
    // The gain lane is at -12 dB (the last seek); the trim halves it again.
    assert_levels(&r.left, &reference, effect + block..r.at, |_| {
        gain_of(-12.0) * 0.5
    });
}

#[test]
fn a_gesture_on_the_master_fader_holds_its_lane_until_the_next_point_and_the_pan_does_not() {
    let block = 128;
    let mut r = Rig::new(block);
    r.lane(
        0,
        MASTER,
        AutomationTarget::MixerGain,
        &[
            pt(0, -20.0, Curve::Step),
            pt(2000, -6.0, Curve::Step),
            pt(4000, 0.0, Curve::Step),
        ],
    );
    let start = r.play(0);
    r.run_to(start + 80 * block);
    // A fader gesture at tick 409.6: it applies (a 5 ms ramp) until tick 2 000.
    let gesture = r.at;
    r.master(0.5, 0.0);
    r.run_to(start + 2000 * FPT + 10 * block);
    // A pan gesture does not hold the gain lane.
    let pan = r.at;
    r.master(0.5, 0.5);
    r.run(40 * block);
    r.check();
    let reference = reference(block, r.at);
    let fade = 240;
    assert_levels(&r.left, &reference, start..gesture, |_| gain_of(-20.0));
    assert_levels(
        &r.left,
        &reference,
        gesture + fade..start + 2000 * FPT,
        |_| 0.5,
    );
    assert_levels(&r.left, &reference, start + 2000 * FPT..pan, |_| {
        gain_of(-6.0)
    });
    // The lane still plays after the pan gesture (the pan moves the channels, not the lane).
    let tail = r.at - 1;
    let ratio = r.left[tail] / reference[tail] / gain_of(-6.0);
    assert!(
        (ratio - (0.5f32 * core::f32::consts::FRAC_PI_2).cos()).abs() < 2e-3,
        "the gain lane must still hold -6 dB under the pan gesture: {ratio}"
    );
}

#[test]
fn a_removed_master_lane_gives_the_master_back_to_its_fader() {
    let block = 128;
    let mut r = Rig::new(block);
    r.lane(
        0,
        MASTER,
        AutomationTarget::MixerGain,
        &[pt(0, -20.0, Curve::Step)],
    );
    r.lane(
        1,
        MASTER,
        AutomationTarget::MixerPan,
        &[pt(0, -0.5, Curve::Step)],
    );
    let start = r.play(0);
    r.run(40 * block);
    let removed = r.at;
    r.send(&[M::AutomationLaneRemove { lane: LaneSlot(0) }]);
    r.run(40 * block);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, start..removed, |_| gain_of(-20.0));
    // Back to the fader (unity) after its ramp; the pan lane still plays.
    assert_levels(&r.left, &reference, removed + 240..r.at, |_| 1.0);
    let right = r.right[r.at - 1] / reference[r.at - 1];
    assert!((right - (0.5f32 * core::f32::consts::FRAC_PI_2).cos()).abs() < 1e-5);
}

#[test]
fn a_master_effect_lane_follows_its_device_when_it_is_put_again_and_survives_its_removal() {
    let block = 128;
    let mut r = Rig::new(block);
    r.lane(0, MASTER, MASTER_TRIM, &[pt(0, 0.5, Curve::Step)]);
    let start = r.play(0);
    r.run(40 * block);
    // The device is built again on its slot: it gets the lane's value again.
    let rebuilt = r.at;
    r.send(&[M::DeviceInsert {
        track: MASTER,
        device: DeviceSlot(0),
        kind: GAIN.kind_id,
        state: encode_state(&[]),
    }]);
    r.run(40 * block);
    // Removed under its lane: the signal goes through, and the lane plays on nothing.
    let removed = r.at;
    r.send(&[M::DeviceRemove {
        track: MASTER,
        device: DeviceSlot(0),
    }]);
    r.run(40 * block);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, start..removed, |_| 0.5);
    assert!(rebuilt > start);
    // The removed device fades out over the graph's fade (#233): 240 frames at 48 kHz.
    assert_levels(&r.left, &reference, removed + 240..r.at, |_| 1.0);
}

#[test]
fn a_master_lane_is_refused_on_no_other_message_than_the_lane_upload() {
    // The master's row is not a track: a lane upload names it, a track message does not.
    let mut r = Rig::new(128);
    r.lane(0, MASTER, AutomationTarget::MixerGain, &[]);
    assert_eq!(r.p.rejected(), 0);
    assert!(
        r.p.handle(M::TrackSetMix {
            track: MASTER,
            gain: 1.0,
            pan: 0.0,
            mute: false,
        })
        .is_err()
    );
}

#[test]
fn the_last_master_lane_removed_brings_back_the_single_pass_and_the_fader() {
    // The transitions of the master pass: no lane, one lane (sub-block by sub-block), no lane.
    let block = 100;
    let mut r = Rig::new(block);
    let start = r.play(0);
    r.run(20 * block);
    let added = r.at;
    r.lane(
        0,
        MASTER,
        AutomationTarget::MixerGain,
        &[pt(0, -12.0, Curve::Step)],
    );
    r.run(40 * block);
    let removed = r.at;
    r.send(&[M::AutomationLaneRemove { lane: LaneSlot(0) }]);
    r.run(40 * block);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, start..added, |_| 1.0);
    assert_levels(&r.left, &reference, added + 240..removed, |_| {
        gain_of(-12.0)
    });
    assert_levels(&r.left, &reference, removed + 240..r.at, |_| 1.0);
}

#[test]
fn a_lane_slot_taken_by_a_track_lane_gives_the_master_its_fader_back() {
    let block = 128;
    let mut r = Rig::new(block);
    r.lane(
        0,
        MASTER,
        AutomationTarget::MixerGain,
        &[pt(0, -20.0, Curve::Step)],
    );
    let start = r.play(0);
    r.run(30 * block);
    let replaced = r.at;
    // The same slot now holds a pan lane of track 0: the master's gain lane is gone.
    r.lane(
        0,
        TrackSlot(0),
        AutomationTarget::MixerPan,
        &[pt(0, 0.0, Curve::Step)],
    );
    r.run(30 * block);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, start..replaced, |_| gain_of(-20.0));
    assert_levels(&r.left, &reference, replaced + 240..r.at, |_| 1.0);
}

#[test]
fn a_master_lane_goes_back_to_its_start_when_the_loop_wraps() {
    let block = 128;
    let mut r = Rig::new(block);
    r.lane(
        0,
        MASTER,
        AutomationTarget::MixerGain,
        &[pt(0, -20.0, Curve::Step), pt(500, -6.0, Curve::Step)],
    );
    r.send(&[M::SetLoop {
        region: engine::time::LoopRegion::new(Tick(0), Tick(1000)),
    }]);
    let start = r.play(0);
    // Two laps of 1 000 ticks (25 000 frames).
    r.run(52_000);
    r.check();
    let reference = reference(block, r.at);
    let lap = 1000 * FPT;
    for lap_start in [start, start + lap] {
        assert_levels(
            &r.left,
            &reference,
            lap_start + 2..lap_start + 500 * FPT,
            |_| gain_of(-20.0),
        );
        assert_levels(
            &r.left,
            &reference,
            lap_start + 500 * FPT + 2..lap_start + lap - 2,
            |_| gain_of(-6.0),
        );
    }
}

#[test]
fn a_hold_mix_on_the_master_holds_its_lane_and_gives_the_master_back_to_its_fader() {
    // #417: a gesture on the master that leaves the value as it was.
    let block = 128;
    let mut r = Rig::new(block);
    r.lane(
        0,
        MASTER,
        AutomationTarget::MixerGain,
        &[
            pt(0, -20.0, Curve::Step),
            pt(2000, -6.0, Curve::Step),
            pt(4000, 0.0, Curve::Step),
        ],
    );
    let start = r.play(0);
    r.run_to(start + 80 * block);
    // A `SetMaster` that repeats the fader holds nothing (this is what a mute looks like).
    r.master(1.0, 0.0);
    r.run(10 * block);
    let gesture = r.at;
    r.send(&[M::HoldMix {
        track: MASTER,
        gain: Some(1.0),
        pan: None,
    }]);
    r.run_to(start + 2000 * FPT + 10 * block);
    r.check();
    let reference = reference(block, r.at);
    let fade = 240;
    // Before the hold the lane plays; the repeated fader changed nothing.
    assert_levels(&r.left, &reference, start..gesture, |_| gain_of(-20.0));
    assert_levels(
        &r.left,
        &reference,
        gesture + fade + block..start + 2000 * FPT,
        |_| 1.0,
    );
    assert_levels(
        &r.left,
        &reference,
        start + 2000 * FPT..start + 2000 * FPT + 10 * block,
        |_| gain_of(-6.0),
    );
}

#[test]
fn a_master_mute_holds_no_lane() {
    let block = 128;
    let mut r = Rig::new(block);
    r.lane(
        0,
        MASTER,
        AutomationTarget::MixerGain,
        &[pt(0, -20.0, Curve::Step), pt(4000, 0.0, Curve::Step)],
    );
    let start = r.play(0);
    r.run(40 * block);
    // The mute repeats the fader values: it holds nothing, the lane keeps its level under it.
    let mute = r.at;
    r.send(&[M::SetMaster {
        gain: 1.0,
        pan: 0.0,
        mute: true,
    }]);
    r.run(40 * block);
    // The lane is still free: a lane held by the mute would show here, whatever its level.
    let manual = |e: &Engine| e.state().lanes.get(0).unwrap().run.manual();
    assert_eq!(manual(&r.e), Manual::Free);
    r.master(1.0, 0.0);
    r.run(40 * block);
    assert_eq!(manual(&r.e), Manual::Free);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, start..mute, |_| gain_of(-20.0));
    assert_levels(
        &r.left,
        &reference,
        mute + 240 + block..mute + 40 * block,
        |_| 0.0,
    );
    let back = mute + 80 * block - 40 * block;
    assert_levels(&r.left, &reference, back + 240 + block..r.at, |_| {
        gain_of(-20.0)
    });
}
