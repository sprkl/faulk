//! A device with a latency inserted, removed or retimed on a playing track (#761): offline
//! renders through the real `Preparer`, `Engine::apply` and `Engine::render`, every audio-side
//! call under the allocation detector.
//!
//! On a track that does not hold the longest chain, the track's total timing does not change:
//! the slot's timing and the track's delay switch at the same frame, so the track's signal is
//! the same, to the sample, as the same render without the change (a Ceiling below its ceiling
//! is its input delayed). On the track that holds the longest chain the whole mix moves by the
//! latency: every track realigns in one fade of 5 ms, all from the same frame, then the slot
//! switches exactly.
//!
//! `cargo test -p engine --test it latency_switch:: -- --nocapture` prints the measured numbers.

use assert_no_alloc::assert_no_alloc;
use engine::device::DeviceSetup;
use engine::device::testing::{DELAY, GAIN, TONE, delay, gain};
use engine::devices::ceiling::{CEILING, DESCRIPTOR as CEILING_DEVICE, LOOKAHEAD};
use engine::devices::drums::{CLOSED_HAT_KEY, DESCRIPTOR as DRUMS, KICK_KEY, SNARE_KEY};
use engine::devices::echo::DESCRIPTOR as ECHO;
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{
    AutomationPoint, AutomationTarget, ControlMsg as M, Curve, DeviceSlot, LaneSlot, ParamId,
    TrackKind, TrackSlot, encode_state,
};
use engine::time::{Quantize, Tempo, Tick, TimeSignature};

const BLOCK: usize = 128;
const RATE: f32 = 48_000.0;
/// A note of the test tone (220 Hz).
const NOTE: u8 = 57;
/// Blocks played before the change, and after it.
const BEFORE: usize = 40;
const AFTER: usize = 60;

struct Rig {
    p: Preparer,
    e: Engine,
}

impl Rig {
    /// A rig whose blocks reach `max_block` frames.
    fn sized(max_block: usize) -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE,
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

    /// One block of `frames` frames.
    fn block(&mut self, out: &mut (Vec<f32>, Vec<f32>), frames: usize) {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        assert_no_alloc(|| self.e.process(&mut l, &mut r));
        for (i, s) in l.iter().chain(&r).enumerate() {
            assert!(s.is_finite(), "sample {i} is {s}");
            assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
        }
        out.0.extend_from_slice(&l);
        out.1.extend_from_slice(&r);
        self.p.collect();
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

/// A Ceiling below its ceiling (the test signals stay under 0 dBFS) with a look-ahead of
/// `lookahead` seconds.
fn ceiling(track: TrackSlot, device: u16, lookahead: f32) -> M {
    insert(
        track,
        device,
        CEILING_DEVICE.kind_id,
        &[(ParamId(CEILING), 0.0), (ParamId(LOOKAHEAD), lookahead)],
    )
}

fn test_delay(track: TrackSlot, device: u16, frames: u32) -> M {
    insert(
        track,
        device,
        DELAY.kind_id,
        &[(delay::FRAMES, frames as f32)],
    )
}

fn remove(track: TrackSlot, device: u16) -> M {
    M::DeviceRemove {
        track,
        device: DeviceSlot(device),
    }
}

fn enabled(track: TrackSlot, device: u16, on: bool) -> M {
    M::DeviceSetEnabled {
        track,
        device: DeviceSlot(device),
        enabled: on,
    }
}

fn param(track: TrackSlot, device: u16, id: u32, value: f32) -> M {
    M::ParamSet {
        track,
        device: DeviceSlot(device),
        param: ParamId(id),
        value,
    }
}

/// What a track plays.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    /// A 220 Hz tone, held.
    Sine,
    /// Kick, snare and closed hat, hit by live notes.
    Drums,
}

/// A track playing `source`, panned to `pan`.
fn track(track: TrackSlot, source: Source, pan: f32) -> Vec<M> {
    let (kind, gain) = match source {
        Source::Sine => (TONE.kind_id, 1.0),
        Source::Drums => (DRUMS.kind_id, 0.3),
    };
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
        insert(track, 0, kind, &[]),
    ]
}

fn note(track: TrackSlot, pitch: u8) -> M {
    M::NoteOn {
        track,
        pitch,
        velocity: 100,
        at: None,
    }
}

/// The notes `source` plays on `tracks` before block `b`.
fn notes(source: Source, tracks: &[TrackSlot], b: usize) -> Vec<M> {
    let mut msgs = Vec::new();
    for &tr in tracks {
        match source {
            Source::Sine if b == 0 => msgs.push(note(tr, NOTE)),
            Source::Drums => {
                // About 120 bpm sixteenths: a hit every 47 blocks of 128 frames is a beat.
                if b.is_multiple_of(47) {
                    msgs.push(note(tr, KICK_KEY));
                }
                if b % 47 == 23 {
                    msgs.push(note(tr, SNARE_KEY));
                }
                if b % 12 == 5 {
                    msgs.push(note(tr, CLOSED_HAT_KEY));
                }
            }
            Source::Sine => {}
        }
    }
    msgs
}

/// Renders `tracks` playing `source` after `setup`, `change` sent at the start of tick
/// [`BEFORE`] (a tick: [`BLOCK`] frames), [`BEFORE`] + [`AFTER`] ticks in all, in blocks of
/// 128 frames; the latency at the end.
fn render(
    source: Source,
    tracks: &[TrackSlot],
    setup: &[M],
    change: &[M],
) -> ((Vec<f32>, Vec<f32>), u32) {
    let (out, latency, _) = render_with(
        source,
        tracks,
        setup,
        &[(BEFORE, change.to_vec())],
        BLOCK,
        |_| (),
    );
    (out, latency)
}

/// [`render`] with `events` sent at the start of their tick, in blocks of `size` frames (the
/// messages of the ticks a block starts reach it at its start), `watch` called with the engine
/// after each block; the latency at the end and what `watch` saw.
fn render_with<W>(
    source: Source,
    tracks: &[TrackSlot],
    setup: &[M],
    events: &[(usize, Vec<M>)],
    size: usize,
    mut watch: impl FnMut(&Engine) -> W,
) -> ((Vec<f32>, Vec<f32>), u32, Vec<W>) {
    let mut rig = Rig::sized(size.max(BLOCK));
    let mut msgs = Vec::new();
    for (i, &tr) in tracks.iter().enumerate() {
        let pan = if tracks.len() == 1 {
            0.0
        } else if i == 0 {
            -1.0
        } else {
            1.0
        };
        msgs.extend(track(tr, source, pan));
    }
    msgs.extend_from_slice(setup);
    rig.send(&msgs);
    let mut warm = (Vec::new(), Vec::new());
    for _ in 0..8 {
        rig.block(&mut warm, BLOCK);
    }
    let total = (BEFORE + AFTER) * BLOCK;
    let mut out = (Vec::new(), Vec::new());
    let mut seen = Vec::new();
    let (mut pos, mut tick) = (0, 0);
    while pos < total {
        let n = size.min(total - pos);
        let mut m = Vec::new();
        while tick * BLOCK < pos + n {
            m.extend(notes(source, tracks, tick));
            for (at, e) in events {
                if *at == tick {
                    m.extend_from_slice(e);
                }
            }
            tick += 1;
        }
        if !m.is_empty() {
            rig.send(&m);
        }
        rig.block(&mut out, n);
        seen.push(watch(&rig.e));
        pos += n;
    }
    (out, rig.latency(), seen)
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|s| f64::from(*s) * f64::from(*s)).sum()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

/// The smallest peak over a sliding window of `window` frames: a dip in a held tone shows as
/// a window whose peak falls.
fn lowest_window_peak(x: &[f32], window: usize) -> f32 {
    (0..x.len().saturating_sub(window))
        .step_by(4)
        .map(|i| peak(&x[i..i + window]))
        .fold(f32::INFINITY, f32::min)
}

/// The frames after the change.
fn after(x: &[f32]) -> &[f32] {
    &x[BEFORE * BLOCK..]
}

/// What a change on a track that does not hold the longest chain did to it: the largest
/// difference from the render without the change, and the energy of that difference over
/// the energy of the signal, after the change.
fn deviation(changed: &[f32], plain: &[f32]) -> (f32, f64) {
    let (a, b) = (after(changed), after(plain));
    let diff: Vec<f32> = a.iter().zip(b).map(|(x, y)| x - y).collect();
    (peak(&diff), energy(&diff) / energy(b).max(1e-30))
}

/// One period of the tone and a little more: a window that always holds its crest.
fn period() -> usize {
    (RATE / 220.0).ceil() as usize + 2
}

/// The setups and changes of a track that does not hold the longest chain: the other track
/// holds a test delay of 400 frames, more than any Ceiling here.
fn shorter_cases() -> Vec<(&'static str, Vec<M>, Vec<M>)> {
    let longest = test_delay(t(1), 1, 400);
    vec![
        (
            "Ceiling inserted",
            vec![longest.clone()],
            vec![ceiling(t(0), 1, 0.0015)],
        ),
        (
            "Ceiling removed",
            vec![longest.clone(), ceiling(t(0), 1, 0.0015)],
            vec![remove(t(0), 1)],
        ),
        (
            "Ceiling of 5 ms inserted",
            vec![longest.clone()],
            vec![ceiling(t(0), 1, 0.005)],
        ),
        (
            "Ceiling of 5 ms removed",
            vec![longest.clone(), ceiling(t(0), 1, 0.005)],
            vec![remove(t(0), 1)],
        ),
        (
            "bypassed Ceiling removed",
            vec![
                longest.clone(),
                ceiling(t(0), 1, 0.0015),
                enabled(t(0), 1, false),
            ],
            vec![remove(t(0), 1)],
        ),
        (
            "bypassed Ceiling, look-ahead 1.5 to 5 ms",
            vec![
                longest.clone(),
                ceiling(t(0), 1, 0.0015),
                enabled(t(0), 1, false),
            ],
            vec![param(t(0), 1, LOOKAHEAD, 0.005)],
        ),
        (
            "bypassed Ceiling, look-ahead 5 to 1.5 ms",
            vec![
                longest.clone(),
                ceiling(t(0), 1, 0.005),
                enabled(t(0), 1, false),
            ],
            vec![param(t(0), 1, LOOKAHEAD, 0.0015)],
        ),
        (
            "Ceiling replaced by a gain",
            vec![longest.clone(), ceiling(t(0), 1, 0.0015)],
            vec![insert(t(0), 1, GAIN.kind_id, &[(gain::GAIN, 1.0)])],
        ),
        (
            "Ceiling before a gain, removed",
            vec![
                longest.clone(),
                ceiling(t(0), 1, 0.0015),
                insert(t(0), 2, GAIN.kind_id, &[(gain::GAIN, 1.0)]),
            ],
            vec![remove(t(0), 1)],
        ),
        (
            "test delay of 1000 frames removed",
            vec![test_delay(t(1), 1, 1200), test_delay(t(0), 1, 1000)],
            vec![remove(t(0), 1)],
        ),
        (
            "test delay of 1000 frames inserted",
            vec![test_delay(t(1), 1, 1200)],
            vec![test_delay(t(0), 1, 1000)],
        ),
    ]
}

/// Measures the shorter track's deviation for every case and source; `exact` asserts it is 0.
fn measure_shorter(exact: bool) {
    let mut failed = Vec::new();
    for source in [Source::Sine, Source::Drums] {
        for (name, setup, change) in shorter_cases() {
            let tracks = [t(0), t(1)];
            let ((changed, _), latency) = render(source, &tracks, &setup, &change);
            let ((plain, _), _) = render(source, &tracks, &setup, &[]);
            assert!(peak(after(&plain)) > 0.01, "{name}: it sounds");
            let (worst, ratio) = deviation(&changed, &plain);
            let dip = if source == Source::Sine {
                let steady = peak(after(&plain));
                1.0 - lowest_window_peak(after(&changed), period()) / steady
            } else {
                0.0
            };
            println!(
                "{source:?}, {name} (latency {latency}): largest difference {worst:e}, \
                 difference energy {ratio:e} of the signal's, dip {:.2} %",
                dip * 100.0
            );
            if worst != 0.0 {
                failed.push(format!("{source:?}, {name}: {worst:e}"));
            }
        }
    }
    assert!(
        !exact || failed.is_empty(),
        "not the same signal to the sample: {failed:?}"
    );
}

#[test]
fn a_change_on_a_shorter_track_leaves_its_signal_unchanged_to_the_sample() {
    measure_shorter(true);
}

/// The residual of a transition from `plain` to `plain` moved `shift` frames later, against the
/// best clean splice between the two (a cut at the frame that leaves the least): the doubled
/// sound a crossfade leaves, over the signal's energy around the change.
fn splice_residual(changed: &[f32], plain: &[f32], shift: usize) -> f64 {
    let (a, b) = (after(changed), after(plain));
    let n = a.len();
    let late = |i: usize| if i >= shift { b[i - shift] } else { 0.0 };
    // Prefix energies of the two differences.
    let mut early_err = vec![0f64; n + 1];
    let mut late_err = vec![0f64; n + 1];
    for i in 0..n {
        let d0 = f64::from(a[i] - b[i]);
        let d1 = f64::from(a[i] - late(i));
        early_err[i + 1] = early_err[i] + d0 * d0;
        late_err[i + 1] = late_err[i] + d1 * d1;
    }
    let best = (0..=n)
        .map(|s| early_err[s] + (late_err[n] - late_err[s]))
        .fold(f64::INFINITY, f64::min);
    best / energy(b).max(1e-30)
}

#[test]
fn a_change_on_the_longest_track_realigns_every_track_in_one_fade() {
    // One track alone, then two (the other one without a latency): a Ceiling of 5 ms inserted
    // or removed moves the whole mix by its latency, one fade of 5 ms on every track. Its dip
    // on a held tone (two copies 240 frames apart crossfaded) is the one any crossfade has.
    let mut worst_dip = 0f32;
    for source in [Source::Sine, Source::Drums] {
        for (name, setup, change, shift) in [
            ("inserted", vec![], vec![ceiling(t(0), 1, 0.005)], 240usize),
            (
                "removed",
                vec![ceiling(t(0), 1, 0.005)],
                vec![remove(t(0), 1)],
                0,
            ),
        ] {
            for tracks in [&[t(0)][..], &[t(0), t(1)][..]] {
                let ((cl, cr), latency) = render(source, tracks, &setup, &change);
                let ((pl, pr), _) = render(source, tracks, &setup, &[]);
                // `plain` is the render without the change: before an insert the mix is not
                // delayed (shift the plain render later), after a removal it is no more (the
                // plain render, with the Ceiling, is later: shift the changed one instead).
                for (side, c, p) in [("left", &cl, &pl), ("right", &cr, &pr)] {
                    if tracks.len() == 1 && side == "right" {
                        continue;
                    }
                    let residual = if shift > 0 {
                        splice_residual(c, p, shift)
                    } else {
                        splice_residual(p, c, 240)
                    };
                    let dip = if source == Source::Sine {
                        let steady = peak(after(p));
                        1.0 - lowest_window_peak(after(c), period()) / steady
                    } else {
                        0.0
                    };
                    worst_dip = worst_dip.max(dip);
                    println!(
                        "{source:?}, {name}, {} track(s), {side} (latency {latency}): residual \
                         against a clean splice {residual:e} of the signal's energy, dip {:.2} %",
                        tracks.len(),
                        dip * 100.0
                    );
                }
            }
        }
    }
    assert!(worst_dip < 0.06, "one crossfade's dip at most: {worst_dip}");
}

/// Whether each of `tracks` moves its delay by a crossfade, and its delay, after a block.
fn moves(e: &Engine, tracks: &[TrackSlot]) -> Vec<(bool, u32)> {
    tracks
        .iter()
        .map(|tr| {
            let g = &e.state().track(*tr).unwrap().graph;
            (g.delay_moving(), g.delay())
        })
        .collect()
}

/// The blocks where each track's delay starts a move.
fn move_starts(seen: &[Vec<(bool, u32)>], tracks: usize) -> Vec<Vec<usize>> {
    (0..tracks)
        .map(|k| {
            (0..seen.len())
                .filter(|&b| seen[b][k].0 && (b == 0 || !seen[b - 1][k].0))
                .collect()
        })
        .collect()
}

#[test]
fn on_the_longest_track_every_track_starts_one_move_at_the_same_block() {
    // A Ceiling of 5 ms (240 frames, more than a block: a removal switches in two steps) and a
    // test delay of 1000 frames (eight steps), inserted on the track that then holds the
    // longest chain or removed from it; the other track has no latency. Each track's delay
    // makes one move, both from the same block; the slot switches exactly around it.
    for (name, setup, change, latency) in [
        (
            "Ceiling inserted",
            vec![],
            vec![ceiling(t(0), 1, 0.005)],
            240,
        ),
        (
            "Ceiling removed",
            vec![ceiling(t(0), 1, 0.005)],
            vec![remove(t(0), 1)],
            0,
        ),
        (
            "test delay inserted",
            vec![],
            vec![test_delay(t(0), 1, 1000)],
            1000,
        ),
        (
            "test delay removed",
            vec![test_delay(t(0), 1, 1000)],
            vec![remove(t(0), 1)],
            0,
        ),
    ] {
        let tracks = [t(0), t(1)];
        let (_, end, seen) = render_with(
            Source::Sine,
            &tracks,
            &setup,
            &[(BEFORE, change)],
            BLOCK,
            |e| moves(e, &tracks),
        );
        assert_eq!(end, latency, "{name}");
        let starts = move_starts(&seen, 2);
        println!("{name}: moves start at blocks {starts:?}");
        assert_eq!(starts[0].len(), 1, "{name}: one move: {starts:?}");
        assert_eq!(starts[0], starts[1], "{name}: from the same block");
        // The slot's switch: the track's delay jumps, with no move before or after.
        let jumped = (1..seen.len())
            .any(|b| seen[b][0].1 != seen[b - 1][0].1 && !seen[b][0].0 && !seen[b - 1][0].0);
        assert!(jumped, "{name}: its slot switched");
    }
}

fn gain_lane(track: TrackSlot) -> M {
    let pt = |tick: i64, value: f32, curve: Curve| AutomationPoint {
        tick: Tick(tick),
        value,
        curve,
    };
    // Ticks of 25 frames at 120 bpm: steps to silence and back, then a ramp, over the changes.
    M::AutomationLaneUpload {
        lane: LaneSlot(track.0),
        track,
        target: AutomationTarget::MixerGain,
        first: 0,
        total: 5,
        points: vec![
            pt(0, 0.0, Curve::Step),
            pt(190, -96.0, Curve::Step),
            pt(205, -6.0, Curve::Linear),
            pt(300, -30.0, Curve::Step),
            pt(330, 0.0, Curve::Step),
        ],
    }
}

fn mute(track: TrackSlot, on: bool) -> M {
    M::TrackSetMix {
        track,
        gain: 1.0,
        pan: -1.0,
        mute: on,
    }
}

/// The shorter track (left) with `events`, and with only those not marked as the change, in
/// blocks of `size`: equal to the sample.
fn same_as_without(name: &str, setup: &[M], events: &[(usize, Vec<M>, bool)], size: usize) {
    for source in [Source::Sine, Source::Drums] {
        let tracks = [t(0), t(1)];
        let with: Vec<(usize, Vec<M>)> = events.iter().map(|(b, m, _)| (*b, m.clone())).collect();
        let without: Vec<(usize, Vec<M>)> = events
            .iter()
            .filter(|(_, _, change)| !change)
            .map(|(b, m, _)| (*b, m.clone()))
            .collect();
        let ((changed, _), _, _) = render_with(source, &tracks, setup, &with, size, |_| ());
        let ((plain, _), _, _) = render_with(source, &tracks, setup, &without, size, |_| ());
        assert!(peak(&plain) > 0.01, "{name}: it sounds");
        let (worst, _) = deviation(&changed, &plain);
        println!("{source:?}, {name}, blocks of {size}: largest difference {worst:e}");
        assert_eq!(worst, 0.0, "{source:?}, {name}, blocks of {size}");
    }
}

#[test]
fn a_switch_keeps_the_strip_on_its_sound() {
    // A gain lane plays (steps to silence and back, a ramp) on both tracks while a Ceiling is
    // inserted on the shorter one, then removed; a mute fades as one is inserted. Blocks of
    // 128, 97 and 512 frames.
    let longest = test_delay(t(1), 1, 400);
    let setup = vec![
        M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::COMMON,
            launch_quantize: Quantize::default(),
        },
        gain_lane(t(0)),
        gain_lane(t(1)),
        longest.clone(),
    ];
    for size in [BLOCK, 97, 512] {
        same_as_without(
            "inserted while a lane plays",
            &setup,
            &[
                (1, vec![M::Play { from: Tick(0) }], false),
                (BEFORE, vec![ceiling(t(0), 1, 0.0015)], true),
            ],
            size,
        );
        same_as_without(
            "removed while a lane plays",
            &[setup.clone(), vec![ceiling(t(0), 1, 0.0015)]].concat(),
            &[
                (1, vec![M::Play { from: Tick(0) }], false),
                (BEFORE, vec![remove(t(0), 1)], true),
            ],
            size,
        );
        same_as_without(
            "inserted then removed while a lane plays",
            &setup,
            &[
                (1, vec![M::Play { from: Tick(0) }], false),
                (BEFORE, vec![ceiling(t(0), 1, 0.0015)], true),
                (BEFORE + 20, vec![remove(t(0), 1)], true),
            ],
            size,
        );
        same_as_without(
            "inserted as a mute fades",
            std::slice::from_ref(&longest),
            &[
                (BEFORE, vec![mute(t(0), true)], false),
                (BEFORE, vec![ceiling(t(0), 1, 0.005)], true),
                (BEFORE + 1, vec![mute(t(0), false)], false),
            ],
            size,
        );
    }
}

#[test]
fn a_switch_under_way_ends_with_a_mute_or_a_panic_cut() {
    // A test delay of 2000 frames inserted on the shorter track pauses what follows it for
    // 2000 frames (16 blocks): muted two blocks later, nobody hears the track, its slot takes
    // its timing at once and the pause is over; or the panic cut, then the note again.
    let longest = test_delay(t(1), 1, 2400);
    same_as_without(
        "muted during a pause",
        std::slice::from_ref(&longest),
        &[
            (BEFORE, vec![test_delay(t(0), 1, 2000)], true),
            (BEFORE + 2, vec![mute(t(0), true)], false),
            (BEFORE + 10, vec![mute(t(0), false)], false),
        ],
        BLOCK,
    );
    same_as_without(
        "panic cut during a pause",
        &[longest],
        &[
            (BEFORE, vec![test_delay(t(0), 1, 2000)], true),
            (BEFORE + 2, vec![M::AllSoundOff], false),
            (BEFORE + 6, vec![note(t(0), NOTE), note(t(1), NOTE)], false),
        ],
        BLOCK,
    );
}

#[test]
fn an_effect_after_the_slot_sees_its_input_unbroken() {
    // An echo after the Ceiling's slot: its tail keeps everything it was fed, nothing twice and
    // nothing missing, so the track is the same as without the change.
    let echo = insert(t(0), 2, ECHO.kind_id, &[]);
    let longest = test_delay(t(1), 1, 400);
    same_as_without(
        "inserted before an echo",
        &[longest.clone(), echo.clone()],
        &[(BEFORE, vec![ceiling(t(0), 1, 0.0015)], true)],
        BLOCK,
    );
    same_as_without(
        "removed before an echo",
        &[longest, ceiling(t(0), 1, 0.005), echo],
        &[(BEFORE, vec![remove(t(0), 1)], true)],
        BLOCK,
    );
}

/// Largest difference between two neighbouring samples.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

#[test]
fn a_capped_track_crossfades_its_slot_as_before() {
    // The other track holds five test delays of 4096 frames (20 480): the shorter track's
    // delay is capped at the limit, it cannot take a switch. A Ceiling inserted, or one
    // removed, crossfades its slot as before #761: no step beyond a crossfade's.
    let mut setup: Vec<M> = (1..=5).map(|d| test_delay(t(1), d, 4096)).collect();
    setup.push(ceiling(t(0), 1, 0.0015));
    for change in [vec![ceiling(t(0), 2, 0.005)], vec![remove(t(0), 1)]] {
        let mut rig = Rig::sized(BLOCK);
        let mut msgs = track(t(0), Source::Sine, -1.0);
        msgs.extend(track(t(1), Source::Sine, 1.0));
        msgs.extend_from_slice(&setup);
        msgs.push(note(t(0), NOTE));
        rig.send(&msgs);
        // The track's delay of 16 383 frames: 128 blocks before it sounds, and before a change
        // is heard.
        let mut early = (Vec::new(), Vec::new());
        for _ in 0..160 {
            rig.block(&mut early, BLOCK);
        }
        assert!(rig.e.graph().latency_capped(), "capped");
        rig.send(&change);
        let mut late = (Vec::new(), Vec::new());
        for _ in 0..170 {
            rig.block(&mut late, BLOCK);
            assert!(rig.e.graph().latency_capped(), "capped all along");
        }
        let tail = &early.0[early.0.len() - 4 * BLOCK..];
        let (steady, level) = (max_step(tail), peak(tail));
        assert!(level > 0.05, "it sounds");
        let mut joined = tail.to_vec();
        joined.extend_from_slice(&late.0);
        let step = max_step(&joined);
        let bound = steady * 1.05 + 2.0 * level / 240.0 + 1e-6;
        println!("capped, {change:?}: largest step {step} (bound {bound})");
        assert!(step <= bound, "a step of {step}, bound {bound}");
    }
}

#[test]
fn a_device_put_in_the_slot_of_a_holder_waits_for_it() {
    // A Ceiling replaced by a gain of 0.5 on the shorter track: the Ceiling holds the slot's
    // timing while it fades its own output into its line (one fade, then its switch at the next
    // block): meanwhile the slot gives what the Ceiling gave, the gain not heard yet, so the
    // track is the same as without the change; then the gain fades in on the new timing.
    let setup = [test_delay(t(1), 1, 400), ceiling(t(0), 1, 0.0015)];
    let change = [insert(t(0), 1, GAIN.kind_id, &[(gain::GAIN, 0.5)])];
    for source in [Source::Sine, Source::Drums] {
        let tracks = [t(0), t(1)];
        let ((changed, _), _) = render(source, &tracks, &setup, &change);
        let ((plain, _), _) = render(source, &tracks, &setup, &[]);
        // The track's delay of 328 frames, then the hold: the fade (240) and the block it
        // ends in.
        let held = 328 + 2 * BLOCK;
        let (a, b) = (&after(&changed)[..held], &after(&plain)[..held]);
        let worst = a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
        println!("{source:?}: during the hold, largest difference {worst:e}");
        assert_eq!(worst, 0.0, "{source:?}: the gain waits for the holder");
        // Then the gain is heard.
        let late = peak(&after(&changed)[held + 4 * BLOCK..]);
        let full = peak(&after(&plain)[held + 4 * BLOCK..]);
        assert!(
            late < 0.6 * full,
            "{source:?}: the gain halves it: {late} against {full}"
        );
    }
}
