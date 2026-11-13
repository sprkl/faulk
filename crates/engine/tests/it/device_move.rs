//! A device moved between slots keeps its instance (#800): `DeviceMove` neither builds,
//! drops nor fades it, so an echo's tail or a compressor's envelope goes on when a device is inserted or removed in front of it. Offline renders through the
//! real `Preparer`, `Engine::apply` and `Engine::process`, every audio-side call under the
//! allocation detector: a move with a transparent device inserted or removed in front gives
//! the same output, to the bit, as no change at all; a removal in front of an echo that moves
//! into its slot fades without a jump, as does a reorder; then the rules of `prepare` and
//! `apply` (an empty source, a taken target, a MIDI effect, a move on itself, a gone track).

use assert_no_alloc::assert_no_alloc;
use engine::device::DeviceSetup;
use engine::device::testing::{GAIN, TONE, TRANSPOSE, gain};
use engine::devices::echo::DESCRIPTOR as ECHO;
use engine::engine::{DeviceEntry, Engine, Preparer, channel};
use engine::graph::fade_frames;
use engine::msg::{
    ControlMsg as M, DeviceKind, DeviceSlot, MsgError, ParamId, Tag, TrackKind, TrackSlot,
    encode_state,
};

const RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
const LOW: u8 = 45;

struct Rig {
    p: Preparer,
    e: Engine,
}

impl Rig {
    fn new() -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE,
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

    /// `blocks` blocks; the left channel (every test track is centred).
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
        assert_eq!(self.e.limiter_silenced(), 0, "a non-finite sample");
        out
    }

    /// One short block of `frames`.
    fn run_frames(&mut self, frames: usize) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        assert_no_alloc(|| self.e.process(&mut l, &mut r));
        self.p.collect();
        l
    }

    fn outgoing(&self) -> usize {
        self.e.state().track(T).unwrap().graph.outgoing_len()
    }

    /// Where the outgoing devices fade out: slot and mode, in the order the chain runs them.
    fn places(&self) -> Vec<(u16, bool)> {
        self.fades().iter().map(|p| (p.0, p.1)).collect()
    }

    fn fades(&self) -> Vec<(u16, bool, f32)> {
        self.e
            .state()
            .track(T)
            .unwrap()
            .graph
            .outgoing_places()
            .collect()
    }
}

const T: TrackSlot = TrackSlot(0);

fn insert(device: u16, kind: DeviceKind, state: &[(ParamId, f32)]) -> M {
    M::DeviceInsert {
        track: T,
        device: DeviceSlot(device),
        kind,
        state: encode_state(state),
    }
}

fn gain_fx(device: u16, level: f32) -> M {
    insert(device, GAIN.kind_id, &[(gain::GAIN, level)])
}

fn echo(device: u16) -> M {
    insert(device, ECHO.kind_id, &[])
}

fn mv(from: u16, to: u16) -> M {
    M::DeviceMove {
        track: T,
        from: DeviceSlot(from),
        to: DeviceSlot(to),
    }
}

fn remove(device: u16) -> M {
    M::DeviceRemove {
        track: T,
        device: DeviceSlot(device),
    }
}

/// A track with the test tone in slot 0 and `chain` after it, playing the low note.
fn playing(chain: &[M]) -> Vec<M> {
    let mut v = vec![
        M::TrackCreate {
            track: T,
            kind: TrackKind::Midi,
        },
        insert(0, TONE.kind_id, &[]),
    ];
    v.extend_from_slice(chain);
    v.push(M::NoteOn {
        track: T,
        pitch: LOW,
        velocity: 127,
        at: None,
    });
    v
}

const NOTE_OFF: M = M::NoteOff {
    track: T,
    pitch: LOW,
    at: None,
};

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()))
}

/// Plays `setup`, stops the note so that only the echo's tail rings, sends `change` during
/// the tail and returns the whole output.
fn tail_with(setup: &[M], change: &[M]) -> Vec<f32> {
    let mut r = Rig::new();
    r.send(setup);
    let mut out = r.run(40);
    r.send(&[NOTE_OFF]);
    out.extend(r.run(60));
    r.send(change);
    out.extend(r.run(200));
    assert_eq!(r.outgoing(), 0, "nothing left fading out");
    out
}

/// Where the change lands in [`tail_with`]'s output, in frames.
const CHANGE_AT: usize = 100 * BLOCK;

#[test]
fn an_echo_moved_behind_a_transparent_insert_keeps_its_tail_to_the_bit() {
    let setup = playing(&[echo(1)]);
    let untouched = tail_with(&setup, &[]);
    // What the commands send for a unity gain inserted in front of the echo.
    let moved = tail_with(&setup, &[mv(1, 2), gain_fx(1, 1.0)]);
    let tail = peak(&untouched[CHANGE_AT..CHANGE_AT + 50 * BLOCK]);
    assert!(tail > 0.01, "the echo rings at the change: {tail}");
    assert_eq!(max_diff(&untouched, &moved), 0.0, "the same output");
    // The control: the echo built again one slot further, as before #800.
    let rebuilt = tail_with(&setup, &[gain_fx(1, 1.0), echo(2)]);
    let after = peak(&rebuilt[CHANGE_AT + 4 * BLOCK..CHANGE_AT + 50 * BLOCK]);
    assert!(
        after < 0.1 * tail,
        "a new echo starts silent: {after} against a tail of {tail}"
    );
}

#[test]
fn an_echo_moved_into_the_slot_of_a_removed_transparent_device_keeps_its_tail_to_the_bit() {
    let setup = playing(&[gain_fx(1, 1.0), echo(2)]);
    let untouched = tail_with(&setup, &[]);
    let moved = tail_with(&setup, &[remove(1), mv(2, 1)]);
    assert_eq!(max_diff(&untouched, &moved), 0.0, "the same output");
}

/// The largest step around frame `at` stays within what 5 ms linear moves of the signal
/// allow: the steady slope, plus twice the window's peak spread over one fade (a level that
/// moves as far as it can within a fade). A cut of the quarter gain at the tone's crest
/// (about 0.07) is two orders above it. Returns the step and the bound.
fn assert_no_jump(name: &str, out: &[f32], at: usize) -> (f32, f32) {
    let n = fade_frames(RATE) as usize;
    let before = &out[at - 2 * BLOCK..at];
    let after = &out[at + 4 * n..at + 4 * n + 2 * BLOCK];
    let window = &out[at - 2 * BLOCK..at + 4 * n + 2 * BLOCK];
    let bound = max_step(before).max(max_step(after)) * 1.05 + 2.0 * peak(window) / n as f32;
    let step = max_step(&out[at - BLOCK..at + 4 * n]);
    assert!(step <= bound, "{name}: a step of {step}, bound {bound}");
    // The cut it rules out: the quarter of the tone at its crest.
    assert!(
        10.0 * bound < 0.75 * 0.9 * 0.125,
        "{name}: the bound {bound} is too loose"
    );
    (step, bound)
}

/// Plays the note through `setup` (held), sends `change` at a block near the tone's crest
/// and returns the output and the frame of the change.
fn held_with(setup: &[M], change: &[M]) -> (Vec<f32>, usize, Rig) {
    let blocks = (40..)
        .find(|k: &usize| {
            let hz = 440.0 * ((f64::from(LOW) - 69.0) / 12.0).exp2();
            (std::f64::consts::TAU * hz * (k * BLOCK) as f64 / f64::from(RATE))
                .cos()
                .abs()
                > 0.9
        })
        .unwrap();
    let mut r = Rig::new();
    r.send(setup);
    let mut out = r.run(blocks);
    r.send(change);
    out.extend(r.run(40));
    (out, blocks * BLOCK, r)
}

#[test]
fn a_device_removed_in_front_of_a_moved_echo_fades_out_in_front_of_it() {
    // A quarter gain in front of the echo is removed and the echo moves into its slot: the
    // echo keeps hearing the quarter while it fades out, so its input, and the output,
    // move smoothly to the full level.
    let setup = playing(&[gain_fx(1, 0.25), echo(2)]);
    let (out, at, r) = held_with(&setup, &[remove(1), mv(2, 1)]);
    assert_no_jump("remove in front of the echo", &out, at);
    assert_eq!(r.outgoing(), 0);
}

#[test]
fn a_reorder_fades_through_the_old_order() {
    // Tone, echo (1), quarter gain (2). The gain moves in front of the echo as the commands
    // send it: the gain leaves (fading out where it was), the echo moves toward the tail into
    // its slot (the gain goes on fading after it, one past the last slot), and a new gain is
    // built in front.
    let setup = playing(&[echo(1), gain_fx(2, 0.25)]);
    let mut x = Rig::new();
    x.send(&setup);
    x.run(4);
    x.send(&[remove(2), mv(1, 2), gain_fx(1, 0.25)]);
    x.run(1);
    assert_eq!(
        x.places(),
        [(3, true)],
        "the old gain fades out after the echo"
    );
    let (out, at, r) = held_with(&setup, &[remove(2), mv(1, 2), gain_fx(1, 0.25)]);
    assert_no_jump("move the gain in front of the echo", &out, at);
    assert_eq!(r.outgoing(), 0);
    // The same at the end of a full row (4 slots): the old gain fades out one past the last
    // slot, which the chain runs too.
    let setup = playing(&[gain_fx(1, 0.5), echo(2), gain_fx(3, 0.25)]);
    let mut x = Rig::new();
    x.send(&setup);
    x.run(4);
    x.send(&[remove(3), mv(2, 3), gain_fx(2, 0.25)]);
    x.run(1);
    assert_eq!(x.places(), [(4, true)], "one past the row");
    assert_eq!(x.e.state().chain(T).unwrap().capacity(), 4);
    let (out, at, r) = held_with(&setup, &[remove(3), mv(2, 3), gain_fx(2, 0.25)]);
    assert_no_jump("move the last gain in front of the echo", &out, at);
    assert_eq!(r.outgoing(), 0);
    // The other way round: the echo goes back to the head.
    let setup = playing(&[gain_fx(1, 0.25), echo(2)]);
    let (out, at, r) = held_with(&setup, &[remove(1), mv(2, 1), gain_fx(2, 0.25)]);
    assert_no_jump("move the gain behind the echo", &out, at);
    assert_eq!(r.outgoing(), 0);
}

#[test]
fn two_removals_within_one_fade_keep_the_order_of_the_chain() {
    // Tone, half gain (1), quarter gain (2), echo (3). The quarter is removed and the echo
    // moves into its slot; within the fade the half is removed too and the echo moves again:
    // the quarter, still fading, follows the echo toward the head, behind the half.
    let setup = playing(&[gain_fx(1, 0.5), gain_fx(2, 0.25), echo(3)]);
    let mut x = Rig::new();
    x.send(&setup);
    x.run(4);
    x.send(&[remove(2), mv(3, 2), remove(1), mv(2, 1)]);
    x.run(1);
    assert_eq!(
        x.places(),
        [(1, true), (1, true)],
        "both fade out in front of the echo, the half first"
    );
    let (out, at, r) = held_with(&setup, &[remove(2), mv(3, 2), remove(1), mv(2, 1)]);
    let (step, bound) = assert_no_jump("two removals", &out, at);
    assert!(step < bound);
    assert_eq!(r.outgoing(), 0);
    // Half a block apart: the second edit lands while the first one fades.
    let mut r = Rig::new();
    r.send(&setup);
    let blocks = 41;
    let mut out = r.run(blocks);
    r.send(&[remove(2), mv(3, 2)]);
    out.extend(r.run_frames(BLOCK / 2));
    r.send(&[remove(1), mv(2, 1)]);
    out.extend(r.run_frames(BLOCK / 2));
    // The half (removed second, more of it heard) runs first, then the quarter: the order of
    // the chain they left.
    let fades = r.fades();
    assert_eq!(fades.len(), 2, "{fades:?}");
    assert!(fades.iter().all(|f| f.0 == 1 && f.1), "{fades:?}");
    assert!(fades[0].2 > fades[1].2, "{fades:?}");
    out.extend(r.run(40));
    assert_no_jump("two removals a block apart", &out, blocks * BLOCK);
    assert_eq!(r.outgoing(), 0);
}

#[test]
fn a_device_that_moves_takes_the_one_it_replaced_with_it() {
    // The echo in slot 2 is replaced (a sample-rate rebuild does that to every device): the
    // old one crossfades in parallel at slot 2. Within the fade the half gain in front is
    // removed and the new echo moves into its slot: the old echo goes with it.
    let setup = playing(&[gain_fx(1, 0.5), echo(2)]);
    let mut x = Rig::new();
    x.send(&setup);
    x.run(4);
    x.send(&[echo(2)]);
    x.run_frames(BLOCK / 2);
    x.send(&[remove(1), mv(2, 1)]);
    x.run_frames(BLOCK / 2);
    let mut places = x.places();
    places.sort_unstable();
    assert_eq!(
        places,
        [(1, false), (1, true)],
        "both at the echo's new slot"
    );
    // And toward the tail: the replaced one follows, the removed one stays in front.
    let setup = playing(&[gain_fx(1, 0.5), echo(2)]);
    let mut x = Rig::new();
    x.send(&setup);
    x.run(4);
    x.send(&[echo(2)]);
    x.run_frames(BLOCK / 2);
    x.send(&[mv(2, 3), gain_fx(2, 1.0)]);
    x.run_frames(BLOCK / 2);
    assert_eq!(
        x.places(),
        [(3, false)],
        "the old echo moved with the new one"
    );
}

#[test]
fn two_devices_with_a_line_in_a_row_hand_the_hold_on() {
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    // Ceiling, Ceiling, echo: the first is removed, the second moves into its held slot (built
    // again there, the old one leaving its own slot, which it then holds), and the echo moves
    // into that one, behind the old one (#838: it has no line): neither move is cut.
    let ceiling = |slot: u16| insert(slot, CEILING.kind_id, &[]);
    let setup = playing(&[ceiling(1), ceiling(2), echo(3)]);
    let (out, at, r) = held_with(&setup, &[remove(1), mv(2, 1), mv(3, 2)]);
    let n = fade_frames(RATE) as usize;
    let step = max_step(&out[at - BLOCK..at + 8 * n]);
    assert!(step < 0.005, "a step of {step} (a cut measured 0.031)");
    assert_eq!(r.outgoing(), 0);
    assert_eq!(r.e.skipped(), 0);
}

#[test]
fn a_replaced_device_with_a_line_that_holds_nothing_follows_its_replacement() {
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    // A Ceiling replaced by another Ceiling: the new one takes its line (#749), so the old
    // one holds no timing (#761) and crossfades in parallel. Within the fade the gain in front
    // is removed and the new Ceiling moves into its slot: the old one goes with it, as a
    // replaced device without a line does; only a holder stays.
    let ceiling = |slot: u16| insert(slot, CEILING.kind_id, &[]);
    let setup = playing(&[gain_fx(1, 1.0), ceiling(2)]);
    let mut x = Rig::new();
    x.send(&setup);
    x.run(4);
    x.send(&[ceiling(2)]);
    x.run_frames(BLOCK / 2);
    assert_eq!(x.places(), [(2, false)], "the old Ceiling crossfades at 2");
    x.send(&[remove(1), mv(2, 1)]);
    x.run_frames(BLOCK / 2);
    let mut places = x.places();
    places.sort_unstable();
    assert_eq!(
        places,
        [(1, false), (1, true)],
        "the old Ceiling followed the new one, the gain fades out in front"
    );
    x.run(10);
    assert_eq!(x.outgoing(), 0);
    assert_eq!(x.e.skipped(), 0);
}

#[test]
fn a_moved_device_keeps_its_parameters_its_bypass_and_its_slot_in_the_shadow() {
    let mut r = Rig::new();
    r.send(&playing(&[gain_fx(1, 0.5)]));
    r.send(&[M::DeviceSetEnabled {
        track: T,
        device: DeviceSlot(1),
        enabled: false,
    }]);
    r.run(4);
    // Past the row (4 slots): it grows.
    r.send(&[mv(1, 9)]);
    r.run(2);
    let moved = r.e.state().device(T, DeviceSlot(9)).expect("moved");
    assert!(!moved.enabled, "its bypass moved with it");
    assert!(r.e.state().device(T, DeviceSlot(1)).is_none());
    // The prepare side follows: the new slot takes messages, the old one refuses them.
    let set = |device: u16| M::ParamSet {
        track: T,
        device: DeviceSlot(device),
        param: gain::GAIN,
        value: 0.25,
    };
    assert!(r.p.prepare(set(9)).is_ok());
    assert_eq!(r.p.prepare(set(1)).map(|_| ()), Err(MsgError::NoSuchTarget));
    // A sample-rate rebuild builds it at its new slot, bypassed (#236).
    r.p.set_sample_rate(engine::time::SampleRate::new(44_100).unwrap())
        .unwrap();
    r.run(4);
    let rebuilt = r.e.state().device(T, DeviceSlot(9)).expect("rebuilt there");
    assert!(!rebuilt.enabled);
    assert_eq!(r.e.skipped(), 0);
}

#[test]
fn prepare_refuses_an_empty_source_a_taken_target_and_a_midi_effect() {
    let mut r = Rig::new();
    r.send(&[
        M::TrackCreate {
            track: T,
            kind: TrackKind::Midi,
        },
        insert(0, TRANSPOSE.kind_id, &[]),
        insert(16, TONE.kind_id, &[]),
        gain_fx(17, 0.5),
    ]);
    let bad = Err(MsgError::BadPayload {
        tag: Tag::DeviceMove as u16,
    });
    let prepare = |r: &mut Rig, msg: M| r.p.prepare(msg).map(|rt| rt.is_some());
    assert_eq!(prepare(&mut r, mv(5, 6)), Err(MsgError::NoSuchTarget));
    assert_eq!(prepare(&mut r, mv(17, 16)), bad, "the target is taken");
    assert_eq!(prepare(&mut r, mv(0, 1)), bad, "a MIDI effect");
    assert_eq!(prepare(&mut r, mv(16, 20)), bad, "the instrument");
    assert_eq!(prepare(&mut r, mv(17, 17)), Ok(false), "nothing to move");
    let other = M::DeviceMove {
        track: TrackSlot(3),
        from: DeviceSlot(17),
        to: DeviceSlot(18),
    };
    assert_eq!(prepare(&mut r, other), Err(MsgError::NoSuchTarget));
    assert_eq!(
        M::DeviceMove {
            track: T,
            from: DeviceSlot(17),
            to: DeviceSlot(u16::MAX),
        }
        .check_limits()
        .map_err(|_| ()),
        Err(())
    );
}

#[test]
fn a_move_whose_track_is_gone_is_skipped_and_its_grown_row_goes_back() {
    let mut r = Rig::new();
    r.send(&playing(&[gain_fx(1, 0.5)]));
    r.run(2);
    // Prepared while the track exists, applied after it left.
    let moved = r.p.prepare(mv(1, 40)).unwrap().expect("an rt message");
    let gone =
        r.p.prepare(M::TrackRemove { track: T })
            .unwrap()
            .expect("an rt message");
    assert_no_alloc(|| r.e.apply(gone));
    let skipped = r.e.skipped();
    assert_no_alloc(|| r.e.apply(moved));
    assert_eq!(r.e.skipped(), skipped + 1);
    r.run(4);
    r.p.collect();
}

#[test]
fn a_device_entry_is_no_larger_than_before() {
    // Every byte of it counts in the worst case of the engine's tables (`msg.rs`): the
    // outgoing mode (#800) fits in its padding.
    assert_eq!(size_of::<DeviceEntry>(), 56);
}

/// Plays the low note through `setup` and sends `change` after 40 blocks: the output and the
/// device latency the graph reports at each block.
fn timed_with(setup: &[M], change: &[M]) -> (Vec<f32>, Vec<u32>) {
    let mut r = Rig::new();
    r.send(setup);
    let (mut out, mut latency) = (Vec::new(), Vec::new());
    for block in 0..120 {
        if block == 40 {
            r.send(change);
        }
        out.extend(r.run(1));
        latency.push(r.e.graph().latency());
    }
    assert_eq!(r.outgoing(), 0);
    assert_eq!(r.e.skipped(), 0);
    (out, latency)
}

#[test]
fn a_ceiling_moved_keeps_its_timing() {
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    // The tone (0.125) is under the Ceiling's ceiling: the Ceiling only delays it, by its
    // look-ahead (72 frames at 48 kHz).
    let ceiling = |slot: u16| insert(slot, CEILING.kind_id, &[]);
    let setup = playing(&[ceiling(1)]);
    let (untouched, steady) = timed_with(&setup, &[]);
    assert!(steady.iter().skip(2).all(|l| *l == 72), "{steady:?}");
    // A unity gain inserted in front: the Ceiling moves one slot further with its lines, so the
    // timing never moves and the output is the same, to the bit.
    let (moved, latency) = timed_with(&setup, &[mv(1, 2), gain_fx(1, 1.0)]);
    assert_eq!(latency, steady, "the timing never moved");
    assert_eq!(max_diff(&untouched, &moved), 0.0);
    // Removed in front of it: the same.
    let setup = playing(&[gain_fx(1, 1.0), ceiling(2)]);
    let (untouched, steady) = timed_with(&setup, &[]);
    let (moved, latency) = timed_with(&setup, &[remove(1), mv(2, 1)]);
    assert_eq!(latency, steady, "the timing never moved");
    assert_eq!(max_diff(&untouched, &moved), 0.0);
    // The control: the Ceiling built again one slot further, as before #800 (its line starts
    // empty, the slot switches its timing).
    let setup = playing(&[ceiling(1)]);
    let (rebuilt, _) = timed_with(&setup, &[gain_fx(1, 1.0), ceiling(2)]);
    assert!(max_diff(&untouched, &rebuilt) > 0.0);
}

#[test]
fn a_device_moved_into_the_slot_a_ceiling_holds_goes_behind_it_without_a_step() {
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    // The Ceiling in front of a quarter gain is removed and the gain moves into its slot: the
    // Ceiling holds the slot's timing (#761), and the gain, which has no line, goes behind it
    // (#838): not built again, it hears the Ceiling's line until the slot switches. No step,
    // no larger than the messages before #800 gave (the gain built again in the held slot).
    let setup = playing(&[insert(1, CEILING.kind_id, &[]), gain_fx(2, 0.25)]);
    let (out, latency) = timed_with(&setup, &[remove(1), mv(2, 1)]);
    assert_eq!(latency[0], 72);
    assert_eq!(
        *latency.last().unwrap(),
        0,
        "the timing left with the Ceiling"
    );
    // Alone on its track, the Ceiling holds the longest chain: the tracks' latency stays
    // while it holds its slot (its own fade into its line), then only goes down, once the
    // slot switched (#761), as any removal of a device with a latency makes it.
    let n = fade_frames(RATE) as usize;
    assert!(
        latency[40..40 + n / BLOCK].iter().all(|l| *l == 72),
        "{latency:?}"
    );
    assert!(latency.windows(2).all(|w| w[1] <= w[0]), "{latency:?}");
    // Settled on the quarter at the end.
    let end = peak(&out[out.len() - 10 * BLOCK..]);
    assert!((end - 0.25 * 0.125).abs() < 1e-3, "{end}");
    // No step: no larger than what the messages before #800 gave (the device built again in
    // place of the Ceiling, the old one removed), far below the cut of a hard switch (an
    // unheld move measured 0.031).
    let n = fade_frames(RATE) as usize;
    for (name, chain, before) in [
        (
            "a gain moved into a held slot",
            gain_fx(2, 0.25),
            [gain_fx(1, 0.25), remove(2)],
        ),
        (
            "an echo moved into a held slot",
            echo(2),
            [echo(1), remove(2)],
        ),
    ] {
        let setup = playing(&[insert(1, CEILING.kind_id, &[]), chain]);
        let (out, at, r) = held_with(&setup, &[remove(1), mv(2, 1)]);
        let (old, old_at, _) = held_with(&setup, &before);
        let step = max_step(&out[at - BLOCK..at + 8 * n]);
        let reference = max_step(&old[old_at - BLOCK..old_at + 8 * n]);
        assert!(
            step <= reference * 1.05 + 1e-6 && step < 0.005,
            "{name}: a step of {step} against {reference} before #800"
        );
        assert_eq!(r.outgoing(), 0);
        assert_eq!(r.e.skipped(), 0);
    }
}

/// A second track whose chain is longer than track 0's (two Ceilings, 144 frames at 48 kHz),
/// silent: it holds the tracks' latency `T`, so a change of track 0's chain moves only track
/// 0's delay (#761).
fn longer_track() -> Vec<M> {
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    let t = TrackSlot(1);
    let mut v = vec![M::TrackCreate {
        track: t,
        kind: TrackKind::Midi,
    }];
    for slot in 0..3 {
        let kind = if slot == 0 {
            TONE.kind_id
        } else {
            CEILING.kind_id
        };
        v.push(M::DeviceInsert {
            track: t,
            device: DeviceSlot(slot),
            kind,
            state: encode_state(&[]),
        });
    }
    v
}

fn compressor(slot: u16, lookahead: f32) -> M {
    use engine::devices::compressor::{DESCRIPTOR as COMPRESSOR, LOOKAHEAD};
    insert(slot, COMPRESSOR.kind_id, &[(ParamId(LOOKAHEAD), lookahead)])
}

#[test]
fn an_echo_moved_into_the_slot_a_ceiling_holds_goes_behind_it_and_keeps_its_timing() {
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    // #838: the Ceiling in front of the echo is removed and the echo moves into its slot. The
    // Ceiling holds the slot's timing (#761) and the echo, not built again, goes behind it:
    // it receives the Ceiling's line, then the slot switches and the echo catches up the
    // frames it skipped. Below its ceiling the Ceiling only delays the tone: the output is the
    // one without the edit, and the reported latency never moves (the other track holds the
    // longest chain).
    let mut setup = longer_track();
    setup.extend(playing(&[insert(1, CEILING.kind_id, &[]), echo(2)]));
    let (untouched, steady) = timed_with(&setup, &[]);
    assert!(steady.iter().skip(2).all(|l| *l == 144), "{steady:?}");
    let (moved, latency) = timed_with(&setup, &[remove(1), mv(2, 1)]);
    assert_eq!(latency, steady, "the timing never moved");
    let diff = max_diff(&untouched, &moved);
    println!("echo behind a held Ceiling: max difference {diff:e}");
    assert_eq!(
        diff, 0.0,
        "the echo kept its input and its tail, to the bit"
    );
    // The control, the behaviour before #838: the echo built again in the held slot.
    let (rebuilt, _) = timed_with(&setup, &[remove(1), echo(1), remove(2)]);
    assert!(max_diff(&untouched, &rebuilt) > 1e-3);
}

#[test]
fn an_echo_moved_behind_a_removed_compressor_keeps_its_tail_whatever_the_look_ahead() {
    // A Compressor has a line whatever its look-ahead (#749): at 0 (its default) it holds a
    // slot of no timing (it held it for ever before #838, the echo silent behind it), at 2 ms
    // one of 96 frames, at 10 ms one of 480 (its slot then switches over four blocks, each
    // a catch-up with the holder still there). Either way the echo moved behind it keeps its
    // tail through the removal and the switch, and nothing is left fading out (`tail_with`).
    for lookahead in [0.0, 0.002, 0.01] {
        let setup = playing(&[compressor(1, lookahead), echo(2)]);
        let untouched = tail_with(&setup, &[]);
        let moved = tail_with(&setup, &[remove(1), mv(2, 1)]);
        let window = CHANGE_AT..CHANGE_AT + 100 * BLOCK;
        let (kept, got) = (
            peak(&untouched[window.clone()]),
            peak(&moved[window.clone()]),
        );
        let diff = max_diff(&untouched, &moved);
        println!(
            "look-ahead {lookahead}: echo tail peak {kept:.5}, moved {got:.5}, max difference \
             {diff:e}"
        );
        assert!(kept > 0.01, "the echo rings: {kept}");
        assert!((got / kept - 1.0).abs() < 0.02, "{got} against {kept}");
        // On a held tone, through the Compressor's own fade into its line and the switch.
        let (out, at, r) = held_with(&setup, &[remove(1), mv(2, 1)]);
        let (step, bound) = assert_no_jump("compressor removed in front of the echo", &out, at);
        println!("look-ahead {lookahead}: held tone step {step:.5}, bound {bound:.5}");
        assert_eq!(r.e.skipped(), 0);
    }
}

#[test]
fn an_echo_moved_behind_a_compressor_removed_from_the_master_keeps_its_tail() {
    // The master's chain does not switch its slots (#761): a device with a line removed from
    // it fades its line out, from the delayed signal to the undelayed one. The echo moved
    // into its slot goes behind it (#838) and so hears that crossfade instead of the
    // undelayed signal at once: no step on a held tone, and its tail goes on.
    let master = |m: &M| match m.clone() {
        M::DeviceInsert {
            device,
            kind,
            state,
            ..
        } => M::DeviceInsert {
            track: TrackSlot::MASTER,
            device,
            kind,
            state,
        },
        M::DeviceRemove { device, .. } => M::DeviceRemove {
            track: TrackSlot::MASTER,
            device,
        },
        M::DeviceMove { from, to, .. } => M::DeviceMove {
            track: TrackSlot::MASTER,
            from,
            to,
        },
        other => other,
    };
    for lookahead in [0.0, 0.002] {
        let mut setup = playing(&[]);
        setup.extend([compressor(0, lookahead), echo(1)].iter().map(master));
        let change: Vec<M> = [remove(0), mv(1, 0)].iter().map(master).collect();
        let untouched = tail_with(&setup, &[]);
        let moved = tail_with(&setup, &change);
        let window = CHANGE_AT..CHANGE_AT + 100 * BLOCK;
        let (kept, got) = (
            peak(&untouched[window.clone()]),
            peak(&moved[window.clone()]),
        );
        println!("master, look-ahead {lookahead}: echo tail peak {kept:.5}, moved {got:.5}");
        assert!(kept > 0.01, "the echo rings: {kept}");
        assert!((got / kept - 1.0).abs() < 0.02, "{got} against {kept}");
        let (out, at, _) = held_with(&setup, &change);
        let (step, bound) = assert_no_jump("compressor removed from the master", &out, at);
        println!("master, look-ahead {lookahead}: held tone step {step:.5}, bound {bound:.5}");
    }
}

/// Plays the note through `setup` (held), sends `first` at a block near the tone's crest,
/// `second` one block later, and returns the output and the frame of `first`.
fn twice_with(setup: &[M], first: &[M], second: &[M]) -> (Vec<f32>, usize, Rig) {
    let (mut out, at, mut r) = held_with(setup, first);
    out.truncate(at + BLOCK);
    // `held_with` ran 40 blocks after `first`: start again, the second edit one block in.
    let mut x = Rig::new();
    x.send(setup);
    let mut again = x.run(at / BLOCK);
    x.send(first);
    again.extend(x.run(1));
    x.send(second);
    again.extend(x.run(60));
    assert_eq!(max_diff(&out, &again[..out.len()]), 0.0);
    r.p.collect();
    (again, at, x)
}

#[test]
fn a_device_removed_behind_a_holder_fades_out_behind_it() {
    // Compressor (2 ms), quarter gain, echo. The Compressor is removed (it holds slot 1, the
    // gain moving behind it), then, one block later, the gain (it fades out in series at slot
    // 1, the echo moving behind it). The chain ran the Compressor, then the gain: the gain
    // keeps fading out behind the Compressor's line, so the echo never hears the tone without
    // the gain while the line still delays it.
    let setup = playing(&[compressor(1, 0.002), gain_fx(2, 0.25), echo(3)]);
    let (out, at, r) = twice_with(
        &setup,
        &[remove(1), mv(2, 1), mv(3, 2)],
        &[remove(1), mv(2, 1)],
    );
    let (step, bound) = assert_no_jump("a gain removed behind a holder", &out, at + BLOCK);
    println!("gain removed behind a holder: step {step:.5}, bound {bound:.5}");
    assert_eq!(r.outgoing(), 0);
    assert_eq!(r.e.skipped(), 0);
}

#[test]
fn the_last_device_behind_a_holder_removed_fades_out_behind_it() {
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    // Ceiling, quarter gain: the Ceiling is removed (the gain moves behind it), then, one
    // block later, the gain, with nothing moving into the slot after it. The gain keeps
    // fading out behind the Ceiling's line, so its input never jumps back by the Ceiling's
    // latency, and the slot's switch catches it up.
    let setup = playing(&[insert(1, CEILING.kind_id, &[]), gain_fx(2, 0.25)]);
    let (out, at, r) = twice_with(&setup, &[remove(1), mv(2, 1)], &[remove(1)]);
    let (step, bound) = assert_no_jump("the last device behind a holder removed", &out, at + BLOCK);
    println!("last device behind a holder removed: step {step:.5}, bound {bound:.5}");
    assert_eq!(r.outgoing(), 0);
    assert_eq!(r.e.skipped(), 0);
}

#[test]
fn a_device_replaced_behind_a_holder_crossfades_through_the_switch() {
    // Ceiling, echo: the Ceiling is removed (the echo moves behind it), then, one block later,
    // the echo is replaced by another echo. The old one fades out in parallel with the new one
    // behind the holder, and both run the frames the slot's switch catches up (#761): no
    // step through the switch, nothing left fading out.
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    let setup = playing(&[insert(1, CEILING.kind_id, &[]), echo(2)]);
    let (out, at, r) = twice_with(&setup, &[remove(1), mv(2, 1)], &[echo(1)]);
    let (step, bound) = assert_no_jump("an echo replaced behind a holder", &out, at + BLOCK);
    println!("echo replaced behind a holder: step {step:.5}, bound {bound:.5}");
    assert_eq!(r.outgoing(), 0);
    assert_eq!(r.e.skipped(), 0);
}

#[test]
fn an_effect_moved_toward_the_tail_into_a_held_slot_is_built_again_there() {
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    // Echo (1), Ceiling (2): the Ceiling moves in front of the echo as the commands send a
    // swap of neighbours: the Ceiling leaves slot 2 (it holds it), the echo moves toward the
    // tail into it, and a new Ceiling goes to slot 1. The echo was in front of the Ceiling,
    // not behind: it does not go behind the holder (its input would jump back by the
    // Ceiling's latency, #838); the device built again by `prepare` takes the slot, held then
    // faded in, as before. No step, nothing left fading out.
    let setup = playing(&[echo(1), insert(2, CEILING.kind_id, &[])]);
    let (out, at, r) = held_with(
        &setup,
        &[remove(2), mv(1, 2), insert(1, CEILING.kind_id, &[])],
    );
    let (step, bound) = assert_no_jump("an echo moved toward the tail into a held slot", &out, at);
    println!("echo moved toward the tail into a held slot: step {step:.5}, bound {bound:.5}");
    assert_eq!(r.outgoing(), 0);
    assert_eq!(r.e.skipped(), 0);
}

#[test]
fn an_effect_put_in_a_held_slot_waits_for_the_holder() {
    use engine::devices::ceiling::DESCRIPTOR as CEILING;
    // A Ceiling removed, then an echo inserted in its slot: the Ceiling holds the slot and the
    // echo, put there (not moved there from behind it), runs unheard until the holder is gone,
    // as before #838: the first blocks give the tone through the Ceiling's line alone.
    let setup = playing(&[insert(1, CEILING.kind_id, &[])]);
    let (alone, _) = timed_with(&setup, &[remove(1)]);
    let (put, _) = timed_with(&setup, &[remove(1), echo(1)]);
    let at = 40 * BLOCK;
    assert_eq!(max_diff(&alone[at..at + BLOCK], &put[at..at + BLOCK]), 0.0);
}
