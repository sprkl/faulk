//! Cascade (#166): the notes of the arpeggio at the sample, every order, the octaves, the
//! repeats, the gate, the velocity, the latch, the restart, the seeded random order, the clock
//! of a stopped transport, no stuck note (an all-notes-off, a reset, a stop, a setting changed
//! in the middle of a note, a full table), the output bounds, a sweep of every parameter and the
//! audio path under the allocation detector. The render through the synth, native and WASM, is
//! in `crates/test-support/tests/it/cascade.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceSetup, Event, EventBuffer, Transport, VoiceId,
};
use engine::devices::cascade::{
    ACCENT, CHORD, Cascade, DESCRIPTOR, DOWN, DOWN_UP, FEEL, FIXED, FREE, GATE, LATCH, OCTAVES,
    ORDER, PER_BLOCK, PLAYED, RANDOM, RATE, REPEATS, RESTART, SEED, SOUNDING, SOURCES, TRIPLET, UP,
    UP_DOWN, VELOCITY, VELOCITY_SOURCE,
};
use engine::msg::ParamId;
use std::collections::BTreeMap;

const RATE_HZ: f32 = 48_000.0;
/// Frames of a tick at 120 BPM: 25.
const FRAMES_PER_TICK: u64 = 25;
/// Frames of a sixteenth (240 ticks).
const SIXTEENTH: u64 = 240 * FRAMES_PER_TICK;

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
    cascade: Cascade,
    out: EventBuffer,
    frame: u64,
    playing: bool,
    /// Added to the playhead (a seek), in ticks.
    shift: f64,
    block: u64,
    log: Vec<Out>,
}

impl Rig {
    fn new(params: &[(u32, f32)]) -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE_HZ,
            ..DeviceSetup::default()
        };
        let mut cascade = Cascade::new(&setup).unwrap();
        for (id, value) in params {
            assert_no_alloc(|| cascade.set_param(ParamId(*id), *value));
        }
        assert_no_alloc(|| cascade.reset());
        Rig {
            cascade,
            out: EventBuffer::try_with_capacity(DESCRIPTOR.midi_output.unwrap().capacity(1024))
                .unwrap(),
            frame: 0,
            playing: true,
            shift: 0.0,
            block: 128,
            log: Vec::new(),
        }
    }

    fn set(&mut self, id: u32, value: f32) {
        let cascade = &mut self.cascade;
        assert_no_alloc(|| cascade.set_param(ParamId(id), value));
    }

    fn reset(&mut self) {
        let cascade = &mut self.cascade;
        assert_no_alloc(|| cascade.reset());
    }

    /// Renders up to `until`, applying `events` (absolute frame, event); the transport stops at
    /// the first block starting at or after `stop_at` (if any), the playhead then standing.
    fn run_until_stop(&mut self, events: &[(u64, Event)], until: u64, stop_at: Option<u64>) {
        let mut pending: Vec<(u64, Event)> = events.to_vec();
        pending.sort_by_key(|(f, _)| *f);
        let mut next = pending.partition_point(|(f, _)| *f < self.frame);
        let mut stopped_at: Option<u64> = None;
        while self.frame < until {
            let frames = self.block.min(until - self.frame) as usize;
            let mut block = Vec::new();
            while next < pending.len() && pending[next].0 < self.frame + frames as u64 {
                let (f, e) = pending[next];
                block.push(e.at((f - self.frame) as u32));
                next += 1;
            }
            if stop_at.is_some_and(|s| self.frame >= s) && self.playing {
                self.playing = false;
                stopped_at = Some(self.frame);
            }
            let clock_frame = stopped_at.unwrap_or(self.frame);
            let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
            let ctx = BlockCtx {
                sample_rate: RATE_HZ,
                frames: frames as u32,
                transport: Transport {
                    playing: self.playing,
                    position_ticks: clock_frame as f64 / FRAMES_PER_TICK as f64 + self.shift,
                    ..Transport::default()
                },
            };
            self.out.clear();
            let (cascade, out) = (&mut self.cascade, &mut self.out);
            assert_no_alloc(|| {
                let mut io = AudioIo {
                    left: &mut l,
                    right: &mut r,
                    events_out: out,
                };
                cascade.process(&ctx, &block, &mut io);
            });
            assert_eq!(self.out.overflowed(), 0, "the output never overflows");
            assert!(
                self.out.events().len() <= DESCRIPTOR.midi_output.unwrap().capacity(block.len()),
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

    fn keys(&self) -> Vec<u8> {
        self.notes().iter().map(|n| n.2).collect()
    }
}

fn on(voice: u32, key: u8) -> Event {
    on_at(voice, key, 0.75)
}

fn on_at(voice: u32, key: u8, velocity: f32) -> Event {
    Event::NoteOn {
        offset: 0,
        voice: VoiceId::new(voice).unwrap(),
        key,
        velocity,
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

/// C, E, G held from `at` (voices 1 to 3).
fn chord(at: u64) -> Vec<(u64, Event)> {
    vec![(at, on(1, 60)), (at, on(2, 64)), (at, on(3, 67))]
}

/// C, E, G released at `at`.
fn release(at: u64) -> Vec<(u64, Event)> {
    vec![(at, off(1, 60)), (at, off(2, 64)), (at, off(3, 67))]
}

#[test]
fn a_held_c_major_chord_goes_up_in_sixteenths_at_120_bpm_to_the_sample() {
    let mut r = Rig::new(&[]);
    r.run(&chord(0), 8 * SIXTEENTH);
    let notes = r.notes();
    assert_eq!(notes.len(), 8);
    for (i, (start, end, key, velocity)) in notes.iter().enumerate() {
        assert_eq!(*start, i as u64 * 6000, "note {i} start");
        // 80 % of 6000 frames.
        assert_eq!(*end - *start, 4800, "note {i} length");
        assert_eq!(*key, [60, 64, 67][i % 3], "note {i} key");
        assert_eq!(*velocity, 0.75);
    }
    // Released: the pattern stops, the note that sounds ends at its gate.
    let mut r = Rig::new(&[]);
    let mut events = chord(0);
    events.extend(release(7000));
    r.run(&events, 8 * SIXTEENTH);
    assert_eq!(r.keys(), [60, 64], "steps at 0 and 6000 only");
    assert!(r.sounding().is_empty());
    assert_eq!(r.cascade.held(), 0);
}

#[test]
fn a_chord_pressed_between_two_steps_waits_for_the_next_step() {
    let mut r = Rig::new(&[]);
    r.run(&chord(1000), 3 * SIXTEENTH);
    assert_eq!(r.notes()[0].0, 6000);
    assert_eq!(r.notes()[1].0, 12_000);
}

#[test]
fn the_rate_and_the_feel_set_the_step() {
    // (rate, feel, step in frames at 120 BPM: 25 frames a tick)
    for (rate, feel, step) in [
        (0, 0, 3840 * 25),
        (2, 0, 960 * 25),
        (3, 0, 480 * 25),
        (4, 1, 360 * 25),
        (4, 2, 160 * 25),
        (6, 0, 60 * 25),
        (6, 1, 90 * 25),
        (6, 2, 40 * 25),
    ] {
        let mut r = Rig::new(&[(RATE, rate as f32), (FEEL, feel as f32)]);
        r.run(&chord(0), step * 4 + 10);
        let starts: Vec<u64> = r.notes().iter().map(|n| n.0).collect();
        assert_eq!(
            starts,
            [0, step, 2 * step, 3 * step, 4 * step],
            "{rate}/{feel}"
        );
    }
    // A block size that is no divisor of a step moves no note off its frame.
    let mut r = Rig::new(&[(FEEL, TRIPLET as f32), (RATE, 5.0)]);
    r.block = 77;
    r.run(&chord(0), 10_000);
    assert!(
        r.notes()
            .iter()
            .enumerate()
            .all(|(i, n)| n.0 == i as u64 * 2000)
    );
}

#[test]
fn every_order_walks_the_pool() {
    let walk = |order: u32, octaves: f32, steps: u64| {
        let mut r = Rig::new(&[(ORDER, order as f32), (OCTAVES, octaves)]);
        // Played G, C, E.
        r.run(
            &[(0, on(1, 67)), (0, on(2, 60)), (0, on(3, 64))],
            (steps - 1) * SIXTEENTH + 10,
        );
        r.keys()
    };
    assert_eq!(walk(UP, 1.0, 7), [60, 64, 67, 60, 64, 67, 60]);
    assert_eq!(walk(DOWN, 1.0, 7), [67, 64, 60, 67, 64, 60, 67]);
    assert_eq!(walk(UP_DOWN, 1.0, 8), [60, 64, 67, 64, 60, 64, 67, 64]);
    assert_eq!(walk(DOWN_UP, 1.0, 8), [67, 64, 60, 64, 67, 64, 60, 64]);
    assert_eq!(walk(PLAYED, 1.0, 6), [67, 60, 64, 67, 60, 64]);
    assert_eq!(walk(UP, 2.0, 7), [60, 64, 67, 72, 76, 79, 60]);
    assert_eq!(walk(PLAYED, 2.0, 7), [67, 60, 64, 79, 72, 76, 67]);
    assert_eq!(
        walk(DOWN_UP, 2.0, 11),
        [79, 76, 72, 67, 64, 60, 64, 67, 72, 76, 79]
    );
}

#[test]
fn octaves_past_the_keyboard_are_left_out_and_equal_keys_merge() {
    let mut r = Rig::new(&[(OCTAVES, 4.0)]);
    // 100 and 112: 112 + 12 = 124 and 100 + 12 = 112 (the same key as the other note, once);
    // 136 and above do not exist.
    r.run(&[(0, on(1, 100)), (0, on(2, 112))], 5 * SIXTEENTH + 10);
    assert_eq!(r.keys(), [100, 112, 124, 100, 112, 124]);
}

#[test]
fn one_note_walks_itself_in_every_order() {
    for order in [UP, DOWN, UP_DOWN, DOWN_UP, PLAYED, RANDOM, CHORD] {
        let mut r = Rig::new(&[(ORDER, order as f32)]);
        r.run(&[(0, on(1, 50))], 3 * SIXTEENTH + 10);
        assert_eq!(r.keys(), [50, 50, 50, 50], "order {order}");
    }
}

#[test]
fn the_chord_order_strikes_every_note_of_the_pool_at_each_step() {
    let mut r = Rig::new(&[(ORDER, CHORD as f32), (OCTAVES, 2.0)]);
    r.run(&chord(0), SIXTEENTH + 10);
    let notes = r.notes();
    assert_eq!(notes.len(), 12);
    for (i, n) in notes.iter().enumerate() {
        assert_eq!(n.0, (i / 6) as u64 * 6000);
    }
    assert_eq!(r.keys()[..6], [60, 64, 67, 72, 76, 79]);
}

#[test]
fn repeats_play_each_note_in_a_row() {
    let mut r = Rig::new(&[(REPEATS, 2.0)]);
    r.run(&chord(0), 6 * SIXTEENTH + 10);
    assert_eq!(r.keys(), [60, 60, 64, 64, 67, 67, 60]);
    let mut r = Rig::new(&[(REPEATS, 3.0), (ORDER, UP_DOWN as f32)]);
    r.run(&chord(0), 8 * SIXTEENTH + 10);
    assert_eq!(r.keys(), [60, 60, 60, 64, 64, 64, 67, 67, 67]);
}

#[test]
fn the_gate_sets_the_length_and_a_long_gate_overlaps_without_a_stuck_note() {
    for (gate, length) in [(1.0, 60), (50.0, 3000), (100.0, 6000), (200.0, 6000)] {
        let mut r = Rig::new(&[(GATE, gate)]);
        r.run(&chord(0), 6 * SIXTEENTH);
        let notes = r.notes();
        // At 200 % a note lasts two steps, but the next strike of its key (the third step
        // after) comes later: three keys, so it lasts the 12000 frames of its gate.
        let expected = if gate == 200.0 { 12_000 } else { length };
        for n in &notes[..4] {
            assert_eq!(n.1 - n.0, expected, "gate {gate}");
        }
        r.run(&release(6 * SIXTEENTH), 12 * SIXTEENTH);
        assert!(r.sounding().is_empty(), "gate {gate}");
    }
    // The same key struck again while it sounds ends first, at the same frame.
    let mut r = Rig::new(&[(GATE, 200.0)]);
    r.run(&[(0, on(1, 60))], 3 * SIXTEENTH);
    let n = r.notes();
    assert_eq!(n[0].1, 6000, "ended by the next strike");
    assert_eq!(n[1].0, 6000);
}

#[test]
fn velocity_is_played_or_fixed_and_the_first_step_of_a_pass_is_accented() {
    let mut r = Rig::new(&[(VELOCITY_SOURCE, FIXED as f32), (VELOCITY, 64.0)]);
    r.run(&chord(0), 3 * SIXTEENTH);
    assert!(r.notes().iter().all(|n| n.3 == 64.0 / 127.0));
    let mut r = Rig::new(&[(ACCENT, 30.0)]);
    r.run(
        &[
            (0, on_at(1, 60, 0.5)),
            (0, on_at(2, 64, 0.5)),
            (0, on_at(3, 67, 0.5)),
        ],
        6 * SIXTEENTH + 10,
    );
    let v: Vec<f32> = r.notes().iter().map(|n| n.3).collect();
    let (low, high) = (0.5, 0.5 + 30.0 / 127.0);
    assert_eq!(v, [high, low, low, high, low, low, high]);
    // Capped at 1.
    let mut r = Rig::new(&[(ACCENT, 60.0)]);
    r.run(&[(0, on_at(1, 60, 1.0))], SIXTEENTH);
    assert_eq!(r.notes()[0].3, 1.0);
}

#[test]
fn latch_goes_on_after_the_release_until_the_next_chord() {
    let mut r = Rig::new(&[(LATCH, 1.0)]);
    let mut events = chord(0);
    events.extend(release(3000));
    r.run(&events, 5 * SIXTEENTH + 10);
    assert_eq!(r.keys(), [60, 64, 67, 60, 64, 67], "the pattern goes on");
    assert_eq!(r.cascade.held(), 3);
    // A new chord replaces the latched notes and starts the pattern over.
    let t = 5 * SIXTEENTH + 100;
    r.run(&[(t, on(4, 62)), (t, on(5, 65))], 8 * SIXTEENTH + 10);
    assert_eq!(r.keys()[6..], [62, 65, 62]);
    assert_eq!(r.cascade.held(), 2);
    // Latch off: the released notes go at once, the notes down stay.
    r.run(&[(8 * SIXTEENTH + 20, off(4, 62))], 9 * SIXTEENTH);
    assert_eq!(r.cascade.held(), 2, "latched");
    r.set(LATCH, 0.0);
    assert_eq!(r.cascade.held(), 1, "the key still down stays");
    r.run(&[], 12 * SIXTEENTH);
    assert_eq!(*r.keys().last().unwrap(), 65);
}

#[test]
fn restart_starts_over_at_each_chord_or_follows_the_grid() {
    // A chord pressed at frame 7000: its first step is at 12000 (step 2 of the grid).
    let mut r = Rig::new(&[(RESTART, 0.0)]);
    r.run(&chord(7000), 12_000 + 2 * 6000 + 10);
    assert_eq!(r.keys(), [60, 64, 67], "new chord: from the first note");
    assert_eq!(r.notes()[0].0, 12_000);
    let mut r = Rig::new(&[(RESTART, FREE as f32)]);
    r.run(&chord(7000), 12_000 + 2 * 6000 + 10);
    assert_eq!(
        r.keys(),
        [67, 60, 64],
        "free: step 2 of the grid plays note 2"
    );
    // A second chord restarts a new_chord pattern.
    let mut r = Rig::new(&[]);
    let mut events = chord(0);
    events.extend(release(13_000));
    events.push((13_500, on(4, 72)));
    events.push((13_500, on(5, 76)));
    r.run(&events, 5 * 6000 + 10);
    assert_eq!(r.keys(), [60, 64, 67, 72, 76, 72]);
}

#[test]
fn the_random_order_is_repeatable_from_any_start_and_seed_dependent() {
    let render = |seed: f32, start: u64| {
        let mut r = Rig::new(&[
            (ORDER, RANDOM as f32),
            (OCTAVES, 3.0),
            (SEED, seed),
            (RESTART, FREE as f32),
        ]);
        r.run(&chord(start), 40 * SIXTEENTH);
        r.notes()
            .iter()
            .filter(|n| n.0 >= 20 * SIXTEENTH)
            .map(|n| (n.0, n.2))
            .collect::<Vec<_>>()
    };
    let a = render(7.0, 0);
    assert!(a.len() >= 19);
    assert_eq!(a, render(7.0, 0), "same seed, same notes");
    assert_eq!(
        a,
        render(7.0, 6000 * 3 + 77),
        "from another start: the same notes at the same steps"
    );
    assert_ne!(a, render(8.0, 0), "another seed, another draw");
    let keys: std::collections::BTreeSet<u8> = a.iter().map(|n| n.1).collect();
    assert!(keys.len() >= 5, "it moves around the whole pool: {keys:?}");
    // Repeats play the drawn note again.
    let mut r = Rig::new(&[(ORDER, RANDOM as f32), (REPEATS, 3.0), (OCTAVES, 4.0)]);
    r.run(&chord(0), 30 * SIXTEENTH);
    let keys = r.keys();
    for g in keys.as_chunks::<3>().0 {
        assert!(g.iter().all(|k| *k == g[0]), "{g:?}");
    }
}

#[test]
fn a_stopped_transport_keeps_its_own_clock() {
    let mut r = Rig::new(&[]);
    r.playing = false;
    r.run(&chord(0), 4 * SIXTEENTH + 10);
    let starts: Vec<u64> = r.notes().iter().map(|n| n.0).collect();
    assert_eq!(starts, [0, 6000, 12_000, 18_000, 24_000]);
    assert_eq!(r.keys(), [60, 64, 67, 60, 64]);
}

#[test]
fn a_stop_and_a_seek_move_the_grid_never_the_notes_sounding() {
    // Playing, the transport stops at 20000 and the pattern goes on, on the device's own clock.
    let mut r = Rig::new(&[]);
    r.run_until_stop(&chord(0), 7 * SIXTEENTH + 10, Some(20_000));
    assert_eq!(r.notes().len(), 8);
    let starts: Vec<u64> = r.notes().iter().map(|n| n.0).collect();
    assert_eq!(
        starts,
        [0, 6000, 12_000, 18_000, 24_000, 30_000, 36_000, 42_000]
    );
    // The stop ends the note sounding (struck at 18000, due at 22800) at its first block, 20096.
    assert_eq!(r.notes()[3].1, 20_096);
    assert!(r.notes()[4..7].iter().all(|n| n.1 - n.0 == 4800));
    // A seek back by 5 ticks places the next step on the grid of the new position: 720 is
    // 25 frames ahead of the playhead (719 ticks at frame 18100).
    let mut r = Rig::new(&[]);
    r.run(&chord(0), 18_100);
    r.shift = -5.0;
    r.run(&[], 6 * SIXTEENTH);
    let starts: Vec<u64> = r.notes().iter().map(|n| n.0).collect();
    assert_eq!(starts, [0, 6000, 12_000, 18_000, 18_125, 24_125, 30_125]);
    r.run(&release(6 * SIXTEENTH), 9 * SIXTEENTH);
    assert!(r.sounding().is_empty());
}

#[test]
fn an_all_notes_off_a_reset_and_a_setting_changed_in_the_middle_of_a_note_leave_nothing_stuck() {
    // AllNotesOff: passed on, the held notes forgotten, the notes sounding ended by it.
    let mut r = Rig::new(&[]);
    let mut events = chord(0);
    events.push((7000, Event::AllNotesOff { offset: 0 }));
    r.run(&events, 8 * SIXTEENTH);
    assert!(r.log.iter().any(|o| matches!(o, Out::All { frame: 7000 })));
    assert_eq!(r.notes().len(), 2, "nothing after it");
    assert!(r.sounding().is_empty());
    assert_eq!((r.cascade.held(), r.cascade.sounding()), (0, 0));

    // A reset empties both tables.
    let mut r = Rig::new(&[]);
    r.run(&chord(0), 100);
    assert_eq!((r.cascade.held(), r.cascade.sounding()), (3, 1));
    r.reset();
    assert_eq!((r.cascade.held(), r.cascade.sounding()), (0, 0));

    // Each setting changed while a note sounds: no note is left on once the keys are up.
    for p in DESCRIPTOR.params {
        let mut r = Rig::new(&[(GATE, 150.0)]);
        let mut events = chord(0);
        events.extend(release(20_000));
        r.run(&events, 3000);
        r.set(p.id.0, p.max as f32);
        r.run(&events, 9000);
        r.set(p.id.0, p.min as f32);
        r.run(&events, 39_000);
        r.set(p.id.0, p.default as f32);
        r.run(&events, 80_000);
        r.set(LATCH, 0.0);
        assert!(
            r.sounding().is_empty(),
            "{} changed mid-play: {:?}",
            p.key,
            r.sounding()
        );
    }
    // A change of the rate in the middle of a step: the next step is on the new grid.
    let mut r = Rig::new(&[]);
    r.run(&chord(0), 7000);
    r.set(RATE, 3.0);
    r.run(&[], 27_000);
    let starts: Vec<u64> = r.notes().iter().map(|n| n.0).collect();
    assert_eq!(starts, [0, 6000, 12_000, 24_000]);
}

#[test]
fn a_param_event_mid_block_takes_effect_at_its_frame() {
    let mut r = Rig::new(&[]);
    r.block = 4096;
    let change = Event::ParamValue {
        offset: 0,
        id: ParamId(RATE),
        value: 3.0,
    };
    let mut events = chord(0);
    events.push((6100, change));
    r.run(&events, 30_000);
    let starts: Vec<u64> = r.notes().iter().map(|n| n.0).collect();
    // 0 and 6000 on the 1/16 grid; from 6100 the 1/8 grid (12000 frames): 12000 and 24000.
    assert_eq!(starts, [0, 6000, 12_000, 24_000]);
}

#[test]
fn the_tables_are_bounded_and_the_oldest_held_note_makes_room() {
    let mut r = Rig::new(&[
        (ORDER, CHORD as f32),
        (OCTAVES, 4.0),
        (GATE, 200.0),
        (RATE, 6.0),
    ]);
    let events: Vec<(u64, Event)> = (0..SOURCES as u32 + 6)
        .map(|i| (0, on(i + 1, 30 + i as u8 * 3)))
        .collect();
    r.block = 4096;
    r.run(&events, 60_000);
    assert_eq!(r.cascade.held(), SOURCES);
    assert!(r.cascade.sounding() <= SOUNDING);
    r.run(&[(60_000, Event::AllNotesOff { offset: 0 })], 70_000);
    assert!(r.sounding().is_empty());
}

#[test]
fn a_storm_of_steps_in_a_huge_block_is_cut_not_overflowed() {
    let mut r = Rig::new(&[
        (ORDER, CHORD as f32),
        (OCTAVES, 4.0),
        (RATE, 6.0),
        (FEEL, TRIPLET as f32),
    ]);
    r.block = 48_000;
    r.run(&chord(0), 96_000);
    assert!(r.cascade.sounding() <= SOUNDING);
    r.run(&[(96_000, Event::AllNotesOff { offset: 0 })], 100_000);
    assert!(r.sounding().is_empty());
}

#[test]
fn every_parameter_at_its_bounds_and_default_gives_finite_bounded_events() {
    for p in DESCRIPTOR.params {
        for value in [p.min, p.default, p.max] {
            let mut r = Rig::new(&[(p.id.0, value as f32)]);
            let mut events = chord(100);
            events.extend(release(60_000));
            r.run(&events, 140_000);
            // With the latch on the notes stay held, and the pattern goes on.
            r.set(LATCH, 0.0);
            r.run(&[], 160_000);
            assert!(r.sounding().is_empty(), "{} at {value}", p.key);
            for (start, end, ..) in r.notes() {
                assert!(end >= start);
            }
        }
    }
    // Garbage values are ignored, for every parameter.
    let mut r = Rig::new(&[]);
    for bad in [f32::NAN, f32::INFINITY, -1e30, 1e30] {
        for p in DESCRIPTOR.params {
            r.set(p.id.0, bad);
        }
        r.set(99, bad);
    }
    r.set(LATCH, 0.0);
    r.run(&chord(0), 60_000);
    r.run(&release(60_000), 120_000);
    assert!(r.sounding().is_empty());
}

#[test]
fn a_stop_ends_the_latched_pattern_and_the_notes_sounding_a_held_key_goes_on() {
    // Latched, then the transport stops at the block of 13056: the note struck at 12000 ends
    // there, nothing is struck after, the latched notes are gone.
    let mut r = Rig::new(&[(LATCH, 1.0)]);
    let mut events = chord(0);
    events.extend(release(3000));
    r.run_until_stop(&events, 4 * SIXTEENTH, Some(13_000));
    let notes = r.notes();
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert_eq!(notes[2].1, 13_056);
    assert!(r.sounding().is_empty());
    assert_eq!(r.cascade.held(), 0);
    // A key still down is not latched: it goes on, on the device's own clock.
    let mut r = Rig::new(&[]);
    r.run_until_stop(&[(0, on(1, 60))], 5 * SIXTEENTH + 10, Some(13_000));
    let starts: Vec<u64> = r.notes().iter().map(|n| n.0).collect();
    assert_eq!(starts, [0, 6000, 12_000, 18_000, 24_000, 30_000]);
    assert_eq!(r.cascade.held(), 1);
}

#[test]
fn the_random_order_with_repeats_draws_from_the_grid_whatever_the_restart() {
    let render = |restart: u32, start: u64| {
        let mut r = Rig::new(&[
            (ORDER, RANDOM as f32),
            (OCTAVES, 3.0),
            (REPEATS, 2.0),
            (SEED, 9.0),
            (RESTART, restart as f32),
        ]);
        r.run(&chord(start), 40 * SIXTEENTH);
        r.notes()
            .iter()
            .filter(|n| n.0 >= 10 * SIXTEENTH)
            .map(|n| (n.0, n.2))
            .collect::<Vec<_>>()
    };
    for restart in [0, FREE] {
        let a = render(restart, 0);
        assert!(a.len() > 25);
        // A chord struck on an odd step, in the middle of a group of repeats: the same notes at
        // the same steps from the next group on.
        assert_eq!(a, render(restart, 5 * SIXTEENTH - 10), "restart {restart}");
        assert_eq!(a, render(restart, 4 * SIXTEENTH - 10), "restart {restart}");
    }
}

/// The notes of `blocks` blocks of `frames` frames, the playhead accumulated as an engine does
/// (with an error of a few ten-millionths of a tick), the tempo of block `b` being
/// `tempo(b)`: each as (step number, its distance from the grid in frames).
fn grid_steps(
    rate: f32,
    frames: u32,
    blocks: u32,
    tempo: impl Fn(u32) -> f64,
    params: &[(u32, f32)],
) -> Vec<(i64, f64)> {
    let setup = DeviceSetup {
        sample_rate: rate,
        ..DeviceSetup::default()
    };
    let mut c = Cascade::new(&setup).unwrap();
    for (id, value) in params {
        c.set_param(ParamId(*id), *value);
    }
    c.reset();
    let step_ticks = 240.0;
    let mut out =
        EventBuffer::try_with_capacity(DESCRIPTOR.midi_output.unwrap().capacity(8)).unwrap();
    let (mut l, mut r) = (vec![0.0; frames as usize], vec![0.0; frames as usize]);
    let mut position = 0.0f64;
    let mut found = Vec::new();
    for b in 0..blocks {
        let tempo = engine::time::Tempo::new(tempo(b)).unwrap();
        let tpf = tempo.ticks_per_second() / f64::from(rate);
        let ctx = BlockCtx {
            sample_rate: rate,
            frames,
            transport: Transport {
                playing: true,
                tempo,
                position_ticks: position,
                ..Transport::default()
            },
        };
        out.clear();
        let held = [on(1, 60)];
        let mut io = AudioIo {
            left: &mut l,
            right: &mut r,
            events_out: &mut out,
        };
        assert_no_alloc(|| c.process(&ctx, if b == 0 { &held } else { &[] }, &mut io));
        for e in out.events() {
            if let Event::NoteOn { offset, .. } = e {
                let tick = position + f64::from(*offset) * tpf;
                let k = (tick / step_ticks).round();
                found.push((k as i64, (tick - k * step_ticks) / tpf));
            }
        }
        position += f64::from(frames) * tpf + if b % 2 == 0 { 3e-7 } else { -3e-7 };
    }
    found
}

#[test]
fn steps_stay_on_the_grid_through_tempo_changes_odd_blocks_and_44_1_khz() {
    let tempo = |b: u32| match b {
        0..=1999 => 120.0,
        2000..=3999 => 93.7,
        _ => 170.0,
    };
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        for frames in [97, 128, 1000] {
            let found = grid_steps(rate, frames, 6000, tempo, &[]);
            assert!(found.len() > 40, "{rate} {frames}: {}", found.len());
            for (i, (k, error)) in found.iter().enumerate() {
                assert_eq!(
                    *k, i as i64,
                    "{rate} Hz, {frames} frames: no step lost or doubled"
                );
                // The note is on the frame its tick falls in: the frame its tick falls in: at most one frame before it.
                assert!(
                    (-1.001..0.001).contains(error),
                    "{rate} Hz, {frames} frames, step {k}: {error} frames from the grid"
                );
            }
        }
    }
}

#[test]
fn no_step_is_skipped_in_the_biggest_block_at_the_fastest_tempo_and_step() {
    // 2048 frames at 44.1 kHz, 999 BPM, a triplet sixty-fourth (40 ticks): 18 steps a block.
    let setup = DeviceSetup {
        sample_rate: 44_100.0,
        ..DeviceSetup::default()
    };
    let mut c = Cascade::new(&setup).unwrap();
    c.set_param(ParamId(RATE), 6.0);
    c.set_param(ParamId(FEEL), TRIPLET as f32);
    c.reset();
    let tempo = engine::time::Tempo::new(999.0).unwrap();
    let tpf = tempo.ticks_per_second() / 44_100.0;
    let mut out =
        EventBuffer::try_with_capacity(DESCRIPTOR.midi_output.unwrap().capacity(8)).unwrap();
    let (mut l, mut r) = (vec![0.0; 2048], vec![0.0; 2048]);
    let mut ons = 0;
    let mut position = 0.0;
    for b in 0..20 {
        let ctx = BlockCtx {
            sample_rate: 44_100.0,
            frames: 2048,
            transport: Transport {
                playing: true,
                tempo,
                position_ticks: position,
                ..Transport::default()
            },
        };
        out.clear();
        let held = [on(1, 60)];
        let mut io = AudioIo {
            left: &mut l,
            right: &mut r,
            events_out: &mut out,
        };
        assert_no_alloc(|| c.process(&ctx, if b == 0 { &held } else { &[] }, &mut io));
        assert_eq!(out.overflowed(), 0);
        ons += out
            .events()
            .iter()
            .filter(|e| matches!(e, Event::NoteOn { .. }))
            .count();
        position += 2048.0 * tpf;
    }
    // Steps at ticks 0, 40, 80, ... below the end of the 20 blocks.
    assert_eq!(ons, (position / 40.0).ceil() as usize);
}

#[test]
fn a_note_and_the_copy_of_another_an_octave_below_merge_keeping_the_held_note() {
    // C4 soft and C5 hard over two octaves: the pool holds C5 once, with C5's own velocity.
    for order in [UP, DOWN] {
        let mut r = Rig::new(&[(ORDER, order as f32), (OCTAVES, 2.0)]);
        r.run(
            &[(0, on_at(1, 60, 0.2)), (0, on_at(2, 72, 0.9))],
            4 * SIXTEENTH + 10,
        );
        let notes = r.notes();
        let c5: Vec<f32> = notes.iter().filter(|n| n.2 == 72).map(|n| n.3).collect();
        assert!(!c5.is_empty() && c5.iter().all(|v| *v == 0.9), "{notes:?}");
        // C4, C5, C6 (the copy of C5): three distinct keys, not four.
        let keys: std::collections::BTreeSet<u8> = notes.iter().map(|n| n.2).collect();
        assert_eq!(keys.len(), 3, "{keys:?}");
    }
}

#[test]
fn a_voice_id_the_host_reuses_never_overwrites_a_released_latched_note() {
    let mut r = Rig::new(&[(LATCH, 1.0)]);
    // Key 48 stays down; 60 is pressed with voice 5 and released (latched); the host then gives
    // voice 5 to 67 while 48 is still down: no new chord, 60 stays in the pattern.
    r.run(
        &[
            (0, on(1, 48)),
            (100, on(5, 60)),
            (200, off(5, 60)),
            (300, on(5, 67)),
        ],
        400,
    );
    assert_eq!(r.cascade.held(), 3);
    r.run(&[], 4 * SIXTEENTH + 10);
    let keys: std::collections::BTreeSet<u8> = r.keys().into_iter().collect();
    assert_eq!(keys, [48, 60, 67].into_iter().collect());
}

#[test]
fn an_absurd_playhead_neither_panics_nor_sticks_a_note() {
    for order in [UP, RANDOM, CHORD] {
        for position in [1e30, -1e30, f64::MAX, 1e18] {
            let setup = DeviceSetup {
                sample_rate: RATE_HZ,
                ..DeviceSetup::default()
            };
            let mut c = Cascade::new(&setup).unwrap();
            c.set_param(ParamId(ORDER), order as f32);
            c.set_param(ParamId(REPEATS), 3.0);
            c.reset();
            let mut out =
                EventBuffer::try_with_capacity(DESCRIPTOR.midi_output.unwrap().capacity(8))
                    .unwrap();
            let (mut l, mut r) = ([0.0; 128], [0.0; 128]);
            for b in 0..40 {
                let ctx = BlockCtx {
                    sample_rate: RATE_HZ,
                    frames: 128,
                    transport: Transport {
                        playing: true,
                        position_ticks: position,
                        ..Transport::default()
                    },
                };
                out.clear();
                let held = [on(1, 60), on(2, 64)];
                let mut io = AudioIo {
                    left: &mut l,
                    right: &mut r,
                    events_out: &mut out,
                };
                assert_no_alloc(|| c.process(&ctx, if b == 0 { &held } else { &[] }, &mut io));
                assert_eq!(out.overflowed(), 0);
            }
            assert!(c.sounding() <= SOUNDING);
        }
    }
}

#[test]
fn the_declared_bounds_leave_room_for_the_effects_after_cascade() {
    use engine::device::MAX_MIDI_OUTPUT_EVENTS;
    use engine::devices::{lattice, scatter, stack};
    let out = DESCRIPTOR.midi_output.unwrap().capacity(64);
    assert!(out > PER_BLOCK as usize);
    for d in [
        &stack::DESCRIPTOR,
        &scatter::DESCRIPTOR,
        &lattice::DESCRIPTOR,
    ] {
        let after = d.midi_output.unwrap().capacity(out);
        assert!(
            after < MAX_MIDI_OUTPUT_EVENTS,
            "{} after Cascade: {after}",
            d.kind
        );
    }
}

#[test]
fn the_tempo_of_the_block_sets_the_frames_of_a_step() {
    // 60 BPM: 50 frames a tick, a sixteenth is 12000 frames.
    let setup = DeviceSetup {
        sample_rate: RATE_HZ,
        ..DeviceSetup::default()
    };
    let mut c = Cascade::new(&setup).unwrap();
    let mut out =
        EventBuffer::try_with_capacity(DESCRIPTOR.midi_output.unwrap().capacity(8)).unwrap();
    let mut starts = Vec::new();
    for b in 0..400u64 {
        let frames = 128u64;
        let ctx = BlockCtx {
            sample_rate: RATE_HZ,
            frames: frames as u32,
            transport: Transport {
                playing: true,
                tempo: engine::time::Tempo::new(60.0).unwrap(),
                position_ticks: (b * frames) as f64 / 50.0,
                ..Transport::default()
            },
        };
        let held = [on(1, 60)];
        out.clear();
        let (mut l, mut r) = ([0.0; 128], [0.0; 128]);
        let mut io = AudioIo {
            left: &mut l,
            right: &mut r,
            events_out: &mut out,
        };
        assert_no_alloc(|| c.process(&ctx, if b == 0 { &held } else { &[] }, &mut io));
        for e in out.events() {
            if let Event::NoteOn { offset, .. } = e {
                starts.push(b * frames + u64::from(*offset));
            }
        }
    }
    assert_eq!(starts, [0, 12_000, 24_000, 36_000, 48_000]);
}
