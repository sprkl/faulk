//! Automation playback (AM-2): lanes on the strip of a track and on a device's parameters,
//! read by the engine while the clock runs. Offline renders through the real `Preparer`,
//! `Engine::apply` and `Engine::render`, every audio-side call under the allocation
//! detector. The signal is a test tone through the test gain effect, so that a rendered
//! sample can be compared with the same sample of a render without lanes:
//! `out[k] = reference[k] × level(k)`.
//!
//! At 120 BPM and 48 kHz a tick is exactly 25 frames and a bar (3 840 ticks) 96 000 frames.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::automation::{CONTROL_FRAMES, Manual};
use engine::device::DeviceSetup;
use engine::device::testing::{GAIN, TONE, gain};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{
    AutomationPoint, AutomationTarget, ControlMsg as M, Curve, DeviceSlot, LaneSlot, ParamId,
    TrackKind, TrackSlot, encode_state,
};
use engine::time::{LoopRegion, Quantize, Tempo, Tick, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
/// Frames per tick at 120 BPM and 48 kHz.
const FPT: usize = 25;
const BAR: i64 = 3_840;
/// Frames rendered before the clock starts: beyond the fade in of a new track.
const WARM: usize = 4_096;
/// Largest difference allowed between a sample and its expected value (−120 dBFS).
const EXACT: f32 = 1e-6;
/// Largest difference allowed between a sample of a volume ramp and the ideal curve of the
/// fader's law: the strip draws chords of at most 64 frames of that curve (#389), and the
/// worst chord of the tests (a one-bar fade across the knee of the taper at -12 dB) is 9e-6 off (absolute, in sample amplitude: the per-bar dB measures hold the law to 0.05 dB).
const CHORD: f32 = 2e-5;

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

const TRIM: AutomationTarget = AutomationTarget::DeviceParam {
    device: DeviceSlot(1),
    param: gain::TRIM,
};
const SMOOTHED: AutomationTarget = AutomationTarget::DeviceParam {
    device: DeviceSlot(1),
    param: gain::GAIN,
};

struct Rig {
    p: Preparer,
    e: Engine,
    block: usize,
    /// Frames rendered so far.
    at: usize,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Rig {
    /// `tracks` tracks, each a tone through the test gain effect, rendered past their fade
    /// in, the clock stopped.
    fn new(block: usize, tracks: u32) -> Rig {
        Rig::at_tempo(block, tracks, BPM, RATE)
    }

    fn at_tempo(block: usize, tracks: u32, bpm: f64, rate: u32) -> Rig {
        let setup = DeviceSetup {
            sample_rate: rate as f32,
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
        r.send(&[M::SetTransport {
            tempo: Tempo::new(bpm).unwrap(),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::default(),
        }]);
        for t in 0..tracks {
            r.send(&[
                M::TrackCreate {
                    track: TrackSlot(t),
                    kind: TrackKind::Midi,
                },
                M::DeviceInsert {
                    track: TrackSlot(t),
                    device: DeviceSlot(0),
                    kind: TONE.kind_id,
                    state: encode_state(&[]),
                },
                M::DeviceInsert {
                    track: TrackSlot(t),
                    device: DeviceSlot(1),
                    kind: GAIN.kind_id,
                    state: encode_state(&[]),
                },
                M::NoteOn {
                    track: TrackSlot(t),
                    pitch: 57 + t as u8 % 24,
                    velocity: 127,
                },
            ]);
        }
        r.run(WARM.div_ceil(block) * block);
        r
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p.handle(m.clone()).unwrap();
        }
        self.p.flush();
    }

    fn lane(
        &mut self,
        lane: u32,
        track: u32,
        target: AutomationTarget,
        points: &[AutomationPoint],
    ) {
        self.send(&[M::AutomationLaneUpload {
            lane: LaneSlot(lane),
            track: TrackSlot(track),
            target,
            first: 0,
            total: points.len() as u32,
            points: points.to_vec(),
        }]);
    }

    fn mix(&mut self, gain: f32, pan: f32) {
        self.send(&[M::TrackSetMix {
            track: TrackSlot(0),
            gain,
            pan,
            mute: false,
        }]);
    }

    /// Renders `frames` frames in blocks of the rig's size (the last one shorter).
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

    /// Renders up to frame `to` (a block boundary when `to - at` is a multiple of the block).
    fn run_to(&mut self, to: usize) {
        self.run(to - self.at);
    }

    fn play(&mut self, from: i64) -> usize {
        self.send(&[M::Play { from: Tick(from) }]);
        self.at
    }

    fn check(&self) {
        for (i, s) in self.left.iter().chain(&self.right).enumerate() {
            assert!(s.is_finite(), "sample {i} is {s}");
            assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
        }
    }
}

/// The same rig without lanes or gestures: what each sample is at level 1.
fn reference(block: usize, frames: usize) -> Vec<f32> {
    let mut r = Rig::new(block, 1);
    r.play(0);
    r.run_to(frames);
    r.left
}

/// Asserts `out[k] = reference[k] × level(k)` over `range`, to the last bit that matters
/// ([`EXACT`]).
fn assert_levels(
    out: &[f32],
    reference: &[f32],
    range: core::ops::Range<usize>,
    level: impl Fn(usize) -> f32,
) {
    assert_levels_within(out, reference, range, EXACT, level);
}

/// The same for a volume ramp, which the strip draws in chords of the fader's curve: within
/// [`CHORD`] of the ideal curve.
fn assert_curve(
    out: &[f32],
    reference: &[f32],
    range: core::ops::Range<usize>,
    level: impl Fn(usize) -> f32,
) {
    assert_levels_within(out, reference, range, CHORD, level);
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

/// −96 dB to 0 dB over one bar, then held.
fn fade_in() -> [AutomationPoint; 2] {
    [pt(0, -96.0, Curve::Linear), pt(BAR, 0.0, Curve::Linear)]
}

/// The fader's law, written again here from its table (position, dB) so that the engine's is
/// checked against it, not against itself.
const LAW: [(f64, f64); 6] = [
    (0.0, -96.0),
    (0.08, -60.0),
    (0.28, -30.0),
    (0.5, -12.0),
    (0.78, 0.0),
    (1.0, 6.0),
];

/// The amplitude at a position of the fader's travel (0 at the bottom).
fn law_gain(position: f64) -> f32 {
    if position <= 0.0 {
        return 0.0;
    }
    let i = LAW.iter().position(|&(p, _)| position <= p).unwrap();
    let ((p0, d0), (p1, d1)) = (LAW[i - 1], LAW[i]);
    let db = d0 + (position - p0) / (p1 - p0) * (d1 - d0);
    10f64.powf(db / 20.0) as f32
}

/// The level at frame `k` of a ramp from -96 dB (position 0) to 0 dB (position 0.78) over
/// one bar, then held.
fn fade_level(k: usize) -> f32 {
    let t = (k as f64 / (BAR as usize * FPT) as f64).min(1.0);
    law_gain(0.78 * t)
}

#[test]
fn a_gain_lane_from_minus_96_to_0_db_over_one_bar_follows_the_fader_law_whatever_the_block_size() {
    let len = BAR as usize * FPT;
    let mut renders = Vec::new();
    for block in [33, 97, 128, 1024] {
        let mut r = Rig::new(block, 1);
        r.lane(0, 0, AutomationTarget::MixerGain, &fade_in());
        // Every rig starts the clock at the same frame.
        r.run_to(8_192);
        let start = r.play(0);
        r.run(len + 4_000);
        r.check();
        let reference = reference(block, r.at);
        let out = &r.left[start..];
        let reference = &reference[start..];
        assert_curve(out, reference, 0..len + 4_000, fade_level);
        // The first point on its sample (−96 dB, silence), the last one on its own (unity).
        assert_eq!(out[0], 0.0);
        assert!((out[len] - reference[len]).abs() <= 1e-7);
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
fn a_seek_into_a_ramp_has_the_right_level_on_its_first_sample() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(0, 0, AutomationTarget::MixerGain, &fade_in());
    let start = r.play(0);
    r.run(100 * block);
    // Half the bar: the playhead jumps from frame 12 800 of the ramp to frame 48 000.
    let seek = r.at;
    r.send(&[M::Seek {
        tick: Tick(BAR / 2),
    }]);
    r.run(200 * block);
    // From the stop, a play from three quarters of the bar.
    r.send(&[M::Stop]);
    r.run(10 * block);
    let replay = r.play(BAR * 3 / 4);
    r.run(100 * block);
    r.check();
    let reference = reference(block, r.at);
    assert_curve(&r.left, &reference, start..seek, |k| fade_level(k - start));
    let half = BAR as usize * FPT / 2;
    assert_curve(&r.left, &reference, seek..seek + 200 * block, |k| {
        fade_level(half + k - seek)
    });
    let three = BAR as usize * FPT * 3 / 4;
    assert_curve(&r.left, &reference, replay..replay + 100 * block, |k| {
        fade_level(three + k - replay)
    });
}

#[test]
fn a_step_lands_on_its_sample_and_the_lane_holds_before_and_after_its_points() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    // Tick 1 001 is frame 25 025, inside a block.
    r.lane(
        0,
        0,
        AutomationTarget::MixerGain,
        &[pt(100, -20.0, Curve::Step), pt(1001, -6.0, Curve::Step)],
    );
    let start = r.play(0);
    r.run(40_000);
    r.check();
    let reference = reference(block, r.at);
    let step = 1001 * FPT;
    assert_levels(&r.left, &reference, start..start + 40_000, |k| {
        if k - start < step {
            gain_of(-20.0)
        } else {
            gain_of(-6.0)
        }
    });
}

#[test]
fn a_loop_wrap_brings_the_lane_back_to_the_loop_start() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(0, 0, AutomationTarget::MixerGain, &fade_in());
    r.send(&[M::SetLoop {
        region: LoopRegion::new(Tick(0), Tick(BAR / 2)),
    }]);
    let start = r.play(0);
    let half = BAR as usize * FPT / 2;
    r.run(half * 2 + 1_000);
    r.check();
    let reference = reference(block, r.at);
    assert_curve(&r.left, &reference, start..start + half * 2 + 1_000, |k| {
        fade_level((k - start) % half)
    });
}

#[test]
fn a_device_lane_lands_on_its_points_and_moves_at_the_control_frames() {
    // Odd blocks: most block starts are not control frames.
    for block in [97, 128] {
        let mut r = Rig::new(block, 1);
        // The test effect's `trim` has no smoothing: the device applies each value at its
        // offset. A step at tick 1 001 (frame 25 025), then a ramp from 0.5 to 1.5.
        r.lane(
            0,
            0,
            TRIM,
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
            // The value of the last instant: a point's frame or a control frame (absolute).
            let instant = if f >= 1801 * FPT {
                1801 * FPT
            } else if f >= 1001 * FPT {
                let control = k - k % CONTROL_FRAMES as usize - start;
                control.max(1001 * FPT)
            } else {
                f
            };
            lane(instant)
        });
    }
}

#[test]
fn a_device_lane_renders_the_same_whatever_the_block_size() {
    // The smoothed `gain` of the test effect: each value starts a 10 ms ramp, at the same
    // frames whatever the blocks.
    let points: Vec<_> = (0..40)
        .map(|i| {
            pt(
                i * 97,
                0.2 + (i % 7) as f32 * 0.25,
                if i % 3 == 0 {
                    Curve::Step
                } else {
                    Curve::Linear
                },
            )
        })
        .collect();
    let frames = 40 * 97 * FPT + 2_000;
    let mut renders: Vec<Vec<f32>> = Vec::new();
    for block in [33, 128, 1024, 64] {
        let mut r = Rig::new(block, 1);
        r.lane(0, 0, SMOOTHED, &points);
        // Every rig starts the clock at the same absolute frame.
        r.run_to(WARM.div_ceil(1024) * 1024 * 2);
        let start = r.play(0);
        assert_eq!(start, 8_192);
        r.run(frames);
        r.check();
        renders.push(r.left[start..start + frames].to_vec());
    }
    for (i, r) in renders.iter().enumerate().skip(1) {
        if let Some(k) = r
            .iter()
            .zip(&renders[0])
            .position(|(a, b)| (a - b).abs() > EXACT)
        {
            eprintln!(
                "render {i} first differs at {k}: {} vs {}",
                r[k], renders[0][k]
            );
        }
        let diff = r
            .iter()
            .zip(&renders[0])
            .fold(0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(diff <= EXACT, "render {i} differs by {diff}");
    }
}

#[test]
fn a_gesture_holds_its_lane_until_the_next_point_and_a_jump_gives_it_back() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    let points = [
        pt(0, -20.0, Curve::Step),
        pt(2000, -6.0, Curve::Step),
        pt(4000, 0.0, Curve::Step),
    ];
    r.lane(0, 0, AutomationTarget::MixerGain, &points);
    let start = r.play(0);
    r.run_to(start + 80 * block);
    // A fader gesture at tick 409.6: it applies (a 5 ms ramp) until tick 2 000.
    let gesture = r.at;
    r.mix(0.5, 0.0);
    r.run_to(start + 2000 * FPT + 10 * block);
    let held = |e: &Engine| e.state().lanes.get(0).unwrap().run.manual();
    assert_eq!(held(&r.e), Manual::Free, "the lane took over at its point");
    // Another gesture, then a seek back into the same transition: the lane takes over at
    // the seek.
    r.mix(0.25, 0.0);
    r.run(20 * block);
    assert!(matches!(held(&r.e), Manual::Until(_)));
    let seek = r.at;
    r.send(&[M::Seek { tick: Tick(2400) }]);
    r.run(20 * block);
    // Stopped: a gesture stays.
    r.send(&[M::Stop]);
    r.run(4 * block);
    let stopped = r.at;
    r.mix(0.125, 0.0);
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
    assert_levels(
        &r.left,
        &reference,
        start + 2000 * FPT..start + 2000 * FPT + 10 * block,
        |_| gain_of(-6.0),
    );
    assert_levels(&r.left, &reference, seek..seek + 20 * block, |_| {
        gain_of(-6.0)
    });
    assert_levels(&r.left, &reference, stopped + fade..r.at, |_| 0.125);
}

#[test]
#[allow(clippy::needless_range_loop)] // Frames of two renders, side by side.
fn a_pan_gesture_does_not_hold_the_gain_lane() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(0, 0, AutomationTarget::MixerGain, &fade_in());
    let start = r.play(0);
    r.run(100 * block);
    // Panned left: the left channel stays at the gain (the near side keeps unity).
    let gesture = r.at;
    r.mix(1.0, -0.5);
    r.run(100 * block);
    assert_eq!(r.e.state().lanes.get(0).unwrap().run.manual(), Manual::Free);
    let reference = reference(block, r.at);
    // During the gesture's 5 ms ramp (240 frames) the lane's values glide with it; exact
    // before and after.
    let fade = 240;
    assert_curve(&r.left, &reference, start..gesture, |k| {
        fade_level(k - start)
    });
    for k in gesture..gesture + fade {
        let want = reference[k] * fade_level(k - start);
        assert!((r.left[k] - want).abs() < 1e-4, "frame {k}");
    }
    assert_curve(&r.left, &reference, gesture + fade..r.at, |k| {
        fade_level(k - start)
    });
    assert!(
        r.right[gesture + 1_000].abs() < r.left[gesture + 1_000].abs()
            || r.left[gesture + 1_000] == 0.0
    );
}

#[test]
fn a_device_gesture_holds_its_lane_until_the_next_point() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(
        0,
        0,
        TRIM,
        &[pt(0, 0.5, Curve::Step), pt(1000, 1.5, Curve::Step)],
    );
    let start = r.play(0);
    r.run(40 * block);
    let gesture = r.at;
    r.send(&[M::ParamSet {
        track: TrackSlot(0),
        device: DeviceSlot(1),
        param: gain::TRIM,
        value: 1.0,
    }]);
    r.run_to(start + 30_000);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, start..gesture, |_| 0.5);
    assert_levels(&r.left, &reference, gesture..start + 1000 * FPT, |_| 1.0);
    assert_levels(
        &r.left,
        &reference,
        start + 1000 * FPT..start + 30_000,
        |_| 1.5,
    );
}

#[test]
fn a_removed_gain_lane_gives_the_strip_back_to_its_fader() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(
        0,
        0,
        AutomationTarget::MixerGain,
        &[pt(0, -20.0, Curve::Step)],
    );
    r.lane(
        1,
        0,
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
    // Back to the fader (unity) after its 5 ms ramp; the pan lane still plays.
    assert_levels(&r.left, &reference, removed + 240..r.at, |_| 1.0);
    let right = r.right[r.at - 1] / reference[r.at - 1];
    assert!((right - (0.5f32 * core::f32::consts::FRAC_PI_2).cos()).abs() < 1e-5);
}

#[test]
#[allow(clippy::needless_range_loop)]
fn a_removed_pan_lane_gives_the_strip_back_to_its_fader() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(
        0,
        0,
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
    // The gain is unity throughout (no gain lane).
    assert_levels(&r.left, &reference, start..r.at, |_| 1.0);
    // Before the remove: the far channel (right) is attenuated by the pan.
    let panned = (0.5f32 * core::f32::consts::FRAC_PI_2).cos();
    for k in start..removed {
        if reference[k].abs() < 1e-3 {
            continue;
        }
        assert!(
            (r.right[k] / reference[k] - panned).abs() < 1e-4,
            "frame {k}: expected pan {panned}, got {}",
            r.right[k] / reference[k]
        );
    }
    // After the remove (past the 5 ms ramp): back to centre pan.
    let fade = 240;
    for k in removed + fade..r.at {
        if reference[k].abs() < 1e-3 {
            continue;
        }
        assert!(
            (r.right[k] / reference[k] - 1.0).abs() < 1e-4,
            "frame {k}: expected pan 1.0, got {}",
            r.right[k] / reference[k]
        );
    }
}

#[test]
fn a_gesture_after_the_last_point_holds_until_a_jump_and_a_new_upload_keeps_it() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    let points = [pt(0, -20.0, Curve::Step), pt(100, -6.0, Curve::Step)];
    r.lane(0, 0, AutomationTarget::MixerGain, &points);
    let start = r.play(0);
    r.run_to(start + 40 * block);
    let held = |e: &Engine| e.state().lanes.get(0).unwrap().run.manual();
    // Past the last point: the gesture has no next point to wait for.
    let gesture = r.at;
    r.mix(0.5, 0.0);
    r.run(40 * block);
    assert_eq!(held(&r.e), Manual::Until(2));
    // The points edited (the lane uploaded again): still held.
    r.lane(
        0,
        0,
        AutomationTarget::MixerGain,
        &[pt(0, -12.0, Curve::Step)],
    );
    r.run(40 * block);
    assert!(matches!(held(&r.e), Manual::Until(_)));
    // A jump gives the gain back to the new points.
    let seek = r.at;
    r.send(&[M::Seek { tick: Tick(0) }]);
    r.run(20 * block);
    assert_eq!(held(&r.e), Manual::Free);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, gesture + 240..seek, |_| 0.5);
    assert_levels(&r.left, &reference, seek..r.at, |_| gain_of(-12.0));
}

#[test]
fn a_track_removed_under_its_lanes_and_dense_notes_with_neutral_lanes_render_cleanly() {
    let block = 128;
    let mut r = Rig::new(block, 2);
    for lane in 0..4 {
        r.lane(
            lane,
            1,
            TRIM,
            &[pt(0, 0.5, Curve::Linear), pt(BAR, 1.5, Curve::Step)],
        );
    }
    r.play(0);
    r.run(10 * block);
    // The lanes stay in the table, unlinked: nothing plays them, nothing breaks.
    r.send(&[M::TrackRemove {
        track: TrackSlot(1),
    }]);
    r.run(40 * block);
    assert_eq!(r.e.state().lanes.iter().count(), 4);
    r.check();
}

#[test]
fn neutral_lanes_leave_a_dense_schedule_untouched() {
    use engine::events::{SCHEDULE_FIRST_VOICE, Schedule, ScheduledNote};
    // 3 000 notes in two seconds: the schedule fills the sink and cuts the sub-blocks
    // (`Read::Full`) inside the lanes' pieces, which a gain at 0 dB, a trim at 1 and a pan
    // between the centre and the left cut at every point and control frame.
    let notes: Vec<ScheduledNote> = (0..3_000i64)
        .map(|i| ScheduledNote {
            start: Tick((i * 7) % 3_840 / 4 * 4),
            length: Tick(5 + i % 40),
            key: 40 + (i % 40) as u8,
            velocity: 0.5,
            tune_cents: 0.0,
        })
        .collect();
    let render = |lanes: bool| {
        let mut r = Rig::new(128, 1);
        let schedule = Schedule::try_new(&notes, &[], SCHEDULE_FIRST_VOICE).unwrap();
        drop(
            r.e.state_mut()
                .track_mut(TrackSlot(0))
                .unwrap()
                .graph
                .set_schedule(Some(Box::new(schedule))),
        );
        if lanes {
            let steps: Vec<_> = (0..200).map(|i| pt(i * 19, 0.0, Curve::Step)).collect();
            r.lane(0, 0, AutomationTarget::MixerGain, &steps);
            let ones: Vec<_> = (0..200).map(|i| pt(i * 23, 1.0, Curve::Linear)).collect();
            r.lane(1, 0, TRIM, &ones);
            let pans: Vec<_> = (0..20)
                .map(|i| pt(i * 199, if i % 2 == 0 { 0.0 } else { -1.0 }, Curve::Linear))
                .collect();
            r.lane(2, 0, AutomationTarget::MixerPan, &pans);
        }
        r.play(0);
        r.run(96_000);
        r.check();
        r.left
    };
    let plain = render(false);
    let laned = render(true);
    assert!(plain.iter().any(|s| s.abs() > 0.01));
    assert_eq!(plain.iter().zip(&laned).position(|(a, b)| a != b), None);
}

#[test]
#[allow(clippy::needless_range_loop)] // Frames of two renders, side by side.
fn a_gesture_on_the_other_field_keeps_its_ramp_under_a_lane() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(
        0,
        0,
        AutomationTarget::MixerGain,
        &[pt(0, -6.0, Curve::Step)],
    );
    r.play(0);
    r.run(20 * block);
    // A pan gesture: the far channel (right) goes down over 5 ms (240 frames), not at once.
    let gesture = r.at;
    r.mix(1.0, -0.5);
    r.run(20 * block);
    r.check();
    let reference = reference(block, r.at);
    let g = gain_of(-6.0);
    let far = (0.5f32 * core::f32::consts::FRAC_PI_2).cos();
    let level = |k: usize| r.right[k] / reference[k];
    let mut last = g;
    for k in gesture..gesture + 400 {
        if reference[k].abs() < 1e-3 {
            continue;
        }
        let l = level(k);
        let want = if k - gesture >= 240 {
            g * far
        } else {
            g * (1.0 - (1.0 - far) * (k - gesture) as f32 / 240.0)
        };
        assert!(
            (l - want).abs() < 1e-3,
            "frame {}: level {l} instead of {want}",
            k - gesture
        );
        assert!(
            last - l < 0.01,
            "a step of {} at frame {}",
            last - l,
            k - gesture
        );
        last = l;
    }
    // The gain lane still owns the gain: the near side stays at −6 dB.
    assert_levels(&r.left, &reference, gesture..r.at, |_| g);
}

#[test]
fn a_device_put_again_on_its_slot_gets_the_lane_value_again() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    // A flat lane: its value goes out once.
    r.lane(0, 0, TRIM, &[pt(0, 0.5, Curve::Step)]);
    r.play(0);
    r.run(20 * block);
    // The effect built again in its slot (an undo, a new state): trim back at 1, then the
    // lane's 0.5 at the next sub-block.
    let put = r.at;
    r.send(&[M::DeviceInsert {
        track: TrackSlot(0),
        device: DeviceSlot(1),
        kind: GAIN.kind_id,
        state: encode_state(&[]),
    }]);
    r.run(20 * block);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, put..put + 20 * block, |_| 0.5);
}

#[test]
fn of_two_lanes_on_one_parameter_the_lower_slot_wins() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(
        3,
        0,
        AutomationTarget::MixerGain,
        &[pt(0, -12.0, Curve::Step)],
    );
    r.lane(
        1,
        0,
        AutomationTarget::MixerGain,
        &[pt(0, -6.0, Curve::Step)],
    );
    let start = r.play(0);
    r.run(20 * block);
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, start..r.at, |_| gain_of(-6.0));
}

#[test]
fn points_closer_than_a_sample_take_the_last_one_reached() {
    // 999 BPM at 8 kHz: half a sample per tick.
    let block = 64;
    let mut r = Rig::at_tempo(block, 1, 999.0, 8_000);
    let points: Vec<_> = (0..40)
        .map(|i| pt(i, 0.1 + i as f32 * 0.02, Curve::Step))
        .collect();
    r.lane(0, 0, TRIM, &points);
    let start = r.play(0);
    r.run(4 * block);
    r.check();
    let mut q = Rig::at_tempo(block, 1, 999.0, 8_000);
    q.play(0);
    q.run_to(r.at);
    let spt = 8_000.0 * 60.0 / (999.0 * 960.0);
    assert_levels(&r.left, &q.left, start..start + 4 * block, |k| {
        let f = (k - start) as f64;
        // The last tick whose sample is at or before this frame.
        let tick = (((f + 1.0) / spt).ceil() as i64 - 1).clamp(0, 39);
        0.1 + tick as f32 * 0.02
    });
}

#[test]
fn a_hundred_lanes_of_a_thousand_points_play_without_allocating() {
    let block = 128;
    let tracks = 10u32;
    let mut r = Rig::new(block, tracks);
    let mut lane = 0;
    for t in 0..tracks {
        for k in 0..10u32 {
            let target = match k {
                0 => AutomationTarget::MixerGain,
                1 => AutomationTarget::MixerPan,
                2 | 5 | 8 => TRIM,
                3 | 6 | 9 => SMOOTHED,
                _ => AutomationTarget::DeviceParam {
                    device: DeviceSlot(0),
                    param: ParamId(0),
                },
            };
            let points: Vec<_> = (0..1_000i64)
                .map(|i| {
                    let x = ((i * 7 + i64::from(lane) * 13) % 100) as f32 / 100.0;
                    let value = match target {
                        AutomationTarget::MixerGain => -40.0 + x * 40.0,
                        AutomationTarget::MixerPan => x * 2.0 - 1.0,
                        _ => x,
                    };
                    pt(
                        i * 17 + i64::from(k),
                        value,
                        if i % 4 == 0 {
                            Curve::Step
                        } else {
                            Curve::Linear
                        },
                    )
                })
                .collect();
            // In chunks, as the compiler sends them.
            for (c, chunk) in points.chunks(256).enumerate() {
                r.send(&[M::AutomationLaneUpload {
                    lane: LaneSlot(lane),
                    track: TrackSlot(t),
                    target,
                    first: (c * 256) as u32,
                    total: points.len() as u32,
                    points: chunk.to_vec(),
                }]);
            }
            lane += 1;
        }
    }
    r.run(block);
    assert_eq!(r.e.state().lanes.iter().count(), 100);
    r.send(&[M::SetLoop {
        region: LoopRegion::new(Tick(1_000), Tick(9_000)),
    }]);
    r.play(0);
    for step in 0..30 {
        r.run(1_000 + step * 37);
        match step % 6 {
            1 => r.send(&[M::Seek {
                tick: Tick(step as i64 * 531),
            }]),
            2 => r.mix(0.5 + step as f32 * 0.01, 0.1),
            3 => r.send(&[M::ParamSet {
                track: TrackSlot(step as u32 % tracks),
                device: DeviceSlot(1),
                param: gain::TRIM,
                value: 0.7,
            }]),
            4 => r.send(&[M::Stop]),
            5 => {
                r.play(step as i64 * 401);
            }
            _ => {}
        }
    }
    r.check();
    assert!(r.left.iter().any(|s| s.abs() > 0.01));
}

/// 0 dB to -96 dB over eight bars: the level at the start of each bar, in dB, by the law of
/// the fader (a position that goes down by an eighth of 0.78 per bar), worked out by hand.
const BAR_DB: [f64; 9] = [
    0.0,
    -4.178_571_428_6,
    -8.357_142_857_1,
    -13.022_727_272_7,
    -21.0,
    -28.977_272_727_3,
    -42.75,
    -57.375,
    f64::NEG_INFINITY,
];

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
fn an_eight_bar_fade_from_0_db_to_minus_infinity_goes_down_like_a_fader_pulled_at_constant_speed() {
    let bar = BAR as usize * FPT;
    let len = 8 * bar;
    let mut renders = Vec::new();
    for block in [33, 128, 1024] {
        let mut r = Rig::new(block, 1);
        r.lane(
            0,
            0,
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
        assert_curve(out, reference, 0..len + 4_000, level);
        // The level at each bar line, measured.
        for (i, want) in BAR_DB.iter().enumerate().take(8).skip(1) {
            let k = i * bar;
            let got = measured_db(out, reference, k);
            assert!(
                (got - want).abs() < 0.05,
                "bar {i}: {got} dB instead of {want} dB"
            );
        }
        // The end of the fade is silence, exactly, and stays so.
        assert!(out[len..len + 4_000].iter().all(|x| *x == 0.0));
        // Not the amplitude law (half way would be -6 dB) and not the dB law (-48 dB): the
        // middle of the travel, 0.39, is -21 dB.
        let mid = measured_db(out, reference, 4 * bar);
        assert!((mid + 21.0).abs() < 0.05, "{mid} dB at the fourth bar");
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
fn a_step_and_a_flat_linear_segment_of_a_volume_lane_hold_their_exact_gain() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(
        0,
        0,
        AutomationTarget::MixerGain,
        &[
            pt(0, 0.0, Curve::Linear),
            pt(100, 0.0, Curve::Step),
            pt(200, -6.0, Curve::Linear),
            pt(300, -6.0, Curve::Linear),
        ],
    );
    let start = r.play(0);
    r.run(12_000);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, start..start + 200 * FPT, |_| 1.0);
    assert_levels(
        &r.left,
        &reference,
        start + 200 * FPT..start + 11_000,
        |_| gain_of(-6.0),
    );
}

#[test]
fn a_gain_lane_outside_the_fader_range_is_clamped_not_a_panic() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(
        0,
        0,
        AutomationTarget::MixerGain,
        &[pt(0, 40.0, Curve::Linear), pt(100, -300.0, Curve::Linear)],
    );
    let start = r.play(0);
    r.run(6_000);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, start..start + 1, |_| gain_of(6.0));
    assert_levels(&r.left, &reference, start + 2_600..start + 6_000, |_| 0.0);
}

// ---------------------------------------------------------------- stopped (#388)

#[test]
fn a_stop_and_every_seek_apply_the_lanes_value_at_the_playhead() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    // Before its first point (tick 100) the lane holds the first value.
    let points = [
        pt(100, -20.0, Curve::Step),
        pt(1000, -6.0, Curve::Step),
        pt(2000, -12.0, Curve::Step),
    ];
    r.lane(0, 0, AutomationTarget::MixerGain, &points);
    // The clock has not run: the lane already applies at the playhead (tick 0).
    let before = r.at;
    r.run(20 * block);
    let start = r.play(0);
    r.run_to(start + 1500 * FPT);
    // The stop applies the lane's value where the clock stands: -6 dB.
    r.send(&[M::Stop]);
    let stopped = r.at;
    r.run(20 * block);
    // Stopped seeks: before the first point, on a point, just before it, past the last.
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
}

#[test]
fn a_stopped_volume_lane_on_a_slope_applies_the_level_at_the_playhead() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(0, 0, AutomationTarget::MixerGain, &fade_in());
    let start = r.play(0);
    r.run(100 * block);
    r.send(&[M::Stop]);
    // The stop took effect at the start of the next block: the playhead stands there.
    let stop = r.at;
    let head = stop - start;
    r.run(40 * block);
    r.send(&[M::Seek {
        tick: Tick(BAR / 4),
    }]);
    let seek = r.at;
    r.run(40 * block);
    r.check();
    let reference = reference(block, r.at);
    assert_curve(&r.left, &reference, stop..stop + 40 * block, |_| {
        fade_level(head)
    });
    assert_curve(&r.left, &reference, seek..seek + 40 * block, |_| {
        fade_level(BAR as usize * FPT / 4)
    });
}

#[test]
fn a_gesture_while_stopped_holds_until_the_next_seek_or_play() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    let points = [pt(0, -20.0, Curve::Step), pt(1000, -6.0, Curve::Step)];
    r.lane(0, 0, AutomationTarget::MixerGain, &points);
    r.run(10 * block);
    let held = |e: &Engine| e.state().lanes.get(0).unwrap().run.manual();
    // The gesture holds over many blocks, and a lane that is edited keeps the hold.
    let gesture = r.at;
    r.mix(0.5, 0.0);
    r.run(40 * block);
    assert_eq!(held(&r.e), Manual::Held);
    r.lane(0, 0, AutomationTarget::MixerGain, &points[..1]);
    r.run(40 * block);
    assert_eq!(held(&r.e), Manual::Held);
    // A seek gives the strip back to the lane.
    let seek = r.at;
    r.send(&[M::Seek { tick: Tick(1500) }]);
    r.run(40 * block);
    assert_eq!(held(&r.e), Manual::Free);
    // A second gesture, then a seek to the tick where the playhead already stands: it is a
    // seek all the same, the lane has the hand again.
    r.mix(0.25, 0.0);
    r.run(40 * block);
    assert_eq!(held(&r.e), Manual::Held);
    let again = r.at;
    r.send(&[M::Seek { tick: Tick(1500) }]);
    r.run(40 * block);
    assert_eq!(held(&r.e), Manual::Free);
    // A third gesture, then a play: the lane has the hand again.
    r.mix(0.125, 0.0);
    r.run(40 * block);
    assert_eq!(held(&r.e), Manual::Held);
    let play = r.play(1500);
    r.run(40 * block);
    r.check();
    let reference = reference(block, r.at);
    let fade = 240;
    assert_levels(&r.left, &reference, gesture + fade..seek, |_| 0.5);
    assert_levels(&r.left, &reference, seek..seek + 40 * block, |_| {
        gain_of(-20.0)
    });
    assert_levels(
        &r.left,
        &reference,
        again + fade..again + 40 * block,
        |_| gain_of(-20.0),
    );
    assert_levels(&r.left, &reference, play..r.at, |_| gain_of(-20.0));
}

#[test]
fn an_edited_lane_and_a_rebuilt_device_apply_again_while_stopped() {
    let block = 128;
    let mut r = Rig::new(block, 1);
    r.lane(0, 0, TRIM, &[pt(0, 0.5, Curve::Step)]);
    r.run(40 * block);
    // Points edited while stopped: the new value applies without a seek.
    let edited = r.at;
    r.lane(0, 0, TRIM, &[pt(0, 1.5, Curve::Step)]);
    r.run(40 * block);
    // The effect built again in its slot: trim back at 1, then the lane's value.
    let put = r.at;
    r.send(&[M::DeviceInsert {
        track: TrackSlot(0),
        device: DeviceSlot(1),
        kind: GAIN.kind_id,
        state: encode_state(&[]),
    }]);
    r.run(40 * block);
    r.check();
    let reference = reference(block, r.at);
    assert_levels(&r.left, &reference, edited + 10 * block..put, |_| 1.5);
    assert_levels(&r.left, &reference, put + 10 * block..r.at, |_| 1.5);
}
