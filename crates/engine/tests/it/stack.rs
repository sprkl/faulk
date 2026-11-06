//! Stack (#690): the chord's notes (intervals, the scale's degrees, velocities, the received
//! note's fine tuning), the strum's onsets to the frame (tension, crescendo, direction, across
//! block cuts), the chance draw at equal seed, no stuck note (a note-off during the strum, a
//! stop, a setting changed mid-note, a full table, a reset), the output bounds, a sweep of every
//! parameter, and the audio path under the allocation detector. The render through the synth,
//! native and WASM, is in `crates/test-support/tests/it/stack.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceSetup, Event, EventBuffer, PitchSet, Transport, VoiceId,
};
use engine::devices::stack::{
    self, CRESCENDO, DESCRIPTOR, DIRECTION, DOWN, HELD, IN_SCALE, MODE, SEED, STRUM, Stack,
    TENSION, VOICE_CHANCE, VOICE_INTERVAL, VOICE_ON, VOICE_VELOCITY, degree_shift, voice_param,
};
use engine::msg::ParamId;
use std::collections::BTreeMap;

const RATE: f32 = 48_000.0;

/// A MIDI pitch class set: A minor (the notes of C major).
const A_MINOR: PitchSet = PitchSet::from_bits(0b1010_1011_0101);

/// A note out of Stack, at its absolute frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Out {
    On {
        frame: u64,
        voice: u32,
        key: u8,
        velocity: f32,
        tune: f32,
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

/// Drives a Stack with events at absolute frames, cut in blocks of `block` frames.
struct Rig {
    stack: Stack,
    out: EventBuffer,
    frame: u64,
    playing: bool,
    scale: PitchSet,
    block: usize,
    log: Vec<Out>,
}

impl Rig {
    fn new(params: &[(u32, f32)]) -> Rig {
        Rig::with_block(params, 128)
    }

    fn with_block(params: &[(u32, f32)], block: usize) -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE,
            ..DeviceSetup::default()
        };
        let mut stack = Stack::new(&setup).unwrap();
        for (id, value) in params {
            stack.set_param(ParamId(*id), *value);
        }
        stack.reset();
        let capacity = DESCRIPTOR.midi_output.unwrap().capacity(64);
        Rig {
            stack,
            out: EventBuffer::try_with_capacity(capacity).unwrap(),
            frame: 0,
            playing: true,
            scale: PitchSet::EMPTY,
            block,
            log: Vec::new(),
        }
    }

    /// Renders up to `until` (absolute frame), applying `events` (absolute frame, event).
    fn run(&mut self, events: &[(u64, Event)], until: u64) {
        let mut pending: Vec<(u64, Event)> = events.to_vec();
        pending.sort_by_key(|(f, _)| *f);
        let mut next = 0;
        while self.frame < until {
            let frames = (self.block as u64).min(until - self.frame) as usize;
            let mut block = Vec::new();
            while next < pending.len() && pending[next].0 < self.frame + frames as u64 {
                let (f, e) = pending[next];
                block.push(e.at((f - self.frame) as u32));
                next += 1;
            }
            let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
            let ctx = BlockCtx {
                sample_rate: RATE,
                frames: frames as u32,
                transport: Transport {
                    playing: self.playing,
                    scale: self.scale,
                    ..Transport::default()
                },
            };
            self.out.clear();
            let (stack, out) = (&mut self.stack, &mut self.out);
            assert_no_alloc(|| {
                let mut io = AudioIo {
                    left: &mut l,
                    right: &mut r,
                    events_out: out,
                };
                stack.process(&ctx, &block, &mut io);
            });
            assert_eq!(self.out.overflowed(), 0, "the output never overflows");
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
                    } => Out::On {
                        frame,
                        voice: voice.get(),
                        key,
                        velocity,
                        tune: tune_cents,
                    },
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

    fn ons(&self) -> Vec<(u64, u8, f32)> {
        self.log
            .iter()
            .filter_map(|o| match *o {
                Out::On {
                    frame,
                    key,
                    velocity,
                    ..
                } => Some((frame, key, velocity)),
                _ => None,
            })
            .collect()
    }

    /// The notes still sounding: every note-on matched by a note-off of its voice, an
    /// all-notes-off ending them all.
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
}

fn on(voice: u32, key: u8, velocity: f32) -> Event {
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

fn v(voice: u32, which: u32) -> u32 {
    voice_param(voice, which)
}

fn ms(frames: f64) -> u64 {
    (frames * f64::from(RATE) / 1000.0).round() as u64
}

// ---------------------------------------------------------------- the chord

#[test]
fn c4_with_the_default_major_triad_gives_c_e_g_at_the_same_frame() {
    let mut r = Rig::new(&[(v(0, VOICE_VELOCITY), 50.0), (v(1, VOICE_VELOCITY), 150.0)]);
    r.run(&[(300, on(1, 60, 0.6))], 1_000);
    let ons = r.ons();
    assert_eq!(
        ons.iter().map(|o| (o.0, o.1)).collect::<Vec<_>>(),
        [(300, 60), (300, 64), (300, 67)]
    );
    // Velocities: the root's, 50 % of it, 150 % of it (at most 1).
    assert!((ons[0].2 - 0.6).abs() < 1e-6);
    assert!((ons[1].2 - 0.3).abs() < 1e-6);
    assert!((ons[2].2 - 0.9).abs() < 1e-6);
    let mut r2 = Rig::new(&[(v(1, VOICE_VELOCITY), 200.0)]);
    r2.run(&[(0, on(1, 60, 0.8))], 128);
    assert_eq!(r2.ons()[2].2, 1.0, "clamped to 1");
}

#[test]
fn every_note_of_the_chord_carries_the_received_tuning() {
    let mut r = Rig::new(&[]);
    let tuned = Event::NoteOn {
        offset: 0,
        voice: VoiceId::new(1).unwrap(),
        key: 60,
        velocity: 1.0,
        tune_cents: -31.5,
    };
    r.run(&[(0, tuned)], 128);
    let tunes: Vec<f32> = r
        .log
        .iter()
        .filter_map(|o| match *o {
            Out::On { tune, .. } => Some(tune),
            _ => None,
        })
        .collect();
    assert_eq!(tunes, [-31.5; 3]);
}

#[test]
fn notes_beyond_the_keyboard_are_dropped_not_doubled() {
    let mut r = Rig::new(&[(v(0, VOICE_INTERVAL), 36.0), (v(1, VOICE_INTERVAL), -36.0)]);
    r.run(&[(0, on(1, 120, 1.0))], 128);
    assert_eq!(
        r.ons().iter().map(|o| o.1).collect::<Vec<_>>(),
        [84, 120],
        "+36 is past 127"
    );
}

#[test]
fn in_a_minor_two_and_four_degrees_give_the_triads_of_the_scale() {
    let mut r = Rig::new(&[
        (MODE, IN_SCALE as f32),
        (v(0, VOICE_INTERVAL), 2.0),
        (v(1, VOICE_INTERVAL), 4.0),
    ]);
    r.scale = A_MINOR;
    // A4 (69) → C5, E5; B4 (71) → D5, F5.
    r.run(
        &[
            (0, on(1, 69, 1.0)),
            (500, off(1, 69)),
            (600, on(2, 71, 1.0)),
        ],
        1_000,
    );
    assert_eq!(
        r.ons().iter().map(|o| o.1).collect::<Vec<_>>(),
        [69, 72, 76, 71, 74, 77]
    );
    // Without a project scale, scale mode reads semitones.
    let mut r = Rig::new(&[
        (MODE, IN_SCALE as f32),
        (v(0, VOICE_INTERVAL), 2.0),
        (v(1, VOICE_INTERVAL), 4.0),
    ]);
    r.run(&[(0, on(1, 69, 1.0))], 128);
    assert_eq!(
        r.ons().iter().map(|o| o.1).collect::<Vec<_>>(),
        [69, 71, 73]
    );
}

#[test]
fn degrees_wrap_the_octave_both_ways_and_a_note_outside_the_scale_keeps_its_offset() {
    // C major: C4 (60) + 7 degrees is C5; −1 degree is B3.
    assert_eq!(degree_shift(60, 7, A_MINOR), 72);
    assert_eq!(degree_shift(60, -1, A_MINOR), 59);
    assert_eq!(degree_shift(60, -8, A_MINOR), 47);
    // C#4 counts from C4: +2 degrees is E4 + 1 = F4.
    assert_eq!(degree_shift(61, 2, A_MINOR), 65);
    // A pentatonic (five classes): +5 degrees is an octave.
    let penta = PitchSet::from_bits(0b0010_1001_0101); // C D E G A
    assert_eq!(degree_shift(62, 5, penta), 74);
}

// ---------------------------------------------------------------- the strum

#[test]
fn a_200_ms_strum_on_three_notes_starts_them_at_0_100_and_200_ms_whatever_the_blocks() {
    for block in [1, 64, 128, 1000, 4096] {
        let mut r = Rig::with_block(&[(STRUM, 0.2)], block);
        r.run(&[(1_000, on(1, 60, 1.0))], 20_000);
        assert_eq!(
            r.ons().iter().map(|o| (o.0, o.1)).collect::<Vec<_>>(),
            [(1_000, 60), (1_000 + 4_800, 64), (1_000 + 9_600, 67)],
            "block {block}"
        );
    }
    assert_eq!(ms(100.0), 4_800);
}

#[test]
fn down_strums_from_the_highest_note() {
    let mut r = Rig::new(&[(STRUM, 0.2), (DIRECTION, DOWN as f32)]);
    r.run(&[(0, on(1, 60, 1.0))], 12_000);
    assert_eq!(
        r.ons().iter().map(|o| (o.0, o.1)).collect::<Vec<_>>(),
        [(0, 67), (4_800, 64), (9_600, 60)]
    );
}

#[test]
fn tension_tightens_or_widens_the_gaps_and_keeps_the_ends() {
    let onsets = |tension: f32| {
        let mut r = Rig::new(&[(STRUM, 0.3), (TENSION, tension), (v(2, VOICE_ON), 1.0)]);
        r.run(&[(0, on(1, 60, 1.0))], 20_000);
        r.ons().iter().map(|o| o.0).collect::<Vec<_>>()
    };
    let even = onsets(0.0);
    assert_eq!(even, [0, 4_800, 9_600, 14_400]);
    let tight = onsets(100.0);
    let wide = onsets(-100.0);
    for o in [&tight, &wide] {
        assert_eq!((o[0], o[3]), (0, 14_400), "{o:?}");
    }
    let gaps = |o: &[u64]| o.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>();
    assert!(gaps(&tight).windows(2).all(|g| g[1] < g[0]), "{tight:?}");
    assert!(gaps(&wide).windows(2).all(|g| g[1] > g[0]), "{wide:?}");
    // f(1/2) = 1 / (1 + w): w = 1/4 at +100 %, 4 at −100 %.
    assert_eq!(
        tight[1],
        (14_400.0f64 * (1.0 / 3.0) / (1.0 / 3.0 + (2.0 / 3.0) * 0.25)).round() as u64
    );
}

#[test]
fn crescendo_ramps_the_velocity_along_the_strum_and_a_lone_note_keeps_its_own() {
    let velocities = |c: f32, voices_on: bool| {
        let mut r = Rig::new(&[
            (STRUM, 0.1),
            (CRESCENDO, c),
            (v(0, VOICE_ON), if voices_on { 1.0 } else { 0.0 }),
            (v(1, VOICE_ON), if voices_on { 1.0 } else { 0.0 }),
        ]);
        r.run(&[(0, on(1, 60, 0.8))], 10_000);
        r.ons().iter().map(|o| o.2).collect::<Vec<_>>()
    };
    let up = velocities(50.0, true);
    assert!(
        (up[0] - 0.4).abs() < 1e-6 && (up[1] - 0.6).abs() < 1e-6 && (up[2] - 0.8).abs() < 1e-6,
        "{up:?}"
    );
    let down = velocities(-50.0, true);
    assert!(
        (down[0] - 0.8).abs() < 1e-6 && (down[2] - 0.4).abs() < 1e-6,
        "{down:?}"
    );
    let full = velocities(100.0, true);
    assert!(
        (full[0] - 1.0 / 127.0).abs() < 1e-6,
        "never below 1/127: {full:?}"
    );
    assert_eq!(velocities(100.0, false), [0.8], "one note: no ramp");
}

// ---------------------------------------------------------------- the draw

#[test]
fn chance_draws_the_same_at_equal_seed_and_about_its_share() {
    let draw = |seed: f32| {
        let mut r = Rig::new(&[
            (SEED, seed),
            (v(0, VOICE_CHANCE), 50.0),
            (v(1, VOICE_CHANCE), 50.0),
        ]);
        let mut events = Vec::new();
        for n in 0..400u64 {
            events.push((n * 100, on(n as u32, 60, 1.0)));
            events.push((n * 100 + 50, off(n as u32, 60)));
        }
        r.run(&events, 40_100);
        assert!(r.sounding().is_empty());
        r.ons().iter().map(|o| (o.0, o.1)).collect::<Vec<_>>()
    };
    let a = draw(7.0);
    assert_eq!(a, draw(7.0), "same seed, same draw");
    assert_ne!(a, draw(8.0), "another seed, another draw");
    let added = a.len() - 400;
    assert!((320..=480).contains(&added), "about half of 800: {added}");
    // A start of the transport restarts the count: the same chords again.
    let mut r = Rig::new(&[
        (SEED, 7.0),
        (v(0, VOICE_CHANCE), 50.0),
        (v(1, VOICE_CHANCE), 50.0),
    ]);
    let notes: Vec<(u64, Event)> = (0..20u64)
        .flat_map(|n| {
            [
                (n * 100, on(n as u32, 60, 1.0)),
                (n * 100 + 50, off(n as u32, 60)),
            ]
        })
        .collect();
    r.run(&notes, 2_048);
    let first: Vec<u8> = r.ons().iter().map(|o| o.1).collect();
    r.playing = false;
    r.run(&[], 2_176);
    r.playing = true;
    r.log.clear();
    let shifted: Vec<(u64, Event)> = notes.iter().map(|(f, e)| (f + 2_176, *e)).collect();
    r.run(&shifted, 4_400);
    assert_eq!(r.ons().iter().map(|o| o.1).collect::<Vec<_>>(), first);
    // 0 % never plays, 100 % always.
    let mut never = Rig::new(&[(v(0, VOICE_CHANCE), 0.0), (v(1, VOICE_CHANCE), 0.0)]);
    never.run(&[(0, on(1, 60, 1.0))], 128);
    assert_eq!(never.ons().len(), 1);
}

// ---------------------------------------------------------------- no stuck note

#[test]
fn a_note_off_during_the_strum_ends_what_started_and_drops_the_rest() {
    let mut r = Rig::new(&[(STRUM, 0.2)]);
    r.run(&[(0, on(1, 60, 1.0)), (6_000, off(1, 60))], 20_000);
    assert_eq!(
        r.ons().iter().map(|o| (o.0, o.1)).collect::<Vec<_>>(),
        [(0, 60), (4_800, 64)],
        "G4 was due at 9 600: dropped"
    );
    assert!(r.sounding().is_empty());
    // A note due at the note-off's frame is dropped too.
    let mut r = Rig::new(&[(STRUM, 0.2)]);
    r.run(&[(0, on(1, 60, 1.0)), (4_800, off(1, 60))], 20_000);
    assert_eq!(r.ons().len(), 1);
    assert!(r.sounding().is_empty());
}

#[test]
fn a_setting_changed_mid_note_ends_the_notes_the_note_started() {
    let mut r = Rig::new(&[(STRUM, 0.1)]);
    r.run(&[(0, on(1, 60, 1.0))], 1_000);
    // Every setting moves while the chord sounds and its strum runs.
    let changes = [
        (v(0, VOICE_ON), 0.0),
        (v(1, VOICE_INTERVAL), 12.0),
        (v(2, VOICE_ON), 1.0),
        (MODE, IN_SCALE as f32),
        (STRUM, 0.0),
        (DIRECTION, DOWN as f32),
    ];
    let events: Vec<(u64, Event)> = changes
        .iter()
        .map(|(id, value)| {
            (
                1_500,
                Event::ParamValue {
                    offset: 0,
                    id: ParamId(*id),
                    value: *value,
                },
            )
        })
        .chain([(8_000, off(1, 60))])
        .collect();
    r.run(&events, 10_000);
    assert_eq!(
        r.ons().iter().map(|o| o.1).collect::<Vec<_>>(),
        [60, 64, 67],
        "the chord started is the one it was"
    );
    assert!(r.sounding().is_empty(), "{:?}", r.sounding());
}

#[test]
fn a_stop_ends_everything_and_drops_the_strum() {
    let mut r = Rig::new(&[(STRUM, 0.4)]);
    r.run(
        &[
            (0, on(1, 60, 1.0)),
            (100, on(2, 62, 1.0)),
            (3_000, Event::AllNotesOff { offset: 0 }),
        ],
        30_000,
    );
    assert!(r.log.contains(&Out::All { frame: 3_000 }));
    assert!(
        r.ons().iter().all(|o| o.0 < 3_000),
        "nothing starts after the stop"
    );
    assert!(r.sounding().is_empty());
    // The note-offs that follow end nothing and write nothing.
    let before = r.log.len();
    r.run(&[(30_100, off(1, 60)), (30_100, off(2, 62))], 30_200);
    assert_eq!(r.log.len(), before);
}

#[test]
fn a_reset_drops_the_strum_so_nothing_starts_after_a_bypass() {
    let mut r = Rig::new(&[(STRUM, 0.2)]);
    r.run(&[(0, on(1, 60, 1.0))], 1_000);
    // The engine resets a MIDI effect when it is switched (the host ends the instrument's notes).
    r.stack.reset();
    r.run(&[(5_000, off(1, 60))], 20_000);
    assert_eq!(r.ons().len(), 1, "the pending E4 and G4 never start");
}

#[test]
fn a_full_table_plays_no_more_and_ends_what_it_holds() {
    let mut r = Rig::new(&[]);
    let notes: Vec<(u64, Event)> = (0..HELD as u32 + 10)
        .map(|n| (0, on(n, (n % 100) as u8, 1.0)))
        .collect();
    r.run(&notes, 128);
    assert_eq!(
        r.ons().len(),
        HELD * 3,
        "the notes past the table are not played"
    );
    let offs: Vec<(u64, Event)> = (0..HELD as u32 + 10)
        .map(|n| (200, off(n, (n % 100) as u8)))
        .collect();
    r.run(&offs, 512);
    assert!(r.sounding().is_empty());
    // The voice ids it wrote were unique among the notes it held (checked by `sounding`), and
    // all under 1024.
    assert!(r.log.iter().all(|o| match *o {
        Out::On { voice, .. } | Out::Off { voice, .. } => voice < 1024,
        Out::All { .. } => true,
    }));
}

// ---------------------------------------------------------------- bounds and sweeps

#[test]
fn the_output_stays_within_the_declared_bounds_with_every_voice_on_and_a_long_strum() {
    let mut params = vec![(STRUM, 0.4)];
    for voice in 0..6 {
        params.push((v(voice, VOICE_ON), 1.0));
    }
    // Every held note's strum lands in the same block: the per-block bound.
    let mut r = Rig::with_block(&params, 4096);
    let notes: Vec<(u64, Event)> = (0..HELD as u32).map(|n| (0, on(n, 60, 1.0))).collect();
    r.run(&notes, 30_000);
    assert_eq!(r.ons().len(), HELD * 7);
    let bounds = DESCRIPTOR.midi_output.unwrap();
    assert_eq!((bounds.per_input, bounds.per_block), (7, (HELD * 7) as u32));
}

#[test]
fn every_parameter_swept_over_its_range_keeps_the_output_sound() {
    for p in DESCRIPTOR.params {
        let steps = 7;
        for i in 0..=steps {
            let value = p.min + (p.max - p.min) * f64::from(i) / f64::from(steps);
            let mut params = vec![(p.id.0, value as f32)];
            for voice in 0..6 {
                params.push((v(voice, VOICE_ON), 1.0));
            }
            let mut r = Rig::new(&params);
            r.scale = A_MINOR;
            let mut events = Vec::new();
            for n in 0..30u64 {
                events.push((n * 700, on(n as u32, (30 + n * 3) as u8, 0.7)));
                events.push((n * 700 + 9_000, off(n as u32, (30 + n * 3) as u8)));
            }
            r.run(&events, 50_000);
            assert!(r.sounding().is_empty(), "{} = {value}", p.key);
            assert!(
                r.ons()
                    .iter()
                    .all(|o| o.2 > 0.0 && o.2 <= 1.0 && o.1 <= 127)
            );
        }
    }
    // Values out of range and non-finite are clamped or ignored.
    let mut r = Rig::new(&[
        (STRUM, f32::NAN),
        (TENSION, 1e9),
        (CRESCENDO, f32::NEG_INFINITY),
        (v(0, VOICE_INTERVAL), -1e9),
    ]);
    r.run(&[(0, on(1, 60, f32::NAN)), (100, off(1, 60))], 200);
    assert!(r.sounding().is_empty());
}

#[test]
fn the_descriptor_is_valid_and_names_the_stack() {
    DESCRIPTOR.validate().unwrap();
    assert_eq!((DESCRIPTOR.kind, DESCRIPTOR.name), ("faulk.stack", "Stack"));
    assert_eq!(DESCRIPTOR.kind_id.0, 9);
    assert_eq!(DESCRIPTOR.params.len(), 30);
    let _ = stack::VOICES;
}
