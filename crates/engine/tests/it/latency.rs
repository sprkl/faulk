//! Device latency compensation (#749): offline renders through the real `Preparer`,
//! `Engine::apply` and `Engine::render`, every audio-side call under the allocation detector.
//!
//! Two identical tracks, one hard left and one hard right, one of them through a Ceiling below
//! its ceiling (transparent: its input delayed by its latency, bit for bit): the left and the
//! right channel are equal to the sample once the graph compensates, at 44.1, 48 and 96 kHz,
//! with every look-ahead and the true-peak detection, with a Ceiling on the master too, and
//! through a bypass and back. The reported latency is the chain's. A change of latency while
//! playing (the look-ahead, a device inserted or removed, a bypassed device removed or rebuilt
//! at a new slot) moves the other track without a step and aligns again; a move longer than a
//! fade has no gap (the unit tests of `latency.rs` hold the gains). The metronome is delayed
//! with the music, the panic cut leaves nothing delayed behind it, and twelve distinct
//! latencies at once are all compensated.
//!
//! `cargo test -p engine --test it latency:: -- --nocapture` prints the measured numbers.

use assert_no_alloc::assert_no_alloc;
use engine::device::DeviceSetup;
use engine::device::testing::{DELAY, GAIN, TONE, delay, gain};
use engine::devices::ceiling::{self, CEILING, DESCRIPTOR as CEILING_DEVICE, LOOKAHEAD, TRUE_PEAK};
use engine::engine::{Engine, Preparer, channel};
use engine::events::{SCHEDULE_FIRST_VOICE, Schedule, ScheduledParam};
use engine::graph::fade_frames;
use engine::mixer::{MIXER_SLOT, MUTE};
use engine::msg::{
    AutomationPoint, AutomationTarget, ControlMsg as M, Curve, DeviceSlot, LaneSlot, ParamId,
    TrackKind, TrackSlot, encode_state,
};
use engine::time::{Quantize, Tempo, Tick, TimeSignature};

const BLOCK: usize = 128;
/// A note of the test tone (220 Hz).
const NOTE: u8 = 57;

struct Rig {
    p: Preparer,
    e: Engine,
}

impl Rig {
    fn new(rate: f32) -> Rig {
        Rig::sized(rate, BLOCK)
    }

    /// A rig whose blocks reach `max_block` frames.
    fn sized(rate: f32, max_block: usize) -> Rig {
        let setup = DeviceSetup {
            sample_rate: rate,
            max_block_frames: max_block,
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

    /// `blocks` blocks of both channels.
    fn run(&mut self, blocks: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
        let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
        for _ in 0..blocks {
            assert_no_alloc(|| self.e.process(&mut l, &mut r));
            out_l.extend_from_slice(&l);
            out_r.extend_from_slice(&r);
            self.p.collect();
        }
        for (i, s) in out_l.iter().chain(&out_r).enumerate() {
            assert!(s.is_finite(), "sample {i} is {s}");
            assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
        }
        assert_eq!(self.e.limiter_silenced(), 0, "a non-finite sample");
        (out_l, out_r)
    }

    fn latency(&self) -> u32 {
        self.e.graph().latency()
    }
}

fn t(slot: u32) -> TrackSlot {
    TrackSlot(slot)
}

fn insert(
    track: TrackSlot,
    device: u16,
    kind: engine::msg::DeviceKind,
    state: &[(ParamId, f32)],
) -> M {
    M::DeviceInsert {
        track,
        device: DeviceSlot(device),
        kind,
        state: encode_state(state),
    }
}

fn ceiling_at(track: TrackSlot, device: u16, params: &[(u32, f32)]) -> M {
    let state: Vec<(ParamId, f32)> = params.iter().map(|(id, v)| (ParamId(*id), *v)).collect();
    insert(track, device, CEILING_DEVICE.kind_id, &state)
}

fn delay_at(track: TrackSlot, device: u16, frames: u32) -> M {
    insert(
        track,
        device,
        DELAY.kind_id,
        &[(delay::FRAMES, frames as f32)],
    )
}

fn param(track: TrackSlot, device: u16, id: ParamId, value: f32) -> M {
    M::ParamSet {
        track,
        device: DeviceSlot(device),
        param: id,
        value,
    }
}

fn enabled(track: TrackSlot, device: u16, enabled: bool) -> M {
    M::DeviceSetEnabled {
        track,
        device: DeviceSlot(device),
        enabled,
    }
}

fn remove(track: TrackSlot, device: u16) -> M {
    M::DeviceRemove {
        track,
        device: DeviceSlot(device),
    }
}

/// A track with the test tone, panned to `pan`.
fn tone_track(track: TrackSlot, pan: f32) -> Vec<M> {
    tone_track_at(track, pan, 1.0)
}

/// A track with the test tone, panned to `pan`, its fader at `gain`.
fn tone_track_at(track: TrackSlot, pan: f32, gain: f32) -> Vec<M> {
    vec![
        M::TrackCreate {
            track,
            kind: TrackKind::Midi,
        },
        M::TrackSetMix {
            track,
            gain,
            pan,
            mute: false,
        },
        insert(track, 0, TONE.kind_id, &[]),
    ]
}

fn note_on(track: TrackSlot) -> M {
    M::NoteOn {
        track,
        pitch: NOTE,
        velocity: 100,
        at: None,
    }
}

/// Two tone tracks, hard left (track 0) and hard right (track 1), `extra` added after, the
/// fades in over; then the note on both.
fn pair(rig: &mut Rig, extra: &[M]) {
    let mut msgs = tone_track(t(0), -1.0);
    msgs.extend(tone_track(t(1), 1.0));
    msgs.extend_from_slice(extra);
    rig.send(&msgs);
    rig.run(8);
    rig.send(&[note_on(t(0)), note_on(t(1))]);
}

/// The largest difference between the two channels.
fn mismatch(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .fold(0f32, |m, (a, b)| m.max((a - b).abs()))
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

/// Largest difference between two neighbouring samples.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// The largest step a crossfade of `fade` frames between two steady signals of steps up to
/// `steady` and peaks up to `level` makes: their own steps, plus their difference (at most
/// twice the peak) spread over the fade. A hard cut between them can step by twice the peak.
fn crossfade_bound(steady: f32, level: f32) -> f32 {
    steady * 1.05 + 2.0 * level / fade_frames(48_000.0) as f32 + 1e-6
}

/// The first sample above `level` in absolute value.
fn onset(x: &[f32], level: f32) -> Option<usize> {
    x.iter().position(|s| s.abs() > level)
}

#[test]
fn two_identical_tracks_one_through_a_ceiling_are_aligned_to_the_sample() {
    for rate in [44_100.0f32, 48_000.0, 96_000.0] {
        for (lookahead, true_peak) in [
            (0.0015f32, 0.0f32),
            (0.0, 0.0),
            (0.005, 0.0),
            (0.0015, 1.0),
            (0.0, 1.0),
        ] {
            let mut rig = Rig::new(rate);
            pair(
                &mut rig,
                &[ceiling_at(
                    t(1),
                    1,
                    &[
                        (CEILING, 0.0),
                        (LOOKAHEAD, lookahead),
                        (TRUE_PEAK, true_peak),
                    ],
                )],
            );
            let (left, right) = rig.run(40);
            let la = ceiling::lookahead_frames(lookahead, rate);
            let expected = if true_peak >= 0.5 {
                la.max(ceiling::TP_MIN_LOOKAHEAD) + ceiling::TP_DELAY
            } else {
                la
            };
            assert_eq!(
                rig.latency(),
                expected,
                "{rate} Hz, {lookahead} s, {true_peak}"
            );
            assert!(peak(&left) > 0.05, "the tone sounds");
            let off = mismatch(&left, &right);
            println!(
                "{rate} Hz, look-ahead {lookahead} s, true peak {true_peak}: latency {expected} frames, \
                 left − right at most {off:e}"
            );
            assert_eq!(off, 0.0, "{rate} Hz, {lookahead} s: aligned to the sample");
            if expected > 0 {
                // The control: the same two signals one latency apart differ.
                let n = expected as usize;
                let shifted = mismatch(&left[n..], &left[..left.len() - n]);
                assert!(
                    shifted > 0.01,
                    "a shift of {n} frames is audible: {shifted}"
                );
            }
        }
    }
}

#[test]
fn a_ceiling_on_the_master_adds_to_the_reported_latency_and_keeps_the_alignment() {
    let mut rig = Rig::new(48_000.0);
    pair(
        &mut rig,
        &[
            ceiling_at(t(1), 1, &[(CEILING, 0.0)]),
            ceiling_at(TrackSlot::MASTER, 0, &[(CEILING, 0.0), (TRUE_PEAK, 1.0)]),
        ],
    );
    let (left, right) = rig.run(40);
    // 72 on the track; 72 + 12 on the master.
    assert_eq!(rig.e.graph().master_latency(), 84);
    assert_eq!(rig.latency(), 72 + 84);
    assert_eq!(mismatch(&left, &right), 0.0);
}

#[test]
fn a_ceiling_bypassed_and_switched_back_on_neither_combs_nor_moves_its_track() {
    let mut rig = Rig::new(48_000.0);
    pair(&mut rig, &[ceiling_at(t(1), 1, &[(CEILING, 0.0)])]);
    rig.run(10);
    let mut all = (Vec::new(), Vec::new());
    for change in [enabled(t(1), 1, false), enabled(t(1), 1, true)] {
        rig.send(&[change]);
        let (l, r) = rig.run(20);
        all.0.extend(l);
        all.1.extend(r);
        // A bypassed device keeps its latency: nothing else moves.
        assert_eq!(rig.latency(), 72);
    }
    let off = mismatch(&all.0, &all.1);
    println!("through a bypass and back: left − right at most {off:e}");
    assert_eq!(off, 0.0, "the bypass fade blends two signals of one timing");
}

/// Plays the pair with `setup`, sends `change`, and checks the left track (no latent device)
/// moves without a step and is aligned again once `settle` blocks passed.
fn moves_cleanly(name: &str, setup: &[M], change: &[M], settle: usize, latency: u32) {
    let mut rig = Rig::new(48_000.0);
    pair(&mut rig, setup);
    let (before, _) = rig.run(20);
    rig.send(change);
    let (after, right) = rig.run(settle + 20);
    let steady = max_step(&before[before.len() - 4 * BLOCK..]);
    let mut joined = before[before.len() - BLOCK..].to_vec();
    joined.extend_from_slice(&after);
    let step = max_step(&joined);
    let bound = crossfade_bound(steady, peak(&before));
    println!("{name}: largest step {step} (steady {steady}, bound {bound})");
    assert!(step <= bound, "{name}: a step of {step}, bound {bound}");
    assert_eq!(rig.latency(), latency, "{name}");
    let tail = settle * BLOCK;
    let off = mismatch(&after[tail..], &right[tail..]);
    assert_eq!(off, 0.0, "{name}: aligned again");
}

#[test]
fn a_change_of_lookahead_while_playing_moves_the_other_track_without_a_step() {
    // The Ceiling settles 50 ms, fades out 5 ms, stays silent for its latency, fades in.
    moves_cleanly(
        "look-ahead 1.5 to 5 ms",
        &[ceiling_at(t(1), 1, &[(CEILING, 0.0)])],
        &[param(t(1), 1, ParamId(LOOKAHEAD), 0.005)],
        40,
        240,
    );
    moves_cleanly(
        "true peak on",
        &[ceiling_at(t(1), 1, &[(CEILING, 0.0)])],
        &[param(t(1), 1, ParamId(TRUE_PEAK), 1.0)],
        40,
        84,
    );
    moves_cleanly(
        "look-ahead 5 to 0 ms",
        &[ceiling_at(t(1), 1, &[(CEILING, 0.0), (LOOKAHEAD, 0.005)])],
        &[param(t(1), 1, ParamId(LOOKAHEAD), 0.0)],
        40,
        0,
    );
}

#[test]
fn a_move_longer_than_a_fade_has_no_step_either_way() {
    // 2000 frames, beyond the 240 of a fade: the old and the new delay cross in output time.
    moves_cleanly(
        "0 to 2000 frames",
        &[delay_at(t(1), 1, 0)],
        &[param(t(1), 1, delay::FRAMES, 2000.0)],
        30,
        2000,
    );
    moves_cleanly(
        "2000 to 0 frames",
        &[delay_at(t(1), 1, 2000)],
        &[param(t(1), 1, delay::FRAMES, 0.0)],
        30,
        0,
    );
}

#[test]
fn a_ceiling_inserted_or_removed_while_playing_moves_the_other_track_without_a_step() {
    moves_cleanly(
        "inserted",
        &[],
        &[ceiling_at(t(1), 1, &[(CEILING, 0.0)])],
        10,
        72,
    );
    moves_cleanly(
        "removed",
        &[ceiling_at(t(1), 1, &[(CEILING, 0.0)])],
        &[remove(t(1), 1)],
        10,
        0,
    );
}

/// Plays the pair with `setup`, sends `change`, and checks the right track (the one whose
/// chain changes) has no step.
fn right_moves_cleanly(name: &str, setup: &[M], change: &[M], latency: u32) {
    let mut rig = Rig::new(48_000.0);
    pair(&mut rig, setup);
    let (_, before) = rig.run(20);
    rig.send(change);
    let (left, after) = rig.run(40);
    let steady = max_step(&before[before.len() - 4 * BLOCK..]);
    let mut joined = before[before.len() - BLOCK..].to_vec();
    joined.extend_from_slice(&after);
    let step = max_step(&joined);
    let bound = crossfade_bound(steady, peak(&before));
    println!("{name}: largest step {step} (steady {steady}, bound {bound})");
    assert!(step <= bound, "{name}: a step of {step}, bound {bound}");
    assert_eq!(rig.latency(), latency, "{name}");
    let tail = 30 * BLOCK;
    assert_eq!(
        mismatch(&left[tail..], &after[tail..]),
        0.0,
        "{name}: aligned again"
    );
}

#[test]
fn a_bypassed_ceiling_removed_or_rebuilt_at_another_slot_never_jumps() {
    let bypassed = [
        ceiling_at(t(1), 1, &[(CEILING, 0.0)]),
        enabled(t(1), 1, false),
    ];
    right_moves_cleanly("bypassed, removed", &bypassed, &[remove(t(1), 1)], 0);
    // Switched on (it holds its fade while it warms up) and removed at once.
    right_moves_cleanly(
        "switched on and removed at once",
        &bypassed,
        &[enabled(t(1), 1, true), remove(t(1), 1)],
        0,
    );
    // Replaced in its slot by another Ceiling of the same rate (a rebuild after an edit): the
    // new one takes the old one's line, so the slot keeps its timing. (A sample-rate change
    // rebuilds every device and restarts the output: the old line's frames are at the old
    // rate and are not taken.)
    let on = [ceiling_at(t(1), 1, &[(CEILING, 0.0)])];
    right_moves_cleanly(
        "replaced in its slot",
        &on,
        &[ceiling_at(t(1), 1, &[(CEILING, 0.0)])],
        72,
    );
    right_moves_cleanly(
        "replaced in its slot while bypassed",
        &bypassed,
        &[
            ceiling_at(t(1), 1, &[(CEILING, 0.0)]),
            enabled(t(1), 1, false),
        ],
        72,
    );
    // A device inserted before it: the project rebuilds it one slot further, bypassed.
    right_moves_cleanly(
        "bypassed, rebuilt one slot further",
        &bypassed,
        &[
            insert(t(1), 1, GAIN.kind_id, &[(gain::GAIN, 1.0)]),
            ceiling_at(t(1), 2, &[(CEILING, 0.0)]),
            enabled(t(1), 2, false),
        ],
        72,
    );
}

/// The smallest peak over a sliding window of `window` frames, from `from` on: a hole in a
/// held tone shows as a window whose peak falls.
fn lowest_window_peak(x: &[f32], from: usize, window: usize) -> f32 {
    (from..x.len().saturating_sub(window))
        .step_by(16)
        .map(|i| peak(&x[i..i + window]))
        .fold(f32::INFINITY, f32::min)
}

#[test]
fn switching_a_latent_device_on_or_off_on_a_held_tone_leaves_no_hole() {
    // A 220 Hz tone held on one track: a window of one period (218 frames) always holds its
    // peak. A device with a latency switched on or off while it sounds: its own line is
    // empty for its latency, the fade's dry side is delayed by it and the fade holds until the
    // device ran it, so the tone never dips. (An insert moves the track by the latency: its
    // dry side crossfades from the undelayed signal to the delayed one, a 5 ms transition
    // that `moves_cleanly` bounds.)
    let period = (48_000.0 / 220.0f32).ceil() as usize + 2;
    for (name, setup, change) in [
        (
            "Ceiling switched on",
            vec![
                ceiling_at(t(0), 1, &[(CEILING, 0.0), (LOOKAHEAD, 0.005)]),
                enabled(t(0), 1, false),
            ],
            vec![enabled(t(0), 1, true)],
        ),
        (
            "Ceiling switched off",
            vec![ceiling_at(t(0), 1, &[(CEILING, 0.0), (LOOKAHEAD, 0.005)])],
            vec![enabled(t(0), 1, false)],
        ),
        (
            "test delay of 1000 frames switched on",
            vec![delay_at(t(0), 1, 1000), enabled(t(0), 1, false)],
            vec![enabled(t(0), 1, true)],
        ),
    ] {
        let mut rig = Rig::new(48_000.0);
        let mut msgs = tone_track(t(0), 0.0);
        msgs.extend(setup);
        rig.send(&msgs);
        rig.run(8);
        rig.send(&[note_on(t(0))]);
        let (before, _) = rig.run(20);
        let steady = peak(&before[before.len() - 4 * BLOCK..]);
        rig.send(&change);
        let (after, _) = rig.run(30);
        let mut all = before[before.len() - 4 * BLOCK..].to_vec();
        all.extend_from_slice(&after);
        let lowest = lowest_window_peak(&all, 0, period);
        println!("{name}: lowest peak over a period {lowest} (steady {steady})");
        assert!(
            lowest > 0.98 * steady,
            "{name}: a hole, {lowest} against {steady}"
        );
    }
}

#[test]
fn a_hard_jump_by_the_latency_breaks_the_step_bound() {
    // The control of `moves_cleanly`: the tone spliced 72 frames later, near a crest, steps
    // far beyond what a crossfade allows.
    let mut rig = Rig::new(48_000.0);
    pair(&mut rig, &[]);
    let (left, _) = rig.run(20);
    let steady = max_step(&left[left.len() - 4 * BLOCK..]);
    let at = left.len() / 2;
    let crest = (at..at + 400)
        .max_by(|a, b| left[*a].abs().total_cmp(&left[*b].abs()))
        .unwrap();
    let mut cut = left[crest - 200..crest].to_vec();
    cut.extend_from_slice(&left[crest - 72..crest + 100]);
    let bound = crossfade_bound(steady, peak(&left));
    assert!(
        max_step(&cut) > 2.0 * bound,
        "{} against {bound}",
        max_step(&cut)
    );
}

#[test]
fn a_chain_switched_off_behind_a_bypassed_ceiling_rings_out_its_delayed_tail() {
    // Both instruments switched off at once: the right track's sound still sits in the bypass
    // line of its Ceiling when its chain stops sounding. The track renders until the line has
    // given it all, so its fade ends as the left one's does, one latency later, to the sample.
    let mut rig = Rig::new(48_000.0);
    pair(
        &mut rig,
        &[
            ceiling_at(t(1), 1, &[(CEILING, 0.0), (LOOKAHEAD, 0.005)]),
            enabled(t(1), 1, false),
        ],
    );
    rig.run(20);
    rig.send(&[enabled(t(0), 0, false), enabled(t(1), 0, false)]);
    let (left, right) = rig.run(20);
    assert_eq!(rig.latency(), 240);
    assert!(peak(&right[..BLOCK]) > 0.05, "it sounded");
    assert_eq!(mismatch(&left, &right), 0.0, "the delayed tail is not cut");
    assert_eq!(peak(&right[10 * BLOCK..]), 0.0, "then silence");
}

#[test]
fn beyond_the_limit_the_compensation_is_capped_flagged_and_aligned_up_to_it() {
    // Track 1 (right) goes through five test delays of 4096 frames: 20 480, more than the
    // 16 383 the engine gives a track. Track 0 (left) has none: it would need 20 480, gets
    // 16 383 and plays 4097 frames early; the report says the compensation is capped.
    let five: Vec<M> = (1..=5).map(|d| delay_at(t(1), d, 4096)).collect();
    let max = engine::msg::limits::MAX_COMPENSATION_FRAMES as usize;
    let mut rig = Rig::new(48_000.0);
    pair(&mut rig, &five);
    let (left, right) = rig.run(220);
    assert_eq!(rig.latency(), 5 * 4096);
    assert!(rig.e.graph().latency_capped(), "the cap is flagged");
    let early = 5 * 4096 - max;
    assert_eq!(early, 4097);
    let n = left.len() - early;
    assert_eq!(
        mismatch(&left[..n], &right[early..]),
        0.0,
        "the plain track is delayed by the limit exactly"
    );
    assert!(peak(&right[25_000..]) > 0.05, "the delayed track sounds");

    // A track within the limit stays aligned: two delays (8192) need 12 288, under it. The
    // plain track still exists (no note: silent) and keeps the cap flagged.
    let mut rig = Rig::new(48_000.0);
    let mut msgs = tone_track(t(0), 0.0);
    msgs.extend(tone_track(t(1), 1.0));
    msgs.extend(five);
    msgs.extend(tone_track(t(2), -1.0));
    msgs.push(delay_at(t(2), 1, 4096));
    msgs.push(delay_at(t(2), 2, 4096));
    rig.send(&msgs);
    rig.run(8);
    rig.send(&[note_on(t(1)), note_on(t(2))]);
    let (left, right) = rig.run(220);
    assert!(rig.e.graph().latency_capped());
    assert!(peak(&left[25_000..]) > 0.05);
    assert_eq!(mismatch(&left, &right), 0.0, "aligned within the limit");

    // Without the plain track nothing is capped.
    let mut rig = Rig::new(48_000.0);
    let mut msgs = tone_track(t(1), 1.0);
    msgs.extend((1..=5).map(|d| delay_at(t(1), d, 4096)));
    msgs.extend(tone_track(t(2), -1.0));
    msgs.push(delay_at(t(2), 1, 4096));
    msgs.push(delay_at(t(2), 2, 4096));
    rig.send(&msgs);
    rig.run(2);
    assert!(!rig.e.graph().latency_capped());
    // The metronome on: its clicks would need 20 480 frames, more than its line holds.
    rig.send(&[M::SetMetronome {
        enabled: true,
        volume: 1.0,
        count_in_bars: 0,
    }]);
    rig.run(2);
    assert!(
        rig.e.graph().latency_capped(),
        "the metronome's cap is flagged"
    );
}

#[test]
fn a_metronome_switched_off_rings_out_the_clicks_its_line_holds() {
    // With 240 frames of latency the first click (frame 0) leaves the line at frame 240. The
    // metronome switched off after the first block: the 128 frames of click already in the
    // line still come out, then nothing.
    let mut rig = Rig::new(48_000.0);
    let mut msgs = tone_track(t(0), 0.0);
    msgs.push(ceiling_at(t(0), 1, &[(CEILING, 0.0), (LOOKAHEAD, 0.005)]));
    msgs.push(M::SetMetronome {
        enabled: true,
        volume: 1.0,
        count_in_bars: 0,
    });
    rig.send(&msgs);
    rig.run(4);
    assert_eq!(rig.latency(), 240);
    rig.send(&[M::Play { from: Tick(0) }]);
    let (first, _) = rig.run(1);
    assert_eq!(peak(&first), 0.0, "the click is still in the line");
    rig.send(&[M::SetMetronome {
        enabled: false,
        volume: 1.0,
        count_in_bars: 0,
    }]);
    let (after, _) = rig.run(8);
    let at = onset(&after, 1e-4).expect("the click rings out") + BLOCK;
    assert_eq!(at, 240);
    assert!(peak(&after[112..240]) > 0.01, "the click held in the line");
    assert_eq!(peak(&after[240..]), 0.0, "nothing after what the line held");
}

#[test]
fn the_metronome_is_delayed_with_the_music() {
    let click = |latent: bool| -> (usize, u32) {
        let mut rig = Rig::new(48_000.0);
        let mut msgs = tone_track(t(0), 0.0);
        if latent {
            msgs.push(ceiling_at(t(0), 1, &[(CEILING, 0.0), (LOOKAHEAD, 0.005)]));
        }
        msgs.push(M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 0,
        });
        rig.send(&msgs);
        rig.run(4);
        rig.send(&[M::Play { from: Tick(0) }]);
        let (left, _) = rig.run(20);
        (onset(&left, 1e-4).expect("a click"), rig.latency())
    };
    let (plain, none) = click(false);
    let (delayed, latency) = click(true);
    assert_eq!(none, 0);
    assert_eq!(latency, 240);
    println!("first click at frame {plain}, with 240 frames of latency at {delayed}");
    assert_eq!(delayed - plain, 240);
}

#[test]
fn the_panic_cut_leaves_nothing_delayed_behind_it() {
    let mut rig = Rig::new(48_000.0);
    pair(&mut rig, &[delay_at(t(1), 1, 3000)]);
    rig.run(40);
    rig.send(&[M::AllSoundOff]);
    let fade = fade_frames(48_000.0) as usize;
    let (left, right) = rig.run(60);
    // Past the cut's fade (and the block it ends in), nothing comes back: neither the ring
    // nor the bypass lines nor the test delay's own line (its tone is reset: no note).
    let from = fade + BLOCK;
    assert_eq!(peak(&left[from..]), 0.0, "the compensation ring is silent");
    assert_eq!(peak(&right[from..]), 0.0);
}

#[test]
fn the_panic_cut_leaves_nothing_in_a_bypass_line() {
    // A bypassed Ceiling of 5 ms holds 240 frames of the tone in its bypass line: after the
    // cut, none of it comes back (the line is cleared with the devices).
    let mut rig = Rig::new(48_000.0);
    pair(
        &mut rig,
        &[
            ceiling_at(t(1), 1, &[(CEILING, 0.0), (LOOKAHEAD, 0.005)]),
            enabled(t(1), 1, false),
        ],
    );
    rig.run(40);
    rig.send(&[M::AllSoundOff]);
    let fade = fade_frames(48_000.0) as usize;
    let (_, right) = rig.run(20);
    assert_eq!(
        peak(&right[fade + BLOCK..]),
        0.0,
        "nothing from before the cut"
    );
}

#[test]
fn twelve_distinct_latencies_are_all_compensated() {
    // Twelve tracks on the left, each through its own delay; one on the right through the
    // longest: the left is twelve times the right, to the rounding of the sum.
    let mut rig = Rig::new(48_000.0);
    let mut msgs = Vec::new();
    for i in 0..12u32 {
        msgs.extend(tone_track_at(t(i), -1.0, 0.25));
        msgs.push(delay_at(t(i), 1, 37 * i));
    }
    msgs.extend(tone_track_at(t(12), 1.0, 0.25));
    msgs.push(delay_at(t(12), 1, 37 * 11));
    rig.send(&msgs);
    rig.run(8);
    let notes: Vec<M> = (0..13).map(|i| note_on(t(i))).collect();
    rig.send(&notes);
    let (left, right) = rig.run(40);
    assert_eq!(rig.latency(), 37 * 11);
    let scaled: Vec<f32> = right.iter().map(|s| 12.0 * s).collect();
    let off = mismatch(&left, &scaled);
    println!("twelve delays from 0 to 407 frames: left − 12 × right at most {off:e}");
    assert!(off < 1e-5, "{off}");
}

// ---------------------------------------------------------------- the strip (#758)

/// Frames per tick at 120 BPM and 48 kHz.
const FPT: usize = 25;

fn pt(tick: i64, value: f32, curve: Curve) -> AutomationPoint {
    AutomationPoint {
        tick: Tick(tick),
        value,
        curve,
    }
}

fn lane(lane: u32, track: TrackSlot, target: AutomationTarget, points: &[AutomationPoint]) -> M {
    M::AutomationLaneUpload {
        lane: LaneSlot(lane),
        track,
        target,
        first: 0,
        total: points.len() as u32,
        points: points.to_vec(),
    }
}

/// A gain lane with hard steps (to silence and back) and a ramp, in dB.
fn gain_points() -> Vec<AutomationPoint> {
    vec![
        pt(0, 0.0, Curve::Step),
        pt(40, -96.0, Curve::Step),
        pt(60, -6.0, Curve::Linear),
        pt(100, -30.0, Curve::Step),
        pt(120, 0.0, Curve::Step),
        pt(170, -12.0, Curve::Step),
    ]
}

/// A pan lane with steps and a ramp.
fn pan_points() -> Vec<AutomationPoint> {
    vec![
        pt(0, 0.0, Curve::Step),
        pt(50, -0.7, Curve::Step),
        pt(90, 0.5, Curve::Linear),
        pt(130, 0.0, Curve::Step),
    ]
}

impl Rig {
    /// `frames` frames of both channels, in blocks of `block` (at most the rig's largest).
    fn run_frames(&mut self, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
        let (mut l, mut r) = (vec![0.0; block], vec![0.0; block]);
        let mut todo = frames;
        while todo > 0 {
            let n = todo.min(block);
            assert_no_alloc(|| self.e.process(&mut l[..n], &mut r[..n]));
            out_l.extend_from_slice(&l[..n]);
            out_r.extend_from_slice(&r[..n]);
            todo -= n;
            self.p.collect();
        }
        for (i, s) in out_l.iter().chain(&out_r).enumerate() {
            assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
        }
        assert_eq!(self.e.limiter_silenced(), 0, "a non-finite sample");
        (out_l, out_r)
    }

    fn transport(&mut self) {
        self.send(&[M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::default(),
        }]);
    }
}

#[test]
fn a_gain_lane_on_a_track_through_a_ceiling_lands_with_the_same_lane_on_a_plain_track() {
    // The pair (the right track through a Ceiling of 72 frames), the same gain lane on both:
    // the steps to silence and back and the ramp land on the same frame of both channels.
    for block in [BLOCK, 97] {
        let mut rig = Rig::new(48_000.0);
        rig.transport();
        pair(&mut rig, &[ceiling_at(t(1), 1, &[(CEILING, 0.0)])]);
        rig.send(&[
            lane(0, t(0), AutomationTarget::MixerGain, &gain_points()),
            lane(1, t(1), AutomationTarget::MixerGain, &gain_points()),
        ]);
        rig.run(4);
        rig.send(&[M::Play { from: Tick(0) }]);
        let (left, right) = rig.run_frames(200 * FPT, block);
        assert_eq!(rig.latency(), 72);
        assert!(peak(&left) > 0.05, "the tone sounds");
        let off = mismatch(&left, &right);
        println!("block {block}: gain lane on both tracks, left − right at most {off:e}");
        assert_eq!(off, 0.0, "block {block}: the lane lands on the same frame");
        // The control: the step to silence sits where the lane puts it, one latency after its
        // tick (the tracks are delayed to the Ceiling's timing).
        let step = 40 * FPT + 72;
        assert!(peak(&left[step - 64..step]) > 0.05, "sound before the step");
        assert_eq!(peak(&left[step..step + 400]), 0.0, "silence after it");
    }
}

/// Where the Ceilings of a [`strip_render`] sit.
#[derive(Clone, Copy, Debug)]
enum Ceilings {
    Track,
    Master,
    Both,
}

/// One tone track, centred, the strip's lanes and a scheduled mute on the track (or on the
/// master, `master`), a Ceiling of `lookahead` seconds where `ceilings` says, or no Ceiling;
/// played from 0, rendered in blocks of `block`.
fn strip_render(
    lookahead: Option<f32>,
    ceilings: Ceilings,
    master: bool,
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut rig = Rig::sized(48_000.0, 512);
    rig.transport();
    let mut msgs = tone_track(t(0), 0.0);
    let on = if master { TrackSlot::MASTER } else { t(0) };
    let mut count = 0;
    if let Some(la) = lookahead {
        let params = [(CEILING, 0.0), (LOOKAHEAD, la)];
        if matches!(ceilings, Ceilings::Track | Ceilings::Both) {
            msgs.push(ceiling_at(t(0), 1, &params));
            count += 1;
        }
        if matches!(ceilings, Ceilings::Master | Ceilings::Both) {
            msgs.push(ceiling_at(TrackSlot::MASTER, 0, &params));
            count += 1;
        }
    }
    rig.send(&msgs);
    rig.run(8);
    rig.send(&[
        note_on(t(0)),
        lane(0, on, AutomationTarget::MixerGain, &gain_points()),
        lane(1, on, AutomationTarget::MixerPan, &pan_points()),
    ]);
    let mute = |tick: i64, value: f32| ScheduledParam {
        tick: Tick(tick),
        device: MIXER_SLOT,
        id: MUTE,
        value,
    };
    let schedule = Box::new(
        Schedule::try_new(&[], &[mute(140, 1.0), mute(155, 0.0)], SCHEDULE_FIRST_VOICE).unwrap(),
    );
    let old = if master {
        rig.e.state_mut().set_master_schedule(Some(schedule))
    } else {
        rig.e
            .state_mut()
            .track_mut(t(0))
            .unwrap()
            .graph
            .set_schedule(Some(schedule))
    };
    drop(old);
    rig.run(4);
    rig.send(&[M::Play { from: Tick(0) }]);
    let out = rig.run_frames(200 * FPT, block);
    let expected = match lookahead {
        Some(la) => count * ceiling::lookahead_frames(la, 48_000.0),
        None => 0,
    };
    assert_eq!(rig.latency(), expected);
    out
}

#[test]
fn the_strip_behind_a_ceiling_gives_the_same_signal_as_without_its_latency_one_latency_later() {
    // The track's lanes behind the track's Ceiling; the master's behind the master's, behind
    // the track's (the sum reaches the master that late), behind both.
    for (ceilings, master, n) in [
        (Ceilings::Track, false, 72),
        (Ceilings::Master, true, 72),
        (Ceilings::Track, true, 72),
        (Ceilings::Both, true, 144),
        (Ceilings::Both, false, 144),
    ] {
        for block in [BLOCK, 97, 1, 512] {
            let what = format!(
                "{} lanes, Ceiling on {ceilings:?}",
                if master { "master" } else { "track" }
            );
            let none = strip_render(None, ceilings, master, block);
            let plain = strip_render(Some(0.0), ceilings, master, block);
            let latent = strip_render(Some(0.0015), ceilings, master, block);
            assert!(peak(&none.0) > 0.05, "the tone sounds");
            // A Ceiling without latency below its ceiling is transparent, through the
            // strip's delayed path as through the direct one.
            assert_eq!(mismatch(&none.0, &plain.0), 0.0, "{what}, block {block}");
            assert_eq!(mismatch(&none.1, &plain.1), 0.0, "{what}, block {block}");
            // With `n` frames of latency: the same signal, `n` frames later, to the sample.
            let len = plain.0.len() - n;
            let off_l = mismatch(&latent.0[n..], &plain.0[..len]);
            let off_r = mismatch(&latent.1[n..], &plain.1[..len]);
            println!(
                "{what}, block {block}: gain and pan lanes and a scheduled mute behind {n} frames \
                 of latency, shifted back: left off by {off_l:e}, right by {off_r:e}"
            );
            assert_eq!(off_l, 0.0, "{what}, block {block}");
            assert_eq!(off_r, 0.0, "{what}, block {block}");
        }
    }
}

#[test]
fn a_track_removed_behind_a_ceiling_ends_its_fade_without_a_cut() {
    // The right track's strip fades it out, 72 frames late with its sound: the track is handed
    // back only once that delayed fade ended, not 72 frames before.
    let mut rig = Rig::new(48_000.0);
    pair(&mut rig, &[ceiling_at(t(1), 1, &[(CEILING, 0.0)])]);
    let (_, before) = rig.run(20);
    rig.send(&[M::TrackRemove { track: t(1) }]);
    let (_, after) = rig.run(10);
    let steady = max_step(&before[before.len() - 4 * BLOCK..]);
    let mut joined = before[before.len() - BLOCK..].to_vec();
    joined.extend_from_slice(&after);
    let step = max_step(&joined);
    let bound = crossfade_bound(steady, peak(&before));
    println!(
        "track removed behind a Ceiling: largest step {step} (steady {steady}, bound {bound})"
    );
    assert!(step <= bound, "a step of {step}, bound {bound}");
    let fade = fade_frames(48_000.0) as usize;
    assert!(
        peak(&after[fade..fade + 64]) > 0.0,
        "its delayed fade still sounds"
    );
    assert_eq!(peak(&after[fade + 72 + BLOCK..]), 0.0, "then nothing");
}

#[test]
fn a_track_muted_behind_a_ceiling_is_not_moved_at_once_while_its_delayed_fade_sounds() {
    // The right track behind a Ceiling of 72 frames, the left one behind a test delay of no
    // latency (delayed by 72). The right one is muted; two blocks later (its strip silent, its
    // delayed fade not over), the left delay grows to 240 frames at once: the right track's
    // delay moves from 0 to 168 by a crossfade, not at once over its fade's last frames.
    let mut rig = Rig::new(48_000.0);
    pair(
        &mut rig,
        &[delay_at(t(0), 1, 0), ceiling_at(t(1), 1, &[(CEILING, 0.0)])],
    );
    let (_, before) = rig.run(20);
    assert_eq!(rig.latency(), 72);
    rig.send(&[M::TrackSetMix {
        track: t(1),
        gain: 1.0,
        pan: 1.0,
        mute: true,
    }]);
    let (_, muted) = rig.run(2);
    rig.send(&[param(t(0), 1, delay::FRAMES, 240.0)]);
    let (_, after) = rig.run(10);
    assert_eq!(rig.latency(), 240);
    let steady = max_step(&before[before.len() - 4 * BLOCK..]);
    let mut joined = before[before.len() - BLOCK..].to_vec();
    joined.extend_from_slice(&muted);
    joined.extend_from_slice(&after);
    let step = max_step(&joined);
    let bound = crossfade_bound(steady, peak(&before));
    println!("muted track moved behind a Ceiling: largest step {step} (bound {bound})");
    assert!(step <= bound, "a step of {step}, bound {bound}");
}

/// The pair with `setup`, the clock running, the same gain lane on both tracks (`points`) and
/// a gain ramp on the master;
/// `change` sent after 20 blocks. Both channels move without a step beyond the crossfade's
/// bound (a lane without steps of its own), and are equal to the sample `settle` blocks after
/// the change (from the change on when `settle` is 0).
fn lanes_move_cleanly(
    name: &str,
    points: &[AutomationPoint],
    setup: &[M],
    change: &[M],
    settle: usize,
    latency: u32,
) {
    let mut rig = Rig::new(48_000.0);
    rig.transport();
    pair(&mut rig, setup);
    rig.send(&[
        lane(0, t(0), AutomationTarget::MixerGain, points),
        lane(1, t(1), AutomationTarget::MixerGain, points),
        // The master's strip waits for the tracks' timing, which the change moves.
        lane(
            2,
            TrackSlot::MASTER,
            AutomationTarget::MixerGain,
            &ramp_points(),
        ),
    ]);
    rig.run(4);
    rig.send(&[M::Play { from: Tick(0) }]);
    let (bl, br) = rig.run(20);
    rig.send(change);
    let (al, ar) = rig.run(settle + 20);
    assert_eq!(rig.latency(), latency, "{name}");
    let smooth = points
        .iter()
        .all(|p| p.curve == Curve::Linear || p == points.last().unwrap());
    for (side, before, after) in [("left", &bl, &al), ("right", &br, &ar)] {
        if !smooth {
            break;
        }
        let steady = max_step(&before[before.len() - 8 * BLOCK..]);
        let mut joined = before[before.len() - BLOCK..].to_vec();
        joined.extend_from_slice(after);
        let step = max_step(&joined);
        let bound = crossfade_bound(steady, peak(before));
        println!("{name}, {side}: largest step {step} (steady {steady}, bound {bound})");
        assert!(
            step <= bound,
            "{name}, {side}: a step of {step}, bound {bound}"
        );
    }
    let tail = settle * BLOCK;
    let off = mismatch(&al[tail..], &ar[tail..]);
    assert_eq!(off, 0.0, "{name}: the lanes land together");
}

/// A slow gain ramp, no step: what a move must not break.
fn ramp_points() -> Vec<AutomationPoint> {
    vec![pt(0, 0.0, Curve::Linear), pt(400, -12.0, Curve::Step)]
}

#[test]
fn a_latent_device_changing_while_a_lane_plays_keeps_the_lane_on_its_sound() {
    let ceiling = || ceiling_at(t(1), 1, &[(CEILING, 0.0)]);
    // The chain gains, then loses its first device with a latency: the strip goes from the
    // direct path to the delayed one and back.
    lanes_move_cleanly("inserted", &ramp_points(), &[], &[ceiling()], 10, 72);
    lanes_move_cleanly(
        "removed",
        &ramp_points(),
        &[ceiling()],
        &[remove(t(1), 1)],
        10,
        0,
    );
    lanes_move_cleanly(
        "look-ahead 1.5 to 5 ms",
        &ramp_points(),
        &[ceiling()],
        &[param(t(1), 1, ParamId(LOOKAHEAD), 0.005)],
        40,
        240,
    );
    lanes_move_cleanly(
        "replaced in its slot",
        &ramp_points(),
        &[ceiling()],
        &[ceiling()],
        30,
        72,
    );
    let bypassed = [ceiling(), enabled(t(1), 1, false)];
    lanes_move_cleanly(
        "bypassed, removed",
        &ramp_points(),
        &bypassed,
        &[remove(t(1), 1)],
        10,
        0,
    );
    lanes_move_cleanly(
        "bypassed, replaced in its slot",
        &ramp_points(),
        &bypassed,
        &[ceiling(), enabled(t(1), 1, false)],
        0,
        72,
    );
    // A bypass and back keeps the latency: the steps of the lane land together all along.
    lanes_move_cleanly(
        "bypassed",
        &gain_points(),
        &[ceiling()],
        &[enabled(t(1), 1, false)],
        0,
        72,
    );
    lanes_move_cleanly(
        "switched back on",
        &gain_points(),
        &bypassed,
        &[enabled(t(1), 1, true)],
        0,
        72,
    );
}

#[test]
fn a_latent_device_removed_while_its_track_fades_out_leaves_no_step() {
    // The track is muted; one block later, while its delayed fade sounds, its Ceiling leaves:
    // the sound's timing moves from 72 frames to none over the outgoing fade, and the strip's
    // levels with it. A strip that dropped its delay at once would jump by 72 frames of the
    // mute's fade (about 0.3 of the level).
    // After the removal, blocks of 128 frames, and of 512: the device's fade then ends within
    // the first block, its strip line must still run its crossfade there.
    for (ceiling, block) in [
        (vec![ceiling_at(t(1), 1, &[(CEILING, 0.0)])], BLOCK),
        (vec![ceiling_at(t(1), 1, &[(CEILING, 0.0)])], 512),
        (
            vec![
                ceiling_at(t(1), 1, &[(CEILING, 0.0)]),
                enabled(t(1), 1, false),
            ],
            BLOCK,
        ),
        (
            vec![
                ceiling_at(t(1), 1, &[(CEILING, 0.0)]),
                enabled(t(1), 1, false),
            ],
            512,
        ),
    ] {
        let mut rig = Rig::sized(48_000.0, 512);
        pair(&mut rig, &ceiling);
        let (_, before) = rig.run(20);
        rig.send(&[M::TrackSetMix {
            track: t(1),
            gain: 1.0,
            pan: 1.0,
            mute: true,
        }]);
        let (_, fading) = rig.run(1);
        rig.send(&[remove(t(1), 1)]);
        let (_, after) = rig.run_frames(10 * BLOCK, block);
        let steady = max_step(&before[before.len() - 4 * BLOCK..]);
        let mut joined = before[before.len() - BLOCK..].to_vec();
        joined.extend_from_slice(&fading);
        joined.extend_from_slice(&after);
        let step = max_step(&joined);
        // Two fades at once (the mute's and the timing's): twice the bound of one.
        let bound = crossfade_bound(steady, 2.0 * peak(&before));
        println!(
            "removed during a mute's fade, blocks of {block}: largest step {step} (steady {steady}, \
             bound {bound})"
        );
        assert!(step <= bound, "a step of {step}, bound {bound}");
        assert_eq!(peak(&after[after.len() - 4 * BLOCK..]), 0.0, "muted");
    }
}

#[test]
fn the_master_strip_holds_no_stale_level_when_the_tracks_latency_comes_back() {
    // A Ceiling on the track (T = 72), removed (T = 0, the master's line idle), then the
    // master's lane steps to silence and a Ceiling comes back during it: the master's line
    // waits for fresh levels, the silence stays silent (a line that kept its old levels would
    // let 72 frames of the old 0 dB through).
    let mut rig = Rig::new(48_000.0);
    rig.transport();
    let mut msgs = tone_track(t(0), 0.0);
    msgs.push(ceiling_at(t(0), 1, &[(CEILING, 0.0)]));
    rig.send(&msgs);
    rig.run(8);
    rig.send(&[
        note_on(t(0)),
        lane(
            0,
            TrackSlot::MASTER,
            AutomationTarget::MixerGain,
            &[pt(0, 0.0, Curve::Step), pt(200, -96.0, Curve::Step)],
        ),
    ]);
    rig.run(4);
    rig.send(&[M::Play { from: Tick(0) }]);
    rig.run(20);
    assert_eq!(rig.latency(), 72);
    rig.send(&[remove(t(0), 1)]);
    rig.run(10);
    assert_eq!(rig.latency(), 0);
    // Tick 200 is frame 5 000 of the clock: block 40 from the play. Past it, silence.
    let (before, _) = rig.run(15);
    assert!(
        peak(&before[..4 * BLOCK]) > 0.05,
        "the tone sounds before the step"
    );
    assert_eq!(
        peak(&before[12 * BLOCK..]),
        0.0,
        "the master's step to silence"
    );
    rig.send(&[ceiling_at(t(0), 1, &[(CEILING, 0.0)])]);
    let (after, _) = rig.run(20);
    assert_eq!(rig.latency(), 72);
    println!(
        "Ceiling back during the master's silence: peak {}",
        peak(&after)
    );
    assert_eq!(peak(&after), 0.0, "no stale level of the master's line");
}
