//! The launch grid in the engine (GR-1): scenes, `StopAll`, `BackToArrangement` and the
//! clip states of the `Feedback`, in offline renders through the real `Preparer`,
//! `Engine::apply` and `Engine::process`, every audio-side call under the allocation
//! detector. A probe instrument logs every event it receives with its absolute frame; the
//! expected frames come from exact rational arithmetic (`tick × 60 × rate / (bpm × 960)`,
//! floored) and from a grid computed here, never from the clock under test. The tests that
//! hold notes check that none is left hanging: each note-on is ended by its note-off or an
//! `AllNotesOff`, and no voice starts twice.

use std::cell::RefCell;
use std::collections::BTreeSet;

use assert_no_alloc::assert_no_alloc;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, Factory,
    ParamDescriptor,
};
use engine::engine::{Engine, Preparer, channel};
use engine::events::SCHEDULE_CHUNK;
use engine::msg::{
    ActiveClip, ClipSlot, ClipState, ControlMsg as M, DeviceKind, DeviceSlot, MsgError, Note,
    ParamId, SceneSlot, SourceSlot, TrackKind, TrackSlot, try_box,
};
use engine::time::{Frame, LoopRegion, Quantize, Tempo, Tick, TimeSignature};

// ---------------------------------------------------------------- the probe

thread_local! {
    /// (track, absolute frame, event).
    static LOG: RefCell<Vec<(u32, u64, Event)>> = const { RefCell::new(Vec::new()) };
}

static NO_PARAMS: [ParamDescriptor; 0] = [];

static PROBE: DeviceDescriptor = DeviceDescriptor {
    kind: "faulk.test_launcher_probe",
    kind_id: DeviceKind(0xFFFF_00C2),
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
        Rig::with(rate, tracks, quantize, TimeSignature::COMMON)
    }

    fn with(rate: u32, tracks: u32, quantize: Quantize, signature: TimeSignature) -> Rig {
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
            signature,
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

/// The cells the engine reports as not idle: (track, scene, state). Never more than two per
/// track, the room the report has.
fn grid(r: &Rig) -> Vec<(u32, u16, ClipState)> {
    let mut cells = Vec::new();
    r.e.active_clips(|c: ActiveClip| cells.push((c.track.0, c.scene.0, c.state)));
    for t in 0..8 {
        assert!(cells.iter().filter(|c| c.0 == t).count() <= 2, "{cells:?}");
    }
    // The tracks the grid holds without a clip are in no cell: `held`.
    cells.retain(|c| c.2 != ClipState::Held);
    cells
}

/// The tracks the engine reports as held by the grid without a clip playing.
fn reported_held(r: &Rig) -> Vec<u32> {
    let mut tracks = Vec::new();
    r.e.active_clips(|c: ActiveClip| {
        if c.state == ClipState::Held {
            tracks.push(c.track.0);
        }
    });
    tracks
}

/// The anchor each reported cell carries, as (scene, state, anchor tick).
fn anchors(r: &Rig) -> Vec<(u16, ClipState, i64)> {
    let mut cells = Vec::new();
    r.e.active_clips(|c: ActiveClip| {
        if c.state != ClipState::Held {
            cells.push((c.scene.0, c.state, c.anchor.0));
        }
    });
    cells
}

/// A one-shot clip with a single short note at its start.
fn blip(clip: u32, pitch: u8) -> M {
    upload(clip, 960, None, vec![note(0, 100, pitch)])
}

/// Sixteen bars of 4/4: longer than any test plays.
const LONG: i64 = 16 * 3840;

/// A clip that loops over `ticks` and holds one note from 0 for its whole length.
fn drone(clip: u32, ticks: i64, pitch: u8) -> M {
    upload(clip, ticks, Some((0, ticks)), vec![note(0, ticks, pitch)])
}

/// The grid step of `q` in ticks under `signature`, written out here (0: none).
fn step(q: Quantize, signature: TimeSignature) -> i64 {
    let beat = 3840 / i64::from(signature.denominator());
    let bar = beat * i64::from(signature.numerator());
    match q {
        Quantize::None => 0,
        Quantize::Sixteenth => 240,
        Quantize::Beat => beat,
        Quantize::Bar => bar,
        Quantize::Bars2 => 2 * bar,
        Quantize::Bars4 => 4 * bar,
        Quantize::Bars8 => 8 * bar,
    }
}

// ---------------------------------------------------------------- scenes

/// A scene launched mid-bar starts on the sample of the first grid point after the launch,
/// on every track of the scene together.
fn scene_start_frames(rate: u32, signature: TimeSignature, q: Quantize, size: usize) {
    let mut r = Rig::with(rate, 3, q, signature);
    // A bar and 992 ticks: a multiple of 32 ticks (an exact frame at 44.1 kHz too), and off
    // every grid of the 7 values.
    let bar = step(Quantize::Bar, signature);
    let launched = bar + 992;
    let g = step(q, signature);
    assert!(g == 0 || launched % g != 0, "the launch is off the grid");
    let boundary = if g == 0 {
        launched
    } else {
        (launched / g + 1) * g
    };
    r.send(&[
        blip(0, 60),
        blip(1, 62),
        slot(0, 0, 0),
        slot(1, 0, 1),
        M::Play { from: Tick(0) },
    ]);
    r.run_to(r.frame(0, 0, launched), size);
    r.send(&[launch_scene(0)]);
    r.run_to(r.frame(0, 0, boundary + 960), size);
    let at = r.frame(0, 0, boundary);
    let why = format!("{rate} Hz, {signature:?}, {q:?}, blocks of {size}");
    assert_eq!(ons(0), vec![(at, 60)], "{why}");
    assert_eq!(ons(1), vec![(at, 62)], "{why}");
    assert!(
        ons(2).is_empty(),
        "the track without a clip in the scene: {why}"
    );
    for t in 0..3 {
        assert_nothing_held(t);
    }
}

#[test]
fn a_scene_starts_all_its_tracks_on_the_boundary_sample_for_each_quantization() {
    for rate in [48_000, 44_100] {
        for signature in [TimeSignature::COMMON, TimeSignature::new(7, 8).unwrap()] {
            for q in Quantize::ALL {
                scene_start_frames(rate, signature, q, 128);
            }
        }
    }
}

#[test]
fn the_block_size_does_not_move_the_start_of_a_scene() {
    for size in [1, 7, 333, 2048] {
        for q in [
            Quantize::None,
            Quantize::Sixteenth,
            Quantize::Beat,
            Quantize::Bar,
        ] {
            scene_start_frames(48_000, TimeSignature::COMMON, q, size);
        }
    }
}

/// Two tracks playing a drone of scene 0 get scene 1 at the next bar: every held note ends
/// on the boundary sample, before the new notes, and nothing stays held.
#[test]
fn a_scene_replaces_each_playing_clip_at_the_boundary_without_stuck_notes() {
    let mut r = Rig::new(48_000, 2, Quantize::Bar);
    r.send(&[
        drone(0, LONG, 60),
        drone(1, LONG, 61),
        drone(2, LONG, 70),
        drone(3, LONG, 71),
        slot(0, 0, 0),
        slot(1, 0, 1),
        slot(0, 1, 2),
        slot(1, 1, 3),
        launch_scene(0),
        M::Play { from: Tick(0) },
    ]);
    r.run_to(r.frame(0, 0, 4000), 128);
    r.send(&[launch_scene(1)]);
    r.run_to(r.frame(0, 0, 3 * 3840), 128);
    let b = r.frame(0, 0, 2 * 3840);
    for (t, old, new) in [(0, 60, 70), (1, 61, 71)] {
        let at_b: Vec<(bool, u8)> = events(t)
            .iter()
            .filter(|e| e.0 == b)
            .map(|e| (e.1, e.3))
            .collect();
        assert_eq!(
            at_b,
            vec![(false, old), (true, new)],
            "track {t}: the old note ends before the new one starts, on the boundary sample"
        );
    }
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(0);
    assert_nothing_held(1);
}

/// The last order before the boundary wins: relaunching the scene that plays restarts it.
#[test]
fn the_last_order_before_the_boundary_wins() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[
        drone(0, LONG, 60),
        drone(1, LONG, 70),
        slot(0, 0, 0),
        slot(0, 1, 1),
        launch_scene(0),
        M::Play { from: Tick(0) },
    ]);
    r.run_to(r.frame(0, 0, 4000), 128);
    r.send(&[launch_scene(1)]);
    r.run(500, 128);
    r.send(&[launch_scene(0)]);
    r.run_to(r.frame(0, 0, 2 * 3840 + 100), 128);
    let b = r.frame(0, 0, 2 * 3840);
    let at_b: Vec<(bool, u8)> = events(0)
        .iter()
        .filter(|e| e.0 == b)
        .map(|e| (e.1, e.3))
        .collect();
    assert_eq!(at_b, vec![(false, 60), (true, 60)], "scene 0 starts over");
    assert!(ons(0).iter().all(|(_, key)| *key == 60));
    r.send(&[M::Stop]);
    r.run(256, 128);
    assert_nothing_held(0);
}

#[test]
fn a_scene_stops_the_tracks_with_an_empty_cell_at_the_same_boundary() {
    let mut r = Rig::new(48_000, 3, Quantize::Bar);
    r.send(&[
        drone(0, LONG, 60),
        drone(1, LONG, 61),
        drone(2, LONG, 70),
        slot(0, 0, 0),
        slot(1, 0, 1),
        slot(0, 1, 2),
        launch_scene(0),
        M::Play { from: Tick(0) },
    ]);
    r.run_to(r.frame(0, 0, 4000), 128);
    r.send(&[launch_scene(1)]);
    r.run(128, 128);
    // Track 1 has no clip in scene 1: it is reported as stopping until the boundary; track 2
    // (idle, no clip either) is not touched.
    assert_eq!(
        grid(&r),
        vec![
            (0, 0, ClipState::Playing),
            (0, 1, ClipState::Queued),
            (1, 0, ClipState::Stopping)
        ]
    );
    r.run_to(r.frame(0, 0, 3 * 3840), 128);
    let b = r.frame(0, 0, 2 * 3840);
    assert!(ons(0).contains(&(b, 70)), "track 0 follows the scene");
    assert_eq!(ons(1), vec![(0, 61)]);
    assert_eq!(
        offs(1),
        vec![(b, 61)],
        "track 1 stops on the boundary sample"
    );
    assert!(events(2).is_empty());
    assert_eq!(grid(&r), vec![(0, 1, ClipState::Playing)]);
    r.send(&[M::Stop]);
    r.run(256, 128);
    for t in 0..3 {
        assert_nothing_held(t);
    }
}

#[test]
fn a_scene_without_any_clip_stops_what_plays_and_an_idle_engine_stays_idle() {
    let mut r = Rig::new(48_000, 2, Quantize::None);
    r.send(&[
        drone(0, LONG, 60),
        slot(0, 0, 0),
        launch_scene(0),
        M::Play { from: Tick(0) },
    ]);
    r.run(128 * 20, 128);
    // Scenes past the track's table are empty too: the track stops, without quantization
    // at once, its note released.
    let at = r.frames;
    r.send(&[launch_scene(5), launch_scene(4095)]);
    r.run(128 * 20, 128);
    // At the first tick of the block (a tick is 25 frames at 48 kHz, 120 BPM).
    let off = offs(0);
    assert_eq!(off.len(), 1);
    assert!(
        (at..at + 25).contains(&off[0].0) && off[0].1 == 60,
        "{off:?}"
    );
    assert!(grid(&r).is_empty());
    assert_nothing_held(0);
    // Nothing plays any more: another empty scene changes nothing.
    let before = events(0);
    r.send(&[launch_scene(5)]);
    r.run(128 * 20, 128);
    assert_eq!(events(0), before);
    assert!(grid(&r).is_empty());
    assert_eq!(r.e.skipped(), 0);
}

// ---------------------------------------------------------------- StopAll

#[test]
fn stop_all_stops_every_playing_track_at_the_boundary_and_cancels_a_pending_launch() {
    let mut r = Rig::new(48_000, 3, Quantize::Bar);
    r.send(&[
        drone(0, LONG, 60),
        drone(1, LONG, 61),
        drone(2, LONG, 70),
        slot(0, 0, 0),
        slot(1, 0, 1),
        slot(0, 1, 2),
        launch(0, 0),
        launch(1, 0),
        M::Play { from: Tick(0) },
    ]);
    r.run_to(r.frame(0, 0, 4000), 128);
    // Track 0 has a launch of scene 1 pending when the order comes; track 2 does nothing.
    r.send(&[launch(0, 1), M::StopAll]);
    r.run(128, 128);
    assert_eq!(
        grid(&r),
        vec![(0, 0, ClipState::Stopping), (1, 0, ClipState::Stopping)]
    );
    r.run_to(r.frame(0, 0, 4 * 3840), 128);
    let b = r.frame(0, 0, 2 * 3840);
    assert_eq!(
        offs(0),
        vec![(b, 60)],
        "stopped at the boundary, the launch is gone"
    );
    assert_eq!(offs(1), vec![(b, 61)]);
    assert_eq!(ons(0).len(), 1, "scene 1 never started");
    assert!(events(2).is_empty());
    assert!(grid(&r).is_empty());
    assert_nothing_held(0);
    assert_nothing_held(1);
}

#[test]
fn stop_all_with_the_transport_stopped_leaves_nothing_to_play() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[drone(0, LONG, 60), slot(0, 0, 0), launch(0, 0), M::StopAll]);
    r.run(128, 128);
    assert!(grid(&r).is_empty(), "a stop is not a clip");
    r.send(&[M::Play { from: Tick(0) }]);
    r.run_to(r.frame(128, 0, 3 * 3840), 128);
    assert!(ons(0).is_empty());
}

// ---------------------------------------------------------------- BackToArrangement

#[test]
fn back_to_arrangement_releases_the_clips_at_once_and_drops_the_pending_launches() {
    let mut r = Rig::new(48_000, 2, Quantize::Bar);
    r.send(&[
        drone(0, LONG, 60),
        drone(1, LONG, 70),
        drone(2, LONG, 80),
        slot(0, 0, 0),
        slot(1, 0, 1),
        slot(0, 1, 2),
        launch_scene(0),
        M::Play { from: Tick(0) },
    ]);
    r.run_to(r.frame(0, 0, 4000), 128);
    // A launch is pending on track 0 when the order comes.
    r.send(&[launch(0, 1)]);
    r.run(128, 128);
    let at = r.frames;
    r.send(&[M::BackToArrangement]);
    r.run_to(r.frame(0, 0, 3 * 3840), 128);
    assert_eq!(
        offs(0),
        vec![(at, 60)],
        "released on the first sample of the block"
    );
    assert_eq!(offs(1), vec![(at, 70)]);
    assert_eq!(ons(0).len(), 1, "the pending launch never ran");
    assert!(grid(&r).is_empty());
    assert!(
        !r.e.state().track(TrackSlot(0)).unwrap().player.engaged(),
        "the track is free for its arrangement"
    );
    assert_nothing_held(0);
    assert_nothing_held(1);
}

#[test]
fn a_track_the_grid_holds_without_a_clip_is_reported_held_until_back_to_arrangement() {
    let mut r = Rig::new(48_000, 2, Quantize::None);
    r.send(&[drone(0, LONG, 60), slot(0, 0, 0), M::Play { from: Tick(0) }]);
    r.run(2_000, 128);
    assert!(
        reported_held(&r).is_empty(),
        "nothing launched: both tracks follow their timeline"
    );
    r.send(&[launch(0, 0)]);
    r.run(2_000, 128);
    assert!(
        reported_held(&r).is_empty(),
        "a track that plays a clip says so by that clip"
    );
    r.send(&[M::StopTrack {
        track: TrackSlot(0),
    }]);
    r.run(2_000, 128);
    assert_eq!(
        reported_held(&r),
        vec![0],
        "silent, held by the grid, in no cell"
    );
    assert!(grid(&r).is_empty());
    r.send(&[M::BackToArrangement]);
    r.run(2_000, 128);
    assert!(reported_held(&r).is_empty());
}

#[test]
fn back_to_arrangement_drops_an_armed_launch_while_stopped() {
    let mut r = Rig::new(48_000, 1, Quantize::Bar);
    r.send(&[drone(0, LONG, 60), slot(0, 0, 0), launch(0, 0)]);
    r.run(128, 128);
    assert_eq!(grid(&r), vec![(0, 0, ClipState::Armed)]);
    r.send(&[M::BackToArrangement]);
    r.run(128, 128);
    assert!(grid(&r).is_empty());
    r.send(&[M::Play { from: Tick(0) }]);
    r.run_to(r.frame(256, 0, 3 * 3840), 128);
    assert!(ons(0).is_empty(), "the armed launch did not survive");
    assert!(!r.e.state().track(TrackSlot(0)).unwrap().player.engaged());
}

#[test]
fn a_launch_after_back_to_arrangement_survives_it_and_one_before_does_not() {
    for launch_first in [true, false] {
        let mut r = Rig::new(48_000, 1, Quantize::Bar);
        r.send(&[
            drone(0, LONG, 60),
            drone(1, LONG, 70),
            slot(0, 0, 0),
            slot(0, 1, 1),
        ]);
        r.send(&[launch(0, 0), M::Play { from: Tick(0) }]);
        r.run_to(r.frame(0, 0, 4000), 128);
        // Both orders reach the same block.
        let both = if launch_first {
            [launch(0, 1), M::BackToArrangement]
        } else {
            [M::BackToArrangement, launch(0, 1)]
        };
        r.send(&both);
        r.run_to(r.frame(0, 0, 3 * 3840 + 100), 128);
        let b = r.frame(0, 0, 2 * 3840);
        let new_started = ons(0).contains(&(b, 70));
        assert_eq!(new_started, !launch_first, "launch first: {launch_first}");
        r.send(&[M::Stop]);
        r.run(256, 128);
        assert_nothing_held(0);
    }
}

/// 6 000 notes held on a track that goes back to the arrangement: the release does not fit
/// one sub-block and goes on in the next ones, on the first samples.
#[test]
fn a_back_to_arrangement_that_fills_the_sink_still_releases_every_note() {
    let notes: Vec<Note> = (0..6000).map(|i| note(0, 3000, (i % 128) as u8)).collect();
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, upload(0, 3840, Some((0, 3840)), notes));
    r.run(1000, 128);
    assert_eq!(events(0).iter().filter(|e| e.1).count(), 6000);
    let at = r.frames;
    r.send(&[M::BackToArrangement]);
    r.run(1000, 128);
    let released = events(0)
        .iter()
        .filter(|e| !e.1 && e.0 >= at && e.0 < at + 8)
        .count();
    assert_eq!(released, 6000, "every note ended within the first samples");
    assert!(!r.e.state().track(TrackSlot(0)).unwrap().player.engaged());
    assert_nothing_held(0);
}

// ---------------------------------------------------------------- one start for a scene

/// A track whose reading was cut by a full sink in the block before takes a launch at the
/// same boundary as the others: an order is never given in the past of its block. Swept over
/// the frames that put the wrap of the dense clip (3 089 notes on one sample) at each place
/// of a block around its end.
#[test]
fn a_track_with_a_cut_from_the_last_block_starts_the_scene_with_the_others() {
    let n = 6 * SCHEDULE_CHUNK + 17;
    let dense: Vec<Note> = (0..n).map(|i| note(0, 100, (i % 100) as u8)).collect();
    for shift in 0..40u64 {
        let mut r = Rig::new(48_000, 2, Quantize::Sixteenth);
        r.send(&[
            upload(0, 960, Some((0, 960)), dense.clone()),
            blip(1, 110),
            blip(2, 111),
            slot(0, 0, 0),
            slot(1, 0, 2),
            slot(0, 1, 1),
            slot(1, 1, 2),
            launch(0, 0),
            M::Play { from: Tick(0) },
        ]);
        // The wrap of the dense clip is the sample 24 000: it lands on the frames around the
        // end of a block of 125 as `shift` goes.
        r.run(24_000 - 20 + shift, 125);
        r.send(&[launch_scene(1)]);
        r.run(12_000, 125);
        // Tick 1200 is the first sixteenth after the wrap: sample 30 000.
        let start = |t: u32, key: u8| -> Vec<u64> {
            events(t)
                .into_iter()
                .filter(|e| e.1 && e.3 == key)
                .map(|e| e.0)
                .collect()
        };
        // A launch made at or before the wrap (tick 960, on the grid) starts there; after it,
        // at the next sixteenth.
        let want = if shift <= 20 { 24_000 } else { 30_000 };
        assert_eq!(start(0, 110), vec![want], "shift {shift}, track 0");
        assert_eq!(start(1, 111), vec![want], "shift {shift}, track 1");
        // The dense clip's whole release (3 089 notes) went through, whatever the cut.
        assert_nothing_held(0);
        assert_nothing_held(1);
    }
}

// ---------------------------------------------------------------- the states of the report

#[test]
fn the_grid_states_follow_a_clip_from_armed_to_idle() {
    let mut r = Rig::new(48_000, 2, Quantize::Bar);
    r.send(&[
        drone(0, LONG, 60),
        drone(1, LONG, 70),
        drone(2, LONG, 80),
        slot(0, 0, 0),
        slot(0, 1, 1),
        slot(1, 2, 2),
    ]);
    // The transport is stopped: the launch is armed, before and after the player took it.
    r.send(&[launch(0, 0)]);
    r.run(128, 128);
    assert_eq!(grid(&r), vec![(0, 0, ClipState::Armed)]);
    // Played from tick 100, the boundary is the next bar.
    let s0 = r.frames;
    r.send(&[M::Play { from: Tick(100) }]);
    r.run(128, 128);
    assert_eq!(grid(&r), vec![(0, 0, ClipState::Queued)]);
    r.run_to(r.frame(s0, 100, 3840) + 128, 128);
    assert_eq!(grid(&r), vec![(0, 0, ClipState::Playing)]);
    // The report carries the tick of the clip's position 0: the boundary it started on.
    assert_eq!(anchors(&r), vec![(0, ClipState::Playing, 3840)]);
    // Another launch on the track: the playing clip and the queued one.
    r.send(&[launch(0, 1)]);
    r.run(128, 128);
    assert_eq!(
        grid(&r),
        vec![(0, 0, ClipState::Playing), (0, 1, ClipState::Queued)]
    );
    let waiting = anchors(&r);
    assert_eq!(waiting[0], (0, ClipState::Playing, 3840));
    // The waiting launch carries its boundary once the player knows it, else 0.
    assert!(
        [0, 7680].contains(&waiting[1].2),
        "queued anchor: {waiting:?}"
    );
    r.run_to(r.frame(s0, 100, 7680) + 128, 128);
    assert_eq!(grid(&r), vec![(0, 1, ClipState::Playing)]);
    assert_eq!(anchors(&r), vec![(1, ClipState::Playing, 7680)]);
    // A stop waits for its boundary, then nothing is left.
    r.send(&[stop_track(0)]);
    r.run(128, 128);
    assert_eq!(grid(&r), vec![(0, 1, ClipState::Stopping)]);
    r.run_to(r.frame(s0, 100, 11520) + 128, 128);
    assert!(grid(&r).is_empty());
    assert_nothing_held(0);
}

/// The launches that leave the track silent are reported as a stop, never as a clip that
/// starts: an empty cell, an audio clip (its start is AU-4's) and a clip another track plays.
#[test]
fn a_launch_that_leaves_the_track_silent_is_reported_as_stopping() {
    let mut r = Rig::new(48_000, 2, Quantize::Bar);
    r.send(&[
        drone(0, LONG, 60),
        drone(1, LONG, 70),
        M::AudioSourceDeclare {
            source: SourceSlot(0),
            sample_rate: 48_000,
            channels: 1,
            frames: Frame(48_000),
            media: [0; 32],
        },
        M::ClipAudioSet {
            clip: ClipSlot(2),
            source: SourceSlot(0),
            start: Frame(0),
            length: Frame(48_000),
            gain: 1.0,
            looping: None,
        },
        slot(0, 0, 0),
        slot(1, 1, 1),
        slot(0, 2, 2),
        // The clip track 1 plays, in a cell of track 0 too.
        slot(0, 3, 1),
        launch(0, 0),
        launch(1, 1),
        M::Play { from: Tick(0) },
    ]);
    r.run(128, 128);
    assert_eq!(
        grid(&r),
        vec![(0, 0, ClipState::Playing), (1, 1, ClipState::Playing)]
    );
    for scene in [7, 2, 3] {
        r.send(&[launch(0, scene)]);
        r.run(128, 128);
        assert_eq!(
            grid(&r),
            vec![(0, 0, ClipState::Stopping), (1, 1, ClipState::Playing)],
            "scene {scene}"
        );
    }
    // At the boundary the track really goes silent: the report did not lie.
    r.run_to(r.frame(0, 0, 3840 + 100), 128);
    assert_eq!(grid(&r), vec![(1, 1, ClipState::Playing)]);
    assert_eq!(offs(0), vec![(r.frame(0, 0, 3840), 60)]);
    assert_eq!(ons(0), vec![(0, 60)]);
    assert_nothing_held(0);
}

#[test]
fn a_playing_clip_is_reported_in_the_cell_it_sits_in_now() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    r.send(&[
        drone(0, LONG, 60),
        slot(0, 4, 0),
        launch(0, 4),
        M::Play { from: Tick(0) },
    ]);
    r.run(128, 128);
    assert_eq!(grid(&r), vec![(0, 4, ClipState::Playing)]);
    // Moved to another cell: reported there.
    r.send(&[slot(0, 6, 0)]);
    r.run(128, 128);
    assert_eq!(grid(&r), vec![(0, 6, ClipState::Playing)]);
    // Cleared from its cell (the one it was reported in): it plays on, in no cell.
    r.send(&[M::SceneSetSlot {
        track: TrackSlot(0),
        scene: SceneSlot(6),
        clip: None,
    }]);
    r.run(128, 128);
    assert!(grid(&r).is_empty());
    assert!(
        r.e.state()
            .track(TrackSlot(0))
            .unwrap()
            .player
            .playing()
            .is_some()
    );
}

/// A track without a device renders nothing, but its grid state is the real one.
#[test]
fn a_track_without_a_device_still_launches_and_reports() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    r.send(&[M::TrackCreate {
        track: TrackSlot(1),
        kind: TrackKind::Midi,
    }]);
    r.send(&[drone(0, LONG, 60), slot(1, 0, 0), launch(1, 0)]);
    r.run(128, 128);
    assert_eq!(grid(&r), vec![(1, 0, ClipState::Armed)]);
    r.send(&[M::Play { from: Tick(0) }]);
    r.run(128, 128);
    assert_eq!(grid(&r), vec![(1, 0, ClipState::Playing)]);
    r.send(&[M::StopAll]);
    r.run(128, 128);
    assert!(grid(&r).is_empty(), "stopped at once (no quantization)");
    assert!(!r.e.state().track(TrackSlot(1)).unwrap().player.engaged());
}
