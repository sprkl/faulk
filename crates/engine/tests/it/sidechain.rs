//! A device's detection listening to another track (#952, `crate::graph` > Sidechain): the
//! Compressor of one track turned down by another's signal, through the real `Preparer`,
//! `Engine::apply` and `Engine::process`, every audio-side call under the allocation
//! detector. The source renders first whatever its slot, is tapped after its devices and
//! before its strip (a muted source, a solo elsewhere, still key), and every change of the
//! tracks or of a chain plans the sources again; a source that is gone, a loop, or one past
//! the buffers gives silence; a source track renders exactly as when nobody listens to it.

use assert_no_alloc::assert_no_alloc;
use engine::device::DeviceSetup;
use engine::device::testing::{TONE, tone};
use engine::devices::compressor::{self, DESCRIPTOR as COMPRESSOR};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{ControlMsg as M, DeviceSlot, MsgError, Tag, TrackKind, TrackSlot, encode_state};
use engine::sidechain::MAX_KEY_SOURCES;

const RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
/// The compressed track (slot 0) and its source (slot 1: after it in slot order).
const PAD: TrackSlot = TrackSlot(0);
const KICK: TrackSlot = TrackSlot(1);
/// The Compressor's slot on the pad's chain (the tone is 0).
const COMP: DeviceSlot = DeviceSlot(1);

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

    /// `blocks` blocks of the left channel, checked finite and free of denormals.
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

    /// The reduction the Compressor of `track` in `slot` reports after the last block.
    fn reduction(&self, track: TrackSlot, slot: DeviceSlot) -> f32 {
        self.e
            .state()
            .track(track)
            .unwrap()
            .devices
            .as_slice()
            .get(slot.0 as usize)
            .and_then(|e| e.as_deref())
            .map_or(-1.0, |e| e.device.gain_reduction_db())
    }
}

fn create(track: TrackSlot) -> M {
    M::TrackCreate {
        track,
        kind: TrackKind::Midi,
    }
}

/// A tone at `gain` on `track`, slot 0: its peak is a quarter of it.
fn tone_at(track: TrackSlot, gain: f32) -> M {
    M::DeviceInsert {
        track,
        device: DeviceSlot(0),
        kind: TONE.kind_id,
        state: encode_state(&[(tone::GAIN, gain)]),
    }
}

/// A Compressor on `track` in `slot`, at a fast attack and release so a test settles fast.
fn comp(track: TrackSlot, slot: DeviceSlot) -> M {
    M::DeviceInsert {
        track,
        device: slot,
        kind: COMPRESSOR.kind_id,
        state: encode_state(&[
            (engine::msg::ParamId(compressor::THRESHOLD), -40.0),
            (engine::msg::ParamId(compressor::ATTACK), 0.0005),
            (engine::msg::ParamId(compressor::RELEASE), 0.01),
        ]),
    }
}

fn key(track: TrackSlot, device: DeviceSlot, source: Option<TrackSlot>) -> M {
    M::DeviceSidechain {
        track,
        device,
        source,
    }
}

fn on(track: TrackSlot, pitch: u8) -> M {
    M::NoteOn {
        track,
        pitch,
        velocity: 127,
        at: None,
    }
}

fn off(track: TrackSlot, pitch: u8) -> M {
    M::NoteOff {
        track,
        pitch,
        at: None,
    }
}

fn mute(track: TrackSlot) -> M {
    M::TrackSetMix {
        track,
        gain: 1.0,
        pan: 0.0,
        mute: true,
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// The pad (quiet, -46 dBFS at its peak, under the -40 dB threshold: its own level never
/// compresses it) with a Compressor, and the kick (-12 dBFS, 28 dB over the threshold: a
/// reduction of 21 dB at 4:1) muted, after it in slot order, so the master holds the pad
/// alone.
fn pad_and_muted_kick(r: &mut Rig) {
    r.send(&[
        create(PAD),
        create(KICK),
        tone_at(PAD, 0.02),
        comp(PAD, COMP),
        tone_at(KICK, 1.0),
        mute(KICK),
        on(PAD, 60),
    ]);
    r.run(40);
}

#[test]
fn a_muted_source_in_a_later_slot_turns_the_pad_down_and_lets_it_back_up() {
    let mut r = Rig::new();
    pad_and_muted_kick(&mut r);
    let alone = rms(&r.run(40));
    assert!(alone > 0.002, "the pad sounds: {alone}");
    assert_eq!(
        r.reduction(PAD, COMP),
        0.0,
        "the pad is under the threshold"
    );

    // Listening to its own track, the kick changes nothing.
    r.send(&[on(KICK, 64)]);
    let own = rms(&r.run(40));
    assert!((own / alone - 1.0).abs() < 0.01, "{own} {alone}");
    r.send(&[off(KICK, 64)]);
    r.run(40);

    r.send(&[key(PAD, COMP, Some(KICK)), on(KICK, 64)]);
    r.run(20);
    let ducked = rms(&r.run(40));
    let reduction = r.reduction(PAD, COMP);
    let db = 20.0 * (ducked / alone).log10();
    assert!(
        reduction > 6.0,
        "the kick drives the reduction: {reduction} dB"
    );
    assert!(db < -6.0, "the pad goes down by the reduction: {db} dB");
    assert!(
        (db + reduction).abs() < 1.5,
        "the level follows the reported reduction: {db} dB against {reduction} dB"
    );

    r.send(&[off(KICK, 64)]);
    r.run(60);
    let back = rms(&r.run(40));
    assert!(
        (back / alone - 1.0).abs() < 0.02,
        "the pad comes back up: {back} against {alone}"
    );
    assert!(r.reduction(PAD, COMP) < 0.05);
}

#[test]
fn the_detection_follows_the_source_while_the_compressed_track_is_silent() {
    let mut r = Rig::new();
    pad_and_muted_kick(&mut r);
    r.send(&[off(PAD, 60), key(PAD, COMP, Some(KICK))]);
    r.run(40);
    assert_eq!(r.reduction(PAD, COMP), 0.0);
    r.send(&[on(KICK, 64)]);
    r.run(20);
    assert!(r.reduction(PAD, COMP) > 6.0, "{}", r.reduction(PAD, COMP));
}

#[test]
fn a_solo_elsewhere_does_not_cut_the_detection() {
    let mut r = Rig::new();
    pad_and_muted_kick(&mut r);
    let other = TrackSlot(2);
    r.send(&[
        create(other),
        M::TrackSetSolo {
            track: other,
            solo: true,
        },
        key(PAD, COMP, Some(KICK)),
        on(KICK, 64),
    ]);
    r.run(30);
    assert!(r.reduction(PAD, COMP) > 6.0, "{}", r.reduction(PAD, COMP));
}

#[test]
fn a_source_track_renders_the_same_samples_as_when_nobody_listens_to_it() {
    // The kick heard alone (the pad muted): listened to or not, the same output to the bit.
    let render = |listened: bool| {
        let mut r = Rig::new();
        r.send(&[
            create(PAD),
            create(KICK),
            tone_at(PAD, 0.02),
            comp(PAD, COMP),
            tone_at(KICK, 1.0),
            mute(PAD),
        ]);
        if listened {
            r.send(&[key(PAD, COMP, Some(KICK))]);
        }
        r.send(&[on(KICK, 64), on(PAD, 60)]);
        let mut out = r.run(50);
        r.send(&[off(KICK, 64)]);
        out.extend(r.run(30));
        out
    };
    let (alone, listened) = (render(false), render(true));
    assert!(rms(&alone) > 0.05);
    assert_eq!(alone, listened);
}

#[test]
fn a_source_listening_to_another_renders_after_it() {
    // A chain of three: the pad (slot 0) listens to the bass (slot 2), which listens to the
    // kick (slot 1). The plan renders the kick, then the bass, then the pad.
    let mut r = Rig::new();
    let bass = TrackSlot(2);
    r.send(&[
        create(PAD),
        create(KICK),
        create(bass),
        tone_at(PAD, 0.02),
        comp(PAD, COMP),
        tone_at(KICK, 1.0),
        tone_at(bass, 1.0),
        comp(bass, COMP),
        mute(KICK),
        mute(bass),
        key(PAD, COMP, Some(bass)),
        key(bass, COMP, Some(KICK)),
        on(PAD, 60),
        on(bass, 48),
    ]);
    r.run(40);
    let bass_alone = r.reduction(bass, COMP);
    let pad_by_bass = r.reduction(PAD, COMP);
    assert_eq!(bass_alone, 0.0, "nothing keys the bass yet");
    assert!(pad_by_bass > 6.0, "the bass keys the pad: {pad_by_bass}");
    // The kick turns the bass down, which turns the pad down less.
    r.send(&[on(KICK, 64)]);
    r.run(40);
    assert!(r.reduction(bass, COMP) > 6.0);
    assert!(r.reduction(PAD, COMP) < pad_by_bass - 3.0);
}

#[test]
fn a_loop_the_project_never_sends_renders_every_track_once_and_never_hangs() {
    let mut r = Rig::new();
    pad_and_muted_kick(&mut r);
    r.send(&[
        comp(KICK, COMP),
        key(PAD, COMP, Some(KICK)),
        key(KICK, COMP, Some(PAD)),
        on(KICK, 64),
    ]);
    let out = r.run(40);
    assert!(rms(&out) > 0.0);
    // One of the two is rendered first and hears silence; the other hears it.
    let (pad, kick) = (r.reduction(PAD, COMP), r.reduction(KICK, COMP));
    assert!(pad > 6.0 || kick == 0.0, "{pad} {kick}");
}

#[test]
fn a_source_that_leaves_gives_silence_and_one_that_comes_gives_its_signal() {
    let mut r = Rig::new();
    pad_and_muted_kick(&mut r);
    r.send(&[key(PAD, COMP, Some(KICK)), on(KICK, 64)]);
    r.run(30);
    assert!(r.reduction(PAD, COMP) > 6.0);
    // Removed: its slot holds no track, the pad hears silence.
    r.send(&[M::TrackRemove { track: KICK }]);
    r.run(60);
    assert!(r.reduction(PAD, COMP) < 0.05, "{}", r.reduction(PAD, COMP));
    // A track put in that slot later is planned and heard (a `TrackPut`).
    r.send(&[create(KICK), tone_at(KICK, 1.0), mute(KICK), on(KICK, 64)]);
    r.run(30);
    assert!(r.reduction(PAD, COMP) > 6.0, "{}", r.reduction(PAD, COMP));
}

#[test]
fn every_change_of_a_chain_plans_the_sources_again() {
    let mut r = Rig::new();
    pad_and_muted_kick(&mut r);
    r.send(&[key(PAD, COMP, Some(KICK)), on(KICK, 64)]);
    r.run(30);
    assert!(r.reduction(PAD, COMP) > 6.0);
    // `DeviceSidechain` back to its own track.
    r.send(&[key(PAD, COMP, None)]);
    r.run(60);
    assert!(r.reduction(PAD, COMP) < 0.05);
    r.send(&[key(PAD, COMP, Some(KICK))]);
    r.run(30);
    assert!(r.reduction(PAD, COMP) > 6.0);
    // `DeviceMove`: the Compressor moves with its source.
    let to = DeviceSlot(5);
    r.send(&[M::DeviceMove {
        track: PAD,
        from: COMP,
        to,
    }]);
    r.run(30);
    assert!(r.reduction(PAD, to) > 6.0, "{}", r.reduction(PAD, to));
    // `DevicePut` (a device built again in its slot): a new device listens to its own track.
    r.send(&[comp(PAD, to)]);
    r.run(60);
    assert!(r.reduction(PAD, to) < 0.05, "{}", r.reduction(PAD, to));
    r.send(&[key(PAD, to, Some(KICK))]);
    r.run(30);
    assert!(r.reduction(PAD, to) > 6.0);
    // `DeviceRemove` of the only listener: the kick is no longer a source; nothing breaks.
    r.send(&[M::DeviceRemove {
        track: PAD,
        device: to,
    }]);
    let out = r.run(60);
    assert!(rms(&out) > 0.0);
    // `Reset`: a new project, nothing listens.
    r.send(&[M::Reset { epoch: 2 }]);
    r.run(10);
}

#[test]
fn a_rebuild_at_another_sample_rate_keeps_the_source() {
    let mut r = Rig::new();
    pad_and_muted_kick(&mut r);
    r.send(&[key(PAD, COMP, Some(KICK))]);
    r.run(10);
    r.p.set_sample_rate(engine::time::SampleRate::new(44_100).unwrap())
        .unwrap();
    r.p.flush();
    r.run(60);
    r.send(&[on(KICK, 64)]);
    r.run(30);
    assert!(r.reduction(PAD, COMP) > 6.0, "{}", r.reduction(PAD, COMP));
}

#[test]
fn sources_past_the_buffers_give_silence() {
    let mut r = Rig::new();
    // One listener per source, each on its own track: 2 × (MAX_KEY_SOURCES + 1) tracks.
    let n = MAX_KEY_SOURCES as u32 + 1;
    let mut msgs = Vec::new();
    for i in 0..n {
        let (listener, source) = (TrackSlot(2 * i), TrackSlot(2 * i + 1));
        msgs.extend([
            create(listener),
            create(source),
            tone_at(source, 1.0),
            mute(source),
            comp(listener, COMP),
            key(listener, COMP, Some(source)),
        ]);
    }
    r.send(&msgs);
    // At most `MAX_APPLY_PER_BLOCK` messages a block: the tracks are there before the notes,
    // which go through the live queue.
    r.run(10);
    r.send(
        &(0..n)
            .map(|i| on(TrackSlot(2 * i + 1), 64))
            .collect::<Vec<_>>(),
    );
    r.run(30);
    let heard = (0..n)
        .filter(|i| r.reduction(TrackSlot(2 * i), COMP) > 6.0)
        .count();
    assert_eq!(heard, MAX_KEY_SOURCES, "the first sources have a buffer");
    assert_eq!(r.reduction(TrackSlot(2 * (n - 1)), COMP), 0.0);
}

#[test]
fn prepare_refuses_what_takes_no_detection_source() {
    let mut r = Rig::new();
    r.send(&[create(PAD), tone_at(PAD, 0.5), comp(PAD, COMP)]);
    let bad = Err(MsgError::BadPayload {
        tag: Tag::DeviceSidechain as u16,
    });
    // The tone declares none.
    assert_eq!(
        r.p.prepare(key(PAD, DeviceSlot(0), Some(KICK))).map(|_| ()),
        bad
    );
    // No device in the slot.
    assert_eq!(
        r.p.prepare(key(PAD, DeviceSlot(7), Some(KICK))).map(|_| ()),
        Err(MsgError::NoSuchTarget)
    );
    // The master's chain, and a source past the tracks, are refused by the message's checks.
    assert!(
        r.p.prepare(key(TrackSlot::MASTER, DeviceSlot(0), Some(KICK)))
            .is_err()
    );
    assert!(
        r.p.prepare(key(
            PAD,
            COMP,
            Some(TrackSlot(engine::msg::limits::MAX_TRACKS))
        ))
        .is_err()
    );
    // Its own track is its own track.
    assert!(r.p.prepare(key(PAD, COMP, Some(PAD))).is_ok());
}
