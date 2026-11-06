//! Live notes stamped with the host frame they were played at (#499, `engine::recorder`,
//! "The sample a note was played at"): offline renders through the real `Preparer`,
//! `Engine::apply` and `Engine::render`, every audio-side call under the allocation detector.
//!
//! At 120 BPM and 48 kHz a beat is 24 000 samples and a tick 25 samples (0.52 ms); every
//! block is 128 frames (the web render quantum, 2.67 ms). The host's clock runs
//! [`ORIGIN`] frames ahead of the engine's sample counter, as an audio context that ran
//! before the engine did; the rig tells the engine the host frame of each block
//! (`Engine::set_host_frame`), as the web worklet does with `currentFrame`.

use assert_no_alloc::assert_no_alloc;
use engine::device::DeviceSetup;
use engine::device::testing::factory;
use engine::engine::{Engine, Preparer, channel};
use engine::msg::{ControlMsg as M, RecordedNote, TrackKind, TrackSlot, limits};
use engine::recorder::MAX_STAMP_DISTANCE_SECONDS;
use engine::time::{LoopRegion, Quantize, Tempo, Tick, TimeSignature};

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
        self.record_shifted(from, 0)
    }

    /// [`Rig::record`] with the take's shift (`SetRecording::shift_us`, #560).
    fn record_shifted(&mut self, from: i64, shift_us: i32) -> u64 {
        self.send(&[
            M::SetRecording {
                recording: true,
                shift_us,
            },
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
    rig.send(&[M::SetRecording {
        recording: false,
        shift_us: 0,
    }]);
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

/// #542: the web page stamps the frame heard plus the output latency (42 ms). In a take without
/// a shift, a note heard 20 ms before the loop's end is stamped 22 ms after the wrap, so the
/// engine places it near the loop's START: the tick `record.commit_take` brings back to the
/// loop's end when it is given the shift (`crates/commands/tests/it/record_loop.rs`). The page's
/// takes carry the shift since #560 (below).
#[test]
fn a_note_heard_just_before_the_loop_end_is_placed_after_the_wrap_by_the_latency() {
    let mut rig = Rig::new(true);
    rig.send(&[M::SetLoop {
        region: LoopRegion::new(Tick(0), Tick(4 * PPQ)),
    }]);
    let zero = rig.record(0);
    let wrap = zero + 4 * BEAT;
    // Heard 20 ms (960 samples) before the end; the stamp adds 42 ms (2016 samples).
    let stamp = wrap - 960 + 2016;
    rig.until(wrap + 3 * BLOCK as u64);
    rig.on(60, Some(rig.host(stamp)));
    rig.blocks(1);
    rig.off(60, Some(rig.host(stamp + 480)));
    rig.blocks(1);
    let note = rig.notes()[0].note;
    // 1 056 samples past the wrap: 42.24 ticks, floored.
    assert_eq!(note.start, Tick(42));
    assert_eq!(rig.e.recorder().unplaced(), 0);
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

// ---------------------------------------------------------------- the take's shift (#560)

/// The page's estimate of the output latency in these tests: 42 ms, 2 016 samples.
const LATENCY_US: i32 = 42_000;
const LATENCY: u64 = 2_016;

/// The criterion of #542, applied in the engine: a note heard 20 ms before the end of the loop,
/// stamped 42 ms later (after the wrap), is placed 20 ms before the end of the loop.
#[test]
fn with_the_shift_a_note_heard_just_before_the_loop_end_stays_there() {
    for late_blocks in [0u64, 3, 20] {
        let mut rig = Rig::new(true);
        rig.send(&[M::SetLoop {
            region: LoopRegion::new(Tick(0), Tick(4 * PPQ)),
        }]);
        let zero = rig.record_shifted(0, LATENCY_US);
        let wrap = zero + 4 * BEAT;
        // Heard 20 ms (960 samples) before the end.
        let heard = wrap - 960;
        let stamp = heard + LATENCY;
        rig.until(stamp + late_blocks * BLOCK as u64);
        rig.on(60, Some(rig.host(stamp)));
        rig.blocks(1);
        rig.until(stamp + 480 + late_blocks * BLOCK as u64);
        rig.off(60, Some(rig.host(stamp + 480)));
        rig.blocks(1);
        let note = rig.notes()[0].note;
        let expected = 4.0 * PPQ as f64 - 960.0 / SPT;
        assert!(
            error_ms(note.start, expected).abs() <= 1.0,
            "{late_blocks} blocks late: {note:?} against {expected}"
        );
        assert_eq!(note.start, Tick(4 * PPQ - 39), "3 801.6 ticks, floored");
        assert_eq!(note.length, Tick(19), "480 samples: the length as played");
        assert_eq!(rig.e.recorder().unplaced(), 0);
    }
}

/// #560: the playhead enters the loop from before it. A note heard 10 ms before the loop's
/// start on that first pass is stamped 32 ms inside the loop: it stays before the loop's start,
/// where it was heard (a command could not tell it from a stamp the engine wrapped). The same
/// take then wraps, and a note heard 20 ms before the loop's end stays there.
#[test]
fn with_the_shift_a_note_heard_before_the_loop_on_the_first_pass_stays_before_it() {
    let mut rig = Rig::new(true);
    rig.send(&[M::SetLoop {
        region: LoopRegion::new(Tick(2 * PPQ), Tick(6 * PPQ)),
    }]);
    let zero = rig.record_shifted(0, LATENCY_US);
    let entry = zero + 2 * BEAT;
    let heard = entry - 480;
    let stamp = heard + LATENCY;
    rig.until(stamp + 2 * BLOCK as u64);
    rig.on(60, Some(rig.host(stamp)));
    rig.blocks(1);
    rig.off(60, Some(rig.host(stamp + 240)));
    rig.blocks(1);
    // The first wrap, four beats after the loop's start.
    let wrap = entry + 4 * BEAT;
    let heard_end = wrap - 960;
    let stamp_end = heard_end + LATENCY;
    rig.until(stamp_end + 2 * BLOCK as u64);
    rig.on(62, Some(rig.host(stamp_end)));
    rig.blocks(1);
    rig.off(62, Some(rig.host(stamp_end + 240)));
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 2, "{notes:?}");
    let before = notes[0].note;
    assert!(
        error_ms(before.start, 2.0 * PPQ as f64 - 480.0 / SPT).abs() <= 1.0,
        "{before:?}"
    );
    assert_eq!(
        before.start,
        Tick(2 * PPQ - 20),
        "1 900.8 ticks, floored: before the loop"
    );
    let end = notes[1].note;
    assert!(
        error_ms(end.start, 6.0 * PPQ as f64 - 960.0 / SPT).abs() <= 1.0,
        "{end:?}"
    );
    assert_eq!(end.start, Tick(6 * PPQ - 39));
    assert_eq!((before.length, end.length), (Tick(10), Tick(10)));
}

/// A note heard less than one shift before the take began (its stamp inside the take): kept
/// where it was heard, before the take's first tick (on the line back from the take's `Play`).
#[test]
fn with_the_shift_a_note_heard_just_before_the_take_stays_where_it_was_heard() {
    let mut rig = Rig::new(true);
    rig.blocks(40);
    let zero = rig.record_shifted(1920, LATENCY_US);
    // Heard 1 000 samples before the take began: stamped 1 016 samples into it.
    let stamp = zero + LATENCY - 1000;
    rig.until(stamp + BLOCK as u64);
    rig.on(60, Some(rig.host(stamp)));
    rig.blocks(1);
    rig.off(60, Some(rig.host(stamp + 2400)));
    rig.blocks(1);
    let note = rig.notes()[0].note;
    assert_eq!(
        note.start,
        Tick(1920 - 40),
        "1 000 samples before the take's first tick"
    );
    assert_eq!(note.length, Tick(96), "2 400 samples, as played");
    // Heard more than one shift before: the stamp counts from the take's first sample (as
    // before the shift), then the shift takes it one shift before the take's first tick.
    let mut rig = Rig::new(true);
    rig.blocks(40);
    let early = rig.frames - 3000;
    let zero = rig.record_shifted(1920, LATENCY_US);
    rig.blocks(2);
    rig.on(60, Some(rig.host(early + LATENCY)));
    rig.blocks(1);
    rig.off(60, Some(rig.host(zero + 4 * BLOCK as u64)));
    rig.blocks(1);
    let note = rig.notes()[0].note;
    assert_eq!(
        note.start,
        Tick(1920 - 81),
        "2 016 samples (80.64 ticks), floored"
    );
}

/// The ticket's case after a count-in: the take begins on the loop's start, and the first note,
/// heard 10 ms early, stays 10 ms before the loop's start.
#[test]
fn with_the_shift_a_downbeat_anticipated_after_a_count_in_stays_before_the_loop() {
    let mut rig = Rig::new(true);
    rig.send(&[
        M::SetMetronome {
            enabled: false,
            volume: 1.0,
            count_in_bars: 1,
        },
        M::SetLoop {
            region: LoopRegion::new(Tick(4 * PPQ), Tick(8 * PPQ)),
        },
    ]);
    // The count-in plays the bar before the cue: the cue is reached four beats later.
    let count_in = rig.record_shifted(4 * PPQ, LATENCY_US);
    let cue = count_in + 4 * BEAT;
    let heard = cue - 480;
    let stamp = heard + LATENCY;
    rig.until(stamp + 2 * BLOCK as u64);
    rig.on(60, Some(rig.host(stamp)));
    rig.blocks(1);
    rig.off(60, Some(rig.host(stamp + 240)));
    rig.blocks(1);
    let note = rig.notes()[0].note;
    assert_eq!(
        note.start,
        Tick(4 * PPQ - 20),
        "1 900.8 ticks past the bar before, floored"
    );
    assert_eq!(note.length, Tick(10));
}

/// A negative shift (an offset larger than the latency) moves a note ahead of its block: past
/// the loop's end, it is folded into the loop, where the playhead will be.
#[test]
fn a_negative_shift_past_the_loop_end_is_folded_into_the_loop() {
    let mut rig = Rig::new(true);
    rig.send(&[M::SetLoop {
        region: LoopRegion::new(Tick(0), Tick(4 * PPQ)),
    }]);
    // 20 ms of latency, -100 ms of offset: -80 ms.
    let zero = rig.record_shifted(0, -80_000);
    let wrap = zero + 4 * BEAT;
    // Heard 30 ms (1 440 samples) before the loop's end, stamped 20 ms later, before the wrap.
    let stamp = wrap - 1440 + 960;
    rig.until(stamp);
    rig.on(60, Some(rig.host(stamp)));
    rig.blocks(1);
    rig.off(60, Some(rig.host(stamp + 240)));
    rig.blocks(1);
    let note = rig.notes()[0].note;
    // 100 ms after it was heard: 3 360 samples past the wrap, 134.4 ticks.
    assert_eq!(note.start, Tick(134));
    assert_eq!(note.length, Tick(10));
}

/// A new sample rate during a take: the shift keeps its milliseconds (counted again in samples
/// of the new rate), and a held note keeps its length.
#[test]
fn the_shift_follows_a_sample_rate_change_during_the_take() {
    let run = |shift_us: i32| {
        let mut rig = Rig::new(false);
        rig.record_shifted(0, shift_us);
        rig.blocks(10);
        rig.on(62, None);
        rig.blocks(10);
        rig.p
            .set_sample_rate(engine::time::SampleRate::HZ_96000)
            .unwrap();
        rig.blocks(10);
        rig.on(60, None);
        rig.blocks(10);
        rig.off(60, None);
        rig.off(62, None);
        rig.blocks(2);
        let mut notes = rig.notes();
        notes.sort_by_key(|n| n.note.pitch);
        notes.iter().map(|n| n.note).collect::<Vec<_>>()
    };
    let plain = run(0);
    let shifted = run(LATENCY_US);
    // 42 ms at 120 BPM: 80.64 ticks, whatever the rate.
    let moved = (plain[0].start.0 - shifted[0].start.0) as f64;
    assert!((moved - 80.64).abs() <= 2.0, "{plain:?} {shifted:?}");
    let moved = (plain[1].start.0 - shifted[1].start.0) as f64;
    assert!((moved - 80.64).abs() <= 2.0, "{plain:?} {shifted:?}");
    assert_eq!(plain[0].length, shifted[0].length);
    assert!(
        (plain[1].length.0 - shifted[1].length.0).abs() <= 1,
        "{plain:?} {shifted:?}"
    );
}

/// Without a stamp the note counts from the start of its block, moved back by the shift too;
/// a negative shift (a manual offset larger than the latency) places the notes later.
#[test]
fn the_shift_moves_unstamped_notes_and_a_negative_shift_places_later() {
    for (shift_us, samples) in [(LATENCY_US, -(LATENCY as i64)), (-10_000, 480i64)] {
        let mut rig = Rig::new(true);
        let zero = rig.record_shifted(0, shift_us);
        rig.until(zero + BEAT);
        let taken = rig.frames;
        rig.on(60, None);
        rig.blocks(4);
        rig.off(60, None);
        rig.blocks(1);
        let note = rig.notes()[0].note;
        let at = taken as i64 - zero as i64 + samples;
        assert_eq!(
            note.start,
            Tick((at as f64 / SPT).floor() as i64),
            "shift {shift_us} us"
        );
        assert_eq!(note.length, Tick((4.0 * BLOCK as f64 / SPT).round() as i64));
    }
}

/// The notes still held when the take ends are cut on the take's clock: a note keeps the
/// length it was held as heard, the shift does not lengthen it.
#[test]
fn with_the_shift_a_note_cut_by_the_end_of_the_take_keeps_its_heard_length() {
    let mut rig = Rig::new(true);
    let zero = rig.record_shifted(0, LATENCY_US);
    let stamp = zero + BEAT + LATENCY;
    rig.until(stamp + BLOCK as u64);
    rig.on(60, Some(rig.host(stamp)));
    rig.blocks(1);
    rig.until(stamp + BEAT);
    let cut = rig.frames;
    rig.send(&[M::SetRecording {
        recording: false,
        shift_us: 0,
    }]);
    rig.blocks(1);
    let note = rig.notes()[0].note;
    assert_eq!(note.start, Tick(PPQ));
    assert_eq!(
        note.length,
        Tick(((cut - stamp) as f64 / SPT).round() as i64),
        "the end of the capture, as heard, minus the note's start, as heard"
    );
}

/// A shift beyond the engine's bound is clamped (2 s), never a panic; a take keeps the shift
/// it began with, whatever a later `SetRecording` says.
#[test]
fn an_out_of_range_shift_is_clamped_and_a_take_keeps_its_own() {
    let mut rig = Rig::new(true);
    rig.blocks(1000);
    let zero = rig.record_shifted(0, i32::MAX);
    rig.blocks(1);
    // A second order while the take runs changes nothing for it.
    rig.send(&[M::SetRecording {
        recording: true,
        shift_us: 0,
    }]);
    rig.until(zero + 5 * BEAT);
    let taken = rig.frames;
    rig.on(60, None);
    rig.blocks(1);
    rig.off(60, None);
    rig.blocks(1);
    // Two seconds (96 000 samples) before its block.
    let at = taken - zero - 96_000;
    assert_eq!(
        rig.notes()[0].note.start,
        Tick((at as f64 / SPT).floor() as i64)
    );
    let mut rig = Rig::new(true);
    rig.blocks(10);
    let zero = rig.record_shifted(0, i32::MIN);
    rig.until(zero + BEAT);
    rig.on(60, None);
    rig.blocks(1);
    rig.off(60, None);
    rig.blocks(1);
    let note = rig.notes()[0].note;
    assert!(note.start.0 >= 2 * 2 * PPQ, "two seconds later: {note:?}");
}

// ---------------------------------------------------------------- the count-in (#723)

impl Rig {
    /// Records from tick 0 after a one-bar count-in with the take's shift: returns the engine
    /// sample of the cue (one bar after the play).
    fn count_in(&mut self, shift_us: i32) -> u64 {
        self.send(&[M::SetMetronome {
            enabled: false,
            volume: 1.0,
            count_in_bars: 1,
        }]);
        self.record_shifted(0, shift_us) + 4 * BEAT
    }

    /// Renders up to the block before the cue's (the last that ends before the cue, the
    /// count-in's last), so that the next event is taken during the count-in.
    fn until_last_count_in_block(&mut self, cue: u64) {
        self.until(cue - 2 * BLOCK as u64);
    }
}

/// The ticket's criterion: after a one-bar count-in, a downbeat heard 35 ms early, stamped
/// with the page's 42 ms estimate, whose event reaches the engine during the count-in (the
/// rendering runs 31 ms ahead of the output: the event comes before the cue's block), is
/// recorded where it was heard, 67.2 ticks before beat 1 (`record.commit_take`'s early rule
/// then writes it on beat 1, `test-support`'s `record_loop`); one heard 130 ms early (more than
/// a sixteenth, 125 ms) is not a note of the take, and not counted as a lost one.
#[test]
fn after_a_count_in_a_downbeat_anticipated_by_35_ms_is_recorded() {
    for hosted in [true, false] {
        let mut rig = Rig::new(hosted);
        rig.blocks(10);
        let cue = rig.count_in(LATENCY_US);
        let too_early = cue - 6_240;
        rig.until(too_early);
        rig.on(59, Some(rig.host(too_early + LATENCY)));
        rig.blocks(1);
        let heard = cue - 1_680;
        rig.until_last_count_in_block(cue);
        rig.on(60, Some(rig.host(heard + LATENCY)));
        rig.blocks(1);
        assert!(
            !rig.e.recorder().capturing(),
            "taken during the count-in, before the capture began"
        );
        rig.blocks(1);
        assert!(rig.e.recorder().capturing());
        rig.until(heard + 6_000 + 3 * BLOCK as u64);
        rig.off(60, Some(rig.host(heard + 6_000 + LATENCY)));
        rig.off(59, None);
        rig.blocks(1);
        let notes = rig.notes();
        assert_eq!(notes.len(), 1, "hosted {hosted}: {notes:?}");
        assert_eq!(notes[0].note.pitch, 60);
        assert_eq!(
            notes[0].note.start,
            Tick(-68),
            "1 680 samples before the cue"
        );
        assert_eq!(notes[0].note.length, Tick(240), "6 000 samples, as played");
        assert_eq!(
            rig.e.recorder().dropped(),
            0,
            "the note heard 130 ms early is ignored, not lost"
        );
        assert_eq!(rig.e.recorder().unplaced(), 0);
    }
}

/// The tolerance's edges, heard exactly a sixteenth before the cue (kept) and one sample more
/// (ignored, not counted); without a shift, as the take places them.
#[test]
fn the_count_in_keeps_a_note_heard_a_sixteenth_before_the_cue_and_no_earlier() {
    let sixteenth = BEAT / 4;
    for (early, kept) in [(sixteenth, true), (sixteenth + 1, false)] {
        let mut rig = Rig::new(true);
        rig.blocks(3);
        let cue = rig.count_in(0);
        let heard = cue - early;
        rig.until(heard + BLOCK as u64);
        rig.on(60, Some(rig.host(heard)));
        rig.blocks(1);
        rig.off(60, Some(rig.host(heard + 2_500)));
        rig.blocks(1);
        rig.until(cue + BLOCK as u64);
        assert!(rig.e.recorder().capturing());
        let notes = rig.notes();
        if kept {
            assert_eq!(notes.len(), 1, "{notes:?}");
            assert_eq!(notes[0].note.start, Tick(-PPQ / 4));
            assert_eq!(notes[0].note.length, Tick(100));
            assert_eq!(rig.e.recorder().dropped(), 0);
        } else {
            assert!(notes.is_empty(), "{notes:?}");
            assert_eq!(rig.e.recorder().dropped(), 0);
        }
    }
}

/// A `SetRecording` that changes the shift during the count-in: the take begins with the new
/// shift, and a note of the count-in held across the cue moves with it, so that it keeps the
/// length it was played with.
#[test]
fn a_shift_changed_in_the_count_in_moves_its_held_notes_with_it() {
    let mut rig = Rig::new(true);
    rig.blocks(3);
    let cue = rig.count_in(LATENCY_US);
    let heard = cue - 1_000;
    rig.until_last_count_in_block(cue);
    rig.on(60, Some(rig.host(heard)));
    rig.blocks(1);
    assert!(!rig.e.recorder().capturing());
    // 10 ms of shift less, before the cue.
    rig.send(&[M::SetRecording {
        recording: true,
        shift_us: LATENCY_US - 10_000,
    }]);
    rig.blocks(1);
    assert!(rig.e.recorder().capturing());
    rig.until(cue + BEAT);
    rig.off(60, Some(rig.host(heard + 12_000)));
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].note.length, Tick(480), "12 000 samples, as played");
}

/// A note of the count-in released just before the cue, whose note off reaches the engine once
/// the take began (stamps behind the block, as a native host gives them): it ends where it was
/// released, before the take's first sample, not at that sample.
#[test]
fn a_count_in_note_released_before_the_cue_ends_at_its_release_after_a_late_note_off() {
    let mut rig = Rig::new(true);
    rig.blocks(3);
    let cue = rig.count_in(0);
    let played = cue - 2_000;
    rig.until(played + BLOCK as u64);
    rig.on(60, Some(rig.host(played)));
    rig.blocks(1);
    assert!(!rig.e.recorder().capturing());
    // Released 1 000 samples later (before the cue), taken some blocks after the cue.
    rig.until(cue + 4 * BLOCK as u64);
    assert!(rig.e.recorder().capturing());
    rig.off(60, Some(rig.host(played + 1_000)));
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].note.start, Tick(-80));
    assert_eq!(notes[0].note.length, Tick(40), "1 000 samples, as played");
    // In the take, a note played before it whose event arrives in it still counts from the
    // take's first sample.
    let early = cue - 3_000;
    rig.on(62, Some(rig.host(early)));
    rig.blocks(1);
    rig.off(62, Some(rig.host(cue + 5 * BLOCK as u64 + 500)));
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[1].note.start >= Tick(-6), "{notes:?}");
}

/// A stamp the count-in cannot use (further than a second from its block) counts for the take
/// in `unplaced`, and its note counts from the start of its block, as during a take.
#[test]
fn an_unusable_stamp_in_the_count_in_counts_from_its_block() {
    let mut rig = Rig::new(true);
    rig.blocks(3);
    let cue = rig.count_in(0);
    rig.until_last_count_in_block(cue);
    let taken = rig.frames;
    rig.on(60, Some(rig.host(taken + 10 * RATE as u64)));
    rig.blocks(1);
    assert!(!rig.e.recorder().capturing());
    rig.blocks(1);
    assert!(rig.e.recorder().capturing());
    assert_eq!(rig.e.recorder().unplaced(), 1);
    rig.off(60, None);
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    let start = -(((cue - taken) as f64 / SPT).ceil() as i64);
    assert_eq!(notes[0].note.start, Tick(start), "the start of its block");
    assert_eq!(rig.e.recorder().dropped(), 0);
}
