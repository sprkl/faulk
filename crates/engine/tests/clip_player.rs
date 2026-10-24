//! The MIDI clip player (CL-2): offline renders through the real `Preparer`,
//! `Engine::apply` and `Engine::process`, every audio-side call under the allocation
//! detector. A probe instrument writes an impulse at each note-on and logs every event it
//! receives with its absolute frame; the expected frames come from exact rational
//! arithmetic (`tick × 60 × rate / (bpm × 960)`, floored), never from the clock under test.
//! The tests that hold notes check that none is left hanging: each note-on is ended by its
//! note-off or an `AllNotesOff`, and no voice starts twice.

use std::cell::RefCell;
use std::collections::BTreeSet;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, Factory,
    ParamDescriptor, VoiceId,
};
use engine::engine::{Engine, Preparer, channel};
use engine::events::{SCHEDULE_CHUNK, SCHEDULE_FIRST_VOICE};
use engine::msg::{
    ClipSlot, ControlMsg as M, DeviceKind, DeviceSlot, MsgError, Note, ParamId, SceneSlot,
    TrackKind, TrackSlot, try_box,
};
use engine::time::{LoopRegion, Quantize, Tempo, Tick, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

// ---------------------------------------------------------------- the probe

thread_local! {
    /// (track, absolute frame, event).
    static LOG: RefCell<Vec<(u32, u64, Event)>> = const { RefCell::new(Vec::new()) };
}

static NO_PARAMS: [ParamDescriptor; 0] = [];

static PROBE: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.test_clip_probe",
    kind_id: DeviceKind(0xFFFF_00C1),
    version: 1,
    name: "Clip probe",
    role: DeviceRole::Instrument,
    memory_bytes: 64,
    midi_output: None,
    named_keys: &[],
    params: &NO_PARAMS,
};

struct Probe {
    track: u32,
    done: u64,
}

thread_local! {
    /// The track of the next probe built (each test builds on its own thread).
    static NEXT_TRACK: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

impl Device for Probe {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &PROBE
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        let frames = ctx.frames as usize;
        LOG.with_borrow_mut(|log| {
            let mut last = 0;
            for e in events {
                assert!(e.offset() >= last, "unsorted events: {events:?}");
                assert!(
                    (e.offset() as usize) < frames,
                    "offset beyond the sub-block"
                );
                last = e.offset();
                if log.len() < log.capacity() {
                    log.push((self.track, self.done + u64::from(e.offset()), *e));
                }
            }
        });
        for e in events {
            if let Event::NoteOn { offset, .. } = e {
                io.left[*offset as usize] = 1.0;
                io.right[*offset as usize] = 1.0;
            }
        }
        self.done += frames as u64;
    }

    fn set_param(&mut self, _id: ParamId, _value: f32) {}

    fn reset(&mut self) {}
}

fn build_probe(_setup: &DeviceSetup) -> Result<Box<dyn Device>, MsgError> {
    // Probes are built in track order by the tests (prepare side, not the audio path).
    let track = NEXT_TRACK.get();
    NEXT_TRACK.set(track + 1);
    Ok(try_box(Probe { track, done: 0 })?)
}

fn resolver(kind: DeviceKind) -> Option<Factory> {
    (kind == PROBE.kind_id).then_some(build_probe as Factory)
}

// ---------------------------------------------------------------- a rig

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
    /// `tracks` tracks with a probe each, at `rate`, 120 BPM in 4/4, with `quantize`.
    fn new(rate: u32, tracks: u32, quantize: Quantize) -> Rig {
        let setup = DeviceSetup {
            sample_rate: rate as f32,
            max_block_frames: 2048,
        };
        let (p, e) = channel(setup, resolver).unwrap();
        LOG.with_borrow_mut(|log| *log = Vec::with_capacity(400_000));
        NEXT_TRACK.set(0);
        let mut rig = Rig {
            p,
            e,
            rate,
            frames: 0,
            out: Vec::new(),
        };
        rig.send(&[M::SetTransport {
            tempo: Tempo::new(BPM as f64).unwrap(),
            signature: TimeSignature::COMMON,
            launch_quantize: quantize,
        }]);
        for t in 0..tracks {
            rig.send(&[
                M::TrackCreate {
                    track: TrackSlot(t),
                    kind: TrackKind::Midi,
                },
                M::DeviceInsert {
                    track: TrackSlot(t),
                    device: DeviceSlot(0),
                    kind: PROBE.kind_id,
                    state: Vec::new(),
                },
            ]);
        }
        rig
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p.handle(m.clone()).unwrap();
        }
        self.p.flush();
    }

    /// Renders `frames` frames in blocks of `size`.
    fn run(&mut self, frames: u64, size: usize) {
        let (mut l, mut r) = (vec![0.0f32; size], vec![0.0f32; size]);
        let mut left = frames as usize;
        while left > 0 {
            let n = left.min(size);
            assert_no_alloc(|| self.e.process(&mut l[..n], &mut r[..n]));
            self.out.extend_from_slice(&l[..n]);
            self.frames += n as u64;
            left -= n;
            self.p.collect();
        }
    }

    /// Renders up to absolute frame `to`.
    fn run_to(&mut self, to: u64, size: usize) {
        let n = to.saturating_sub(self.frames);
        self.run(n, size);
    }

    /// The frame of `tick` when the clock started at tick `from` at frame `s0`: exact,
    /// floored.
    fn frame(&self, s0: u64, from: i64, tick: i64) -> u64 {
        let num = i128::from(tick - from) * 60 * i128::from(self.rate);
        s0 + num.div_euclid(BPM * 960) as u64
    }
}

fn note(start: i64, length: i64, pitch: u8) -> Note {
    Note {
        start: Tick(start),
        length: Tick(length),
        pitch,
        velocity: 100,
        ..Note::default()
    }
}

fn upload(clip: u32, length: i64, looping: Option<(i64, i64)>, notes: Vec<Note>) -> M {
    M::ClipNotesUpload {
        clip: ClipSlot(clip),
        length: Tick(length),
        looping: looping.map(|(s, e)| LoopRegion::new(Tick(s), Tick(e)).unwrap()),
        first: 0,
        total: notes.len() as u32,
        notes,
    }
}

fn slot(track: u32, scene: u16, clip: u32) -> M {
    M::SceneSetSlot {
        track: TrackSlot(track),
        scene: SceneSlot(scene),
        clip: Some(ClipSlot(clip)),
    }
}

fn launch(track: u32, scene: u16) -> M {
    M::LaunchClip {
        track: TrackSlot(track),
        scene: SceneSlot(scene),
    }
}

/// What a track's probe received: (frame, on?, voice, key, tune), all-notes-off as
/// (frame, false, 0, 255, 0).
type Ev = (u64, bool, u32, u8, i32);

fn events(track: u32) -> Vec<Ev> {
    LOG.with_borrow(|log| {
        log.iter()
            .filter(|(t, _, _)| *t == track)
            .filter_map(|&(_, at, e)| match e {
                Event::NoteOn {
                    voice,
                    key,
                    tune_cents,
                    ..
                } => Some((at, true, voice.get(), key, tune_cents as i32)),
                Event::NoteOff { voice, key, .. } => Some((at, false, voice.get(), key, 0)),
                // The release that inserting an instrument sends at its first block.
                Event::AllNotesOff { .. } if at == 0 => None,
                Event::AllNotesOff { .. } => Some((at, false, 0, 255, 0)),
                _ => None,
            })
            .collect()
    })
}

fn ons(track: u32) -> Vec<(u64, u8)> {
    events(track)
        .into_iter()
        .filter(|e| e.1)
        .map(|e| (e.0, e.3))
        .collect()
}

fn offs(track: u32) -> Vec<(u64, u8)> {
    events(track)
        .into_iter()
        .filter(|e| !e.1 && e.3 != 255)
        .map(|e| (e.0, e.3))
        .collect()
}

/// Every note-on is ended (its note-off or an `AllNotesOff`) and no note-off ends a
/// note that is not held; returns the voices still held.
fn held(track: u32) -> BTreeSet<u32> {
    let mut held = BTreeSet::new();
    for (at, on, voice, key, _) in events(track) {
        if on {
            assert!(held.insert(voice), "voice {voice} started twice at {at}");
        } else if key == 255 {
            held.clear();
        } else {
            assert!(
                held.remove(&voice),
                "note-off of a free voice {voice} at {at}"
            );
        }
    }
    held
}

fn assert_nothing_held(track: u32) {
    let h = held(track);
    assert!(h.is_empty(), "stuck notes on track {track}: {h:?}");
}

/// Plays clip 0 on track 0 from scene 0, launched before `Play` from 0.
fn playing(rig: &mut Rig, clip: M) {
    rig.send(&[clip, slot(0, 0, 0), launch(0, 0), M::Play { from: Tick(0) }]);
}

// ---------------------------------------------------------------- placement and loop

/// One bar, looped over the whole clip: notes at 0, 1000 (with fine tuning) and 3500 (to
/// the clip's end).
fn bar_clip() -> M {
    let mut tuned = note(1000, 300, 64);
    tuned.cents = 25;
    upload(
        0,
        3840,
        Some((0, 3840)),
        vec![note(0, 480, 60), tuned, note(3500, 340, 67)],
    )
}

fn bar_clip_events(rate: u32, size: usize) -> (Vec<Ev>, Rig) {
    let mut r = Rig::new(rate, 1, Quantize::None);
    playing(&mut r, bar_clip());
    let end = r.frame(0, 0, 4 * 3840 + 100);
    r.run_to(end, size);
    (events(0), r)
}

#[test]
fn a_looped_clip_plays_each_note_at_its_exact_sample_every_pass() {
    for rate in [48_000, 44_100] {
        let (ev, r) = bar_clip_events(rate, 128);
        let f = |t: i64| r.frame(0, 0, t);
        let mut want = Vec::new();
        for k in 0..4 {
            let b = k * 3840;
            let v = |i: u32| SCHEDULE_FIRST_VOICE + i;
            // The note-off at the loop end comes from the wrap's release, on its sample.
            want.push((f(b), true, v(0), 60, 0));
            want.push((f(b + 480), false, v(0), 60, 0));
            want.push((f(b + 1000), true, v(1), 64, 25));
            want.push((f(b + 1300), false, v(1), 64, 0));
            want.push((f(b + 3500), true, v(2), 67, 0));
            want.push((f(b + 3840), false, v(2), 67, 0));
        }
        want.push((f(4 * 3840), true, SCHEDULE_FIRST_VOICE, 60, 0));
        assert_eq!(ev, want, "at {rate} Hz");
        // The impulses are on the same samples.
        let impulses: Vec<u64> = r
            .out
            .iter()
            .enumerate()
            .filter(|(_, s)| **s != 0.0)
            .map(|(i, _)| i as u64)
            .collect();
        // The first one is under the new track's fade in (MO-3).
        let starts: Vec<u64> = want.iter().filter(|e| e.1).map(|e| e.0).skip(1).collect();
        let impulses: Vec<u64> = impulses.into_iter().filter(|&i| i > 0).collect();
        assert_eq!(impulses, starts);
    }
}

#[test]
fn the_block_size_changes_nothing() {
    let (reference, _) = bar_clip_events(44_100, 128);
    for size in [1, 7, 2048] {
        let (ev, _) = bar_clip_events(44_100, size);
        assert_eq!(ev, reference, "blocks of {size}");
        held(0);
    }
}

#[test]
fn a_loop_region_after_an_intro_cuts_the_note_that_crosses_its_end() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    // Intro note at 0; loop [960, 2880); a note from 2500 to 3500 crosses the loop end.
    playing(
        &mut r,
        upload(
            0,
            3840,
            Some((960, 2880)),
            vec![note(0, 100, 50), note(1000, 100, 60), note(2500, 1000, 70)],
        ),
    );
    let end = r.frame(0, 0, 2880 + 3 * 1920 + 10);
    r.run_to(end, 128);
    let f = |t: i64| r.frame(0, 0, t);
    let mut want_on = vec![(f(0), 50)];
    let mut want_off = vec![(f(100), 50)];
    for k in 0..4 {
        // Pass k of the loop starts at transport tick 960 + k × 1920.
        let b = k * 1920;
        want_on.push((f(b + 1000), 60));
        want_off.push((f(b + 1100), 60));
        want_on.push((f(b + 2500), 70));
        // Cut at the wrap, not at 3500.
        want_off.push((f(b + 2880), 70));
    }
    let end_tick = 2880 + 3 * 1920 + 10;
    want_on.retain(|(at, _)| *at < f(end_tick));
    want_off.retain(|(at, _)| *at < f(end_tick));
    assert_eq!(ons(0), want_on);
    assert_eq!(offs(0), want_off);
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(0);
}

#[test]
fn a_clip_played_once_stops_after_its_end() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    // A raw upload may carry a note beyond the clip's length: released at the end.
    playing(
        &mut r,
        upload(0, 1920, None, vec![note(0, 100, 60), note(1800, 1000, 62)]),
    );
    r.run_to(r.frame(0, 0, 8000), 128);
    let f = |t: i64| r.frame(0, 0, t);
    assert_eq!(ons(0), vec![(f(0), 60), (f(1800), 62)]);
    // The note past the end is released on the sample of the clip's last tick.
    assert_eq!(offs(0), vec![(f(100), 60), (f(1920), 62)]);
    assert!(
        r.e.state()
            .track(TrackSlot(0))
            .unwrap()
            .player
            .playing()
            .is_none()
    );
    assert_nothing_held(0);
}

// ---------------------------------------------------------------- launch quantization

#[test]
fn a_launch_starts_at_the_boundary_of_each_quantization() {
    for q in Quantize::ALL {
        let mut r = Rig::new(48_000, 1, q);
        r.send(&[
            upload(0, 960, Some((0, 960)), vec![note(0, 10, 60)]),
            slot(0, 0, 0),
            M::Play { from: Tick(0) },
        ]);
        // 120 BPM at 48 kHz: 25 frames a tick; the order is applied at tick 1000.
        r.run(25_000, 100);
        r.send(&[launch(0, 0)]);
        r.run(25 * 32_000, 2048);
        let boundary = q
            .next_boundary_at_or_after(Tick(1000), TimeSignature::COMMON)
            .unwrap()
            .0;
        let first = ons(0)[0].0;
        assert_eq!(first, r.frame(0, 0, boundary), "{q:?}");
        r.send(&[M::Stop]);
        r.run(256, 128);
        assert_nothing_held(0);
    }
}

#[test]
fn a_launch_while_stopped_starts_at_the_first_boundary_after_play() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(0, 960, Some((0, 960)), vec![note(0, 10, 60)]),
        slot(0, 0, 0),
    ]);
    r.send(&[launch(0, 0)]);
    r.run(1000, 128);
    assert!(ons(0).is_empty());
    let s0 = r.frames;
    r.send(&[M::Play { from: Tick(100) }]);
    r.run(25 * 4000, 128);
    assert_eq!(ons(0)[0].0, r.frame(s0, 100, 3840));
}

#[test]
fn a_launch_in_a_loop_without_boundary_runs_at_the_wrap() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    // The transport loop [960, 2880) holds no bar line.
    r.send(&[
        upload(0, 960, Some((0, 960)), vec![note(0, 10, 60)]),
        slot(0, 0, 0),
        M::SetLoop {
            region: LoopRegion::new(Tick(960), Tick(2880)),
        },
        M::Play { from: Tick(1000) },
        launch(0, 0),
    ]);
    r.run(25 * 4000, 128);
    // The first wrap: 1880 ticks after the play.
    assert_eq!(ons(0)[0].0, r.frame(0, 1000, 2880));
    // Then every 960 ticks, the clip's loop, in step with the transport loop.
    assert_eq!(ons(0)[1].0, r.frame(0, 1000, 2880 + 960));
}

#[test]
fn a_second_clip_replaces_the_first_at_the_boundary_without_stuck_notes() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        // A: a note held over the whole bar; B: a note at its start.
        upload(
            0,
            3840,
            Some((0, 3840)),
            vec![note(0, 3840, 60), note(100, 5000, 61)],
        ),
        upload(1, 3840, Some((0, 3840)), vec![note(0, 480, 72)]),
        slot(0, 0, 0),
        slot(0, 1, 1),
        launch(0, 0),
        M::Play { from: Tick(0) },
    ]);
    r.run(25 * 1000, 128);
    r.send(&[launch(0, 1)]);
    r.run(25 * 5000, 128);
    let f = |t: i64| r.frame(0, 0, t);
    let ev = events(0);
    // At the boundary: A's two notes end, then B starts, on the same sample.
    let at: Vec<Ev> = ev.iter().copied().filter(|e| e.0 == f(3840)).collect();
    assert_eq!(at.len(), 3, "{at:?}");
    assert!(!at[0].1 && !at[1].1 && at[2].1 && at[2].3 == 72);
    assert_eq!(ons(0).last().unwrap().1, 72);
    r.send(&[M::StopTrack {
        track: TrackSlot(0),
    }]);
    r.run(25 * 8000, 128);
    assert_nothing_held(0);
    // The stop runs at the next bar: B does not start again there.
    assert_eq!(ons(0).last().copied(), Some((r.frame(0, 0, 3840), 72)));
}

// ---------------------------------------------------------------- no stuck note

#[test]
fn a_transport_stop_releases_the_clip() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(0, 3840, Some((0, 3840)), vec![note(0, 3000, 60)]),
    );
    r.run(25 * 1000 + 3, 128);
    r.send(&[M::Stop]);
    let stop = r.frames;
    r.run(1024, 128);
    assert_eq!(offs(0), vec![(stop, 60)]);
    assert_nothing_held(0);
    // Playing again does not bring the clip back: a stop stops it.
    r.send(&[M::Play { from: Tick(0) }]);
    r.run(25 * 4000, 128);
    assert_eq!(ons(0).len(), 1);
}

#[test]
fn a_seek_and_a_transport_wrap_release_and_keep_the_clip_in_step() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(0, 960, Some((0, 960)), vec![note(0, 900, 60)]),
    );
    r.run(25 * 500, 128);
    // A seek into the middle of the note: released, not restarted (MO-4's rule).
    r.send(&[M::Seek { tick: Tick(1300) }]);
    let s0 = r.frames;
    r.run(25 * 1000, 128);
    assert_eq!(offs(0)[0], (s0, 60));
    assert_eq!(ons(0)[1].0, r.frame(s0, 1300, 1920));
    r.send(&[M::SetLoop {
        region: LoopRegion::new(Tick(0), Tick(2400)),
    }]);
    r.run(25 * 5000, 128);
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(0);
}

#[test]
fn a_clip_removed_or_replaced_while_it_plays_leaves_no_stuck_note() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(0, 3840, Some((0, 3840)), vec![note(0, 3000, 60)]),
    );
    r.run(25 * 500, 128);
    // Replaced: the old table's note is ended by a release, the new one plays on.
    r.send(&[upload(0, 3840, Some((0, 3840)), vec![note(1000, 100, 62)])]);
    let at = r.frames;
    r.run(25 * 1000, 128);
    assert!(events(0).contains(&(at, false, 0, 255, 0)), "released");
    assert_eq!(ons(0).last().unwrap().1, 62);
    assert_nothing_held(0);
    let clip = r.e.state().clip(ClipSlot(0)).unwrap();
    assert_eq!(clip.on_track, Some(TrackSlot(0)));
    // Removed mid-note: released, the clip stops.
    r.run_to(r.frame(0, 0, 3840 + 1050), 128);
    assert_eq!(held(0).len(), 1);
    r.send(&[M::ClipRemove { clip: ClipSlot(0) }]);
    r.run(25 * 8000, 128);
    assert_nothing_held(0);
    assert!(
        r.e.state()
            .track(TrackSlot(0))
            .unwrap()
            .player
            .playing()
            .is_none()
    );
}

#[test]
fn a_track_replaced_while_it_plays_gives_its_clip_back() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(0, 960, Some((0, 960)), vec![note(0, 900, 60)]),
    );
    r.run(25 * 100, 128);
    // A new track in the same slot, then a second track: both may launch the clip once it
    // is free again.
    r.send(&[
        M::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Midi,
        },
        M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(0),
            kind: PROBE.kind_id,
            state: Vec::new(),
        },
        slot(0, 0, 0),
        launch(0, 0),
    ]);
    r.run(25 * 2000, 128);
    // Probe 1 is the new track's.
    assert_eq!(ons(1).len(), 3, "{:?}", events(1));
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(1);
}

#[test]
fn a_clip_plays_on_one_track_at_a_time() {
    let mut r = Rig::new(48_000, 2, Quantize::None);
    r.send(&[
        upload(0, 960, Some((0, 960)), vec![note(0, 10, 60)]),
        slot(0, 0, 0),
        slot(1, 0, 0),
        launch(0, 0),
        launch(1, 0),
        M::Play { from: Tick(0) },
    ]);
    r.run(25 * 2000, 128);
    assert_eq!(ons(0).len(), 3);
    assert!(ons(1).is_empty(), "the second track stays silent");
}

// ---------------------------------------------------------------- overflow

#[test]
fn more_notes_on_one_sample_than_the_sink_holds_are_all_delivered_at_each_wrap() {
    let n = 3 * SCHEDULE_CHUNK + 17;
    // At 48 kHz a pass of 960 ticks is 24 000 frames.
    let notes: Vec<Note> = (0..n).map(|i| note(0, 900, (i % 128) as u8)).collect();
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, upload(0, 960, Some((0, 960)), notes));
    // A first block of one frame: every wrap (24 000 k) is then the last frame of a block.
    r.run(1, 1);
    r.run(25 * 960 * 3 + 10, 125);
    let ev = events(0);
    let on = ev.iter().filter(|e| e.1).count();
    assert_eq!(on, 4 * n, "every note-on of 4 passes, once");
    held(0);
    // Switching to another clip at a boundary while that many are held: every release
    // reaches the device before the new clip's notes of the same voices.
    r.send(&[
        upload(1, 960, Some((0, 960)), vec![note(0, 10, 99)]),
        slot(0, 1, 1),
    ]);
    r.send(&[launch(0, 1)]);
    r.run(25 * 960 * 2, 125);
    r.send(&[M::Stop]);
    r.run(256, 125);
    assert_nothing_held(0);
}

#[test]
fn chunked_upload_and_voice_ids_of_a_clip_never_cross_the_live_ones() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    let notes: Vec<Note> = (0..10).map(|i| note(i * 10, 5, 60)).collect();
    let mut first = upload(0, 960, Some((0, 960)), notes[..4].to_vec());
    let mut second = upload(0, 960, Some((0, 960)), notes[4..].to_vec());
    if let M::ClipNotesUpload { total, .. } = &mut first {
        *total = 10;
    }
    if let M::ClipNotesUpload {
        first: f, total, ..
    } = &mut second
    {
        *f = 4;
        *total = 10;
    }
    r.send(&[
        first,
        second,
        slot(0, 0, 0),
        launch(0, 0),
        M::Play { from: Tick(0) },
    ]);
    r.run(25 * 960, 128);
    let voices: BTreeSet<u32> = events(0).iter().map(|e| e.2).collect();
    assert_eq!(voices.len(), 10);
    assert!(
        voices
            .iter()
            .all(|&v| v >= SCHEDULE_FIRST_VOICE && VoiceId::new(v).is_some())
    );
}

#[test]
fn an_unfinished_upload_leaves_the_playing_table_alone() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(0, 960, Some((0, 960)), vec![note(0, 10, 60)]),
    );
    r.run(25 * 100, 128);
    // The first of two chunks of a new table: nothing changes until the last one.
    let mut first = upload(0, 960, Some((0, 960)), vec![note(500, 10, 70)]);
    if let M::ClipNotesUpload { total, .. } = &mut first {
        *total = 2;
    }
    r.send(&[first]);
    r.run(25 * 2000, 128);
    assert!(ons(0).iter().all(|(_, key)| *key == 60), "{:?}", ons(0));
    assert_eq!(ons(0).len(), 3);
    // An out-of-order chunk is refused and the old table still plays.
    let mut stray = upload(0, 960, Some((0, 960)), vec![note(500, 10, 70)]);
    if let M::ClipNotesUpload { first, total, .. } = &mut stray {
        *first = 5;
        *total = 9;
    }
    r.send(&[stray]);
    r.run(25 * 1000, 128);
    assert!(ons(0).iter().all(|(_, key)| *key == 60));
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(0);
}

#[test]
fn a_release_larger_than_the_sink_in_the_middle_of_a_block_stays_in_its_sub_blocks() {
    // 6 000 notes held across the loop end (beyond the sink's room for a sub-block), the
    // wrap in the middle of a block, after other notes of the same block.
    let mut notes: Vec<Note> = (0..6000).map(|i| note(100, 800, (i % 128) as u8)).collect();
    notes.push(note(880, 10, 1));
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, upload(0, 960, Some((0, 900)), notes));
    // The wrap at tick 900 is frame 22 500, frame 100 of a block of 128 (the probe
    // asserts every offset is below its sub-block's length).
    r.run(25 * 900 * 3, 128);
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(0);
    let offs = offs(0);
    let at_wrap = offs
        .iter()
        .filter(|(at, _)| *at >= 22_500 && *at < 22_500 + 10)
        .count();
    assert_eq!(
        at_wrap, 6000,
        "every held note released at the wrap (one frame late at most)"
    );
}

#[test]
fn a_transport_loop_that_wraps_before_the_launch_point_keeps_the_clip_in_phase() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    r.send(&[
        upload(0, 960, Some((0, 960)), vec![note(0, 10, 60)]),
        slot(0, 0, 0),
        M::SetLoop {
            region: LoopRegion::new(Tick(0), Tick(3840)),
        },
        M::Play { from: Tick(0) },
    ]);
    r.run(25 * 1000, 100);
    // Launched at tick 1000: the clip's notes are at 1000 + 960 k, also before 1000 once
    // the transport loop wraps back to 0: at 40 (= 1000 − 960).
    r.send(&[launch(0, 0)]);
    r.run(25 * (3840 + 1100), 100);
    let ons: Vec<u64> = ons(0).into_iter().map(|(at, _)| at).collect();
    let pass = 25 * 3840;
    assert_eq!(
        ons,
        [
            25_000,
            25 * 1960,
            25 * 2920,
            pass + 25 * 40,
            pass + 25_000,
            pass + 25 * 1960
        ]
    );
    r.send(&[M::Stop]);
    r.run(256, 100);
    assert_nothing_held(0);
}
