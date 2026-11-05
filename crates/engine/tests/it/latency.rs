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
use engine::graph::fade_frames;
use engine::msg::{ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state};
use engine::time::Tick;

const BLOCK: usize = 128;
/// A note of the test tone (220 Hz).
const NOTE: u8 = 57;

struct Rig {
    p: Preparer,
    e: Engine,
}

impl Rig {
    fn new(rate: f32) -> Rig {
        let setup = DeviceSetup {
            sample_rate: rate,
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
