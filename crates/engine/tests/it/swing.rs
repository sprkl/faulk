//! Swing (#168): the delay of the off-beats at the sample, the on-beats and the note lengths
//! unchanged, the humanize draws at equal seed, no stuck note (a stop, a reset, an
//! all-notes-off, a reused voice, a full table), the output bounds, a sweep of every parameter,
//! and the audio path under the allocation detector. The render through the synth, native and
//! WASM, is in `crates/test-support/tests/it/swing.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceSetup, Event, EventBuffer, Transport, VoiceId,
};
use engine::devices::swing::{
    AMOUNT, DESCRIPTOR, EIGHTH, GRID, HELD, HUMANIZE_TIME, HUMANIZE_VELOCITY, PENDING, SEED, Swing,
    THIRTY_SECOND, WINDOW,
};
use engine::msg::ParamId;
use std::collections::BTreeMap;

const RATE: f32 = 48_000.0;
/// Frames of a tick at 120 BPM: 25.
const FRAMES_PER_TICK: u64 = 25;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Out {
    On {
        frame: u64,
        voice: u32,
        key: u8,
        velocity: f32,
    },
    Off {
        frame: u64,
        voice: u32,
        key: u8,
    },
    All {
        frame: u64,
    },
}

struct Rig {
    swing: Swing,
    out: EventBuffer,
    frame: u64,
    playing: bool,
    block: u64,
    log: Vec<Out>,
}

impl Rig {
    fn new(params: &[(u32, f32)]) -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE,
            ..DeviceSetup::default()
        };
        let mut swing = Swing::new(&setup).unwrap();
        for (id, value) in params {
            assert_no_alloc(|| swing.set_param(ParamId(*id), *value));
        }
        assert_no_alloc(|| swing.reset());
        Rig {
            swing,
            out: EventBuffer::try_with_capacity(DESCRIPTOR.midi_output.unwrap().capacity(1024))
                .unwrap(),
            frame: 0,
            playing: true,
            block: 128,
            log: Vec::new(),
        }
    }

    fn set(&mut self, id: u32, value: f32) {
        let swing = &mut self.swing;
        assert_no_alloc(|| swing.set_param(ParamId(id), value));
    }

    fn reset(&mut self) {
        let swing = &mut self.swing;
        assert_no_alloc(|| swing.reset());
    }

    /// Renders up to `until`, applying `events` (absolute frame, event); the transport stops
    /// at the first block starting at or after `stop_at` (if any).
    fn run_until_stop(&mut self, events: &[(u64, Event)], until: u64, stop_at: Option<u64>) {
        let mut pending: Vec<(u64, Event)> = events.to_vec();
        pending.sort_by_key(|(f, _)| *f);
        let mut next = 0;
        while self.frame < until {
            let frames = self.block.min(until - self.frame) as usize;
            let mut block = Vec::new();
            while next < pending.len() && pending[next].0 < self.frame + frames as u64 {
                let (f, e) = pending[next];
                block.push(e.at((f - self.frame) as u32));
                next += 1;
            }
            if stop_at.is_some_and(|s| self.frame >= s) {
                self.playing = false;
            }
            let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
            let ctx = BlockCtx {
                sample_rate: RATE,
                frames: frames as u32,
                transport: Transport {
                    playing: self.playing,
                    position_ticks: self.frame as f64 / FRAMES_PER_TICK as f64,
                    ..Transport::default()
                },
            };
            self.out.clear();
            let (swing, out) = (&mut self.swing, &mut self.out);
            assert_no_alloc(|| {
                let mut io = AudioIo {
                    left: &mut l,
                    right: &mut r,
                    events_out: out,
                };
                swing.process(&ctx, &block, &mut io);
            });
            assert_eq!(self.out.overflowed(), 0, "the output never overflows");
            assert!(
                self.out.events().len() <= 4 * block.len() + PENDING,
                "within the declared bounds"
            );
            let mut last = 0;
            for e in self.out.events() {
                assert!(e.offset() >= last, "sorted output");
                assert!((e.offset() as usize) < frames.max(1), "inside the block");
                last = e.offset();
                let frame = self.frame + u64::from(e.offset());
                self.log.push(match *e {
                    Event::NoteOn {
                        voice,
                        key,
                        velocity,
                        tune_cents,
                        ..
                    } => {
                        assert!(velocity.is_finite() && (0.0..=1.0).contains(&velocity));
                        assert!(tune_cents.is_finite());
                        Out::On {
                            frame,
                            voice: voice.get(),
                            key,
                            velocity,
                        }
                    }
                    Event::NoteOff { voice, key, .. } => Out::Off {
                        frame,
                        voice: voice.get(),
                        key,
                    },
                    Event::AllNotesOff { .. } => Out::All { frame },
                    Event::ParamValue { .. } => panic!("a MIDI effect writes notes only"),
                });
            }
            self.frame += frames as u64;
        }
    }

    fn run(&mut self, events: &[(u64, Event)], until: u64) {
        self.run_until_stop(events, until, None);
    }

    /// Notes still sounding at the end: every on matched by an off at its voice and key.
    fn sounding(&self) -> BTreeMap<u32, u8> {
        let mut held = BTreeMap::new();
        for o in &self.log {
            match *o {
                Out::On { voice, key, .. } => {
                    assert!(
                        held.insert(voice, key).is_none(),
                        "voice {voice} started twice"
                    );
                }
                Out::Off { voice, key, .. } => {
                    assert_eq!(held.remove(&voice), Some(key), "off of a held note");
                }
                Out::All { .. } => held.clear(),
            }
        }
        held
    }

    /// (on frame, off frame, key, velocity) of every note, in start order.
    fn notes(&self) -> Vec<(u64, u64, u8, f32)> {
        let mut notes = Vec::new();
        for o in &self.log {
            if let Out::On {
                frame,
                voice,
                key,
                velocity,
            } = *o
            {
                let end = self
                    .log
                    .iter()
                    .find_map(|e| match *e {
                        Out::Off {
                            frame: f, voice: v, ..
                        } if v == voice && f >= frame => Some(f),
                        _ => None,
                    })
                    .unwrap_or(u64::MAX);
                notes.push((frame, end, key, velocity));
            }
        }
        notes
    }
}

fn on(voice: u32, key: u8) -> Event {
    Event::NoteOn {
        offset: 0,
        voice: VoiceId::new(voice).unwrap(),
        key,
        velocity: 0.75,
        tune_cents: 0.0,
    }
}

fn off(voice: u32, key: u8) -> Event {
    Event::NoteOff {
        offset: 0,
        voice: VoiceId::new(voice).unwrap(),
        key,
    }
}

/// A clip of `n` sixteenth notes (240 ticks apart, 200 ticks long), key 42, one voice each
/// (ids recycled every 100 notes).
fn sixteenths(n: u32) -> Vec<(u64, Event)> {
    let mut events = Vec::new();
    for i in 0..n {
        let start = u64::from(i) * 240 * FRAMES_PER_TICK;
        events.push((start, on(i % 100 + 1, 42)));
        events.push((start + 200 * FRAMES_PER_TICK, off(i % 100 + 1, 42)));
    }
    events
}

#[test]
fn at_66_percent_the_off_beats_of_a_sixteenth_clip_move_by_76_8_ticks_to_the_sample_and_the_beats_do_not()
 {
    let mut r = Rig::new(&[(AMOUNT, 66.0)]);
    r.run(&sixteenths(32), 32 * 6000 + 20_000);
    let notes = r.notes();
    assert_eq!(notes.len(), 32);
    // 76.8 ticks at 120 BPM, 48 kHz: 0.04 s, 1920 frames.
    for (i, (start, end, key, _)) in notes.iter().enumerate() {
        let straight = i as u64 * 6000;
        let delay = if i % 2 == 1 { 1920 } else { 0 };
        assert_eq!(*start, straight + delay, "note {i} start");
        assert_eq!(*end - *start, 5000, "note {i} keeps its length");
        assert_eq!(*key, 42);
    }
    assert!(r.sounding().is_empty());
}

#[test]
fn amount_50_changes_nothing_and_the_grids_follow_their_cell() {
    let mut r = Rig::new(&[(AMOUNT, 50.0)]);
    r.run(&sixteenths(8), 8 * 6000 + 8000);
    for (i, (start, ..)) in r.notes().iter().enumerate() {
        assert_eq!(*start, i as u64 * 6000);
    }
    // 1/8 grid at 66 %: pairs of 960 ticks, the off-beat at 480; the sixteenths at 480 (i = 2,
    // 6) move by 2 * 480 * 0.16 = 153.6 ticks = 3840 frames; those at 240 and 720 are 240 ticks
    // off it, outside the window (25 % of 480 = 120).
    let mut r = Rig::new(&[(AMOUNT, 66.0), (GRID, EIGHTH as f32)]);
    r.run(&sixteenths(8), 8 * 6000 + 20_000);
    for (i, (start, ..)) in r.notes().iter().enumerate() {
        let delay = if i % 4 == 2 { 3840 } else { 0 };
        assert_eq!(
            *start,
            i as u64 * 6000 + delay,
            "note {i} on the eighth grid"
        );
    }
    // 1/32 grid: cells of 120 ticks, pairs of 240; every sixteenth sits on a pair start.
    let mut r = Rig::new(&[(AMOUNT, 66.0), (GRID, THIRTY_SECOND as f32)]);
    r.run(&sixteenths(8), 8 * 6000 + 8000);
    for (i, (start, ..)) in r.notes().iter().enumerate() {
        assert_eq!(*start, i as u64 * 6000);
    }
}

#[test]
fn the_window_decides_which_notes_are_off_beat() {
    // A note 30 ticks after the off-beat of a 1/16 grid (r = 270 of 480).
    let late = 270 * FRAMES_PER_TICK;
    let events = [(late, on(1, 60)), (late + 1000, off(1, 60))];
    // 30 ticks is 12.5 % of a cell. At 75 % the delay is 2 * 240 * 0.25 = 120 ticks = 3000 frames.
    for (window, moved) in [(10.0, false), (25.0, true), (0.0, false), (50.0, true)] {
        let mut r = Rig::new(&[(AMOUNT, 75.0), (WINDOW, window)]);
        r.run(&events, 20_000);
        let start = r.notes()[0].0;
        assert_eq!(
            start,
            late + if moved { 3000 } else { 0 },
            "window {window}"
        );
    }
}

#[test]
fn equal_seeds_draw_alike_another_seed_differs_and_humanize_only_goes_forward() {
    let params = |seed: f32| {
        [
            (AMOUNT, 50.0),
            (HUMANIZE_TIME, 0.02),
            (HUMANIZE_VELOCITY, 20.0),
            (SEED, seed),
        ]
    };
    let render = |seed: f32| {
        let mut r = Rig::new(&params(seed));
        r.run(&sixteenths(64), 64 * 6000 + 20_000);
        r.log
    };
    let a = render(7.0);
    assert_eq!(a, render(7.0), "same seed, same render");
    assert_ne!(a, render(8.0), "another seed, another draw");
    let mut r = Rig::new(&params(7.0));
    r.run(&sixteenths(64), 64 * 6000 + 20_000);
    let mut moved = 0;
    for (i, (start, end, _, velocity)) in r.notes().iter().enumerate() {
        let straight = i as u64 * 6000;
        assert!(
            (straight..=straight + 960).contains(start),
            "0 to 20 ms forward"
        );
        assert_eq!(end - start, 5000, "length kept");
        assert!((velocity - 0.75).abs() <= 20.0 / 127.0 + 1e-6);
        moved += usize::from(*start != straight);
    }
    assert!(moved > 50, "most notes are moved");
}

#[test]
fn swing_and_humanize_together_keep_every_length() {
    let mut r = Rig::new(&[
        (AMOUNT, 66.0),
        (HUMANIZE_TIME, 0.02),
        (HUMANIZE_VELOCITY, 20.0),
        (SEED, 5.0),
    ]);
    r.run(&sixteenths(64), 64 * 6000 + 40_000);
    let notes = r.notes();
    assert_eq!(notes.len(), 64);
    for (i, (start, end, ..)) in notes.iter().enumerate() {
        let straight = i as u64 * 6000 + if i % 2 == 1 { 1920 } else { 0 };
        assert!((straight..=straight + 960).contains(start), "note {i}");
        assert_eq!(end - start, 5000, "note {i} keeps its length");
    }
}

#[test]
fn a_stopped_transport_swings_nothing() {
    let mut r = Rig::new(&[(AMOUNT, 75.0)]);
    r.playing = false;
    r.run(&sixteenths(4), 4 * 6000 + 6000);
    for (i, (start, ..)) in r.notes().iter().enumerate() {
        assert_eq!(*start, i as u64 * 6000);
    }
}

#[test]
fn a_stop_drops_the_delayed_note_ons_and_ends_the_notes_nothing_rings_on() {
    // Off-beat note at tick 240 (frame 6000), delayed to 7920; the clock stops at 6144: the
    // note-on never sounds, nor does its note-off.
    let mut r = Rig::new(&[(AMOUNT, 66.0)]);
    let events = [(6000, on(1, 60)), (6500, off(1, 60))];
    r.run_until_stop(&events, 12_000, Some(6144));
    assert!(r.notes().is_empty(), "{:?}", r.log);
    assert!(r.sounding().is_empty());
    assert_eq!(r.swing.pending(), 0);
    assert_eq!(r.swing.held(), 0);

    // A short note whose end came before the stop while its start is still delayed: neither
    // sounds, no orphan note-off.
    let mut r = Rig::new(&[(AMOUNT, 66.0)]);
    let events = [(6000, on(1, 60)), (6100, off(1, 60))];
    r.run_until_stop(&events, 12_000, Some(6144));
    assert!(r.log.is_empty(), "{:?}", r.log);
    assert_eq!(r.swing.pending(), 0);

    // A note sounding at the stop ends undelayed when its note-off comes after it.
    let mut r = Rig::new(&[(AMOUNT, 66.0)]);
    let events = [(6000, on(1, 60)), (9000, off(1, 60))];
    r.run_until_stop(&events, 12_000, Some(8064));
    assert_eq!(r.notes().len(), 1);
    assert!(r.sounding().is_empty());
    assert!(
        r.log
            .iter()
            .any(|o| matches!(o, Out::Off { frame: 9000, .. }))
    );
}

#[test]
fn a_reset_an_all_notes_off_and_a_reused_voice_leave_nothing_stuck() {
    // The host reuses voice 1 right after its note-off, while the delayed end is pending.
    let mut r = Rig::new(&[(AMOUNT, 75.0), (WINDOW, 50.0)]);
    let events = [
        (6000, on(1, 60)),
        (6200, off(1, 60)),
        (6300, on(1, 62)),
        (6600, off(1, 62)),
    ];
    r.run(&events, 40_000);
    assert_eq!(r.notes().len(), 2);
    assert!(r.sounding().is_empty());

    // AllNotesOff: the delayed note-on never sounds.
    let mut r = Rig::new(&[(AMOUNT, 75.0)]);
    let all = Event::AllNotesOff { offset: 0 };
    r.run(&[(6000, on(1, 60)), (6050, all)], 40_000);
    assert!(r.notes().is_empty());
    assert!(matches!(r.log.as_slice(), [Out::All { frame: 6050 }]));
    assert_eq!((r.swing.pending(), r.swing.held()), (0, 0));

    // A reset empties both tables.
    let mut r = Rig::new(&[(AMOUNT, 75.0)]);
    r.run(&[(6000, on(1, 60))], 6100);
    assert_eq!((r.swing.pending(), r.swing.held()), (1, 1));
    r.reset();
    assert_eq!((r.swing.pending(), r.swing.held()), (0, 0));

    // A note-on for a held voice ends the held one first; a note-off nobody holds is dropped.
    let mut r = Rig::new(&[(AMOUNT, 50.0)]);
    r.run(
        &[
            (10, on(1, 60)),
            (20, on(1, 62)),
            (30, off(1, 62)),
            (40, off(9, 1)),
        ],
        400,
    );
    assert!(r.sounding().is_empty());
    assert_eq!(r.notes().len(), 2);
}

#[test]
fn full_tables_end_the_oldest_and_flush_the_earliest_never_lose_a_note_off() {
    // More simultaneous notes than the table holds: the oldest ends.
    let mut r = Rig::new(&[(AMOUNT, 50.0)]);
    let mut events = Vec::new();
    for i in 0..(HELD as u32 + 20) {
        events.push((u64::from(i), on(i + 1, (i % 100) as u8)));
    }
    r.run(&events, 2000);
    assert_eq!(r.sounding().len(), HELD);

    // A storm of delayed notes past the pending table (1/8 grid, 75 %: 6000 frames of delay):
    // 300 notes of 2 events each fill the 512 slots, the earliest come out at once, and every
    // note starts and ends in order, none stuck.
    let mut r = Rig::new(&[(AMOUNT, 75.0), (GRID, EIGHTH as f32), (WINDOW, 50.0)]);
    let mut events = Vec::new();
    for i in 0..300u32 {
        let at = 12_000 + u64::from(i);
        events.push((at, on(i % 100 + 1, (i % 100) as u8)));
        events.push((at, off(i % 100 + 1, (i % 100) as u8)));
    }
    r.run(&events, 13_000);
    assert_eq!(r.swing.pending(), PENDING, "the table is full");
    r.run(&[], 40_000);
    assert_eq!(r.swing.pending(), 0);
    assert!(r.sounding().is_empty(), "{:?}", r.sounding());
    assert_eq!(r.notes().len(), 300);
}

#[test]
fn every_parameter_at_its_bounds_and_default_gives_finite_bounded_events() {
    for p in DESCRIPTOR.params {
        for value in [p.min, p.default, p.max] {
            let mut r = Rig::new(&[(AMOUNT, 75.0), (WINDOW, 50.0), (p.id.0, value as f32)]);
            r.run(&sixteenths(40), 40 * 6000 + 40_000);
            assert!(r.sounding().is_empty(), "{} at {value}", p.key);
            for (start, end, ..) in r.notes() {
                assert!(end >= start);
            }
        }
    }
    // The device at its own defaults, whole.
    let mut r = Rig::new(&[]);
    r.run(&sixteenths(40), 40 * 6000 + 40_000);
    assert!(r.sounding().is_empty());
    // Garbage values are ignored, for every parameter.
    let mut r = Rig::new(&[]);
    for bad in [f32::NAN, f32::INFINITY, -1e30, 1e30] {
        for p in DESCRIPTOR.params {
            r.set(p.id.0, bad);
        }
        r.set(99, bad);
    }
    r.run(&sixteenths(8), 60_000);
    assert!(r.sounding().is_empty());
}

#[test]
fn a_note_end_uses_the_delay_fixed_at_its_start() {
    let mut r = Rig::new(&[(AMOUNT, 66.0)]);
    r.run(&[(6000, on(1, 60)), (11_000, off(1, 60))], 30_000);
    let n = r.notes();
    assert_eq!((n[0].0, n[0].1), (7920, 12_920));
}
