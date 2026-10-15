//! Sample-accurate timed events (MO-4): offline renders through the real `Preparer`,
//! `Engine::apply` and `Engine::process`, every audio-side call under the allocation
//! detector. A probe instrument writes an impulse at each note-on and logs every event it
//! receives with its absolute frame, and every sub-block it renders; the expected frames
//! come from exact rational arithmetic (`tick × 60 × rate / (bpm × 960)`, floored), never
//! from the clock under test.

use std::cell::RefCell;
use std::collections::BTreeSet;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::testing::TRANSPOSE;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, Factory,
    ParamDescriptor, Scale, Unit, VoiceId,
};
use engine::engine::{Engine, Preparer, channel};
use engine::events::{
    NOTE_ROOM, PARAM_CHUNK, SCHEDULE_FIRST_VOICE, Schedule, ScheduledNote, ScheduledParam, Timed,
    TimedKind,
};
use engine::msg::{
    ControlMsg as M, DeviceKind, DeviceSlot, MsgError, ParamId, TrackKind, TrackSlot, try_box,
};
use engine::time::{LoopRegion, Quantize, Tempo, Tick, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

// ---------------------------------------------------------------- the probe

/// What a probe saw.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Rec {
    /// A sub-block: its first absolute frame, its frames, the transport it was given.
    Sub {
        who: u8,
        start: u64,
        frames: u32,
        position: f64,
        playing: bool,
    },
    /// An event, at its absolute frame (`start + offset`).
    Ev { who: u8, at: u64, event: Event },
}

thread_local! {
    static LOG: RefCell<Vec<Rec>> = const { RefCell::new(Vec::new()) };
}

const LEVEL: ParamId = ParamId(7);

static PROBE_PARAMS: [ParamDescriptor; 1] = [ParamDescriptor {
    id: LEVEL,
    key: "level",
    unit: Unit::None,
    min: 0.0,
    max: 1.0,
    default: 0.5,
    scale: Scale::Linear,
    step: 0.0,
    smoothing_seconds: 0.0,
    values: &[],
}];

static PROBE: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.test_probe",
    kind_id: DeviceKind(0xFFFF_00B1),
    version: 1,
    name: "Probe",
    role: DeviceRole::Instrument,
    memory_bytes: 64,
    midi_output: None,
    params: &PROBE_PARAMS,
};

static PROBE_FX: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.test_probe_fx",
    kind_id: DeviceKind(0xFFFF_00B2),
    name: "Probe FX",
    role: DeviceRole::Effect,
    ..PROBE
};

const INSTRUMENT: u8 = 0;
const EFFECT: u8 = 1;

/// Logs what it receives; the instrument writes 1.0 at each note-on, the effect passes the
/// sound through.
struct Probe {
    who: u8,
    done: u64,
}

impl Device for Probe {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        if self.who == INSTRUMENT {
            &PROBE
        } else {
            &PROBE_FX
        }
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = ctx.frames as usize;
        LOG.with_borrow_mut(|log| {
            if log.len() < log.capacity() {
                log.push(Rec::Sub {
                    who: self.who,
                    start: self.done,
                    frames: ctx.frames,
                    position: ctx.transport.position_ticks,
                    playing: ctx.transport.playing,
                });
            }
            let mut last = 0;
            for e in events {
                // CT-5: sorted, below the sub-block's length.
                assert!(e.offset() >= last, "unsorted events: {events:?}");
                assert!(
                    (e.offset() as usize) < frames,
                    "offset beyond the sub-block"
                );
                last = e.offset();
                if log.len() < log.capacity() {
                    log.push(Rec::Ev {
                        who: self.who,
                        at: self.done + u64::from(e.offset()),
                        event: *e,
                    });
                }
            }
        });
        if self.who == INSTRUMENT {
            for e in events {
                if let Event::NoteOn { offset, .. } = e {
                    io.left[*offset as usize] = 1.0;
                    io.right[*offset as usize] = 1.0;
                }
            }
        }
        self.done += frames as u64;
    }

    fn set_param(&mut self, _id: ParamId, _value: f32) {}

    fn reset(&mut self) {}
}

fn build_probe(_setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Probe {
        who: INSTRUMENT,
        done: 0,
    })?)
}

fn build_probe_fx(_setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    Ok(try_box(Probe {
        who: EFFECT,
        done: 0,
    })?)
}

fn resolver(kind: DeviceKind) -> Option<Factory> {
    if kind == PROBE.kind_id {
        Some(build_probe)
    } else if kind == PROBE_FX.kind_id {
        Some(build_probe_fx)
    } else {
        engine::device::testing::factory(kind)
    }
}

// ---------------------------------------------------------------- a rig

const TRACK: TrackSlot = TrackSlot(0);
const BPM: i128 = 120;

struct Rig {
    p: Preparer,
    e: Engine,
    rate: u32,
    /// Frames rendered so far.
    frames: u64,
    out: Vec<f32>,
}

impl Rig {
    /// A track with the probe instrument (slot 0), at `rate`, 120 BPM, after one stopped
    /// block of `first` frames.
    fn new(rate: u32, first: usize) -> Rig {
        let setup = DeviceSetup {
            sample_rate: rate as f32,
            max_block_frames: 2048,
        };
        let (p, e) = channel(setup, resolver).unwrap();
        LOG.with_borrow_mut(|log| {
            *log = Vec::with_capacity(400_000);
        });
        let mut rig = Rig {
            p,
            e,
            rate,
            frames: 0,
            out: Vec::new(),
        };
        rig.send(&[
            M::SetTransport {
                tempo: Tempo::new(BPM as f64).unwrap(),
                signature: TimeSignature::COMMON,
                launch_quantize: Quantize::default(),
            },
            M::TrackCreate {
                track: TRACK,
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TRACK,
                device: DeviceSlot(0),
                kind: PROBE.kind_id,
                state: Vec::new(),
            },
        ]);
        rig.run(first, first);
        rig
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p.handle(m.clone()).unwrap();
        }
        self.p.flush();
    }

    /// Puts a schedule on the track, off the audio path.
    fn schedule(&mut self, notes: &[ScheduledNote], params: &[ScheduledParam]) {
        let s = Schedule::try_new(notes, params, SCHEDULE_FIRST_VOICE).unwrap();
        let old = self
            .e
            .state_mut()
            .track_mut(TRACK)
            .unwrap()
            .graph
            .set_schedule(Some(Box::new(s)));
        drop(old);
    }

    /// Renders `frames` frames in blocks of `size`.
    fn run(&mut self, frames: usize, size: usize) {
        let (mut l, mut r) = (vec![0.0f32; size], vec![0.0f32; size]);
        let mut left = frames;
        while left > 0 {
            let n = left.min(size);
            assert_no_alloc(|| self.e.process(&mut l[..n], &mut r[..n]));
            assert_eq!(l[..n], r[..n]);
            self.out.extend_from_slice(&l[..n]);
            self.frames += n as u64;
            left -= n;
            self.p.collect();
        }
    }

    /// The absolute frame of `tick` when the clock started at tick `from` at frame `s0`:
    /// exact, floored.
    fn frame(&self, s0: u64, from: i64, tick: i64) -> u64 {
        let num = i128::from(tick - from) * 60 * i128::from(self.rate);
        let den = BPM * 960;
        s0 + num.div_euclid(den) as u64
    }
}

fn log() -> Vec<Rec> {
    LOG.with_borrow(|log| log.clone())
}

/// The events `who` received, without the release that inserting an instrument sends at
/// the first frame of its first block (MO-3).
fn events(who: u8) -> Vec<(u64, Event)> {
    log()
        .into_iter()
        .filter_map(|r| match r {
            Rec::Ev { who: w, at, event } if w == who => Some((at, event)),
            _ => None,
        })
        .filter(|(at, e)| !(*at == 0 && matches!(e, Event::AllNotesOff { .. })))
        .collect()
}

fn subs(who: u8) -> Vec<(u64, u32, f64, bool)> {
    log()
        .into_iter()
        .filter_map(|r| match r {
            Rec::Sub {
                who: w,
                start,
                frames,
                position,
                playing,
            } if w == who => Some((start, frames, position, playing)),
            _ => None,
        })
        .collect()
}

fn note(start: i64, length: i64) -> ScheduledNote {
    ScheduledNote {
        start: Tick(start),
        length: Tick(length),
        key: 60,
        velocity: 1.0,
        tune_cents: 0.0,
    }
}

fn voice(i: usize) -> VoiceId {
    VoiceId::new(SCHEDULE_FIRST_VOICE + i as u32).unwrap()
}

/// Every note-off ends a held voice, every note-on starts a free one, no `AllNotesOff`;
/// returns the voices still held.
fn check_voices(events: &[(u64, Event)]) -> BTreeSet<u32> {
    let mut held = BTreeSet::new();
    for (at, e) in events {
        match e {
            Event::NoteOn { voice, .. } => {
                assert!(
                    held.insert(voice.get()),
                    "voice {voice:?} started twice at {at}"
                )
            }
            Event::NoteOff { voice, .. } => {
                assert!(
                    held.remove(&voice.get()),
                    "voice {voice:?} ended unheld at {at}"
                )
            }
            Event::AllNotesOff { .. } => panic!("unexpected AllNotesOff at {at}"),
            Event::ParamValue { .. } => {}
        }
    }
    held
}

fn note_ons(events: &[(u64, Event)]) -> Vec<(u64, u32)> {
    events
        .iter()
        .filter_map(|(at, e)| match e {
            Event::NoteOn { voice, .. } => Some((*at, voice.get())),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------- onsets

#[test]
fn a_note_scheduled_at_sample_k_starts_at_k() {
    for rate in [48_000u32, 44_100] {
        for size in [64usize, 128, 512, 2048, 1, 7, 333] {
            let mut rig = Rig::new(rate, size);
            let s0 = rig.frames;
            // Notes on many offsets, block boundaries included (7 ticks apart).
            let notes: Vec<ScheduledNote> = (0..120).map(|i| note(30 + i * 7, 3)).collect();
            rig.schedule(&notes, &[]);
            rig.send(&[M::Play { from: Tick(0) }]);
            let end = rig.frame(s0, 0, 30 + 120 * 7) + 100;
            rig.run((end - s0) as usize, size);

            let expected: Vec<u64> = notes.iter().map(|n| rig.frame(s0, 0, n.start.0)).collect();
            // The signal: an impulse at each onset, nothing else.
            let onsets: Vec<u64> = rig
                .out
                .iter()
                .enumerate()
                .filter(|(_, s)| **s != 0.0)
                .map(|(i, _)| i as u64)
                .collect();
            assert_eq!(onsets, expected, "{rate} Hz, blocks of {size}");
            let ev = events(INSTRUMENT);
            let ons: Vec<u64> = note_ons(&ev).iter().map(|(at, _)| *at).collect();
            assert_eq!(ons, expected, "{rate} Hz, blocks of {size}");
            let offs: Vec<u64> = ev
                .iter()
                .filter(|(_, e)| matches!(e, Event::NoteOff { .. }))
                .map(|(at, _)| *at)
                .collect();
            let expected_offs: Vec<u64> = notes
                .iter()
                .map(|n| rig.frame(s0, 0, n.start.0 + 3))
                .collect();
            assert_eq!(offs, expected_offs, "{rate} Hz, blocks of {size}");
            assert!(check_voices(&ev).is_empty());
        }
    }
}

// ---------------------------------------------------------------- loops

/// The exact sample (as numerator over `den`) of `tick` in the `n`th pass of the loop
/// `[l0, l1)` when the clock started at `from` (before `l1`) at frame 0.
fn loop_sample(rate: u32, from: i64, l0: i64, l1: i64, n: i64, tick: i64) -> (i128, i128) {
    let r = |t: i64| i128::from(t) * 60 * i128::from(rate);
    let den = BPM * 960;
    if n == 0 {
        (r(tick - from), den)
    } else {
        (
            r(l1 - from) + r(l1 - l0) * i128::from(n - 1) + r(tick - l0),
            den,
        )
    }
}

fn floor(x: (i128, i128)) -> u64 {
    x.0.div_euclid(x.1) as u64
}

fn ceil(x: (i128, i128)) -> u64 {
    (x.0 + x.1 - 1).div_euclid(x.1) as u64
}

#[test]
fn loops_place_notes_at_the_same_offset_after_every_wrap() {
    let rate = 44_100;
    let (from, l0, l1) = (900i64, 960i64, 960 + 1001);
    for size in [128usize, 333, 2048] {
        let mut rig = Rig::new(rate, size);
        let s0 = rig.frames;
        // On the loop start, inside, and one held across the loop end.
        let notes = [note(l0, 50), note(1200, 30), note(l1 - 20, 100)];
        rig.schedule(&notes, &[]);
        rig.send(&[
            M::SetLoop {
                region: LoopRegion::new(Tick(l0), Tick(l1)),
            },
            M::Play { from: Tick(from) },
        ]);
        let passes = 5;
        let span = floor(loop_sample(rate, from, l0, l1, passes, l0)) + 10;
        rig.run(span as usize, size);
        let at = |x: (i128, i128)| s0 + x.0.div_euclid(x.1) as u64;
        let wrap = |n: i64| s0 + ceil(loop_sample(rate, from, l0, l1, n, l0));

        let ev = events(INSTRUMENT);
        let mut expected = Vec::new();
        for n in 0..passes {
            let mut pass = Vec::new();
            for (i, nt) in notes.iter().enumerate() {
                if n == 0 && nt.start.0 < from {
                    continue;
                }
                // The loop start is clamped into the first frame of its segment.
                let k = if n > 0 && nt.start.0 == l0 {
                    wrap(n)
                } else {
                    at(loop_sample(rate, from, l0, l1, n, nt.start.0))
                };
                pass.push((k, voice(i).get()));
            }
            pass.sort();
            expected.extend(pass);
        }
        let ons = note_ons(&ev);
        assert_eq!(ons[..expected.len()], expected[..], "blocks of {size}");

        // The note held across the loop end is released at each wrap, by its own note-off.
        for n in 1..passes {
            let w = wrap(n);
            assert!(
                ev.iter().any(|(a, e)| *a == w
                    && matches!(e, Event::NoteOff { voice: v, .. } if *v == voice(2))),
                "no release at wrap {n} ({w}), blocks of {size}"
            );
        }
        // A sub-block starts at every wrap, and none crosses one.
        let sb = subs(INSTRUMENT);
        for n in 1..=passes {
            let w = wrap(n);
            if w < rig.frames {
                assert!(sb.iter().any(|s| s.0 == w), "no sub-block at wrap {w}");
            }
        }
        // The position of each sub-block is the playhead at its first frame.
        for &(start, frames, position, playing) in &sb {
            if start < s0 + size as u64 {
                continue;
            }
            assert!(playing);
            assert!(frames > 0);
            let f = i128::from((start - s0) as i64) * BPM * 960;
            let r = 60 * i128::from(rate);
            // Which pass: the last wrap at or before `start`.
            let n = (1..=passes + 1)
                .take_while(|&n| wrap(n) <= start)
                .last()
                .unwrap_or(0);
            let tick = if n == 0 {
                from as f64 + f as f64 / r as f64
            } else {
                let w = loop_sample(rate, from, l0, l1, n, l0);
                l0 as f64
                    + (f as f64 * w.1 as f64 - w.0 as f64 * (BPM * 960) as f64)
                        / (w.1 as f64 * r as f64)
            };
            assert!(
                (position - tick).abs() < 1e-6,
                "position {position} at {start}, expected {tick}"
            );
        }

        // Stop: the held notes end at the first frame of the stopped block.
        let held_before = check_voices(&ev).len();
        let stop_at = rig.frames;
        rig.send(&[M::Stop]);
        rig.run(size, size);
        let ev = events(INSTRUMENT);
        assert!(check_voices(&ev).is_empty(), "a note stuck after the stop");
        let released = ev
            .iter()
            .filter(|(a, e)| *a == stop_at && matches!(e, Event::NoteOff { .. }))
            .count();
        assert_eq!(released, held_before);
        let last = *subs(INSTRUMENT).last().unwrap();
        assert!(!last.3, "a stopped block is not playing");
    }
}

// ---------------------------------------------------------------- 10 000 events

#[test]
fn ten_thousand_events_in_one_block_are_all_delivered() {
    let rate = 48_000;
    let mut rig = Rig::new(rate, 2048);
    let s0 = rig.frames;
    // 10 000 notes over 70 ticks (25 samples each) and 2 000 parameter values: one block.
    let notes: Vec<ScheduledNote> = (0..10_000).map(|i| note(90 + (i % 70), 1)).collect();
    let params: Vec<ScheduledParam> = (0..2_000)
        .map(|i| ScheduledParam {
            tick: Tick(90 + (i % 70)),
            device: DeviceSlot(0),
            id: LEVEL,
            value: i as f32,
        })
        .collect();
    rig.schedule(&notes, &params);
    rig.send(&[M::Play { from: Tick(0) }]);
    rig.run(2048 * 3, 2048);
    let block = (rig.frame(s0, 0, 90) / 2048, rig.frame(s0, 0, 160) / 2048);
    assert_eq!(block.0, block.1, "the events fit in one block");

    let ev = events(INSTRUMENT);
    let mut ons = 0;
    let mut offs = 0;
    let mut values = Vec::new();
    for (at, e) in &ev {
        match e {
            Event::NoteOn { voice, .. } => {
                let i = (voice.get() - SCHEDULE_FIRST_VOICE) as usize;
                assert_eq!(*at, rig.frame(s0, 0, notes[i].start.0));
                ons += 1;
            }
            Event::NoteOff { voice, .. } => {
                let i = (voice.get() - SCHEDULE_FIRST_VOICE) as usize;
                assert_eq!(*at, rig.frame(s0, 0, notes[i].start.0 + 1));
                offs += 1;
            }
            Event::ParamValue { value, .. } => {
                let i = *value as usize;
                assert_eq!(*at, rig.frame(s0, 0, params[i].tick.0));
                values.push(i);
            }
            Event::AllNotesOff { .. } => panic!("AllNotesOff"),
        }
    }
    assert_eq!((ons, offs), (10_000, 10_000));
    values.sort();
    assert_eq!(values, (0..2_000).collect::<Vec<_>>());
    assert!(check_voices(&ev).is_empty());
    // The block was cut in sub-blocks that tile it.
    let sb = subs(INSTRUMENT);
    assert!(sb.len() > 4);
    for w in sb.windows(2) {
        assert_eq!(
            w[0].0 + u64::from(w[0].1),
            w[1].0,
            "sub-blocks tile the blocks"
        );
    }
}

#[test]
fn a_group_larger_than_the_sink_on_one_sample_is_delivered_in_order() {
    let rate = 48_000;
    let mut rig = Rig::new(rate, 2048);
    let s0 = rig.frames;
    let notes: Vec<ScheduledNote> = (0..10_000).map(|_| note(100, 10)).collect();
    let params: Vec<ScheduledParam> = (0..2_000)
        .map(|i| ScheduledParam {
            tick: Tick(100),
            device: DeviceSlot(0),
            id: LEVEL,
            value: i as f32,
        })
        .collect();
    rig.schedule(&notes, &params);
    let table: Vec<Timed> = rig
        .e
        .state()
        .track(TRACK)
        .unwrap()
        .graph
        .schedule()
        .unwrap()
        .entries()
        .to_vec();
    rig.send(&[M::Play { from: Tick(0) }]);
    rig.run(2048 * 4, 2048);
    let k = rig.frame(s0, 0, 100);
    let ev = events(INSTRUMENT);
    // Every entry at tick 100, in the table's order, none lost; the sink takes
    // `PARAM_CHUNK` parameter values at k, the rest one frame at a time after.
    let got: Vec<(u64, Event)> = ev.iter().filter(|(a, _)| *a < k + 50).copied().collect();
    let want: Vec<&Timed> = table.iter().filter(|t| t.tick == Tick(100)).collect();
    assert_eq!(got.len(), want.len());
    for ((_, e), t) in got.iter().zip(&want) {
        match (e, t.kind) {
            (Event::ParamValue { value, .. }, TimedKind::Param { value: v, .. }) => {
                assert_eq!(*value, v)
            }
            (Event::NoteOn { voice, .. }, TimedKind::NoteOn { voice: v, .. }) => {
                assert_eq!(*voice, v)
            }
            other => panic!("out of order: {other:?}"),
        }
    }
    assert!(got.iter().all(|(a, _)| *a >= k && *a <= k + 3));
    assert_eq!(got.iter().filter(|(a, _)| *a == k).count(), PARAM_CHUNK);
    let ons_late = got
        .iter()
        .filter(|(a, e)| *a > k + 1 && matches!(e, Event::NoteOn { .. }))
        .count();
    assert_eq!(ons_late, 10_000 - NOTE_ROOM);
    assert!(check_voices(&ev).is_empty());
}

#[test]
fn a_group_cut_on_the_last_frame_before_a_seek_is_not_lost() {
    let rate = 48_000;
    let size = 128;
    let mut rig = Rig::new(rate, size);
    let s0 = rig.frames;
    // 25 × 87 + 128 = 2303: the last frame of a block.
    let tick = 87;
    let notes: Vec<ScheduledNote> = (0..6_000).map(|_| note(tick, 1_000)).collect();
    rig.schedule(&notes, &[]);
    rig.send(&[M::Play { from: Tick(0) }]);
    let k = rig.frame(s0, 0, tick);
    assert_eq!(k % size as u64, size as u64 - 1);
    rig.run((k + 1 - s0) as usize, size);
    rig.send(&[M::Seek { tick: Tick(2_000) }]);
    rig.run(size * 8, size);
    let ev = events(INSTRUMENT);
    let ons = note_ons(&ev);
    assert_eq!(ons.len(), 6_000, "no note-on lost");
    assert_eq!(ons.iter().filter(|(a, _)| *a == k).count(), NOTE_ROOM);
    assert!(ons.iter().all(|(a, _)| *a == k || *a == k + 1));
    // The seek released every one of them, each by its own note-off.
    assert!(check_voices(&ev).is_empty());
}

// ---------------------------------------------------------------- routing

#[test]
fn parameter_values_reach_only_their_device_before_the_notes() {
    let rate = 48_000;
    let mut rig = Rig::new(rate, 128);
    rig.send(&[M::DeviceInsert {
        track: TRACK,
        device: DeviceSlot(1),
        kind: PROBE_FX.kind_id,
        state: Vec::new(),
    }]);
    rig.run(128, 128);
    let s0 = rig.frames;
    let param = |device: u16, value: f32| ScheduledParam {
        tick: Tick(50),
        device: DeviceSlot(device),
        id: LEVEL,
        value,
    };
    rig.schedule(&[note(50, 10)], &[param(0, 0.25), param(1, 0.75)]);
    rig.send(&[M::Play { from: Tick(0) }]);
    rig.run(4096, 128);
    let k = rig.frame(s0, 0, 50);
    let inst = events(INSTRUMENT);
    let at_k: Vec<Event> = inst
        .iter()
        .filter(|(a, _)| *a == k)
        .map(|(_, e)| *e)
        .collect();
    assert_eq!(at_k.len(), 2);
    assert!(matches!(at_k[0], Event::ParamValue { value, .. } if value == 0.25));
    assert!(matches!(at_k[1], Event::NoteOn { .. }));
    let fx = events(EFFECT);
    assert_eq!(
        fx.len(),
        1,
        "the effect receives its value and no note: {fx:?}"
    );
    // The effect counts its frames from its first block, one block after the instrument.
    assert_eq!(fx[0].0 + 128, k);
    assert!(matches!(fx[0].1, Event::ParamValue { value, .. } if value == 0.75));
}

#[test]
fn a_jump_releases_scheduled_notes_only() {
    let rate = 48_000;
    let size = 128;
    let mut rig = Rig::new(rate, size);
    let s0 = rig.frames;
    rig.schedule(&[note(20, 2_000)], &[]);
    rig.send(&[M::Play { from: Tick(0) }]);
    rig.run(1024, size);
    // A live note, then a seek in the same block as another live note.
    rig.send(&[M::NoteOn {
        track: TRACK,
        pitch: 40,
        velocity: 100,
    }]);
    rig.run(size, size);
    let seek_at = rig.frames;
    rig.send(&[
        M::Seek { tick: Tick(10) },
        M::NoteOn {
            track: TRACK,
            pitch: 41,
            velocity: 100,
        },
    ]);
    rig.run(size * 30, size);
    let ev = events(INSTRUMENT);
    let live = |p: u8| VoiceId::new(u32::from(p) + 1).unwrap();
    // The scheduled note ended at the seek by its own note-off, the live ones did not.
    let at_seek: Vec<Event> = ev
        .iter()
        .filter(|(a, _)| *a == seek_at)
        .map(|(_, e)| *e)
        .collect();
    assert!(at_seek.contains(&Event::NoteOff {
        offset: 0,
        voice: voice(0),
        key: 60
    }));
    assert!(
        at_seek
            .iter()
            .any(|e| matches!(e, Event::NoteOn { voice: v, .. } if *v == live(41)))
    );
    let held = check_voices(&ev);
    assert!(held.contains(&live(40).get()) && held.contains(&live(41).get()));
    // After the seek the note started again from tick 20.
    let ons = note_ons(&ev);
    assert_eq!(
        ons.iter()
            .filter(|(_, v)| *v == voice(0).get())
            .map(|(a, _)| *a)
            .collect::<Vec<_>>(),
        vec![rig.frame(s0, 0, 20), rig.frame(seek_at, 10, 20)]
    );
}

#[test]
fn a_seek_drops_the_note_off_of_a_note_it_never_started() {
    let rate = 48_000;
    let size = 64;
    let mut rig = Rig::new(rate, size);
    rig.schedule(&[note(100, 200), note(400, 10)], &[]);
    // Into the middle of the first note: its note-off is never delivered.
    rig.send(&[M::Play { from: Tick(150) }]);
    rig.run(25 * 400, size);
    let ev = events(INSTRUMENT);
    assert!(check_voices(&ev).is_empty());
    let ons = note_ons(&ev);
    assert_eq!(ons.len(), 1);
    assert_eq!(ons[0].1, voice(1).get());
}

// ---------------------------------------------------------------- the table and sizing

#[test]
fn the_schedule_orders_values_then_ends_then_starts() {
    let notes = [note(10, 5), note(15, 5)];
    let params = [ScheduledParam {
        tick: Tick(15),
        device: DeviceSlot(0),
        id: LEVEL,
        value: 1.0,
    }];
    let s = Schedule::try_new(&notes, &params, SCHEDULE_FIRST_VOICE).unwrap();
    let kinds: Vec<(i64, u8)> = s
        .entries()
        .iter()
        .map(|t| {
            (
                t.tick.0,
                match t.kind {
                    TimedKind::Param { .. } => 0,
                    TimedKind::NoteOff { .. } => 1,
                    TimedKind::NoteOn { .. } => 2,
                },
            )
        })
        .collect();
    assert_eq!(kinds, vec![(10, 2), (15, 0), (15, 1), (15, 2), (20, 1)]);
    // A note of no length still ends, one tick later.
    let s = Schedule::try_new(&[note(10, 0)], &[], SCHEDULE_FIRST_VOICE).unwrap();
    assert_eq!(s.entries()[1].tick, Tick(11));
    // Voice ids are 1..=VoiceId::MAX.
    assert!(matches!(
        Schedule::try_new(&[note(0, 1)], &[], 0),
        Err(MsgError::LimitExceeded { .. })
    ));
    assert!(matches!(
        Schedule::try_new(&[note(0, 1), note(1, 1)], &[], VoiceId::MAX),
        Err(MsgError::LimitExceeded { .. })
    ));
    assert!(Schedule::try_new(&[note(0, 1)], &[], VoiceId::MAX).is_ok());
}

#[test]
fn a_midi_effect_after_another_is_sized_from_its_output() {
    let mut rig = Rig::new(48_000, 128);
    rig.send(&[
        M::TrackCreate {
            track: TrackSlot(1),
            kind: TrackKind::Midi,
        },
        M::DeviceInsert {
            track: TrackSlot(1),
            device: DeviceSlot(0),
            kind: TRANSPOSE.kind_id,
            state: Vec::new(),
        },
        M::DeviceInsert {
            track: TrackSlot(1),
            device: DeviceSlot(1),
            kind: TRANSPOSE.kind_id,
            state: Vec::new(),
        },
    ]);
    rig.run(128, 128);
    let bounds = TRANSPOSE.midi_output.unwrap();
    let cap = |slot: u16| {
        rig.e
            .state()
            .device(TrackSlot(1), DeviceSlot(slot))
            .unwrap()
            .events_out
            .as_ref()
            .unwrap()
            .capacity()
    };
    assert_eq!(cap(0), bounds.capacity(NOTE_ROOM));
    assert_eq!(cap(1), bounds.capacity(cap(0)));
}
