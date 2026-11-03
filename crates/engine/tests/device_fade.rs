//! Device fades (#233): a device switched off or on, removed or replaced while its chain is
//! heard fades over the graph's fade instead of cutting the sound. Offline renders through
//! the real `Preparer`, `Engine::apply` and `Engine::render`, every audio-side call under the
//! allocation detector. For each change, the largest step between two samples around the
//! change stays under what a 5 ms ramp between the two steady signals allows, a hard splice
//! of the same two signals at the same frame breaks that bound (the metric catches a cut),
//! and once the fade is over the output is the new chain's. Then each transition of the
//! outgoing list: the retire ring, a full ring, the room, a silent chain, a switch back
//! during the fade, the panic cut, a removed track and a `Reset`.

use std::f64::consts::TAU;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::DeviceSetup;
use engine::device::testing::{GAIN, TONE, gain};
use engine::devices::echo::{self, DESCRIPTOR as ECHO};
use engine::engine::{Engine, Preparer, channel};
use engine::graph::{OUTGOING_ROOM, RETIRE_RING_CAPACITY, fade_frames};
use engine::msg::{
    ControlMsg as M, DeviceKind, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state,
};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
/// A low note (110 Hz): its steady signal moves little from one sample to the next, so a
/// cut stands out.
const LOW: u8 = 45;
/// Level of the test tone at velocity 127 (its gain 0.5 × 0.25).
const TONE_LEVEL: f32 = 0.125;

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
            assert_eq!(l, r, "a centred track: both channels alike");
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

    /// One block applied and rendered with no collect (the retire ring is not drained).
    fn block_no_collect(&mut self) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
        assert_no_alloc(|| self.e.process(&mut l, &mut r));
        l
    }

    fn outgoing(&self, track: TrackSlot) -> usize {
        if track.is_master() {
            self.e.graph().master_outgoing_len()
        } else {
            self.e.state().track(track).unwrap().graph.outgoing_len()
        }
    }
}

fn t(slot: u32) -> TrackSlot {
    TrackSlot(slot)
}

const MASTER: TrackSlot = TrackSlot::MASTER;

fn insert(track: TrackSlot, device: u16, kind: DeviceKind, state: &[(ParamId, f32)]) -> M {
    M::DeviceInsert {
        track,
        device: DeviceSlot(device),
        kind,
        state: encode_state(state),
    }
}

fn gain_fx(track: TrackSlot, device: u16, level: f32) -> M {
    insert(track, device, GAIN.kind_id, &[(gain::GAIN, level)])
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

/// A track playing the low note on the test tone.
fn playing() -> Vec<M> {
    vec![
        M::TrackCreate {
            track: t(0),
            kind: TrackKind::Midi,
        },
        insert(t(0), 0, TONE.kind_id, &[]),
        M::NoteOn {
            track: t(0),
            pitch: LOW,
            velocity: 127,
            at: None,
        },
    ]
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

/// Largest difference between two neighbouring samples.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// The first block count from `min` whose start falls where the tone is near its crest
/// (|cos| > 0.9): a hard cut there is a large step, whatever the levels.
fn blocks_at_crest(min: usize) -> usize {
    let hz = 440.0 * ((f64::from(LOW) - 69.0) / 12.0).exp2();
    (min..)
        .find(|k| {
            (TAU * hz * (k * BLOCK) as f64 / f64::from(RATE))
                .cos()
                .abs()
                > 0.9
        })
        .unwrap()
}

/// Blocks after the change: the fade (240 frames) and a long steady stretch.
const AFTER: usize = 12;

/// Plays `setup` (the low note sounding), sends `change` at a block that starts near the
/// tone's crest, and checks the change is a fade: the largest step around it is no larger
/// than a linear ramp between the steady signal before and the one after allows, a hard
/// splice of the two at the same frame is larger, and after the fade the output is
/// `reference`'s (the chain the change leads to, played from the start; `None`: silence).
fn fades(name: &str, setup: &[M], change: &[M], reference: Option<&[M]>) {
    let pre = blocks_at_crest(20);
    let mut r = Rig::new();
    r.send(setup);
    let before = r.run(pre);
    r.send(change);
    let after = r.run(AFTER);
    let expected = match reference {
        Some(msgs) => {
            let mut x = Rig::new();
            x.send(msgs);
            let all = x.run(pre + AFTER);
            all[pre * BLOCK..].to_vec()
        }
        None => vec![0.0; AFTER * BLOCK],
    };
    let n = fade_frames(RATE) as usize;
    assert_eq!(n, 240);
    // The steady signals on both sides, and how far the level moves.
    let steady_before = &before[before.len() - 2 * BLOCK..];
    let steady_after = &expected[2 * n..];
    let (step_before, step_after) = (max_step(steady_before), max_step(steady_after));
    let moved = (peak(steady_before) - peak(steady_after)).abs();
    let bound = step_before.max(step_after) * 1.05 + 2.0 * moved / n as f32 + 1e-6;
    let mut joined = before[before.len() - BLOCK..].to_vec();
    joined.extend_from_slice(&after[..4 * BLOCK]);
    let step = max_step(&joined);
    assert!(
        step <= bound,
        "{name}: a step of {step} at the change, bound {bound} (steady {step_before} → {step_after})"
    );
    // The same two signals spliced without a fade: the bound catches it.
    let mut cut = before[before.len() - BLOCK..].to_vec();
    cut.extend_from_slice(&expected[..4 * BLOCK]);
    assert!(
        max_step(&cut) > 2.0 * bound,
        "{name}: a hard cut ({}) must break the bound {bound}",
        max_step(&cut)
    );
    // Once the fade is over the output is the new chain's.
    let diff = after[2 * n..]
        .iter()
        .zip(&expected[2 * n..])
        .fold(0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(diff < 1e-5, "{name}: after the fade, off by {diff}");
    assert_eq!(r.outgoing(t(0)), 0, "{name}: the outgoing devices left");
    assert_eq!(r.outgoing(MASTER), 0, "{name}: the outgoing devices left");
}

fn with(base: Vec<M>, more: &[M]) -> Vec<M> {
    let mut v = base;
    v.extend_from_slice(more);
    v
}

#[test]
fn switching_removing_and_replacing_an_effect_fades() {
    let quarter = with(playing(), &[gain_fx(t(0), 1, 0.25)]);
    let off = with(quarter.clone(), &[enabled(t(0), 1, false)]);
    fades(
        "switch an effect off",
        &quarter,
        &[enabled(t(0), 1, false)],
        Some(&playing()),
    );
    fades(
        "switch an effect on",
        &off,
        &[enabled(t(0), 1, true)],
        Some(&quarter),
    );
    fades(
        "remove an effect",
        &quarter,
        &[remove(t(0), 1)],
        Some(&playing()),
    );
    fades(
        "replace an effect",
        &quarter,
        &[gain_fx(t(0), 1, 0.5)],
        Some(&with(playing(), &[gain_fx(t(0), 1, 0.5)])),
    );
    fades(
        "insert an effect",
        &quarter,
        &[gain_fx(t(0), 2, 4.0)],
        Some(&with(quarter.clone(), &[gain_fx(t(0), 2, 4.0)])),
    );
}

#[test]
fn switching_removing_and_replacing_the_instrument_fades_to_silence() {
    let chain = with(playing(), &[gain_fx(t(0), 1, 2.0)]);
    fades(
        "switch the instrument off",
        &chain,
        &[enabled(t(0), 0, false)],
        None,
    );
    fades("remove the instrument", &chain, &[remove(t(0), 0)], None);
    // The new instrument holds no note: the old one fades out under it.
    fades(
        "replace the instrument",
        &chain,
        &[insert(t(0), 0, TONE.kind_id, &[])],
        None,
    );
}

#[test]
fn the_masters_chain_fades_too() {
    let mastered = with(playing(), &[gain_fx(MASTER, 0, 0.25)]);
    fades(
        "switch a master effect off",
        &mastered,
        &[enabled(MASTER, 0, false)],
        Some(&playing()),
    );
    fades(
        "remove a master effect",
        &mastered,
        &[remove(MASTER, 0)],
        Some(&playing()),
    );
    fades(
        "replace a master effect",
        &mastered,
        &[gain_fx(MASTER, 0, 0.5)],
        Some(&with(playing(), &[gain_fx(MASTER, 0, 0.5)])),
    );
}

#[test]
fn an_outgoing_device_returns_through_the_retire_ring() {
    let mut r = Rig::new();
    r.send(&with(playing(), &[gain_fx(t(0), 1, 0.25)]));
    r.run(10);
    r.send(&[remove(t(0), 1)]);
    r.block_no_collect();
    assert_eq!(r.outgoing(t(0)), 1, "fading out");
    r.block_no_collect();
    assert_eq!(r.outgoing(t(0)), 0, "240 frames: over after two blocks");
    assert_eq!(r.p.collect(), 1, "the device came back to the prepare side");
}

#[test]
fn a_full_retire_ring_keeps_the_outgoing_device_until_there_is_room() {
    let mut r = Rig::new();
    r.send(&with(playing(), &[gain_fx(t(0), 1, 0.25)]));
    let before = r.run(10);
    // Fill the retire ring with empty tracks put again and again on slot 1, no collect.
    for _ in 0..=RETIRE_RING_CAPACITY {
        let create = M::TrackCreate {
            track: t(1),
            kind: TrackKind::Midi,
        };
        let rt = r.p.prepare(create).unwrap().unwrap();
        assert_no_alloc(|| r.e.apply(rt));
    }
    let rt = r.p.prepare(remove(t(0), 1)).unwrap().unwrap();
    assert_no_alloc(|| r.e.apply(rt));
    let mut out = Vec::new();
    for _ in 0..4 {
        out.extend(r.block_no_collect());
    }
    assert_eq!(
        r.e.graph().retiring(),
        0,
        "the empty tracks filled the ring"
    );
    assert_eq!(r.outgoing(t(0)), 1, "the ring is full: the device waits");
    let mut joined = before[before.len() - BLOCK..].to_vec();
    joined.extend_from_slice(&out);
    assert!(
        max_step(&joined) < 0.05 * TONE_LEVEL,
        "{}",
        max_step(&joined)
    );
    // It waits silent: the output is the tone alone.
    let alone = peak(&out[2 * BLOCK..]);
    assert!((alone - TONE_LEVEL).abs() < 1e-3, "{alone}");
    assert_eq!(r.p.collect(), RETIRE_RING_CAPACITY);
    r.block_no_collect();
    assert_eq!(r.outgoing(t(0)), 0, "room again: it left");
    assert_eq!(r.p.collect(), 1);
}

#[test]
fn past_the_room_a_device_leaves_at_once() {
    let mut r = Rig::new();
    let extra = 3;
    let count = OUTGOING_ROOM + extra;
    let mut setup = playing();
    for i in 0..count {
        setup.push(gain_fx(t(0), 1 + i as u16, 1.0));
    }
    r.send(&setup);
    r.run(10);
    let removals: Vec<M> = (0..count).map(|i| remove(t(0), 1 + i as u16)).collect();
    r.send(&removals);
    r.block_no_collect();
    assert_eq!(r.outgoing(t(0)), OUTGOING_ROOM, "the room is full");
    // The extra ones went to the garbage with their message.
    assert!(r.p.collect() >= extra);
    r.run(4);
    assert_eq!(r.outgoing(t(0)), 0);
}

#[test]
fn a_chain_nobody_hears_changes_at_once() {
    let mut r = Rig::new();
    r.send(&with(playing(), &[gain_fx(t(0), 1, 0.25)]));
    r.run(10);
    r.send(&[M::TrackSetMix {
        track: t(0),
        gain: 1.0,
        pan: 0.0,
        mute: true,
    }]);
    r.run(4);
    assert_eq!(r.e.state().track(t(0)).unwrap().graph.levels(), (0.0, 0.0));
    r.send(&[remove(t(0), 1), enabled(t(0), 0, false)]);
    r.block_no_collect();
    assert_eq!(r.outgoing(t(0)), 0, "muted: no fade");
    let tone = r.e.state().device(t(0), DeviceSlot(0)).unwrap();
    assert_eq!(tone.fade(), 0.0, "switched off at once");
    // A device put on a new track (not yet faded in) is there at once.
    let mut r = Rig::new();
    r.send(&with(playing(), &[gain_fx(t(0), 1, 0.25)]));
    r.block_no_collect();
    let gain = r.e.state().device(t(0), DeviceSlot(1)).unwrap();
    assert_eq!(gain.fade(), 1.0);
}

/// A switch back during the fade continues from where it is: an echo's tail is not reset
/// (it was still heard), and the output stays continuous.
#[test]
fn an_effect_switched_back_on_during_its_fade_keeps_its_tail() {
    let echo_state = [
        (ParamId(echo::MODE), 0.0),
        (ParamId(echo::TIME), 0.1),
        (ParamId(echo::MIX), 0.5),
        (ParamId(echo::FEEDBACK), 0.6),
    ];
    let setup = with(playing(), &[insert(t(0), 1, ECHO.kind_id, &echo_state)]);
    let release = M::NoteOff {
        track: t(0),
        pitch: LOW,
        at: None,
    };
    // Control: the echo left alone rings on after the note.
    let mut c = Rig::new();
    c.send(&setup);
    c.run(40);
    c.send(std::slice::from_ref(&release));
    let control = c.run(30);
    let mut r = Rig::new();
    r.send(&setup);
    r.run(40);
    r.send(&[release, enabled(t(0), 1, false)]);
    let mut out = r.block_no_collect();
    r.send(&[enabled(t(0), 1, true)]);
    out.extend(r.run(29));
    let fade = r.e.state().device(t(0), DeviceSlot(1)).unwrap().fade();
    assert_eq!(fade, 1.0, "back to full");
    assert!(max_step(&out) <= max_step(&control) * 1.05 + 1e-6);
    // The tail after the switch: the same echo as the control's (the dip of the switch
    // aside).
    let tail = peak(&out[20 * BLOCK..]);
    let control_tail = peak(&control[20 * BLOCK..]);
    assert!(
        tail > 0.5 * control_tail && control_tail > 0.01,
        "{tail} vs {control_tail}"
    );
}

#[test]
fn the_panic_cut_during_a_fade_leaves_nothing() {
    let mut r = Rig::new();
    r.send(&with(playing(), &[gain_fx(t(0), 1, 0.25)]));
    r.run(10);
    r.send(&[remove(t(0), 1), enabled(t(0), 0, false), M::AllSoundOff]);
    r.run(4);
    assert_eq!(r.outgoing(t(0)), 0);
    let tone = r.e.state().device(t(0), DeviceSlot(0)).unwrap();
    assert_eq!(tone.fade(), 0.0);
    assert_eq!(peak(&r.run(4)), 0.0);
}

#[test]
fn a_track_removed_while_a_device_fades_takes_it_along() {
    let mut r = Rig::new();
    r.send(&with(playing(), &[gain_fx(t(0), 1, 0.25)]));
    let before = r.run(10);
    r.send(&[remove(t(0), 1)]);
    let mut out = r.block_no_collect();
    assert_eq!(r.outgoing(t(0)), 1);
    r.send(&[M::TrackRemove { track: t(0) }]);
    out.extend(r.run(6));
    assert_eq!(r.e.graph().retiring(), 0, "the track and its device left");
    assert_eq!(peak(&out[4 * BLOCK..]), 0.0);
    let mut joined = before[before.len() - BLOCK..].to_vec();
    joined.extend_from_slice(&out);
    assert!(
        max_step(&joined) < 0.05 * TONE_LEVEL,
        "{}",
        max_step(&joined)
    );
}

#[test]
fn a_reset_ends_the_masters_fades() {
    let mut r = Rig::new();
    let mut setup = playing();
    // Slot 5: beyond the 4 slots a new state's master chain starts with.
    setup.push(gain_fx(MASTER, 5, 0.25));
    r.send(&setup);
    r.run(10);
    r.send(&[remove(MASTER, 5)]);
    r.block_no_collect();
    assert_eq!(r.outgoing(MASTER), 1);
    r.send(&[M::Reset { epoch: 1 }]);
    r.block_no_collect();
    assert_eq!(r.outgoing(MASTER), 0, "gone with the old chain");
    // An outgoing device whose slot the chain no longer has ends at the sweep too.
    let mut r = Rig::new();
    r.send(&setup);
    r.run(10);
    r.send(&[remove(MASTER, 5)]);
    r.run(4);
    assert_eq!(r.outgoing(MASTER), 0);
}
