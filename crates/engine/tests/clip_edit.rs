//! CL-3: a clip edited while it plays. Offline renders through the real `Preparer`,
//! `Engine::apply` and `Engine::process`, every audio-side call under the allocation
//! detector, with a probe instrument that logs each event with its absolute frame.
//! A held note that an edit leaves unchanged (same start, length, pitch, wherever it sits
//! in the table) goes on sounding under the same voice id and is ended by its own
//! note-off; any other held note is ended by the track with a note-off of its own, the
//! notes played live untouched. No stuck note, no voice started twice.

use std::cell::RefCell;
use std::collections::BTreeSet;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event, Factory,
    ParamDescriptor,
};
use engine::engine::{Engine, Preparer, channel};
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

// ---------------------------------------------------------------- the tests

/// 25 frames per tick at 48 kHz and 120 BPM.
const F: u64 = 25;

fn all_notes_offs() -> Vec<u64> {
    events(0)
        .into_iter()
        .filter(|e| e.3 == 255)
        .map(|e| e.0)
        .collect()
}

/// A pad (clip tick 0 to 3000) and short notes after it; looped over one bar.
fn pad_clip(extra: Vec<Note>) -> M {
    let mut notes = vec![note(0, 3000, 48)];
    notes.extend(extra);
    upload(0, 3840, Some((0, 3840)), notes)
}

/// The pad clip with its first note replaced.
fn with_first(m: M, first: Note) -> M {
    let M::ClipNotesUpload {
        clip,
        length,
        looping,
        total,
        mut notes,
        ..
    } = m
    else {
        panic!("an upload")
    };
    notes[0] = first;
    M::ClipNotesUpload {
        clip,
        length,
        looping,
        first: 0,
        total,
        notes,
    }
}

#[test]
fn an_edit_after_the_sounding_note_does_not_cut_it() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, pad_clip(vec![note(3300, 100, 60)]));
    r.run(500 * F, 128);
    r.send(&[pad_clip(vec![note(3300, 100, 60), note(3500, 100, 62)])]);
    r.run_to(3700 * F, 128);
    assert!(all_notes_offs().is_empty(), "{:?}", events(0));
    let pad_on = ons(0).iter().filter(|e| e.1 == 48).count();
    assert_eq!(pad_on, 1, "the pad is not started again");
    assert_eq!(offs(0).iter().find(|e| e.1 == 48), Some(&(3000 * F, 48)));
    assert!(ons(0).contains(&(3500 * F, 62)));
    assert_nothing_held(0);
}

#[test]
fn an_edit_that_changes_the_sounding_note_ends_it_once_and_spares_the_live_notes() {
    for first in [note(0, 300, 48), note(100, 3000, 48), note(0, 3000, 50)] {
        let mut r = Rig::new(48_000, 1, Quantize::None);
        playing(&mut r, pad_clip(vec![note(3300, 100, 60)]));
        r.run(500 * F, 128);
        r.send(&[M::NoteOn {
            track: TrackSlot(0),
            pitch: 72,
            velocity: 100,
        }]);
        r.run(128, 128);
        r.send(&[with_first(pad_clip(vec![note(3300, 100, 60)]), first)]);
        let at = r.frames;
        r.run_to(3700 * F, 128);
        assert!(all_notes_offs().is_empty(), "{:?}", events(0));
        assert!(
            events(0).contains(&(at, false, 129, 48, 0)),
            "the pad is ended by its own note-off: {:?}",
            events(0)
        );
        // The live note (voice key + 1) sounded and was never ended.
        assert!(
            events(0).iter().any(|e| e.1 && e.2 == 73 && e.3 == 72),
            "{:?}",
            events(0)
        );
        assert!(
            events(0).iter().all(|e| e.2 != 73 || e.1),
            "{:?}",
            events(0)
        );
        r.send(&[M::AllNotesOff {
            track: TrackSlot(0),
        }]);
        r.run(128, 128);
        assert_nothing_held(0);
    }
    // Removed altogether.
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, pad_clip(vec![]));
    r.run(500 * F, 128);
    r.send(&[upload(0, 3840, Some((0, 3840)), vec![note(3300, 100, 60)])]);
    let at = r.frames;
    r.run_to(3700 * F, 128);
    assert!(all_notes_offs().is_empty());
    assert!(events(0).contains(&(at, false, 129, 48, 0)));
    assert_nothing_held(0);
}

#[test]
fn a_note_added_before_the_sounding_one_does_not_cut_it() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(0, 3840, Some((0, 3840)), vec![note(0, 3000, 48)]),
    );
    r.run(500 * F, 128);
    // The pad moves to index 1, then 2.
    r.send(&[upload(
        0,
        3840,
        Some((0, 3840)),
        vec![note(0, 100, 40), note(0, 3000, 48)],
    )]);
    r.run(100 * F, 128);
    r.send(&[upload(
        0,
        3840,
        Some((0, 3840)),
        vec![note(0, 100, 40), note(0, 100, 41), note(0, 3000, 48)],
    )]);
    r.run_to(3700 * F, 128);
    assert!(all_notes_offs().is_empty(), "{:?}", events(0));
    assert_eq!(ons(0).iter().filter(|e| e.1 == 48).count(), 1);
    assert_eq!(offs(0).iter().find(|e| e.1 == 48), Some(&(3000 * F, 48)));
    // The next pass: every note starts once, no voice twice (`held` checks it).
    r.run_to(3840 * 2 * F, 128);
    assert!(ons(0).iter().filter(|e| e.1 == 48).count() >= 2);
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn a_note_removed_before_the_sounding_one_does_not_cut_it() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(
            0,
            3840,
            Some((0, 3840)),
            vec![note(0, 100, 40), note(0, 3000, 48)],
        ),
    );
    r.run(500 * F, 128);
    r.send(&[upload(0, 3840, Some((0, 3840)), vec![note(0, 3000, 48)])]);
    r.run_to(3700 * F, 128);
    assert!(all_notes_offs().is_empty(), "{:?}", events(0));
    assert_eq!(offs(0).iter().find(|e| e.1 == 48), Some(&(3000 * F, 48)));
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn two_sounding_notes_that_swap_places_keep_their_voices() {
    // The voice of each is the own voice of the other in the new table.
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(
            0,
            3840,
            Some((0, 3840)),
            vec![note(0, 3000, 48), note(0, 2000, 50)],
        ),
    );
    r.run(500 * F, 128);
    r.send(&[upload(
        0,
        3840,
        Some((0, 3840)),
        vec![note(0, 2000, 50), note(0, 3000, 48)],
    )]);
    r.run_to(3700 * F, 128);
    assert!(all_notes_offs().is_empty(), "{:?}", events(0));
    let voice_of = |key: u8| {
        events(0)
            .into_iter()
            .find(|e| e.1 && e.3 == key)
            .map(|e| e.2)
    };
    let end_of = |key: u8| {
        events(0)
            .into_iter()
            .find(|e| !e.1 && e.3 == key)
            .map(|e| (e.0, e.2))
    };
    assert_eq!(end_of(48), Some((3000 * F, voice_of(48).unwrap())));
    assert_eq!(end_of(50), Some((2000 * F, voice_of(50).unwrap())));
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn a_displaced_note_starts_on_a_spare_voice_while_the_kept_one_sounds() {
    // The pad (voice 129) moves to index 1; the note now at index 0 would have voice 129
    // and starts at tick 100 while the pad sounds.
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(0, 3840, Some((0, 3840)), vec![note(0, 3000, 48)]),
    );
    r.run(50 * F, 128);
    r.send(&[upload(
        0,
        3840,
        Some((0, 3840)),
        vec![note(100, 500, 40), note(0, 3000, 48)],
    )]);
    r.run_to(300 * F, 128);
    // `held` panics on a voice started twice.
    let h = held(0);
    assert_eq!(h.len(), 2, "{h:?}");
    let displaced = events(0).into_iter().find(|e| e.1 && e.3 == 40).unwrap();
    assert!(displaced.2 >= 131, "a spare voice: {displaced:?}");
    assert!(all_notes_offs().is_empty());
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn many_chained_edits_keep_unique_voices() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    let mut rng = Lcg(5);
    let pads: Vec<Note> = (0..6).map(|i| note(0, 3000, 40 + i)).collect();
    playing(&mut r, upload(0, 3840, Some((0, 3840)), pads.clone()));
    for _ in 0..300 {
        // The pads stay, in another place each time, among short notes.
        let mut notes = pads.clone();
        for _ in 0..rng.next(8) {
            let at = rng.next(notes.len() as u64 + 1) as usize;
            notes.insert(
                at,
                note(
                    rng.next(2500) as i64,
                    1 + rng.next(400) as i64,
                    60 + rng.next(20) as u8,
                ),
            );
        }
        r.send(&[upload(0, 3840, Some((0, 3840)), notes)]);
        r.run(rng.next(300) + 1, 64);
        let _ = held(0);
    }
    r.run(3840 * F, 256);
    let cuts = all_notes_offs();
    assert!(cuts.is_empty(), "{cuts:?}");
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn up_to_the_carry_limit_of_moved_notes_are_kept() {
    let crowd: Vec<Note> = (0..48).map(|i| note(0, 3000, i as u8)).collect();
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, upload(0, 3840, Some((0, 3840)), crowd.clone()));
    r.run(500 * F, 128);
    let mut edited = vec![note(0, 100, 100)];
    edited.extend(crowd);
    r.send(&[upload(0, 3840, Some((0, 3840)), edited)]);
    r.run_to(3700 * F, 128);
    assert!(all_notes_offs().is_empty(), "{:?}", events(0));
    assert_eq!(ons(0).len(), 48, "none started twice");
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn thousands_of_unchanged_notes_are_kept() {
    let crowd: Vec<Note> = (0..4000).map(|i| note(0, 3000, (i % 128) as u8)).collect();
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, upload(0, 3840, Some((0, 3840)), crowd.clone()));
    r.run(500 * F, 128);
    let mut edited = crowd;
    edited.push(note(3300, 100, 60));
    r.send(&[upload(0, 3840, Some((0, 3840)), edited)]);
    r.run_to(3700 * F, 128);
    assert!(all_notes_offs().is_empty(), "{:?}", events(0));
    assert_eq!(ons(0).iter().filter(|e| e.0 < 3000 * F).count(), 4000);
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn more_changed_notes_than_the_track_can_end_are_released_by_the_track_once() {
    let crowd: Vec<Note> = (0..200).map(|i| note(0, 3000, i as u8 % 128)).collect();
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, upload(0, 3840, Some((0, 3840)), crowd));
    r.run(500 * F, 128);
    let changed: Vec<Note> = (0..200).map(|i| note(0, 2000, i as u8 % 128)).collect();
    r.send(&[upload(0, 3840, Some((0, 3840)), changed)]);
    let at = r.frames;
    r.run(100 * F, 128);
    assert!(all_notes_offs().contains(&at), "released at the swap");
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn more_sounding_notes_than_apply_looks_at_are_released_by_the_track() {
    // Above `MAX_HELD_NOTES`: nothing is carried, even unchanged.
    let crowd: Vec<Note> = (0..4200).map(|i| note(0, 3000, (i % 128) as u8)).collect();
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, upload(0, 3840, Some((0, 3840)), crowd.clone()));
    r.run(500 * F, 128);
    r.send(&[upload(0, 3840, Some((0, 3840)), crowd)]);
    let at = r.frames;
    r.run(100 * F, 128);
    assert!(all_notes_offs().contains(&at), "released at the swap");
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn two_edits_in_one_batch_share_the_room_of_the_track() {
    // 100 pads; the first edit shortens 40 of them (40 notes to end, the track takes 64),
    // the second 40 others (40 more: the room left is too small). The second releases the
    // track; nothing stays held, the first edit's note-offs are covered by the release.
    let pads = |short: std::ops::Range<usize>| -> Vec<Note> {
        (0..100)
            .map(|i| {
                note(
                    0,
                    if short.contains(&i) { 2000 } else { 3000 },
                    20 + i as u8,
                )
            })
            .collect()
    };
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, upload(0, 3840, Some((0, 3840)), pads(0..0)));
    r.run(500 * F, 128);
    r.send(&[
        upload(0, 3840, Some((0, 3840)), pads(0..40)),
        upload(0, 3840, Some((0, 3840)), pads(0..80)),
    ]);
    r.run_to(3700 * F, 128);
    assert_eq!(all_notes_offs().len(), 1, "{:?}", all_notes_offs());
    assert_nothing_held_after_stop(&mut r);
}

#[test]
fn removing_a_clip_whose_notes_moved_ends_each_under_its_own_voice() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(
        &mut r,
        upload(0, 3840, Some((0, 3840)), vec![note(0, 3000, 48)]),
    );
    r.run(500 * F, 128);
    // The pad moves to index 1 (voice 129 kept through an alias).
    r.send(&[upload(
        0,
        3840,
        Some((0, 3840)),
        vec![note(0, 100, 40), note(0, 3000, 48)],
    )]);
    r.run(100 * F, 128);
    r.send(&[M::ClipRemove { clip: ClipSlot(0) }]);
    let at = r.frames;
    r.run(100 * F, 128);
    assert!(
        events(0).contains(&(at, false, 129, 48, 0)),
        "{:?}",
        events(0)
    );
    assert!(all_notes_offs().is_empty());
    assert_nothing_held_after_stop(&mut r);
}

fn assert_nothing_held_after_stop(r: &mut Rig) {
    r.send(&[M::Stop]);
    r.run(512, 128);
    assert_nothing_held(0);
}

#[test]
fn an_added_note_that_straddles_the_playhead_is_not_started_and_its_end_is_ignored() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, pad_clip(vec![note(3300, 100, 60)]));
    r.run(500 * F, 128);
    r.send(&[pad_clip(vec![note(400, 500, 55), note(3300, 100, 60)])]);
    r.run_to(3700 * F, 128);
    assert!(ons(0).iter().all(|e| e.1 != 55));
    assert!(offs(0).iter().all(|e| e.1 != 55));
    assert_nothing_held(0);
}

#[test]
fn an_edit_that_shortens_the_clip_under_the_playhead_releases_the_notes() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, pad_clip(vec![]));
    r.run(500 * F, 128);
    r.send(&[upload(0, 400, None, vec![note(0, 3000, 48)])]);
    r.run_to(3700 * F, 128);
    assert_nothing_held(0);
}

#[test]
fn several_edits_in_one_block_keep_the_sounding_note() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, pad_clip(vec![note(3300, 100, 60)]));
    r.run(500 * F, 128);
    for k in 0..10 {
        r.send(&[pad_clip(vec![note(3300, 100, 60), note(3400 + k, 20, 61)])]);
    }
    r.run_to(3700 * F, 128);
    assert!(all_notes_offs().is_empty(), "{:?}", events(0));
    assert_eq!(ons(0).iter().filter(|e| e.1 == 48).count(), 1);
    assert_nothing_held(0);
}

#[test]
fn an_edit_in_the_middle_of_a_delivery_that_filled_the_sink_loses_no_note() {
    for size in [1usize, 7, 64] {
        let mut r = Rig::new(48_000, 1, Quantize::None);
        let crowd = |extra: i64| -> Vec<Note> {
            let mut v: Vec<Note> = (0..6000).map(|i| note(0, 3000, (i % 128) as u8)).collect();
            v.push(note(3500 + extra, 10, 1));
            v
        };
        playing(&mut r, upload(0, 3840, Some((0, 3840)), crowd(0)));
        for k in 0..40 {
            r.run(size as u64 * 2, size);
            r.send(&[upload(0, 3840, Some((0, 3840)), crowd(k))]);
        }
        r.run_to(3700 * F, 256);
        r.send(&[M::Stop]);
        r.run(512, 128);
        assert_nothing_held(0);
    }
}

#[test]
fn an_edit_refused_by_apply_leaves_the_clip_playing() {
    let mut r = Rig::new(48_000, 1, Quantize::None);
    playing(&mut r, pad_clip(vec![]));
    r.run(500 * F, 128);
    r.send(&[upload(99, 3840, Some((0, 3840)), vec![note(0, 100, 50)])]);
    r.run_to(3700 * F, 128);
    assert_eq!(offs(0), vec![(3000 * F, 48)]);
    assert_nothing_held(0);
}

/// A small generator: the edits are the same on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

#[test]
fn a_thousand_edits_after_the_pad_never_cut_it() {
    for size in [1usize, 31, 128, 500] {
        let mut r = Rig::new(48_000, 1, Quantize::None);
        let mut rng = Lcg(7);
        playing(&mut r, pad_clip(vec![]));
        for _ in 0..1000 {
            // The pad is note 0; every other note starts after it (rank after it).
            let n = rng.next(12);
            let mut extra: Vec<Note> = (0..n)
                .map(|_| {
                    note(
                        3000 + rng.next(800) as i64,
                        1 + rng.next(60) as i64,
                        60 + rng.next(24) as u8,
                    )
                })
                .collect();
            extra.sort_by_key(|x| (x.start, x.pitch, x.length));
            r.send(&[pad_clip(extra)]);
            r.run(rng.next(900) + 1, size);
        }
        let passes = r.frames / (3840 * F);
        r.run(3840 * F, 256);
        assert!(passes >= 3, "the edits spanned {passes} loop passes");
        let cuts: Vec<u64> = all_notes_offs()
            .into_iter()
            .filter(|f| f % (3840 * F) < 3000 * F)
            .collect();
        assert!(cuts.is_empty(), "block {size}: the pad was cut at {cuts:?}");
        let pads = ons(0).iter().filter(|e| e.1 == 48).count() as u64;
        let ends = offs(0).iter().filter(|e| e.1 == 48).count() as u64;
        assert!(
            pads == ends || pads == ends + 1,
            "{pads} starts, {ends} ends"
        );
        assert!(pads >= passes);
        r.send(&[M::Stop]);
        r.run(512, 128);
        assert_nothing_held(0);
    }
}

#[test]
fn a_thousand_random_edits_leave_no_stuck_note_and_no_double_start() {
    for size in [1usize, 31, 128, 500] {
        let mut r = Rig::new(48_000, 1, Quantize::None);
        let mut rng = Lcg(11 + size as u64);
        playing(&mut r, pad_clip(vec![]));
        for _ in 0..1000 {
            let n = 1 + rng.next(14);
            let mut notes: Vec<Note> = (0..n)
                .map(|_| {
                    note(
                        rng.next(3840) as i64,
                        1 + rng.next(2500) as i64,
                        30 + rng.next(60) as u8,
                    )
                })
                .collect();
            notes.sort_by_key(|x| (x.start, x.pitch, x.length));
            let looping = match rng.next(4) {
                0 => None,
                1 => Some((960, 2880)),
                _ => Some((0, 3840)),
            };
            r.send(&[upload(0, 3840, looping, notes)]);
            r.run(rng.next(700) + 1, size);
        }
        // Checks that no voice starts twice and no free voice is ended.
        let _ = held(0);
        r.send(&[M::Stop]);
        r.run(512, 128);
        assert_nothing_held(0);
    }
}
