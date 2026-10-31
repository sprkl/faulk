//! The mixer (MX-1): the gain, the pan law, mute and solo, the master, and the timed
//! parameters a schedule addresses to the strip of a track or of the master. Offline renders
//! through the real `Preparer`, `Engine::apply` and `Engine::render`, every audio-side call
//! under the allocation detector. The signal is a test tone, so that a rendered sample can be
//! compared with the same sample of an unmixed render: `out[k] = tone[k] × level(k)`.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::DeviceSetup;
use engine::device::testing::TONE;
use engine::engine::{Engine, Preparer, channel};
use engine::events::{SCHEDULE_FIRST_VOICE, Schedule, ScheduledParam};
use engine::graph::{FADE_SECONDS, fade_frames};
use engine::mixer::{GAIN, MIXER_SLOT, MUTE, PAN, balance, mix_levels};
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackMeter, TrackSlot, encode_state,
};
use engine::time::{Quantize, Tempo, Tick, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
/// Level of a test tone at velocity 127: its gain (default 0.5) × 0.25.
const TONE_LEVEL: f32 = 0.125;
/// Frames before the clock starts: well beyond the fade-in of a new track.
const WARM: usize = 2_048;

fn db(x: f32) -> f32 {
    20.0 * x.log10()
}

fn gain_of(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

struct Rig {
    p: Preparer,
    e: Engine,
    block: usize,
}

impl Rig {
    fn new(block: usize) -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE as f32,
            max_block_frames: block,
        };
        let (p, e) = channel(setup, engine::device::testing::factory).unwrap();
        let mut r = Rig { p, e, block };
        r.send(&[M::SetTransport {
            tempo: Tempo::new(BPM).unwrap(),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::default(),
        }]);
        r
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p.handle(m.clone()).unwrap();
        }
        self.p.flush();
    }

    /// A track with a tone playing `pitch`; returns after the fade-in.
    fn tone(&mut self, slot: u32, pitch: u8) {
        self.send(&[
            M::TrackCreate {
                track: TrackSlot(slot),
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TrackSlot(slot),
                device: DeviceSlot(0),
                kind: TONE.kind_id,
                state: encode_state(&[]),
            },
            M::NoteOn {
                track: TrackSlot(slot),
                pitch,
                velocity: 127,
            },
        ]);
    }

    fn mix(&mut self, slot: u32, gain: f32, pan: f32, mute: bool) {
        self.send(&[M::TrackSetMix {
            track: TrackSlot(slot),
            gain,
            pan,
            mute,
        }]);
    }

    fn master(&mut self, gain: f32, mute: bool) {
        self.send(&[M::SetMaster {
            gain,
            pan: 0.0,
            mute,
        }]);
    }

    /// Renders `frames` frames in blocks of the rig's size; returns (left, right).
    fn run(&mut self, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let (mut l, mut r) = (vec![0.0; self.block], vec![0.0; self.block]);
        let mut todo = frames;
        while todo > 0 {
            let n = todo.min(self.block);
            assert_no_alloc(|| self.e.process(&mut l[..n], &mut r[..n]));
            left.extend_from_slice(&l[..n]);
            right.extend_from_slice(&r[..n]);
            todo -= n;
            self.p.collect();
        }
        check(&left);
        check(&right);
        // `check`'s is_finite is vacuous since the output protection (#402) silences non-finite
        // samples; this is the real no-NaN guard for the track summing and mixer paths.
        assert_eq!(
            self.e.limiter_silenced(),
            0,
            "the mix produced a non-finite sample"
        );
        (left, right)
    }

    /// Puts a schedule of mixer values on a track (`Some`) or on the master (`None`).
    fn schedule(&mut self, track: Option<u32>, params: &[ScheduledParam]) {
        let s = Box::new(Schedule::try_new(&[], params, SCHEDULE_FIRST_VOICE).unwrap());
        let old = match track {
            Some(t) => self
                .e
                .state_mut()
                .track_mut(TrackSlot(t))
                .unwrap()
                .graph
                .set_schedule(Some(s)),
            None => self.e.state_mut().set_master_schedule(Some(s)),
        };
        drop(old);
    }

    fn play(&mut self) {
        self.send(&[M::Play { from: Tick(0) }]);
    }
}

fn check(x: &[f32]) {
    for (i, s) in x.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is {s}");
        assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// The frame, from the start of the clock, of `tick` at `BPM`: the floor of its fractional
/// sample (TR-1's rule).
fn frame_of(tick: i64) -> usize {
    (tick as f64 * 60.0 * f64::from(RATE) / (BPM * 960.0)).floor() as usize
}

fn param(tick: i64, id: ParamId, value: f32) -> ScheduledParam {
    ScheduledParam {
        tick: Tick(tick),
        device: MIXER_SLOT,
        id,
        value,
    }
}

/// A tone track after its fade-in, and the unmixed render of the same tone over `frames`
/// frames from the clock's start (`left` = `right`): the reference every test compares to.
fn reference(block: usize, frames: usize) -> Vec<f32> {
    let mut r = Rig::new(block);
    r.tone(0, 69);
    r.run(WARM);
    r.play();
    r.run(frames).0
}

// ---------------------------------------------------------------- the law

#[test]
fn the_pan_law_keeps_unity_at_the_centre_and_lowers_the_far_side() {
    assert_eq!(balance(0.0), (1.0, 1.0));
    // The near side stays at unity, the far side follows a quarter cosine.
    let (l, r) = balance(0.5);
    assert_eq!(r, 1.0);
    assert!((db(l) + 3.0103).abs() < 1e-3, "half pan: {} dB", db(l));
    let (l, r) = balance(-0.5);
    assert_eq!(l, 1.0);
    assert!((db(r) + 3.0103).abs() < 1e-3);
    assert_eq!(balance(1.0), (0.0, 1.0));
    assert_eq!(balance(-1.0), (1.0, 0.0));
    // Out of range is the hard pan; not finite is the centre.
    assert_eq!(balance(7.0), (0.0, 1.0));
    assert_eq!(balance(-7.0), (1.0, 0.0));
    assert_eq!(balance(f32::NAN), (1.0, 1.0));
    assert_eq!(balance(f32::INFINITY), (1.0, 1.0));
    // Each far gain falls monotonically from unity to 0, and the slope is flat at the centre.
    let mut last = 1.0;
    for i in 1..=100 {
        let far = balance(i as f32 / 100.0).0;
        assert!(far <= last && (0.0..=1.0).contains(&far), "pan {i}%");
        last = far;
    }
    assert!(1.0 - balance(0.01).0 < 2e-4, "no jump near the centre");
    // The levels: audible x gain x balance.
    assert_eq!(mix_levels(2.0, 1.0, true), (0.0, 2.0));
    assert_eq!(mix_levels(1.0, 0.0, false), (0.0, 0.0));
    assert_eq!(mix_levels(-1.0, 0.0, true), (0.0, 0.0));
    assert_eq!(mix_levels(f32::NAN, 0.0, true), (0.0, 0.0));
}

// ---------------------------------------------------------------- measured levels

#[test]
fn a_requested_level_is_the_measured_level() {
    let mut base = Rig::new(128);
    base.tone(0, 69);
    base.run(WARM);
    let unity = rms(&base.run(4_800).0);
    assert!((unity - TONE_LEVEL / 2f32.sqrt()).abs() < 1e-3, "{unity}");
    for requested in [-96.0f32, -60.0, -20.0, -12.0, -6.0, -1.0, 0.0, 3.0, 6.0] {
        // On a track.
        let mut r = Rig::new(128);
        r.tone(0, 69);
        r.mix(0, gain_of(requested), 0.0, false);
        r.run(WARM);
        let (l, rr) = r.run(4_800);
        let (measured_l, measured_r) = (db(rms(&l) / unity), db(rms(&rr) / unity));
        assert!(
            (measured_l - requested).abs() < 0.01 && (measured_r - requested).abs() < 0.01,
            "track: {requested} dB requested, {measured_l} and {measured_r} measured"
        );
        // On the master.
        let mut r = Rig::new(128);
        r.tone(0, 69);
        r.master(gain_of(requested), false);
        r.run(WARM);
        let (l, _) = r.run(4_800);
        let measured = db(rms(&l) / unity);
        assert!(
            (measured - requested).abs() < 0.01,
            "master: {requested} dB requested, {measured} measured"
        );
    }
    println!("measured: -6 dB requested = {:.4} dB", {
        let mut r = Rig::new(128);
        r.tone(0, 69);
        r.mix(0, gain_of(-6.0), 0.0, false);
        r.run(WARM);
        db(rms(&r.run(4_800).0) / unity)
    });
}

#[test]
fn the_pan_lowers_only_the_far_channel_by_the_law() {
    let mut base = Rig::new(128);
    base.tone(0, 69);
    base.run(WARM);
    let unity = rms(&base.run(4_800).0);
    for pan in [-1.0f32, -0.5, -0.1, 0.0, 0.25, 0.5, 1.0] {
        let mut r = Rig::new(128);
        r.tone(0, 69);
        r.mix(0, 1.0, pan, false);
        r.run(WARM);
        let (l, rr) = r.run(4_800);
        let (el, er) = balance(pan);
        assert!((rms(&l) / unity - el).abs() < 1e-3, "pan {pan}: left");
        assert!((rms(&rr) / unity - er).abs() < 1e-3, "pan {pan}: right");
    }
}

// ---------------------------------------------------------------- no click

#[test]
fn mute_and_solo_never_click() {
    let n = fade_frames(RATE as f32) as usize;
    assert_eq!(n, (FADE_SECONDS * f64::from(RATE)).round() as usize);
    let mut r = Rig::new(128);
    r.tone(0, 57);
    r.tone(1, 69);
    let steady = r.run(WARM).0;
    // The steepest step of the unmixed signal, plus one step of a full-scale ramp.
    let bound = (max_step(&steady[WARM / 2..]) + 2.0 * TONE_LEVEL / n as f32) * 1.05;
    let solo = |track: u32, solo: bool| M::TrackSetSolo {
        track: TrackSlot(track),
        solo,
    };
    let gestures: Vec<(&str, Vec<M>)> = vec![
        (
            "mute 0",
            vec![M::TrackSetMix {
                track: TrackSlot(0),
                gain: 1.0,
                pan: 0.0,
                mute: true,
            }],
        ),
        (
            "unmute 0",
            vec![M::TrackSetMix {
                track: TrackSlot(0),
                gain: 1.0,
                pan: 0.0,
                mute: false,
            }],
        ),
        ("solo 1", vec![solo(1, true)]),
        ("solo 0 too", vec![solo(0, true)]),
        ("unsolo 1", vec![solo(1, false)]),
        ("unsolo 0", vec![solo(0, false)]),
        (
            "master mute",
            vec![M::SetMaster {
                gain: 1.0,
                pan: 0.0,
                mute: true,
            }],
        ),
        (
            "master unmute",
            vec![M::SetMaster {
                gain: 1.0,
                pan: 0.0,
                mute: false,
            }],
        ),
    ];
    for (name, msgs) in gestures {
        r.send(&msgs);
        let (l, rr) = r.run(8 * 128);
        for (side, x) in [("left", &l), ("right", &rr)] {
            assert!(
                max_step(x) <= bound,
                "{name}, {side}: step {} above {bound}",
                max_step(x)
            );
        }
        // A gesture lasts one fade: the render after it is steady again.
        assert!(max_step(&l[l.len() - 256..]) <= max_step(&steady[WARM / 2..]) * 1.05);
    }
}

#[test]
fn a_timed_mute_fades_and_a_timed_unmute_fades_back() {
    let n = fade_frames(RATE as f32) as usize;
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.run(WARM);
    // Muted at tick 960 (one beat), unmuted at tick 1920.
    r.schedule(Some(0), &[param(960, MUTE, 1.0), param(1_920, MUTE, 0.0)]);
    r.play();
    let (l, _) = r.run(frame_of(2_880));
    let reference = reference(128, frame_of(2_880));
    let (a, b) = (frame_of(960), frame_of(1_920));
    // Before the mute: the signal. During the fade: never above it, never a jump.
    for k in 0..a {
        assert_eq!(l[k], reference[k], "frame {k}");
    }
    assert_eq!(peak(&l[a + n..b]), 0.0, "silent once the fade is over");
    assert!(max_step(&l[a - 4..b]) <= (max_step(&reference) + 2.0 * TONE_LEVEL / n as f32) * 1.05);
    assert!(max_step(&l[b - 4..]) <= (max_step(&reference) + 2.0 * TONE_LEVEL / n as f32) * 1.05);
    assert!(
        (peak(&l[b + n + 200..]) - TONE_LEVEL).abs() < 1e-3,
        "back to the signal"
    );
}

// ---------------------------------------------------------------- timed parameters

/// Per-sample level of a lane of `(tick, value)` points held until the next one, `initial`
/// before the first (an event takes effect at its own sample, floored).
fn level_at(points: &[(i64, f32)], initial: f32, frame: usize) -> f32 {
    points
        .iter()
        .take_while(|(tick, _)| frame_of(*tick) <= frame)
        .last()
        .map_or(initial, |p| p.1)
}

#[test]
fn a_volume_ramp_received_as_timed_events_is_followed_to_the_sample() {
    // One event per tick (25 samples apart at 120 BPM) from 1.0 down to 0.05 and back to 1.
    let down: Vec<(i64, f32)> = (0..=240)
        .map(|i| (i as i64, 1.0 - 0.95 * i as f32 / 240.0))
        .collect();
    let up: Vec<(i64, f32)> = (1..=240)
        .map(|i| (240 + i as i64, 0.05 + 0.95 * i as f32 / 240.0))
        .collect();
    let points: Vec<(i64, f32)> = down.into_iter().chain(up).collect();
    let params: Vec<ScheduledParam> = points.iter().map(|(t, v)| param(*t, GAIN, *v)).collect();
    let frames = frame_of(500) + 600;
    // The result does not depend on the block size, odd ones included.
    let mut outputs = Vec::new();
    for block in [128usize, 97, 1_024, 33] {
        let reference = reference(block, frames);
        let mut r = Rig::new(block);
        r.tone(0, 69);
        r.run(WARM);
        r.schedule(Some(0), &params);
        r.play();
        let (l, rr) = r.run(frames);
        assert_eq!(l, rr, "centred: both channels equal");
        let mut worst = 0f32;
        for k in 0..frames {
            let expected = reference[k] * level_at(&points, 1.0, k);
            worst = worst.max((l[k] - expected).abs());
        }
        assert!(worst < 1e-6, "block {block}: off by {worst}");
        // The ramp is a ramp: the gain, read off the render, at the frame of an event.
        for (tick, value) in points.iter().step_by(37) {
            let k = frame_of(*tick);
            if reference[k].abs() > 0.05 {
                assert!((l[k] / reference[k] - value).abs() < 1e-5, "tick {tick}");
            }
        }
        outputs.push(l);
    }
    for o in &outputs[1..] {
        assert_eq!(o, &outputs[0], "the block size changes nothing");
    }
}

#[test]
fn a_timed_pan_and_a_timed_gain_apply_at_their_sample() {
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.run(WARM);
    let pan = 0.5f32;
    // Hard right at tick 480, back to the centre at tick 960, gain 0.5 from tick 1440.
    let points = [
        param(480, PAN, 1.0),
        param(960, PAN, 0.0),
        param(1_440, GAIN, 0.5),
        param(1_440, PAN, pan),
    ];
    r.schedule(Some(0), &points);
    r.play();
    let frames = frame_of(1_920);
    let (l, rr) = r.run(frames);
    let reference = reference(128, frames);
    let (a, b, c) = (frame_of(480), frame_of(960), frame_of(1_440));
    for k in 0..frames {
        let (gain, p) = if k < a {
            (1.0, 0.0)
        } else if k < b {
            (1.0, 1.0)
        } else if k < c {
            (1.0, 0.0)
        } else {
            (0.5, pan)
        };
        let (el, er) = balance(p);
        assert!((l[k] - reference[k] * gain * el).abs() < 1e-6, "left {k}");
        assert!((rr[k] - reference[k] * gain * er).abs() < 1e-6, "right {k}");
    }
    assert_eq!(
        peak(&l[a..b]),
        0.0,
        "hard right: the left is exactly silent"
    );
}

#[test]
fn the_master_follows_its_own_timed_gain_pan_and_mute() {
    let frames = frame_of(1_920);
    let reference = reference(128, frames);
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.run(WARM);
    r.schedule(
        None,
        &[
            param(0, GAIN, 0.5),
            param(480, PAN, -1.0),
            param(960, PAN, 0.0),
            param(1_440, GAIN, 0.25),
            param(1_680, MUTE, 1.0),
        ],
    );
    r.play();
    let (l, rr) = r.run(frames);
    let (a, b, c) = (frame_of(480), frame_of(960), frame_of(1_440));
    let m = frame_of(1_680);
    let n = fade_frames(RATE as f32) as usize;
    for k in 0..m {
        let gain = if k < c { 0.5 } else { 0.25 };
        let (el, er) = balance(if (a..b).contains(&k) { -1.0 } else { 0.0 });
        assert!((l[k] - reference[k] * gain * el).abs() < 1e-6, "left {k}");
        assert!((rr[k] - reference[k] * gain * er).abs() < 1e-6, "right {k}");
    }
    // The timed mute fades by the gate: never above the lane's level, silent after the fade.
    for k in m..frames {
        assert!(
            l[k].abs() <= reference[k].abs() * 0.25 + 1e-6,
            "mute fade {k}"
        );
    }
    assert_eq!(peak(&l[m + n..]), 0.0, "the master is muted");
    let bound = (max_step(&reference) + 2.0 * TONE_LEVEL / n as f32) * 1.05;
    assert!(
        max_step(&l[m - 4..]) <= bound,
        "the master's timed mute does not click"
    );
    // The master's message state and its schedule share the strip: a SetMaster whose values
    // changed is a gesture, which the strip takes by a ramp; the lane's last value (0.25)
    // holds until then.
    let (l, _) = r.run(4_800);
    assert_eq!(peak(&l[2_400..]), 0.0, "the lane's mute holds");
    r.master(0.9, true);
    r.run(256);
    r.master(0.9, false);
    let (l, _) = r.run(4_800);
    assert!((peak(&l[2_400..]) - 0.9 * TONE_LEVEL).abs() < 1e-3);
}

#[test]
fn a_message_changes_the_strip_only_when_it_changed() {
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.mix(0, 1.0, 0.0, false);
    r.run(WARM);
    // A lane sets 0.5 at the start of the clock; the track's message state is still 1.0.
    r.schedule(Some(0), &[param(0, GAIN, 0.5)]);
    r.play();
    let (l, _) = r.run(4_800);
    assert!(
        (peak(&l[1_000..]) - 0.5 * TONE_LEVEL).abs() < 1e-3,
        "the lane holds"
    );
    // The same message again does not take the value back; a changed one does.
    r.mix(0, 1.0, 0.0, false);
    let (l, _) = r.run(4_800);
    assert!(
        (peak(&l[1_000..]) - 0.5 * TONE_LEVEL).abs() < 1e-3,
        "unchanged message"
    );
    r.mix(0, 0.25, 0.0, false);
    let (l, _) = r.run(4_800);
    assert!(
        (peak(&l[1_000..]) - 0.25 * TONE_LEVEL).abs() < 1e-3,
        "a gesture"
    );
}

#[test]
fn hostile_values_are_silence_or_ignored_and_nothing_is_heard_as_nan() {
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.run(WARM);
    r.schedule(
        Some(0),
        &[
            param(0, GAIN, f32::NAN),
            param(100, GAIN, f32::INFINITY),
            param(200, GAIN, -3.0),
            param(300, GAIN, 1.0),
            param(400, PAN, f32::NAN),
            param(500, PAN, 1e30),
            param(600, PAN, -1e30),
            param(700, PAN, 0.0),
            param(800, ParamId(99), 5.0),
            param(900, MUTE, f32::NAN),
            param(1_000, MUTE, 0.0),
        ],
    );
    r.schedule_master(&[param(0, GAIN, f32::NAN), param(300, GAIN, 1.0)]);
    r.play();
    let (l, rr) = r.run(frame_of(1_200));
    // check() ran: finite, no denormal. The values that mean silence are silent.
    assert_eq!(peak(&l[..frame_of(300)]), 0.0);
    assert_eq!(peak(&rr[..frame_of(300)]), 0.0);
    // Back at unity after the NaN mute (NaN >= 0.5 is false: not muted).
    assert!((peak(&l[frame_of(1_000) + 400..]) - TONE_LEVEL).abs() < 1e-3);
}

impl Rig {
    fn schedule_master(&mut self, params: &[ScheduledParam]) {
        self.schedule(None, params);
    }
}

// ---------------------------------------------------------------- lanes and gestures

#[test]
fn a_lane_is_followed_to_the_sample_from_the_first_block_and_through_a_fade() {
    // The lane starts while the track fades in (the schedule is on the track at once) and
    // a solo on another track fades while it runs: neither smooths the lane.
    let points: Vec<(i64, f32)> = (0..=100)
        .map(|i| (i as i64, 0.2 + 0.006 * i as f32))
        .collect();
    let params: Vec<ScheduledParam> = points.iter().map(|(t, v)| param(*t, GAIN, *v)).collect();
    let frames = frame_of(100) + 400;
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.tone(1, 81);
    r.run(1);
    r.schedule(Some(0), &params);
    r.play();
    r.send(&[M::TrackSetSolo {
        track: TrackSlot(0),
        solo: true,
    }]);
    let (l, _) = r.run(frames);
    // Track 0 alone is audible (solo), by the gate: the fade in and the solo are ramps of
    // the gate only. Compare with the same tone through a rig without any lane.
    let mut plain = Rig::new(128);
    plain.tone(0, 69);
    plain.run(1);
    plain.play();
    plain.send(&[M::TrackSetSolo {
        track: TrackSlot(0),
        solo: true,
    }]);
    let (tone, _) = plain.run(frames);
    let n = fade_frames(RATE as f32) as usize;
    for k in n + 128..frames {
        let expected = tone[k] * level_at(&points, 1.0, k);
        assert!(
            (l[k] - expected).abs() < 1e-6,
            "frame {k}: {} vs {expected}",
            l[k]
        );
    }
}

#[test]
fn a_gesture_on_one_field_leaves_the_lanes_other_fields_alone() {
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.run(WARM);
    r.schedule(Some(0), &[param(0, GAIN, 0.5), param(0, PAN, 1.0)]);
    r.play();
    r.run(WARM);
    // Mute then unmute: the gain and the pan the lane set stay.
    r.mix(0, 1.0, 0.0, true);
    r.run(WARM);
    r.mix(0, 1.0, 0.0, false);
    let (l, rr) = r.run(WARM);
    assert_eq!(peak(&l), 0.0, "the lane's pan is still hard right");
    assert!((peak(&rr[WARM / 2..]) - 0.5 * TONE_LEVEL).abs() < 1e-3);
    // The same on the master: a fader move leaves the master's lane pan alone.
    r.schedule(None, &[param(0, PAN, -1.0)]);
    r.send(&[M::Stop]);
    r.play();
    r.run(WARM);
    r.master(0.5, false);
    let (_, rr) = r.run(WARM);
    assert_eq!(peak(&rr), 0.0, "the master's pan stayed hard left");
}

#[test]
fn the_faders_take_the_strip_back_when_the_lane_is_gone() {
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.mix(0, 0.8, 0.0, false);
    r.master(0.5, false);
    r.run(WARM);
    r.schedule(Some(0), &[param(0, GAIN, 0.1)]);
    r.schedule(None, &[param(0, GAIN, 0.2)]);
    r.play();
    let (l, _) = r.run(WARM);
    assert!((peak(&l[WARM / 2..]) - 0.1 * 0.2 * TONE_LEVEL).abs() < 1e-3);
    // The lanes go: the same messages, unchanged, are what plays again.
    let old =
        r.e.state_mut()
            .track_mut(TrackSlot(0))
            .unwrap()
            .graph
            .set_schedule(None);
    drop(old);
    drop(r.e.state_mut().set_master_schedule(None));
    let (l, _) = r.run(WARM);
    assert!((peak(&l[WARM / 2..]) - 0.8 * 0.5 * TONE_LEVEL).abs() < 1e-3);
    // A Reset gives the master its default state, lane included: the schedule of the old
    // state goes with it (it is still on the master when the Reset arrives).
    r.schedule(None, &[param(0, GAIN, 0.2)]);
    r.run(WARM);
    assert!(r.e.state().master_schedule().is_some());
    r.send(&[M::Reset { epoch: 1 }]);
    r.tone(0, 69);
    r.run(WARM);
    let (l, _) = r.run(WARM);
    assert!(
        (peak(&l[WARM / 2..]) - TONE_LEVEL).abs() < 1e-3,
        "{}",
        peak(&l)
    );
}

#[test]
fn the_master_pan_comes_from_its_message() {
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.send(&[M::SetMaster {
        gain: 1.0,
        pan: 1.0,
        mute: false,
    }]);
    r.run(WARM);
    let (l, rr) = r.run(WARM);
    assert_eq!(peak(&l), 0.0);
    assert!((peak(&rr) - TONE_LEVEL).abs() < 1e-3);
}

// ---------------------------------------------------------------- meters

#[test]
fn the_meters_read_the_signal_after_the_fader() {
    let mut r = Rig::new(128);
    r.tone(0, 69);
    r.tone(1, 69);
    r.mix(0, gain_of(-6.0), 1.0, false);
    r.mix(1, 1.0, 0.0, true);
    r.master(0.5, false);
    r.run(WARM);
    let mut out = [TrackMeter::default(); 2];
    r.e.take_meters(WARM as u32, &mut out);
    r.run(4_800);
    let (used, master) = r.e.take_meters(4_800, &mut out);
    assert_eq!(used, 2);
    // Track 0: -6 dB, hard right: the right channel at the fader's level, the left silent.
    let expected = TONE_LEVEL * gain_of(-6.0);
    assert!((out[0].peak_r - expected).abs() < 1e-3, "{}", out[0].peak_r);
    assert_eq!(out[0].peak_l, 0.0);
    assert!((out[0].rms_r - expected / 2f32.sqrt()).abs() < 1e-3);
    // Track 1 is muted: its meter is silent (after the fader, not before).
    assert_eq!((out[1].peak_l, out[1].peak_r), (0.0, 0.0));
    // The output's meter is after the master (x 0.5).
    assert!(
        (master.peak_r - 0.5 * expected).abs() < 1e-3,
        "{}",
        master.peak_r
    );
    assert_eq!(master.peak_l, 0.0);
}

// ---------------------------------------------------------------- listening

/// Writes `.qa/renders/mx1-gestures.wav` for listening: two tones, one swept across the
/// pan and one faded down and up by timed events, then the mute and the solo.
/// Run with `cargo test -p engine --test mixer -- --ignored listening`.
#[test]
#[ignore]
fn listening_render() {
    let mut r = Rig::new(128);
    r.tone(0, 57);
    r.tone(1, 69);
    r.run(WARM);
    // Seconds to ticks at 120 BPM: 1 s = 1 920 ticks.
    let sweep: Vec<ScheduledParam> = (0..=480)
        .map(|i| {
            let t = i as f32 / 480.0;
            param(i * 8, PAN, (t * std::f32::consts::TAU).sin())
        })
        .collect();
    r.schedule(Some(0), &sweep);
    let fade: Vec<ScheduledParam> = (0..=480)
        .map(|i| {
            let t = i as f32 / 480.0;
            // Down to -40 dB and back, in dB so that the ramp sounds even.
            param(
                i * 8,
                GAIN,
                gain_of(-40.0 * (1.0 - (t * std::f32::consts::TAU).cos()) / 2.0),
            )
        })
        .collect();
    // The second track has its own schedule through the master slot of the same table: a
    // track has one schedule, so the fade goes on track 1.
    let s = Box::new(Schedule::try_new(&[], &fade, SCHEDULE_FIRST_VOICE).unwrap());
    drop(
        r.e.state_mut()
            .track_mut(TrackSlot(1))
            .unwrap()
            .graph
            .set_schedule(Some(s)),
    );
    r.play();
    let (mut left, mut right) = r.run(RATE as usize * 4);
    r.send(&[M::Stop]);
    r.mix(0, 1.0, 0.0, true);
    for (msgs, secs) in [
        (vec![], 1usize),
        (
            vec![M::TrackSetSolo {
                track: TrackSlot(1),
                solo: true,
            }],
            1,
        ),
        (
            vec![M::TrackSetSolo {
                track: TrackSlot(1),
                solo: false,
            }],
            1,
        ),
        (
            vec![M::TrackSetMix {
                track: TrackSlot(0),
                gain: 1.0,
                pan: 0.0,
                mute: false,
            }],
            1,
        ),
    ] {
        r.send(&msgs);
        let (l, rr) = r.run(RATE as usize * secs);
        left.extend(l);
        right.extend(rr);
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.qa/renders");
    std::fs::create_dir_all(&dir).unwrap();
    let mut bytes = Vec::new();
    let data = left.len() as u32 * 4;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for (l, r) in left.iter().zip(&right) {
        for s in [l, r] {
            bytes.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
        }
    }
    std::fs::write(dir.join("mx1-gestures.wav"), bytes).unwrap();
}
