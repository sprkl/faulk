//! MIDI recording in the engine (RE-2, `engine::recorder`): offline renders through the real
//! `Preparer`, `Engine::apply` and `Engine::render`, every audio-side call under the
//! allocation detector. At 120 BPM and 48 kHz a tick is 25 samples, and every block here is
//! 128 frames (the Web render quantum), so the stamp of a note is the playhead of the start
//! of its block, known exactly.

use assert_no_alloc::assert_no_alloc;
use engine::device::DeviceSetup;
use engine::device::testing::factory;
use engine::engine::{Engine, Preparer, channel};
use engine::host::{channel as host_channel, read_feedback};
use engine::msg::{ControlMsg as M, RecordedNote, TrackKind, TrackSlot, encode_stream, limits};
use engine::recorder::MAX_HELD;
use engine::time::{LoopRegion, Quantize, SampleRate, Tempo, Tick, TimeSignature};

const BLOCK: usize = 128;
/// Samples per tick at 120 BPM, 48 kHz.
const SPT: f64 = 25.0;

struct Rig {
    p: Preparer,
    e: Engine,
    frames: usize,
}

impl Rig {
    /// Three MIDI tracks (slots 0, 1, 2), slots 0 and 2 armed, 120 BPM in 4/4, no count-in.
    fn new() -> Rig {
        let setup = DeviceSetup {
            sample_rate: 48_000.0,
            max_block_frames: limits::MAX_BLOCK_FRAMES,
        };
        let (p, e) = channel(setup, factory).unwrap();
        let mut rig = Rig { p, e, frames: 0 };
        let mut msgs = vec![
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
        ];
        for t in 0..3 {
            msgs.push(M::TrackCreate {
                track: TrackSlot(t),
                kind: TrackKind::Midi,
            });
        }
        msgs.push(M::TrackArm {
            track: TrackSlot(0),
            armed: true,
        });
        msgs.push(M::TrackArm {
            track: TrackSlot(2),
            armed: true,
        });
        rig.send(&msgs);
        rig
    }

    fn send(&mut self, msgs: &[M]) {
        for m in msgs {
            self.p.handle(m.clone()).unwrap();
        }
        self.p.flush();
    }

    /// `blocks` blocks of 128 frames.
    fn blocks(&mut self, blocks: usize) {
        for _ in 0..blocks {
            let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
            assert_no_alloc(|| self.e.process(&mut l, &mut r));
            self.p.collect();
            self.frames += BLOCK;
        }
    }

    /// The playhead tick of the start of the next block, from the frames rendered since the
    /// play from tick `from`.
    fn tick_since(&self, played: usize, from: i64) -> i64 {
        from + ((self.frames - played) as f64 / SPT).floor() as i64
    }

    fn on(&mut self, track: u32, pitch: u8) {
        self.send(&[M::NoteOn {
            track: TrackSlot(track),
            pitch,
            velocity: 100,
            at: None,
        }]);
    }

    fn off(&mut self, track: u32, pitch: u8) {
        self.send(&[M::NoteOff {
            track: TrackSlot(track),
            pitch,
            at: None,
        }]);
    }

    fn record(&mut self, from: i64) -> usize {
        self.send(&[
            M::SetRecording {
                recording: true,
                shift_us: 0,
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

#[test]
fn a_note_on_an_armed_track_is_stamped_at_its_block_and_keeps_its_played_length() {
    let mut rig = Rig::new();
    let played = rig.record(0);
    rig.blocks(10);
    let start = rig.tick_since(played, 0);
    rig.on(0, 60);
    // The selected but unarmed track 1 plays the same key: heard, not recorded.
    rig.on(1, 60);
    rig.blocks(30);
    rig.off(0, 60);
    rig.off(1, 60);
    rig.blocks(2);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].track, TrackSlot(0));
    assert_eq!(notes[0].note.start, Tick(start));
    assert_eq!(
        notes[0].note.length,
        Tick((30.0 * BLOCK as f64 / SPT).round() as i64)
    );
    assert_eq!((notes[0].note.pitch, notes[0].note.velocity), (60, 100));
    assert_eq!(rig.e.recorder().take(), 1);
    assert!(rig.e.recorder().capturing());
}

#[test]
fn nothing_is_captured_unless_the_transport_records() {
    let mut rig = Rig::new();
    // Playing without recording.
    rig.send(&[M::Play { from: Tick(0) }]);
    rig.blocks(2);
    rig.on(0, 60);
    rig.blocks(2);
    rig.off(0, 60);
    rig.blocks(2);
    assert!(rig.notes().is_empty());
    assert_eq!(rig.e.recorder().take(), 0);
    // Recording asked while stopped: nothing either.
    rig.send(&[
        M::Stop,
        M::SetRecording {
            recording: true,
            shift_us: 0,
        },
    ]);
    rig.blocks(2);
    rig.on(0, 61);
    rig.blocks(2);
    rig.off(0, 61);
    rig.blocks(2);
    assert!(rig.notes().is_empty());
    assert_eq!(rig.e.recorder().take(), 0);
}

#[test]
fn the_notes_held_when_the_take_ends_are_cut_there() {
    for end in [
        vec![M::SetRecording {
            recording: false,
            shift_us: 0,
        }],
        vec![M::Stop],
    ] {
        let mut rig = Rig::new();
        rig.record(0);
        rig.blocks(4);
        rig.on(0, 60);
        rig.on(2, 64);
        rig.blocks(8);
        rig.send(&end);
        rig.blocks(1);
        assert!(!rig.e.recorder().capturing());
        let notes = rig.notes();
        assert_eq!(notes.len(), 2, "{notes:?}");
        let length = Tick((8.0 * BLOCK as f64 / SPT).round() as i64);
        assert!(notes.iter().all(|n| n.note.length == length), "{notes:?}");
        // A note off after the end changes nothing; the next take starts a new count.
        rig.off(0, 60);
        rig.blocks(1);
        assert_eq!(rig.notes().len(), 2);
        rig.send(&[
            M::SetRecording {
                recording: true,
                shift_us: 0,
            },
            M::Play { from: Tick(0) },
        ]);
        rig.blocks(1);
        assert_eq!(rig.e.recorder().take(), 2);
        assert!(rig.notes().is_empty());
    }
}

#[test]
fn a_note_across_a_loop_wrap_keeps_the_length_it_was_played_with() {
    let mut rig = Rig::new();
    rig.send(&[M::SetLoop {
        region: LoopRegion::new(Tick(0), Tick(960)),
    }]);
    // 960 ticks are 24 000 samples: start 10 blocks before the end of the loop.
    let played = rig.record(0);
    rig.blocks(24_000 / BLOCK - 10);
    let start = rig.tick_since(played, 0);
    rig.on(0, 60);
    rig.blocks(40);
    rig.off(0, 60);
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].note.start, Tick(start));
    assert!(
        start > 900,
        "the note starts near the end of the loop: {start}"
    );
    assert_eq!(
        notes[0].note.length,
        Tick((40.0 * BLOCK as f64 / SPT).round() as i64)
    );
}

/// One bar of count-in at 120 BPM: 3 840 ticks, 96 000 frames, 750 blocks before the cue.
const COUNT_IN: i64 = 3840;

impl Rig {
    /// Starts recording from tick 0 after a one-bar count-in: returns the frames rendered before.
    fn count_in(&mut self) -> usize {
        self.send(&[M::SetMetronome {
            enabled: false,
            volume: 1.0,
            count_in_bars: 1,
        }]);
        self.record(0)
    }
}

/// A sixteenth at 960 ticks per beat (`engine::recorder::COUNT_IN_EARLY`, #723).
#[test]
fn the_count_in_tolerance_is_a_sixteenth() {
    assert_eq!(engine::recorder::COUNT_IN_EARLY, 240);
}

#[test]
fn the_count_in_is_heard_and_only_its_last_sixteenth_is_kept() {
    let mut rig = Rig::new();
    let played = rig.count_in();
    rig.blocks(10);
    assert!(rig.e.clock().playhead_tick() < Tick(0));
    // Played along with the count-in, long before the cue: not a note of the take, not counted.
    rig.on(0, 60);
    rig.blocks(10);
    rig.off(0, 60);
    rig.blocks(670);
    // 60 blocks (307 ticks) before the cue, held across it: not a note either, its note off ignored.
    rig.on(0, 61);
    rig.blocks(30);
    // 30 blocks (154 ticks, 80 ms) before the cue: a note of the take, where it was heard.
    let early = rig.tick_since(played, -COUNT_IN);
    assert_eq!(early, -154);
    rig.on(0, 62);
    rig.blocks(10);
    // On an unarmed track: neither kept nor counted.
    rig.on(1, 50);
    rig.blocks(10);
    assert!(!rig.e.recorder().capturing());
    assert_eq!(rig.e.recorder().take(), 0);
    rig.blocks(20);
    assert!(rig.e.recorder().capturing());
    assert_eq!(rig.e.recorder().take(), 1);
    assert_eq!(
        rig.e.recorder().dropped(),
        0,
        "notes played before the last sixteenth are not lost notes"
    );
    rig.off(0, 61);
    rig.off(0, 62);
    rig.off(1, 50);
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].track, TrackSlot(0));
    assert_eq!(notes[0].note.pitch, 62);
    assert_eq!(notes[0].note.start, Tick(early));
    assert_eq!(
        notes[0].note.length,
        Tick((40.0 * BLOCK as f64 / SPT).round() as i64),
        "its played length"
    );
    // After the cue a note is captured as before.
    rig.on(0, 64);
    rig.blocks(4);
    rig.off(0, 64);
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 2);
    assert!(notes[1].note.start >= Tick(0));
    assert_eq!(rig.e.recorder().dropped(), 0);
}

/// Released before the cue, pressed twice, ended by an all-notes-off or an overflow: each note
/// of the count-in's last sixteenth is one of the take's first notes, with its played length.
#[test]
fn the_count_in_notes_ended_before_the_cue_open_the_take() {
    let blocks = |n: f64| Tick((n * BLOCK as f64 / SPT).round() as i64);
    let mut rig = Rig::new();
    let played = rig.count_in();
    rig.blocks(705);
    // Track 0, key 60: pressed, released, pressed again, pressed once more, released.
    let first = rig.tick_since(played, -COUNT_IN);
    rig.on(0, 60);
    rig.blocks(5);
    rig.off(0, 60);
    rig.blocks(5);
    let second = rig.tick_since(played, -COUNT_IN);
    rig.on(0, 60);
    rig.blocks(3);
    // A second press without a release in between ends the first.
    let third = rig.tick_since(played, -COUNT_IN);
    rig.on(0, 60);
    rig.blocks(2);
    rig.off(0, 60);
    // Track 2: ended by an all-notes-off of its track, while track 0's key 61 still holds.
    let held = rig.tick_since(played, -COUNT_IN);
    rig.on(2, 70);
    rig.on(0, 61);
    rig.blocks(4);
    rig.send(&[M::AllNotesOff {
        track: TrackSlot(2),
    }]);
    rig.blocks(1);
    // An overflow of the live queue ends key 61 in the next block.
    for i in 0..=limits::LIVE_QUEUE_CAPACITY {
        let _ = rig.p.handle(M::NoteOff {
            track: TrackSlot(1),
            pitch: (i % 128) as u8,
            at: None,
        });
    }
    rig.blocks(2);
    assert!(!rig.e.recorder().capturing());
    rig.blocks(40);
    assert!(rig.e.recorder().capturing());
    // Late note offs of the ended keys change nothing.
    rig.off(0, 60);
    rig.off(0, 61);
    rig.off(2, 70);
    rig.blocks(1);
    let mut notes: Vec<_> = rig
        .notes()
        .into_iter()
        .map(|n| (n.track.0, n.note.pitch, n.note.start, n.note.length))
        .collect();
    notes.sort_by_key(|n| (n.2, n.0, n.1));
    assert_eq!(
        notes,
        vec![
            (0, 60, Tick(first), blocks(5.0)),
            (0, 60, Tick(second), blocks(3.0)),
            (0, 60, Tick(third), blocks(2.0)),
            (0, 61, Tick(held), blocks(5.0)),
            (2, 70, Tick(held), blocks(4.0)),
        ]
    );
    assert_eq!(rig.e.recorder().dropped(), 0);
}

/// A stop, or the recording turned off, before the cue: what the count-in kept and counted is
/// forgotten, and the next take starts clean.
#[test]
fn a_cancelled_count_in_forgets_what_it_kept() {
    for cancel in [
        M::Stop,
        M::SetRecording {
            recording: false,
            shift_us: 0,
        },
    ] {
        let mut rig = Rig::new();
        rig.count_in();
        rig.blocks(10);
        rig.on(0, 59); // too early: ignored
        rig.blocks(700);
        rig.on(0, 60); // kept, held
        rig.on(2, 62);
        rig.blocks(2);
        rig.off(2, 62); // kept, released
        rig.blocks(2);
        rig.send(std::slice::from_ref(&cancel));
        rig.blocks(60);
        assert!(!rig.e.recorder().capturing(), "{cancel:?}");
        assert_eq!(rig.e.recorder().take(), 0);
        // A new recording without a count-in: none of it comes back.
        rig.send(&[
            M::Stop,
            M::SetMetronome {
                enabled: false,
                volume: 1.0,
                count_in_bars: 0,
            },
        ]);
        rig.blocks(1);
        rig.record(0);
        rig.blocks(2);
        rig.off(0, 60);
        rig.blocks(1);
        assert!(rig.e.recorder().capturing());
        assert_eq!(rig.e.recorder().take(), 1);
        assert!(rig.notes().is_empty(), "{cancel:?}: {:?}", rig.notes());
        assert_eq!(rig.e.recorder().dropped(), 0, "{cancel:?}");
    }
}

/// A stop, then a new recording with a count-in: the note still held when the take stopped is
/// cut in that take, and is not carried into the next one. (A stop and a play applied in the
/// same block do not count in: the clock still runs, `Engine::start_clock`.)
#[test]
fn a_note_held_when_a_take_restarts_with_a_count_in_stays_in_the_ended_take() {
    let mut rig = Rig::new();
    rig.record(0);
    rig.blocks(4);
    rig.on(0, 60);
    rig.blocks(4);
    rig.send(&[M::Stop]);
    rig.blocks(1);
    rig.send(&[
        M::SetMetronome {
            enabled: false,
            volume: 1.0,
            count_in_bars: 1,
        },
        M::SetRecording {
            recording: true,
            shift_us: 30_000,
        },
        M::Play { from: Tick(0) },
    ]);
    rig.blocks(1);
    assert!(!rig.e.recorder().capturing());
    assert_eq!(rig.e.recorder().take(), 1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(
        notes[0].note.length,
        Tick((4.0 * BLOCK as f64 / SPT).round() as i64),
        "cut on the ended take's clock"
    );
    rig.blocks(800);
    assert!(rig.e.recorder().capturing());
    assert_eq!(rig.e.recorder().take(), 2);
    rig.off(0, 60);
    rig.blocks(1);
    assert!(rig.notes().is_empty(), "{:?}", rig.notes());
    assert_eq!(rig.e.recorder().dropped(), 0);
}

/// More keys than the held table keeps during a count-in: the extra notes count for the take.
#[test]
fn a_full_held_table_in_the_count_in_counts_for_the_take() {
    let mut rig = Rig::new();
    rig.count_in();
    rig.blocks(720);
    rig.send(&[M::TrackArm {
        track: TrackSlot(1),
        armed: true,
    }]);
    for pitch in 0..128u8 {
        rig.on(0, pitch);
        rig.on(1, pitch);
        rig.on(2, pitch);
    }
    rig.blocks(41);
    assert!(rig.e.recorder().capturing());
    assert_eq!(rig.e.recorder().dropped(), (3 * 128 - MAX_HELD) as u32);
    rig.send(&[M::SetRecording {
        recording: false,
        shift_us: 0,
    }]);
    rig.blocks(1);
    assert_eq!(rig.e.recorder().total(), MAX_HELD as u32);
}

/// A tempo change in the count-in: a note released before the cue keeps the ticks it ran, one
/// held across the cue too.
#[test]
fn a_tempo_change_in_the_count_in_keeps_the_ticks_its_notes_ran() {
    let tempo = |bpm: f64| M::SetTransport {
        tempo: Tempo::new(bpm).unwrap(),
        signature: TimeSignature::new(4, 4).unwrap(),
        launch_quantize: Quantize::default(),
    };
    let mut rig = Rig::new();
    rig.count_in();
    rig.blocks(720);
    rig.on(0, 60);
    rig.blocks(10);
    rig.send(&[tempo(60.0)]);
    rig.blocks(10);
    rig.off(0, 60);
    rig.on(0, 62);
    rig.blocks(10);
    rig.send(&[tempo(120.0)]);
    rig.blocks(40);
    assert!(rig.e.recorder().capturing());
    rig.off(0, 62);
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!(notes[0].note.pitch, 60);
    assert_eq!(notes[0].note.length, HELD_ACROSS_A_CHANGE);
    // 10 blocks at 60 BPM (25.6 ticks), then 40 at 120 BPM (204.8): 230.4.
    assert_eq!(notes[1].note.pitch, 62);
    assert_eq!(notes[1].note.length, Tick(230));
}

#[test]
fn the_capture_starts_with_the_block_that_reaches_the_cue() {
    let mut rig = Rig::new();
    rig.send(&[M::SetMetronome {
        enabled: false,
        volume: 1.0,
        count_in_bars: 1,
    }]);
    // 96 000 frames of count-in are 750 blocks: the cue is the start of block 751.
    rig.record(0);
    rig.blocks(749);
    assert!(!rig.e.recorder().capturing());
    rig.on(0, 60);
    rig.blocks(1);
    assert!(rig.e.recorder().capturing());
    rig.off(0, 60);
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(
        notes.len(),
        1,
        "a note of the block that reaches the cue is kept"
    );
    assert!(notes[0].note.start < Tick(0) && notes[0].note.start > Tick(-10));
}

#[test]
fn a_second_press_closes_the_first_and_a_disarmed_note_still_ends() {
    let mut rig = Rig::new();
    rig.record(0);
    rig.blocks(2);
    rig.on(0, 60);
    rig.blocks(2);
    // A lost note off (or a repeat) then the same key again: the first note ends here.
    rig.on(0, 60);
    rig.blocks(2);
    // Disarmed while held: its note off still closes it.
    rig.send(&[M::TrackArm {
        track: TrackSlot(0),
        armed: false,
    }]);
    rig.off(0, 60);
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 2, "{notes:?}");
    let two = Tick((2.0 * BLOCK as f64 / SPT).round() as i64);
    assert!(notes.iter().all(|n| n.note.length == two), "{notes:?}");
    // Disarmed: a new press is not recorded.
    rig.on(0, 61);
    rig.blocks(1);
    rig.off(0, 61);
    rig.blocks(1);
    assert_eq!(rig.notes().len(), 2);
}

#[test]
fn a_full_held_table_counts_the_lost_notes() {
    let mut rig = Rig::new();
    rig.record(0);
    rig.blocks(1);
    // Two armed tracks, 128 keys each: 256 held fills the table exactly.
    for pitch in 0..128u8 {
        rig.on(0, pitch);
        rig.on(2, pitch);
        rig.blocks(1);
    }
    assert_eq!(rig.e.recorder().dropped(), (256 - MAX_HELD) as u32);
    // One more armed track: its notes find no room and are counted.
    rig.send(&[M::TrackArm {
        track: TrackSlot(1),
        armed: true,
    }]);
    rig.on(1, 10);
    rig.on(1, 11);
    rig.blocks(1);
    assert_eq!(rig.e.recorder().dropped(), 2);
    rig.send(&[M::SetRecording {
        recording: false,
        shift_us: 0,
    }]);
    rig.blocks(1);
    assert_eq!(rig.e.recorder().total(), 256);
}

#[test]
fn the_report_carries_the_take_its_count_its_last_notes_and_whether_it_captures() {
    let (mut prepare, mut host) = host_channel(48_000.0).unwrap();
    let mut seq = 0u32;
    let mut send = |prepare: &mut engine::host::Prepare, msgs: &[M]| {
        let bytes = encode_stream(1, seq, msgs).unwrap();
        seq = seq.wrapping_add(msgs.len() as u32);
        prepare.submit(&bytes).unwrap();
    };
    send(
        &mut prepare,
        &[
            M::Reset { epoch: 1 },
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
            M::SetRecording {
                recording: true,
                shift_us: 0,
            },
            M::Play { from: Tick(0) },
        ],
    );
    assert_no_alloc(|| host.render(BLOCK));
    // More notes than one report carries: the report holds the last ones.
    let notes = 300usize;
    for i in 0..notes {
        let pitch = (i % 128) as u8;
        send(
            &mut prepare,
            &[
                M::NoteOn {
                    track: TrackSlot(0),
                    pitch,
                    velocity: 90,
                    at: None,
                },
                M::NoteOff {
                    track: TrackSlot(0),
                    pitch,
                    at: None,
                },
            ],
        );
        // The note on and its note off land in the same block: the off closes the on.
        assert_no_alloc(|| host.render(BLOCK));
    }
    host.write_feedback();
    let report = read_feedback(host.feedback()).unwrap();
    assert_eq!(report.record_take, 1);
    assert_eq!(report.record_total, notes as u32);
    assert!(report.record_capturing);
    let carried = engine::msg::MAX_RECORDED_PER_REPORT.min(notes);
    assert_eq!(report.recorded.len(), carried);
    let last = report.recorded.last().unwrap();
    assert_eq!(last.note.pitch, ((notes - 1) % 128) as u8);
    assert_eq!(last.note.velocity, 90);
    assert_eq!(last.note.length, Tick(1));
    // The take ends: the flag drops, the notes stay for the reader.
    send(
        &mut prepare,
        &[M::SetRecording {
            recording: false,
            shift_us: 0,
        }],
    );
    assert_no_alloc(|| host.render(BLOCK));
    host.write_feedback();
    let report = read_feedback(host.feedback()).unwrap();
    assert!(!report.record_capturing);
    assert_eq!(report.record_total, notes as u32);
    assert_eq!(report.recorded.len(), carried);
    // A while later the reports stop carrying the finished take; its count stays.
    for _ in 0..engine::host::TAKE_REPORTS {
        host.write_feedback();
    }
    let report = read_feedback(host.feedback()).unwrap();
    assert_eq!(report.record_total, notes as u32);
    assert!(report.recorded.is_empty());
}

#[test]
fn an_all_notes_off_closes_its_track_and_an_overflow_closes_every_note() {
    let mut rig = Rig::new();
    rig.record(0);
    rig.blocks(2);
    rig.on(0, 60);
    rig.on(2, 64);
    rig.blocks(4);
    // All notes off on track 2 only: its note ends here, track 0's still holds.
    rig.send(&[M::AllNotesOff {
        track: TrackSlot(2),
    }]);
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].track, TrackSlot(2));
    assert_eq!(
        notes[0].note.length,
        Tick((4.0 * BLOCK as f64 / SPT).round() as i64)
    );
    // The live queue overflows (more events than it holds before a block): every held note
    // ends in the next block, the overflow's release.
    for i in 0..=limits::LIVE_QUEUE_CAPACITY {
        let _ = rig.p.handle(M::NoteOff {
            track: TrackSlot(1),
            pitch: (i % 128) as u8,
            at: None,
        });
    }
    rig.blocks(2);
    let notes = rig.notes();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!(notes[1].track, TrackSlot(0));
    assert_eq!(notes[1].note.pitch, 60);
}

/// Ten blocks, a change of pace, ten blocks: the ticks the note ran (#513).
/// 1 280 frames at 120 BPM and 48 kHz are 51.2 ticks; after a change that doubles the samples
/// per tick (60 BPM, or 96 kHz) the same frames are 25.6 ticks: 76.8 ticks in all, 77 rounded.
const HELD_ACROSS_A_CHANGE: Tick = Tick(77);

#[test]
fn a_note_held_across_a_sample_rate_change_keeps_the_ticks_it_ran() {
    let mut rig = Rig::new();
    rig.record(0);
    rig.on(0, 60);
    rig.blocks(10);
    rig.p.set_sample_rate(SampleRate::HZ_96000).unwrap();
    rig.blocks(10);
    rig.off(0, 60);
    rig.blocks(2);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].note.length, HELD_ACROSS_A_CHANGE);
    // Control: the same notes with no change run 20 blocks of 128 frames at 25 samples a tick.
    let mut rig = Rig::new();
    rig.record(0);
    rig.on(0, 60);
    rig.blocks(20);
    rig.off(0, 60);
    rig.blocks(2);
    assert_eq!(
        rig.notes()[0].note.length,
        Tick((20.0 * BLOCK as f64 / SPT).round() as i64)
    );
}

#[test]
fn a_note_held_across_a_tempo_change_keeps_the_ticks_it_ran() {
    let mut rig = Rig::new();
    rig.record(0);
    rig.on(0, 60);
    rig.blocks(10);
    rig.send(&[M::SetTransport {
        tempo: Tempo::new(60.0).unwrap(),
        signature: TimeSignature::new(4, 4).unwrap(),
        launch_quantize: Quantize::default(),
    }]);
    rig.blocks(10);
    rig.off(0, 60);
    rig.blocks(2);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].note.length, HELD_ACROSS_A_CHANGE);
}

#[test]
fn a_change_of_pace_moves_every_held_note_and_the_ones_pressed_after_it_are_exact() {
    let mut rig = Rig::new();
    rig.record(0);
    rig.on(0, 60);
    rig.on(2, 64);
    rig.blocks(4);
    // 240 BPM halves the samples per tick.
    rig.send(&[M::SetTransport {
        tempo: Tempo::new(240.0).unwrap(),
        signature: TimeSignature::new(4, 4).unwrap(),
        launch_quantize: Quantize::default(),
    }]);
    rig.blocks(4);
    rig.on(0, 62);
    rig.blocks(4);
    rig.off(0, 60);
    rig.off(2, 64);
    rig.off(0, 62);
    rig.blocks(2);
    let mut lengths: Vec<(u8, i64)> = rig
        .notes()
        .iter()
        .map(|n| (n.note.pitch, n.note.length.0))
        .collect();
    lengths.sort();
    // 4 blocks at 25 samples a tick, then 8 at 12.5.
    let long = (4.0 * 128.0 / 25.0 + 8.0 * 128.0 / 12.5_f64).round() as i64;
    let late = (4.0 * 128.0 / 12.5_f64).round() as i64;
    assert_eq!(lengths, vec![(60, long), (62, late), (64, long)]);
}
