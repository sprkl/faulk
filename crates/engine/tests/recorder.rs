//! MIDI recording in the engine (RE-2, `engine::recorder`): offline renders through the real
//! `Preparer`, `Engine::apply` and `Engine::render`, every audio-side call under the
//! allocation detector. At 120 BPM and 48 kHz a tick is 25 samples, and every block here is
//! 128 frames (the Web render quantum), so the stamp of a note is the playhead of the start
//! of its block, known exactly.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::DeviceSetup;
use engine::device::testing::factory;
use engine::engine::{Engine, Preparer, channel};
use engine::host::{channel as host_channel, read_feedback};
use engine::msg::{ControlMsg as M, RecordedNote, TrackKind, TrackSlot, encode_stream, limits};
use engine::recorder::MAX_HELD;
use engine::time::{LoopRegion, Quantize, Tempo, Tick, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

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
        }]);
    }

    fn off(&mut self, track: u32, pitch: u8) {
        self.send(&[M::NoteOff {
            track: TrackSlot(track),
            pitch,
        }]);
    }

    fn record(&mut self, from: i64) -> usize {
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
    rig.send(&[M::Stop, M::SetRecording { recording: true }]);
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
    for end in [vec![M::SetRecording { recording: false }], vec![M::Stop]] {
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
            M::SetRecording { recording: true },
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

#[test]
fn the_count_in_is_heard_not_captured() {
    let mut rig = Rig::new();
    rig.send(&[M::SetMetronome {
        enabled: false,
        volume: 1.0,
        count_in_bars: 1,
    }]);
    // One bar is 2 s: 96 000 frames, 750 blocks.
    rig.record(0);
    rig.blocks(10);
    assert!(rig.e.clock().playhead_tick() < Tick(0));
    rig.on(0, 60);
    rig.blocks(10);
    rig.off(0, 60);
    rig.blocks(700);
    assert!(!rig.e.recorder().capturing());
    assert_eq!(rig.e.recorder().take(), 0);
    // A key pressed in the count-in and released after the cue is not a note of the take.
    rig.on(0, 62);
    rig.blocks(40);
    assert!(rig.e.recorder().capturing());
    assert_eq!(rig.e.recorder().take(), 1);
    rig.off(0, 62);
    rig.blocks(1);
    assert!(rig.notes().is_empty(), "{:?}", rig.notes());
    // After the cue a note is captured.
    rig.on(0, 64);
    rig.blocks(4);
    rig.off(0, 64);
    rig.blocks(1);
    let notes = rig.notes();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].note.start >= Tick(0));
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
    rig.send(&[M::SetRecording { recording: false }]);
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
            M::SetRecording { recording: true },
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
                },
                M::NoteOff {
                    track: TrackSlot(0),
                    pitch,
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
    send(&mut prepare, &[M::SetRecording { recording: false }]);
    assert_no_alloc(|| host.render(BLOCK));
    host.write_feedback();
    let report = read_feedback(host.feedback()).unwrap();
    assert!(!report.record_capturing);
    assert_eq!(report.record_total, notes as u32);
    assert_eq!(report.recorded.len(), carried);
}
