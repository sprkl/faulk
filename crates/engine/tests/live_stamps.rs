//! Live notes stamped with the host frame they were played at (#499, `engine::recorder`,
//! "The sample a note was played at"): offline renders through the real `Preparer`,
//! `Engine::apply` and `Engine::render`, every audio-side call under the allocation detector.
//!
//! At 120 BPM and 48 kHz a beat is 24 000 samples and a tick 25 samples (0.52 ms); every
//! block is 128 frames (the web render quantum, 2.67 ms). The host's clock runs
//! [`ORIGIN`] frames ahead of the engine's sample counter, as an audio context that ran
//! before the engine did; the rig tells the engine the host frame of each block
//! (`Engine::set_host_frame`), as the web worklet does with `currentFrame`.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::DeviceSetup;
use engine::device::testing::factory;
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{ControlMsg as M, RecordedNote, TrackKind, TrackSlot, limits};
use engine::recorder::MAX_STAMP_DISTANCE_SECONDS;
use engine::time::{LoopRegion, Quantize, Tempo, Tick, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: f64 = 48_000.0;
const BLOCK: usize = 128;
/// Samples per tick at 120 BPM, 48 kHz.
const SPT: f64 = 25.0;
/// Samples of a beat at 120 BPM, 48 kHz.
const BEAT: u64 = 24_000;
/// Ticks of a beat.
const PPQ: i64 = 960;
/// The host's frame of the engine's first sample.
const ORIGIN: u64 = 1_000_000;

struct Rig {
    p: Preparer,
    e: Engine,
    /// Engine samples rendered so far.
    frames: u64,
    /// Whether the rig gives the engine its host clock.
    hosted: bool,
}

impl Rig {
    /// Track 0 an armed MIDI track, 120 BPM in 4/4, no count-in.
    fn new(hosted: bool) -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE as f32,
            max_block_frames: limits::MAX_BLOCK_FRAMES,
        };
        let (p, e) = channel(setup, factory).unwrap();
        let mut rig = Rig {
            p,
            e,
            frames: 0,
            hosted,
        };
        rig.send(&[
            M::Reset { epoch: 1 },
            M::SetTransport {
                tempo: Tempo::new(120.0).unwrap(),
                signature: TimeSignature::new(4, 4).unwrap(),
                launch_quantize: Quantize::default(),
            },
            M::SetMetronome {
                enabled: false,
                volume: 1.0,
                count_in_bars: 0,
            },
            M::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
            M::TrackArm {
                track: TrackSlot(0),
                armed: true,
            },
        ]);
        rig
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p.handle(m.clone()).unwrap();
        }
        self.p.flush();
    }

    /// `blocks` blocks of 128 frames.
    fn blocks(&mut self, blocks: u64) {
        for _ in 0..blocks {
            if self.hosted {
                self.e.set_host_frame(ORIGIN + self.frames);
            }
            let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
            assert_no_alloc(|| self.e.process(&mut l, &mut r));
            self.p.collect();
            self.frames += BLOCK as u64;
        }
    }

    /// Renders until the next block starts at or after the engine sample `sample`.
    fn until(&mut self, sample: u64) {
        while self.frames < sample {
            self.blocks(1);
        }
    }

    /// The host frame of the engine sample `sample`.
    fn host(&self, sample: u64) -> u64 {
        if self.hosted { ORIGIN + sample } else { sample }
    }

    fn on(&mut self, pitch: u8, at: Option<u64>) {
        self.send(&[M::NoteOn {
            track: TrackSlot(0),
            pitch,
            velocity: 100,
            at,
        }]);
    }

    fn off(&mut self, pitch: u8, at: Option<u64>) {
        self.send(&[M::NoteOff {
            track: TrackSlot(0),
            pitch,
            at,
        }]);
    }

    /// Starts recording from tick `from`: returns the engine sample the take plays `from` at
    /// (the start of the next block, which applies the order).
    fn record(&mut self, from: i64) -> u64 {
        self.send(&[
            M::SetRecording { recording: true },
            M::Play { from: Tick(from) },
        ]);
        self.frames
    }

    fn notes(&self) -> Vec<RecordedNote> {
        let mut out = vec![];
        self.e.recorder().recent(|_, n| out.push(*n));
        out
    }
}

/// The error of a recorded start against the tick played at `sample`, in milliseconds.
fn error_ms(start: Tick, played: f64) -> f64 {
    (start.0 as f64 - played) * SPT / RATE * 1000.0
}

#[test]
fn a_note_played_on_a_beat_is_recorded_on_it_within_a_millisecond_however_late_it_arrives() {
    for hosted in [true, false] {
        for late_blocks in [0u64, 1, 5, 37, 300] {
            let mut rig = Rig::new(hosted);
            rig.blocks(3);
            let zero = rig.record(0);
            // Beats 1 to 4 of the take, each a little off the beat by a few samples too.
            for (beat, jitter) in [(1u64, 0u64), (2, 13), (3, 24), (4, 1)] {
                let played = zero + beat * BEAT + jitter;
                // The note reaches the engine `late_blocks` blocks after it was played.
                rig.until(played + late_blocks * BLOCK as u64);
                rig.on(60, Some(rig.host(played)));
                rig.blocks(1);
                // Released a quarter of a beat later, and late as well.
                let released = played + BEAT / 4;
                rig.until(released + late_blocks * BLOCK as u64);
                rig.off(60, Some(rig.host(released)));
                rig.blocks(1);
            }
            let notes = rig.notes();
            assert_eq!(notes.len(), 4, "{notes:?}");
            for (i, (beat, jitter)) in [(1i64, 0u64), (2, 13), (3, 24), (4, 1)]
                .into_iter()
                .enumerate()
            {
                let played = (beat * PPQ) as f64 + jitter as f64 / SPT;
                let note = notes[i].note;
                let error = error_ms(note.start, played);
                assert!(
                    error.abs() <= 1.0,
                    "hosted {hosted}, {late_blocks} blocks late, beat {beat}: {error} ms ({note:?})"
                );
                // On the beat itself: the tick of the beat exactly.
                if jitter == 0 {
                    assert_eq!(note.start, Tick(beat * PPQ));
                }
                assert_eq!(note.length, Tick(PPQ / 4), "the played length");
            }
            assert_eq!(rig.e.recorder().unplaced(), 0);
        }
    }
}

#[test]
fn an_unstamped_note_is_placed_at_the_block_that_takes_it_as_before() {
    let mut rig = Rig::new(true);
    let zero = rig.record(0);
    let played = zero + BEAT;
    rig.until(played + 5 * BLOCK as u64);
    let taken = rig.frames;
    rig.on(60, None);
    rig.blocks(10);
    rig.off(60, None);
    rig.blocks(1);
    let note = rig.notes()[0].note;
    assert_eq!(
        note.start,
        Tick(((taken - zero) as f64 / SPT).floor() as i64)
    );
    // Five blocks late: 13 ms, far outside ±1 ms. The stamp is what fixes it.
    assert!(error_ms(note.start, PPQ as f64) > 10.0);
}

#[test]
fn a_stamp_within_a_second_of_its_block_either_way_is_used_and_one_further_counts_from_the_block() {
    let mut rig = Rig::new(true);
    let zero = rig.record(0);
    rig.until(zero + BEAT);
    let block = rig.frames;
    let bound = (MAX_STAMP_DISTANCE_SECONDS * RATE) as u64;
    // Inside the block that takes it: placed at the stamp.
    rig.on(60, Some(rig.host(block + 100)));
    rig.blocks(1);
    // Ahead of its block (the page's latency estimate larger than the real lead of the
    // rendering over the output): placed at the stamp, by the current anchor.
    let ahead = rig.frames;
    rig.on(62, Some(rig.host(ahead + 480)));
    rig.blocks(1);
    // Ahead by exactly the bound: placed; by more: the start of its block.
    let far = rig.frames;
    rig.on(63, Some(rig.host(far + bound)));
    rig.blocks(1);
    let too_far = rig.frames;
    rig.on(66, Some(rig.host(too_far + bound + 1)));
    rig.blocks(1);
    // Older than the bound (a stalled page): the start of its block.
    rig.until(zero + 3 * BEAT);
    let stale_block = rig.frames;
    rig.on(64, Some(rig.host(stale_block - bound - 1)));
    rig.blocks(1);
    // Exactly at the bound: still placed.
    let edge_block = rig.frames;
    rig.on(65, Some(rig.host(edge_block - bound)));
    rig.blocks(1);
    rig.send(&[M::SetRecording { recording: false }]);
    rig.blocks(1);
    let mut notes = rig.notes();
    notes.sort_by_key(|n| n.note.pitch);
    let tick = |sample: u64| Tick(((sample - zero) as f64 / SPT).floor() as i64);
    let starts: Vec<(u8, Tick)> = notes.iter().map(|n| (n.note.pitch, n.note.start)).collect();
    assert_eq!(
        starts,
        [
            (60, tick(block + 100)),
            (62, tick(ahead + 480)),
            (63, tick(far + bound)),
            (64, tick(stale_block)),
            (65, tick(edge_block - bound)),
            (66, tick(too_far)),
        ]
    );
    assert_eq!(
        rig.e.recorder().unplaced(),
        2,
        "the stamp too far ahead and the stale one"
    );
}

#[test]
fn a_note_played_before_the_take_counts_from_its_first_sample() {
    let mut rig = Rig::new(true);
    rig.blocks(40);
    let before = rig.frames - 1000;
    let zero = rig.record(1920);
    rig.blocks(2);
    // Played 1000 samples before the take began, taken two blocks into it.
    rig.on(60, Some(rig.host(before)));
    rig.blocks(1);
    rig.off(60, None);
    rig.blocks(1);
    let note = rig.notes()[0].note;
    assert_eq!(
        note.start,
        Tick(1920),
        "the take's first tick, as before stamps"
    );
    assert_eq!(
        note.length,
        Tick(((rig.frames - BLOCK as u64 - zero) as f64 / SPT).round() as i64)
    );
}

#[test]
fn a_note_played_just_before_a_loop_wrap_lands_at_the_end_of_the_loop() {
    let mut rig = Rig::new(true);
    rig.send(&[M::SetLoop {
        region: LoopRegion::new(Tick(0), Tick(4 * PPQ)),
    }]);
    let zero = rig.record(0);
    let wrap = zero + 4 * BEAT;
    // Played 2 ms (96 samples) before the loop's end, taken three blocks after the wrap.
    let played = wrap - 96;
    rig.until(wrap + 3 * BLOCK as u64);
    rig.on(60, Some(rig.host(played)));
    rig.blocks(1);
    // Released 1 ms after the wrap.
    rig.off(60, Some(rig.host(wrap + 48)));
    rig.blocks(1);
    let note = rig.notes()[0].note;
    let expected = 4.0 * PPQ as f64 - 96.0 / SPT;
    assert!(
        error_ms(note.start, expected).abs() <= 1.0,
        "{note:?} against {expected}"
    );
    assert_eq!(note.start, Tick(4 * PPQ - 4), "3 836.16 ticks, floored");
    assert_eq!(note.length, Tick(((96 + 48) as f64 / SPT).round() as i64));
}

#[test]
fn a_tempo_change_between_the_note_and_its_block_keeps_the_old_tempo_for_it() {
    let mut rig = Rig::new(true);
    let zero = rig.record(0);
    let played = zero + BEAT;
    rig.until(played + 2 * BLOCK as u64);
    let change = rig.frames;
    rig.send(&[M::SetTransport {
        tempo: Tempo::new(60.0).unwrap(),
        signature: TimeSignature::new(4, 4).unwrap(),
        launch_quantize: Quantize::default(),
    }]);
    rig.blocks(2);
    rig.on(60, Some(rig.host(played)));
    rig.blocks(1);
    rig.off(60, None);
    rig.blocks(1);
    assert!(change > played);
    assert_eq!(rig.notes()[0].note.start, Tick(PPQ), "placed at 120 BPM");
}

#[test]
fn a_note_played_before_a_seek_counts_from_the_seek() {
    let mut rig = Rig::new(true);
    let zero = rig.record(0);
    let played = zero + BEAT;
    rig.until(played + BLOCK as u64);
    rig.send(&[M::Seek {
        tick: Tick(8 * PPQ),
    }]);
    let seek = rig.frames;
    rig.blocks(2);
    rig.on(60, Some(rig.host(played)));
    rig.blocks(1);
    rig.off(60, None);
    rig.blocks(1);
    assert!(seek > played);
    assert_eq!(
        rig.notes()[0].note.start,
        Tick(8 * PPQ),
        "the playhead jumped: nothing before the seek is placed"
    );
}

#[test]
fn a_second_press_ends_the_first_at_its_own_stamp() {
    let mut rig = Rig::new(true);
    let zero = rig.record(0);
    let first = zero + BEAT;
    let second = first + 2400;
    // Both presses (no release in between: a lost note off) reach the engine in one block.
    rig.until(second + 4 * BLOCK as u64);
    rig.on(60, Some(rig.host(first)));
    rig.on(60, Some(rig.host(second)));
    rig.blocks(1);
    rig.until(second + 1200 + BLOCK as u64);
    rig.off(60, Some(rig.host(second + 1200)));
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!(notes[0].note.start, Tick(PPQ));
    assert_eq!(
        notes[0].note.length,
        Tick(96),
        "2 400 samples: up to the second press"
    );
    assert_eq!(notes[1].note.start, Tick(PPQ + 96));
    assert_eq!(notes[1].note.length, Tick(48));
}

#[test]
fn a_stamp_before_the_hosts_first_engine_sample_counts_from_its_block() {
    let mut rig = Rig::new(true);
    let zero = rig.record(0);
    rig.blocks(10);
    let block = rig.frames;
    // A host frame below the origin maps to no engine sample.
    rig.on(60, Some(ORIGIN - 10));
    rig.blocks(1);
    rig.off(60, None);
    rig.blocks(1);
    assert_eq!(
        rig.notes()[0].note.start,
        Tick(((block - zero) as f64 / SPT).floor() as i64)
    );
    assert_eq!(rig.e.recorder().unplaced(), 0, "no engine sample: no stamp");
}
