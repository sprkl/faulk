//! The arrangement in the engine (AR-2): the placements of a track played in step with the
//! transport, the rule of a start in the middle of a note, the region loop and the grid
//! taking a track over until `BackToArrangement`, in offline renders through the real
//! `Preparer`, `Engine::apply` and `Engine::process`, every audio-side call under the
//! allocation detector. A probe instrument logs every event it receives with its absolute
//! frame; the expected frames come from exact rational arithmetic
//! (`tick × 60 × rate / (bpm × 960)`, floored), never from the clock under test. The tests
//! that hold notes check that none is left hanging: each note-on is ended by its note-off
//! or an `AllNotesOff`, and no voice starts twice.

use std::cell::RefCell;
use std::collections::BTreeSet;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, Factory,
    ParamDescriptor,
};
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{
    ActiveClip, ClipSlot, ClipState, ControlMsg as M, DeviceKind, DeviceSlot, MsgError, Note,
    ParamId, Placement, PlacementSlot, SceneSlot, Span, TrackKind, TrackSlot, try_box,
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
    kind: "faulk.test_arrangement_probe",
    kind_id: DeviceKind(0xFFFF_00C3),
    version: 1,
    name: "Arrangement probe",
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

    /// The frame of `tick` when the clock started at tick 0 at frame 0: exact, floored.
    fn at(&self, tick: i64) -> u64 {
        self.frame(0, 0, tick)
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

fn launch_scene(scene: u16) -> M {
    M::LaunchScene {
        scene: SceneSlot(scene),
    }
}

fn stop_track(track: u32) -> M {
    M::StopTrack {
        track: TrackSlot(track),
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

/// The cells the engine reports as not idle.
fn grid(r: &Rig) -> Vec<(u32, u16, ClipState)> {
    let mut cells = Vec::new();
    r.e.active_clips(|c: ActiveClip| cells.push((c.track.0, c.scene.0, c.state)));
    cells
}

// ---------------------------------------------------------------- the arrangement

/// A placement of clip `clip` from tick `start`, reading `length` ticks of it from `offset`.
fn placed(slot: u32, clip: u32, start: i64, offset: i64, length: i64) -> Placement {
    Placement {
        placement: PlacementSlot(slot),
        clip: ClipSlot(clip),
        start: Tick(start),
        span: Span::Ticks {
            offset: Tick(offset),
            length: Tick(length),
        },
    }
}

/// The whole arrangement of a track, as the project sends it.
fn arrange(track: u32, placements: Vec<Placement>) -> M {
    M::TrackPlacementsUpload {
        track: TrackSlot(track),
        first: 0,
        total: placements.len() as u32,
        placements,
    }
}

fn play(from: i64) -> M {
    M::Play { from: Tick(from) }
}

/// Sixteen bars of 4/4: longer than any test plays.
const LONG: i64 = 16 * 3840;

/// A clip that holds one note for its whole length.
fn drone(clip: u32, ticks: i64, pitch: u8) -> M {
    upload(clip, ticks, None, vec![note(0, ticks, pitch)])
}

/// The grid step of `q` in ticks in 4/4, written out here (0: none).
fn step(q: Quantize) -> i64 {
    match q {
        Quantize::None => 0,
        Quantize::Sixteenth => 240,
        Quantize::Beat => 960,
        Quantize::Bar => 3840,
        Quantize::Bars2 => 2 * 3840,
        Quantize::Bars4 => 4 * 3840,
        Quantize::Bars8 => 8 * 3840,
    }
}

// ---------------------------------------------------------------- placements

fn exact_frames(rate: u32, size: usize) {
    let mut r = Rig::new(rate, 1, Quantize::Bar);
    r.send(&[
        upload(0, 1920, None, vec![note(0, 480, 60), note(960, 480, 62)]),
        arrange(0, vec![placed(0, 0, 960, 0, 1920)]),
        play(0),
    ]);
    r.run_to(r.at(3000), size);
    let why = format!("{rate} Hz, blocks of {size}");
    assert_eq!(ons(0), vec![(r.at(960), 60), (r.at(1920), 62)], "{why}");
    assert_eq!(offs(0), vec![(r.at(1440), 60), (r.at(2400), 62)], "{why}");
    assert_nothing_held(0);
    assert!(grid(&r).is_empty(), "the arrangement is not a grid clip");
}

#[test]
fn a_placement_plays_its_notes_on_their_exact_samples() {
    for rate in [48_000, 44_100] {
        for size in [1, 7, 64, 333, 2048] {
            exact_frames(rate, size);
        }
    }
}

#[test]
fn a_placement_plays_the_window_of_its_clip_that_its_offset_and_length_name() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(
            0,
            3840,
            None,
            vec![
                note(0, 100, 60),
                note(960, 100, 62),
                note(1500, 100, 64),
                note(1400, 400, 65),
            ],
        ),
        // The clip's ticks 480 to 1440, from tick 2000.
        arrange(0, vec![placed(0, 0, 2000, 480, 960)]),
        play(0),
    ]);
    r.run_to(r.at(5000), 128);
    // Clip tick 960 is transport tick 2480. The note at 1400 starts inside the window and
    // runs past its end: cut at the end of the placement, which is clip tick 1440.
    assert_eq!(ons(0), vec![(r.at(2480), 62), (r.at(2920), 65)]);
    assert_eq!(offs(0), vec![(r.at(2580), 62), (r.at(2960), 65)]);
    assert_nothing_held(0);
}

#[test]
fn the_end_of_a_placement_cuts_a_long_note_on_its_sample_and_starts_nothing_after() {
    for rate in [48_000, 44_100] {
        let mut r = Rig::new(rate, 1, Quantize::Bar);
        r.send(&[
            upload(
                0,
                3840,
                None,
                vec![note(0, 3000, 60), note(960, 100, 61), note(959, 100, 62)],
            ),
            arrange(0, vec![placed(0, 0, 0, 0, 960)]),
            play(0),
        ]);
        r.run_to(r.at(3000), 100);
        assert_eq!(ons(0), vec![(r.at(0), 60), (r.at(959), 62)]);
        // The note that ends at the clip's tick 1059 is cut at the placement's end too.
        assert_eq!(offs(0), vec![(r.at(960), 60), (r.at(960), 62)]);
        assert_nothing_held(0);
    }
}

#[test]
fn a_placement_longer_than_a_looped_clip_repeats_the_loop() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(
            0,
            960,
            Some((0, 480)),
            vec![note(0, 100, 60), note(600, 100, 61)],
        ),
        arrange(0, vec![placed(0, 0, 0, 0, 1920)]),
        play(0),
    ]);
    r.run_to(r.at(3000), 128);
    // The loop is the first 480 ticks: four passes in 1920, the note after it never plays.
    let expected: Vec<(u64, u8)> = (0..4).map(|i| (r.at(i * 480), 60)).collect();
    assert_eq!(ons(0), expected);
    assert_nothing_held(0);
}

#[test]
fn a_clip_without_loop_is_silent_after_its_end_inside_a_longer_placement() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(0, 960, None, vec![note(0, 100, 60)]),
        arrange(0, vec![placed(0, 0, 0, 0, 3840)]),
        play(0),
    ]);
    r.run_to(r.at(5000), 128);
    assert_eq!(ons(0), vec![(r.at(0), 60)]);
    assert_nothing_held(0);
}

#[test]
fn a_note_that_runs_past_the_end_of_a_clip_without_loop_ends_with_the_clip() {
    for size in [1, 128, 2048] {
        let mut r = Rig::new(48_000, 1, Quantize::Bar);
        // Raw messages: a project keeps its notes inside the clip.
        r.send(&[
            upload(0, 960, None, vec![note(500, 2000, 60), note(0, 100, 61)]),
            arrange(0, vec![placed(0, 0, 0, 0, 3840)]),
            play(0),
        ]);
        r.run_to(r.at(5000), size);
        assert_eq!(ons(0), vec![(r.at(0), 61), (r.at(500), 60)], "{size}");
        // 61 ends on its tick; 60 ends with the clip, not with the placement.
        assert_eq!(offs(0), vec![(r.at(100), 61), (r.at(960), 60)], "{size}");
        assert_nothing_held(0);
    }
}

#[test]
fn two_placements_of_the_same_clip_play_it_twice_with_a_gap_between() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(0, 960, None, vec![note(0, 500, 60), note(900, 60, 62)]),
        // The second after a gap: two linked clips.
        arrange(0, vec![placed(0, 0, 0, 0, 960), placed(1, 0, 1920, 0, 960)]),
        play(0),
    ]);
    r.run_to(r.at(4000), 128);
    assert_eq!(
        ons(0),
        vec![
            (r.at(0), 60),
            (r.at(900), 62),
            (r.at(1920), 60),
            (r.at(2820), 62)
        ]
    );
    assert_nothing_held(0);
}

#[test]
fn back_to_back_placements_hand_over_on_the_boundary_sample() {
    let mut r = Rig::new(44_100, 1, Quantize::Bar);
    r.send(&[
        drone(0, 960, 60),
        drone(1, 960, 70),
        arrange(0, vec![placed(0, 0, 0, 0, 960), placed(1, 1, 960, 0, 960)]),
        play(0),
    ]);
    r.run_to(r.at(2400), 77);
    let b = r.at(960);
    let at_b: Vec<(bool, u8)> = events(0)
        .iter()
        .filter(|e| e.0 == b)
        .map(|e| (e.1, e.3))
        .collect();
    assert_eq!(
        at_b,
        vec![(false, 60), (true, 70)],
        "the old note ends before the new one starts"
    );
    assert_eq!(offs(0), vec![(b, 60), (r.at(1920), 70)]);
    assert_nothing_held(0);
}

#[test]
fn a_later_placement_cuts_the_one_it_overlaps() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        drone(0, 1920, 60),
        upload(1, 960, None, vec![note(0, 100, 70)]),
        // The project never writes this (AR-1 trims); the engine does not trust that.
        arrange(0, vec![placed(1, 1, 480, 0, 960), placed(0, 0, 0, 0, 1920)]),
        play(0),
    ]);
    r.run_to(r.at(3000), 128);
    assert_eq!(ons(0), vec![(r.at(0), 60), (r.at(480), 70)]);
    assert_eq!(offs(0), vec![(r.at(480), 60), (r.at(580), 70)]);
    assert_nothing_held(0);
}

#[test]
fn a_table_in_any_order_with_any_value_never_panics_and_plays_what_is_sane() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(0, 960, None, vec![note(0, 100, 60)]),
        arrange(
            0,
            vec![
                placed(0, 0, i64::MAX - 5, 0, i64::MAX),
                placed(1, 0, 0, 0, 0),
                placed(2, 0, 960, -100, 960),
                placed(3, 0, i64::MIN, 0, i64::MAX),
                placed(4, 0, 100, i64::MAX, i64::MAX),
                placed(5, 0, 2000, 0, -5),
                placed(6, 7, 3000, 0, 100),
            ],
        ),
        play(0),
    ]);
    r.run_to(r.at(4000), 128);
    // The placement of the slot 3 starts in the far past and ends before the next ones;
    // the one that reads past its clip's end and the one of a clip that does not exist
    // play nothing.
    assert_eq!(ons(0), vec![(r.at(960), 60)]);
    assert_nothing_held(0);
    // Far from anything: a seek to the limits of the clock.
    for tick in [1i64 << 40, i64::MAX / 2, 0] {
        r.send(&[M::Seek { tick: Tick(tick) }]);
        r.run(2048, 512);
    }
    assert_nothing_held(0);
}

#[test]
fn a_clip_that_another_track_holds_is_not_played_by_a_second_track() {
    let mut r = Rig::new(48_000, 2, Quantize::Bar);
    r.send(&[
        upload(0, 960, None, vec![note(0, 100, 60)]),
        arrange(0, vec![placed(0, 0, 0, 0, 960)]),
        arrange(1, vec![placed(0, 0, 0, 0, 960)]),
        play(0),
    ]);
    r.run_to(r.at(2000), 128);
    // Tracks are read in slot order: the first one holds the clip for its placement.
    assert_eq!(ons(0).len(), 1);
    assert!(ons(1).is_empty());
    for t in 0..2 {
        assert_nothing_held(t);
    }
}

// ---------------------------------------------------------------- the start in the middle of a note

/// A clip of two bars with a long note and a short one after it, placed from tick 0.
fn long_note_project(r: &mut Rig) {
    r.send(&[
        upload(0, 3840, None, vec![note(0, 1920, 60), note(2400, 100, 62)]),
        arrange(0, vec![placed(0, 0, 0, 0, 3840)]),
    ]);
}

#[test]
fn a_seek_into_a_long_note_neither_restarts_it_nor_leaves_it_held() {
    for size in [1, 128, 2048] {
        let mut r = Rig::new(48_000, 1, Quantize::Bar);
        long_note_project(&mut r);
        r.send(&[play(0)]);
        r.run_to(r.at(480), size);
        r.send(&[M::Seek { tick: Tick(960) }]);
        let at = r.frames;
        r.run_to(at + 100_000, size);
        // The long note started once, on its tick; the seek ended it on the first sample
        // and its own note-off (tick 1920) was dropped; the note after it is exact.
        assert_eq!(
            ons(0),
            vec![(0, 60), (at + 36_000, 62)],
            "blocks of {size}: not late, not twice"
        );
        assert_eq!(
            offs(0),
            vec![(at, 60), (at + 38_500, 62)],
            "blocks of {size}"
        );
        assert_nothing_held(0);
    }
}

#[test]
fn playing_from_the_middle_of_a_note_does_not_start_it() {
    for rate in [48_000, 44_100] {
        let mut r = Rig::new(rate, 1, Quantize::Bar);
        long_note_project(&mut r);
        r.send(&[play(960)]);
        r.run(120_000, 100);
        let f = |tick: i64| r.frame(0, 960, tick);
        assert_eq!(ons(0), vec![(f(2400), 62)], "{rate} Hz");
        assert_eq!(offs(0), vec![(f(2500), 62)]);
        assert_nothing_held(0);
    }
}

#[test]
fn a_note_that_starts_on_the_first_tick_of_a_seek_starts_on_the_first_sample() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    long_note_project(&mut r);
    r.send(&[play(0)]);
    r.run_to(r.at(480), 64);
    r.send(&[M::Seek { tick: Tick(2400) }]);
    let at = r.frames;
    r.run_to(at + 20_000, 64);
    assert_eq!(ons(0), vec![(0, 60), (at, 62)]);
    // One tick earlier: one tick (25 samples) later.
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    long_note_project(&mut r);
    r.send(&[play(2399)]);
    r.run(20_000, 64);
    assert_eq!(ons(0), vec![(25, 62)]);
    assert_nothing_held(0);
}

#[test]
fn a_seek_out_of_a_placement_ends_its_notes_and_a_seek_back_starts_it_again() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(0, 960, None, vec![note(0, 900, 60), note(480, 100, 61)]),
        arrange(0, vec![placed(0, 0, 0, 0, 960)]),
        play(0),
    ]);
    r.run_to(r.at(300), 128);
    r.send(&[M::Seek { tick: Tick(5000) }]);
    let gone = r.frames;
    r.run(10_000, 128);
    assert_eq!(offs(0), vec![(gone, 60)]);
    r.send(&[M::Seek { tick: Tick(0) }]);
    let back = r.frames;
    r.run(60_000, 128);
    assert_eq!(
        ons(0),
        vec![(0, 60), (back, 60), (back + 12_000, 61)],
        "a seek back to the start of the note starts it"
    );
    assert_nothing_held(0);
}

#[test]
fn a_stop_ends_the_notes_and_a_play_starts_the_arrangement_again() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    long_note_project(&mut r);
    r.send(&[play(0)]);
    r.run_to(r.at(700), 128);
    r.send(&[M::Stop]);
    let stopped = r.frames;
    r.run(10_000, 128);
    assert_eq!(offs(0), vec![(stopped, 60)]);
    assert_nothing_held(0);
    r.send(&[play(0)]);
    let again = r.frames;
    r.run(100_000, 128);
    assert_eq!(ons(0), vec![(0, 60), (again, 60), (again + 60_000, 62)]);
    assert_nothing_held(0);
}

// ---------------------------------------------------------------- the region loop

#[test]
fn a_region_loop_plays_the_arrangement_again_each_pass_and_cuts_what_crosses_the_wrap() {
    for rate in [48_000, 44_100] {
        let mut r = Rig::new(rate, 1, Quantize::Bar);
        r.send(&[
            upload(
                0,
                3840,
                None,
                vec![note(0, 100, 60), note(1000, 1200, 62), note(1500, 100, 63)],
            ),
            arrange(0, vec![placed(0, 0, 0, 0, 3840)]),
            M::SetLoop {
                region: Some(LoopRegion::new(Tick(0), Tick(1920)).unwrap()),
            },
            play(0),
        ]);
        // 1920 ticks are exactly one second at 120 BPM: three and a bit passes.
        let pass = i64::from(rate);
        r.run(3 * i64::from(rate) as u64 + 5_000, 128);
        let at = |p: i64, tick: i64| (p * pass) as u64 + r.at(tick);
        let mut on = Vec::new();
        let mut off = Vec::new();
        for p in 0..3 {
            on.extend([(at(p, 0), 60), (at(p, 1000), 62), (at(p, 1500), 63)]);
            off.extend([(at(p, 100), 60), (at(p, 1600), 63), (at(p + 1, 0), 62)]);
        }
        on.push((at(3, 0), 60));
        off.push((at(3, 100), 60));
        on.sort_unstable();
        off.sort_unstable();
        let mut got_on = ons(0);
        got_on.sort_unstable();
        let mut got_off = offs(0);
        got_off.sort_unstable();
        assert_eq!(got_on, on, "{rate} Hz");
        assert_eq!(got_off, off, "{rate} Hz");
        r.send(&[M::Stop]);
        r.run(256, 128);
        assert_nothing_held(0);
    }
}

#[test]
fn a_note_that_crosses_the_start_of_the_loop_region_is_not_started_by_the_wrap() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(0, 3840, None, vec![note(0, 960, 64), note(600, 100, 65)]),
        arrange(0, vec![placed(0, 0, 0, 0, 3840)]),
        M::SetLoop {
            region: Some(LoopRegion::new(Tick(480), Tick(1920)).unwrap()),
        },
        play(0),
    ]);
    r.run(3 * 72_000, 128);
    let key = |k: u8| ons(0).iter().filter(|e| e.1 == k).count();
    assert_eq!(key(64), 1, "started once, from the start of the clip");
    assert!(key(65) >= 3, "the note inside the region plays each pass");
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(0);
}

// ---------------------------------------------------------------- the table and the clip change while it plays

fn playing_a_drone_of_two_clips(r: &mut Rig) {
    r.send(&[
        drone(0, 3840, 60),
        drone(1, 3840, 70),
        arrange(0, vec![placed(0, 0, 0, 0, 3840)]),
        play(0),
    ]);
    r.run_to(r.at(500), 128);
    assert_eq!(ons(0), vec![(0, 60)]);
}

#[test]
fn a_table_that_puts_another_clip_under_the_playhead_ends_the_note_and_does_not_start_the_new_one()
{
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    playing_a_drone_of_two_clips(&mut r);
    r.send(&[arrange(0, vec![placed(0, 1, 0, 0, 3840)])]);
    let at = r.frames;
    r.run_to(r.at(3000), 128);
    assert_eq!(offs(0), vec![(at, 60)]);
    assert_eq!(
        ons(0),
        vec![(0, 60)],
        "its note started before the playhead"
    );
    assert_nothing_held(0);
}

#[test]
fn a_table_without_the_placement_or_with_it_moved_or_cut_ends_its_notes() {
    let edits: [(&str, Vec<Placement>); 4] = [
        ("emptied", vec![]),
        ("cut before the playhead", vec![placed(0, 0, 0, 0, 100)]),
        ("moved", vec![placed(0, 0, 200, 0, 3840)]),
        ("another slot", vec![placed(5, 0, 0, 0, 3840)]),
    ];
    for (what, table) in edits {
        let mut r = Rig::new(48_000, 1, Quantize::Bar);
        playing_a_drone_of_two_clips(&mut r);
        r.send(&[arrange(0, table)]);
        let at = r.frames;
        r.run_to(r.at(3000), 128);
        assert_eq!(offs(0), vec![(at, 60)], "{what}");
        assert_eq!(ons(0), vec![(0, 60)], "{what}: no second start");
        assert_nothing_held(0);
    }
}

#[test]
fn a_table_that_lengthens_the_placement_goes_on_without_a_break() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(0, 3840, None, vec![note(0, 3000, 60), note(2500, 100, 61)]),
        arrange(0, vec![placed(0, 0, 0, 0, 1920)]),
        play(0),
    ]);
    r.run_to(r.at(500), 128);
    r.send(&[arrange(0, vec![placed(0, 0, 0, 0, 3840)])]);
    r.run_to(r.at(5000), 128);
    // The note goes on and ends on its own tick; nothing was cut or started again.
    assert_eq!(ons(0), vec![(0, 60), (r.at(2500), 61)]);
    assert_eq!(offs(0), vec![(r.at(2600), 61), (r.at(3000), 60)]);
    assert_nothing_held(0);
}

#[test]
fn a_clip_replaced_while_it_plays_goes_on_in_its_new_table_without_a_second_start() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    long_note_project(&mut r);
    r.send(&[play(0)]);
    r.run_to(r.at(500), 128);
    // The same clip with another note after the one that sounds.
    r.send(&[upload(
        0,
        3840,
        None,
        vec![note(0, 1920, 60), note(2400, 100, 62), note(3000, 100, 63)],
    )]);
    r.run_to(r.at(5000), 128);
    let started_60 = ons(0).iter().filter(|e| e.1 == 60).count();
    assert_eq!(started_60, 1, "no second start of the note that sounds");
    assert!(ons(0).contains(&(r.at(2400), 62)));
    assert!(ons(0).contains(&(r.at(3000), 63)));
    assert_nothing_held(0);
}

#[test]
fn a_clip_removed_while_it_plays_stops_the_placement_and_frees_the_track() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    long_note_project(&mut r);
    r.send(&[play(0)]);
    r.run_to(r.at(500), 128);
    r.send(&[M::ClipRemove { clip: ClipSlot(0) }]);
    r.run_to(r.at(5000), 128);
    assert_eq!(ons(0), vec![(0, 60)]);
    assert_nothing_held(0);
}

// ---------------------------------------------------------------- the sink fills

fn crowd(notes: usize, start: i64, length: i64) -> Vec<Note> {
    (0..notes)
        .map(|i| note(start, length, (i % 128) as u8))
        .collect()
}

#[test]
fn six_thousand_notes_that_start_and_end_together_are_all_delivered_and_all_ended() {
    for size in [1, 64, 2048] {
        let mut r = Rig::new(48_000, 1, Quantize::Bar);
        r.send(&[
            upload(0, 3840, None, crowd(6000, 0, 3000)),
            // The notes start with the placement and end with it.
            arrange(0, vec![placed(0, 0, 960, 0, 960)]),
            play(0),
        ]);
        r.run_to(r.at(1920) + 1000, size);
        let (start, end) = (r.at(960), r.at(1920));
        let on: Vec<u64> = ons(0).iter().map(|e| e.0).collect();
        let off: Vec<u64> = offs(0).iter().map(|e| e.0).collect();
        assert_eq!(on.len(), 6000, "blocks of {size}");
        assert_eq!(off.len(), 6000, "blocks of {size}");
        assert!(
            on.iter().all(|&f| f >= start && f < start + 16),
            "blocks of {size}: every start within the first samples"
        );
        assert!(
            off.iter().all(|&f| f >= end && f < end + 16),
            "blocks of {size}: every end within the first samples of the end"
        );
        assert_nothing_held(0);
    }
}

// ---------------------------------------------------------------- the grid takes the track

/// Track 0 plays the drone of clip 0 from the arrangement; the grid has the drone of
/// clip 1 in scene 0 and a clip with two notes in scene 1 (clip 2).
fn takeover_project(r: &mut Rig) {
    r.send(&[
        upload(
            0,
            LONG,
            None,
            vec![
                note(0, 3000, 60),
                note(7000, 100, 61),
                note(10_000, 100, 62),
            ],
        ),
        drone(1, LONG, 70),
        arrange(0, vec![placed(0, 0, 0, 0, LONG)]),
        slot(0, 0, 1),
    ]);
}

#[test]
fn a_launch_lets_the_arrangement_play_up_to_the_boundary_for_every_quantization() {
    for q in Quantize::ALL {
        let mut r = Rig::new(48_000, 1, q);
        takeover_project(&mut r);
        // A bar and 992 ticks: off every grid, an exact frame.
        let launched = 3840 + 992;
        r.send(&[play(0)]);
        r.run_to(r.at(launched), 128);
        r.send(&[launch(0, 0)]);
        let g = step(q);
        let boundary = if g == 0 {
            launched
        } else {
            (launched / g + 1) * g
        };
        r.run_to(r.at(boundary + 960), 128);
        let b = r.at(boundary);
        // The note of the arrangement (tick 0 to 3000) ends on its tick when the boundary
        // is after it; the arrangement played until the boundary, not from the launch.
        let off60 = offs(0).iter().find(|e| e.1 == 60).copied();
        assert_eq!(
            off60,
            Some((r.at(3000), 60)),
            "{q:?}: note ends on its tick"
        );
        // The notes of the timeline before the boundary play, on their ticks.
        let mut expected: Vec<(u64, u8)> = [(0, 60), (7000, 61), (10_000, 62)]
            .iter()
            .filter(|(tick, _)| *tick < boundary)
            .map(|&(tick, key)| (r.at(tick), key))
            .collect();
        expected.push((b, 70));
        assert_eq!(ons(0), expected, "{q:?}");
        let at_b: Vec<(bool, u8)> = events(0)
            .iter()
            .filter(|e| e.0 == b)
            .map(|e| (e.1, e.3))
            .collect();
        assert_eq!(at_b, vec![(true, 70)], "{q:?}");
        assert_eq!(grid(&r), vec![(0, 0, ClipState::Playing)], "{q:?}");
        assert_eq!(held(0), BTreeSet::from([129]), "{q:?}: the grid clip holds");
    }
}

#[test]
fn a_launch_ends_the_notes_the_arrangement_holds_on_the_boundary_sample() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        drone(0, LONG, 60),
        drone(1, LONG, 70),
        arrange(0, vec![placed(0, 0, 0, 0, LONG)]),
        slot(0, 0, 1),
        play(0),
    ]);
    r.run_to(r.at(4000), 128);
    // The launch comes in the bar: the drone goes on until the next bar line.
    assert!(offs(0).is_empty());
    r.send(&[launch(0, 0)]);
    r.run_to(r.at(3 * 3840), 128);
    let b = r.at(2 * 3840);
    let at_b: Vec<(bool, u8)> = events(0)
        .iter()
        .filter(|e| e.0 == b)
        .map(|e| (e.1, e.3))
        .collect();
    assert_eq!(at_b, vec![(false, 60), (true, 70)]);
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(0);
}

#[test]
fn a_stop_from_the_grid_silences_the_arrangement_until_back_to_arrangement() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        upload(
            0,
            LONG,
            None,
            vec![
                note(0, 5000, 60),
                note(7000, 100, 61),
                note(10_000, 100, 62),
            ],
        ),
        arrange(0, vec![placed(0, 0, 0, 0, LONG)]),
        play(0),
    ]);
    r.run_to(r.at(1000), 128);
    r.send(&[stop_track(0)]);
    r.run_to(r.at(8000), 128);
    // The note at 7000 is on the timeline and the grid holds the track: silent.
    assert_eq!(ons(0), vec![(0, 60)]);
    assert_eq!(
        offs(0),
        vec![(r.at(3840), 60)],
        "the stop ran at the bar line"
    );
    // The grid holds the track without a clip: reported as held, in no cell.
    assert_eq!(grid(&r), vec![(0, u16::MAX, ClipState::Held)]);
    // Back to the arrangement in the middle of nothing: the next note plays on its tick.
    r.send(&[M::BackToArrangement]);
    r.run_to(r.at(11_000), 128);
    assert_eq!(ons(0), vec![(0, 60), (r.at(10_000), 62)]);
    assert!(
        grid(&r).is_empty(),
        "a track that follows its arrangement is not reported"
    );
    assert_nothing_held(0);
}

#[test]
fn back_to_arrangement_ends_the_grid_clip_at_once_and_the_arrangement_resumes_by_the_rule() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        // A note of the arrangement that sounds when the grid gives the track back, and
        // one that comes later.
        upload(0, LONG, None, vec![note(0, 9000, 60), note(8000, 100, 61)]),
        drone(1, LONG, 70),
        arrange(0, vec![placed(0, 0, 0, 0, LONG)]),
        slot(0, 0, 1),
        play(0),
    ]);
    r.run_to(r.at(500), 128);
    r.send(&[launch(0, 0)]);
    r.run_to(r.at(5000), 128);
    assert_eq!(ons(0), vec![(0, 60), (r.at(3840), 70)]);
    r.send(&[M::BackToArrangement]);
    let at = r.frames;
    r.run_to(r.at(9500), 128);
    // The grid clip ended on the first sample of the block; the note of the arrangement
    // that was sounding is not started late (nor again); the next one is on its tick.
    assert_eq!(offs(0), vec![(r.at(3840), 60), (at, 70), (r.at(8100), 61)]);
    assert_eq!(ons(0), vec![(0, 60), (r.at(3840), 70), (r.at(8000), 61)]);
    assert!(grid(&r).is_empty());
    assert_nothing_held(0);
}

#[test]
fn a_launch_given_after_back_to_arrangement_in_the_same_block_is_taken() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    takeover_project(&mut r);
    r.send(&[launch(0, 0), play(0)]);
    r.run_to(r.at(5000), 128);
    assert_eq!(ons(0), vec![(0, 70)], "the grid launched before play");
    r.send(&[M::BackToArrangement, launch(0, 0)]);
    r.run_to(r.at(3 * 3840), 128);
    // The arrangement came back at once (its notes before this point are the grid's to
    // skip) and the launch ran at the next bar line.
    // The note of the timeline at tick 7000 is before the bar line: it plays.
    assert_eq!(
        ons(0),
        vec![(0, 70), (r.at(7000), 61), (r.at(2 * 3840), 70)]
    );
    assert_eq!(offs(0).len(), 2);
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(0);
}

#[test]
fn the_grid_keeps_the_track_after_a_stop_of_the_transport_until_back_to_arrangement() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    takeover_project(&mut r);
    r.send(&[play(0)]);
    r.run_to(r.at(1000), 128);
    r.send(&[stop_track(0)]);
    r.run_to(r.at(4000), 128);
    r.send(&[M::Stop]);
    r.run(2_000, 128);
    r.send(&[play(0)]);
    r.run(3 * 48_000, 128);
    assert_eq!(ons(0), vec![(0, 60)], "silent on the second pass");
    r.send(&[M::BackToArrangement]);
    r.run(3 * 48_000, 128);
    assert!(ons(0).len() >= 2, "the arrangement plays again");
    assert_nothing_held(0);
}

#[test]
fn back_to_arrangement_while_the_transport_is_stopped_gives_the_track_back() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    takeover_project(&mut r);
    r.send(&[stop_track(0), play(0)]);
    r.run(2_000, 128);
    r.send(&[M::Stop]);
    r.run(2_000, 128);
    // Stopped: the order comes, nothing plays; the arrangement is the track's again.
    r.send(&[M::BackToArrangement]);
    r.run(2_000, 128);
    let again = r.frames;
    r.send(&[play(0)]);
    r.run(100_000, 128);
    assert_eq!(ons(0), vec![(again, 60)], "nothing before the return");
    assert!(!r.e.state().track(TrackSlot(0)).unwrap().player.taken());
    assert_nothing_held(0);
}

#[test]
fn back_track_to_arrangement_gives_back_one_track_and_leaves_the_others_held() {
    let mut r = Rig::new(48_000, 2, Quantize::None);
    r.send(&[
        upload(
            0,
            LONG,
            None,
            vec![
                note(0, 3000, 60),
                note(7000, 100, 61),
                note(10_000, 100, 62),
            ],
        ),
        drone(1, LONG, 70),
        slot(0, 0, 0),
        slot(1, 0, 1),
        arrange(0, vec![placed(0, 0, 0, 0, LONG)]),
        arrange(1, vec![placed(1, 1, 0, 0, LONG)]),
        play(0),
    ]);
    r.run_to(r.at(500), 128);
    r.send(&[launch_scene(0)]);
    r.run_to(r.at(1000), 128);
    r.send(&[M::StopAll]);
    r.run_to(r.at(1500), 128);
    assert_eq!(
        grid(&r),
        vec![
            (0, u16::MAX, ClipState::Held),
            (1, u16::MAX, ClipState::Held)
        ]
    );
    let before = ons(1).len();
    r.send(&[M::BackTrackToArrangement {
        track: TrackSlot(0),
    }]);
    r.run_to(r.at(2000), 128);
    assert_eq!(grid(&r), vec![(1, u16::MAX, ClipState::Held)]);
    assert!(!r.e.state().track(TrackSlot(0)).unwrap().player.taken());
    assert!(r.e.state().track(TrackSlot(1)).unwrap().player.taken());
    // A track that follows its arrangement is left alone.
    r.send(&[M::BackTrackToArrangement {
        track: TrackSlot(0),
    }]);
    r.run_to(r.at(2500), 128);
    assert_eq!(grid(&r), vec![(1, u16::MAX, ClipState::Held)]);
    // The signal: track 0's timeline plays again on its own ticks, track 1 stays silent.
    r.run_to(r.at(11_000), 128);
    let on0 = ons(0);
    assert_eq!(on0[on0.len() - 2..], [(r.at(7000), 61), (r.at(10_000), 62)]);
    assert_eq!(
        ons(1).len(),
        before,
        "track 1 played its grid clip and nothing since"
    );
    assert_nothing_held(0);
    assert_nothing_held(1);
}

#[test]
fn a_scene_launch_and_stop_all_leave_a_track_that_follows_its_arrangement_alone() {
    let mut r = Rig::new(48_000, 2, Quantize::None);
    r.send(&[
        drone(0, LONG, 60),
        drone(1, LONG, 70),
        arrange(0, vec![placed(0, 0, 0, 0, LONG)]),
        slot(1, 0, 1),
        play(0),
    ]);
    r.run_to(r.at(500), 128);
    r.send(&[launch_scene(0)]);
    r.run_to(r.at(1000), 128);
    r.send(&[M::StopAll]);
    r.run_to(r.at(1500), 128);
    assert_eq!(ons(0), vec![(0, 60)]);
    assert!(offs(0).is_empty(), "the arrangement never stopped");
    assert_eq!(ons(1).len(), 1, "the grid track played its scene");
    assert_eq!(offs(1).len(), 1, "and stopped with StopAll");
    // Track 1 is held by the grid; track 0 follows its arrangement and is not reported.
    assert_eq!(grid(&r), vec![(1, u16::MAX, ClipState::Held)]);
}

/// The same clip in a grid slot and in a placement of one track, 6 000 notes held: the
/// takeover and the way back release them all before anything starts, the sink filling
/// meanwhile.
#[test]
fn the_same_clip_in_the_arrangement_and_in_the_grid_never_sticks_a_note() {
    for size in [1, 128, 2048] {
        let mut r = Rig::new(48_000, 1, Quantize::Bar);
        r.send(&[
            upload(0, LONG, None, crowd(6000, 0, 40_000)),
            arrange(0, vec![placed(0, 0, 0, 0, LONG)]),
            slot(0, 0, 0),
            play(0),
        ]);
        r.run_to(r.at(1000), size);
        assert_eq!(ons(0).len(), 6000);
        r.send(&[launch(0, 0)]);
        r.run_to(r.at(3840 + 1000), size);
        // At the bar line the 6 000 notes of the timeline end, then the grid's start.
        let b = r.at(3840);
        let first_on_after = events(0)
            .iter()
            .filter(|e| e.1 && e.0 >= b)
            .map(|e| e.0)
            .min();
        let last_off_before = offs(0).iter().map(|e| e.0).max();
        assert_eq!(offs(0).len(), 6000, "blocks of {size}");
        assert!(
            last_off_before <= first_on_after,
            "blocks of {size}: the release is complete before the clip starts"
        );
        assert_eq!(ons(0).len(), 12_000, "blocks of {size}");
        r.send(&[M::BackToArrangement]);
        r.run(2_000, size);
        assert_eq!(offs(0).len(), 12_000, "blocks of {size}: the grid's end");
        // The timeline's notes started before the playhead are not started.
        assert_eq!(ons(0).len(), 12_000, "blocks of {size}");
        assert_nothing_held(0);
        r.send(&[M::Stop]);
        r.run(256, size);
        assert_nothing_held(0);
    }
}

// ---------------------------------------------------------------- a random session

/// A small deterministic generator: the sessions below replay from their seed.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }

    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }
}

fn random_clip(rng: &mut Lcg, clip: u32) -> M {
    let length = rng.range(480, 7680);
    let notes = (0..rng.below(40))
        .map(|_| {
            let start = rng.range(0, length - 1);
            note(
                start,
                rng.range(1, 3000.min(length - start)),
                rng.range(0, 127) as u8,
            )
        })
        .collect();
    let looping = (rng.below(3) == 0).then(|| {
        let a = rng.range(0, length - 2);
        (a, rng.range(a + 1, length))
    });
    upload(clip, length, looping, notes)
}

/// Random edits, transport moves, launches and returns to the arrangement, in blocks of
/// random sizes, under the allocation detector: no panic, and once everything is stopped no
/// note is left held on either track and no voice has started twice.
fn random_session(seed: u64) {
    let mut rng = Lcg(seed);
    let q = Quantize::ALL[rng.below(7) as usize];
    let mut r = Rig::new(48_000, 2, q);
    let mut exists = [false; 5];
    for step in 0..100 {
        let t = rng.below(2) as u32;
        let msg = match rng.below(15) {
            0 | 1 => {
                let clip = rng.below(5) as u32;
                exists[clip as usize] = true;
                random_clip(&mut rng, clip)
            }
            2 => {
                let clip = rng.below(5) as usize;
                if !exists[clip] {
                    continue;
                }
                exists[clip] = false;
                M::ClipRemove {
                    clip: ClipSlot(clip as u32),
                }
            }
            3..=5 => {
                let table = (0..rng.below(6))
                    .map(|i| {
                        placed(
                            i as u32,
                            rng.below(5) as u32,
                            rng.range(0, 12_000),
                            rng.range(-100, 3000),
                            rng.range(-10, 6000),
                        )
                    })
                    .collect();
                arrange(t, table)
            }
            6 => M::Play {
                from: Tick(rng.range(0, 8000)),
            },
            7 => M::Stop,
            8 => M::Seek {
                tick: Tick(rng.range(0, 12_000)),
            },
            9 => M::SetLoop {
                region: (rng.below(2) == 0).then(|| {
                    let a = rng.range(0, 6000);
                    LoopRegion::new(Tick(a), Tick(rng.range(a + 1, 9000))).unwrap()
                }),
            },
            10 => slot(t, rng.below(3) as u16, rng.below(5) as u32),
            11 => launch(t, rng.below(3) as u16),
            12 => {
                if rng.below(2) == 0 {
                    stop_track(t)
                } else {
                    M::StopAll
                }
            }
            13 => M::BackToArrangement,
            _ => launch_scene(rng.below(3) as u16),
        };
        // A slot may name a clip that was never uploaded: the engine ignores it.
        r.send(&[msg]);
        let size = [1, 3, 64, 100, 333, 2048][rng.below(6) as usize];
        r.run(rng.range(1, 30_000) as u64, size);
        if step == 0 {
            r.send(&[M::Play { from: Tick(0) }]);
        }
    }
    r.send(&[M::Stop, M::BackToArrangement]);
    r.run(4_000, 128);
    for t in 0..2 {
        assert_nothing_held(t);
    }
}

#[test]
fn random_sessions_never_panic_allocate_or_leave_a_note_held() {
    for seed in 0..12 {
        random_session(seed);
    }
}
