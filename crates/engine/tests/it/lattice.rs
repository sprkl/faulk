//! Lattice (#692): the twelve chromatic notes on the project's scale under each rule, every
//! scale of the device on every root against a reference search, the matrix (a blocked cell, a
//! moved one, ties), the transposition and the keyboard's ends, a scale changed under held notes,
//! two received notes on one key (one note-off, at the last release), no stuck note (a stop, a
//! reset with a full table, a setting changed mid-note, a full table, a repeated voice), the
//! output bounds, a sweep of every parameter, and the audio path under the allocation detector.
//! The render through the synth, native and WASM, is in `crates/test-support/tests/it/lattice.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceSetup, Event, EventBuffer, PitchSet, Transport, VoiceId,
};
use engine::devices::lattice::{
    BLOCK, BLOCKED, DESCRIPTOR, DOWN, FROM_DEVICE, FROM_MATRIX, FROM_PROJECT, HELD, Lattice, MODES,
    NEAREST, ROOT, RULE, SCALE, SOURCE, TRANSPOSE, UP, map_param, mode_set,
};
use engine::msg::ParamId;
use std::collections::BTreeMap;

const RATE: f32 = 48_000.0;

/// C major (A minor): C D E F G A B.
const C_MAJOR: PitchSet = PitchSet::from_bits(0b1010_1011_0101);
/// A major: A B C# D E F# G#.
const A_MAJOR: PitchSet = PitchSet::from_bits(0b1011_0101_0110);

/// A note out of Lattice.
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
    All,
}

/// Drives a Lattice with events at absolute frames, in blocks of 128 frames.
struct Rig {
    lattice: Lattice,
    out: EventBuffer,
    frame: u64,
    scale: PitchSet,
    log: Vec<Out>,
}

impl Rig {
    fn new(params: &[(u32, f32)]) -> Rig {
        let mut lattice = Lattice::new(&DeviceSetup {
            sample_rate: RATE,
            ..DeviceSetup::default()
        })
        .unwrap();
        for (id, value) in params {
            lattice.set_param(ParamId(*id), *value);
        }
        lattice.reset();
        let capacity = DESCRIPTOR.midi_output.unwrap().capacity(HELD * 2);
        Rig {
            lattice,
            out: EventBuffer::try_with_capacity(capacity).unwrap(),
            frame: 0,
            scale: C_MAJOR,
            log: Vec::new(),
        }
    }

    fn set(&mut self, id: u32, value: f32) {
        self.lattice.set_param(ParamId(id), value);
    }

    /// One block of `frames` frames with `events` (offset, event), the output logged.
    fn block(&mut self, events: &[(u32, Event)], frames: u32) {
        let block: Vec<Event> = events.iter().map(|(o, e)| e.at(*o)).collect();
        let (mut l, mut r) = (vec![0.0; frames as usize], vec![0.0; frames as usize]);
        let ctx = BlockCtx {
            sample_rate: RATE,
            frames,
            transport: Transport {
                playing: true,
                scale: self.scale,
                ..Transport::default()
            },
        };
        self.out.clear();
        let (lattice, out) = (&mut self.lattice, &mut self.out);
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut l,
                right: &mut r,
                events_out: out,
            };
            lattice.process(&ctx, &block, &mut io);
        });
        assert_eq!(self.out.overflowed(), 0, "the output never overflows");
        let bound = DESCRIPTOR
            .midi_output
            .unwrap()
            .capacity(block.len())
            .saturating_sub(1);
        assert!(
            self.out.events().len() <= bound,
            "within the declared bounds"
        );
        let mut last = 0;
        for e in self.out.events() {
            assert!(e.offset() >= last, "sorted output");
            assert!(e.offset() < frames.max(1), "inside the block");
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
                Event::AllNotesOff { .. } => Out::All,
                Event::ParamValue { .. } => panic!("a MIDI effect writes notes only"),
            });
        }
        self.frame += u64::from(frames);
    }

    /// The key each received note plays: a note-on of `key` then its note-off, alone; `None`
    /// when nothing comes out (and then its note-off writes nothing either).
    fn play(&mut self, key: u8) -> Option<u8> {
        let before = self.log.len();
        self.block(&[(0, on(1, key, 0.8))], 64);
        self.block(&[(0, off(1, key))], 64);
        let out = &self.log[before..];
        match out {
            [] => None,
            [
                Out::On { key: a, voice, .. },
                Out::Off {
                    key: b, voice: w, ..
                },
            ] if a == b && voice == w => Some(*a),
            other => panic!("{key}: {other:?}"),
        }
    }

    fn ons(&self) -> Vec<u8> {
        self.log
            .iter()
            .filter_map(|o| match *o {
                Out::On { key, .. } => Some(key),
                _ => None,
            })
            .collect()
    }

    fn offs(&self) -> Vec<u8> {
        self.log
            .iter()
            .filter_map(|o| match *o {
                Out::Off { key, .. } => Some(key),
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
                Out::All => held.clear(),
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

/// The reference: the key `key` goes to on `scale` (a list of pitch classes) by `rule`, by
/// searching every key of the keyboard.
fn reference(key: u8, scale: &[u8], rule: u32) -> Option<i32> {
    let key = i32::from(key);
    let inside = |k: i32| scale.contains(&(k.rem_euclid(12) as u8));
    if scale.is_empty() || inside(key) {
        return Some(key);
    }
    let candidates: Vec<i32> = (-24..152).filter(|&k| inside(k)).collect();
    let below = candidates.iter().copied().filter(|&k| k < key).max();
    let above = candidates.iter().copied().filter(|&k| k > key).min();
    match rule {
        BLOCK => None,
        DOWN => below,
        UP => above,
        // The nearest on the keyboard.
        _ => match (below.filter(|b| *b >= 0), above.filter(|a| *a <= 127)) {
            (Some(b), Some(a)) => Some(if key - b <= a - key { b } else { a }),
            (b, a) => b.or(a),
        },
    }
}

fn on_keyboard(k: Option<i32>) -> Option<u8> {
    k.and_then(|k| u8::try_from(k).ok()).filter(|k| *k <= 127)
}

// ---------------------------------------------------------------- the project's scale

#[test]
fn the_twelve_chromatic_notes_in_c_major_follow_each_rule() {
    // C4 to B4: the seven notes of C major pass under every rule; the five others go where
    // the rule says (a tie, a black key between two white ones a tone apart, goes down for
    // `nearest`).
    let pass = [60u8, 62, 64, 65, 67, 69, 71];
    let cases: [(u32, [Option<u8>; 5]); 4] = [
        (NEAREST, [Some(60), Some(62), Some(65), Some(67), Some(69)]),
        (DOWN, [Some(60), Some(62), Some(65), Some(67), Some(69)]),
        (UP, [Some(62), Some(64), Some(67), Some(69), Some(71)]),
        (BLOCK, [None; 5]),
    ];
    for (rule, black) in cases {
        let mut r = Rig::new(&[(RULE, rule as f32)]);
        for key in pass {
            assert_eq!(r.play(key), Some(key), "rule {rule}: {key} passes");
        }
        for (key, want) in [61u8, 63, 66, 68, 70].into_iter().zip(black) {
            assert_eq!(r.play(key), want, "rule {rule}: {key}");
        }
        assert!(r.sounding().is_empty());
    }
}

#[test]
fn nearest_takes_the_closer_note_and_a_tie_goes_down() {
    // C major pentatonic (C D E G A): F is a semitone from E and a tone from G (→ E), F# a
    // tone from E and a semitone from G (→ G), A# a semitone from A, B a semitone from C.
    let mut r = Rig::new(&[(SOURCE, FROM_DEVICE as f32), (SCALE, 9.0)]);
    assert_eq!(r.play(65), Some(64));
    assert_eq!(r.play(66), Some(67));
    assert_eq!(r.play(70), Some(69));
    assert_eq!(r.play(71), Some(72));
    // The blues scale of C (C Eb F F# G Bb): B is a semitone from Bb and from C (a tie: down),
    // D a tone from C and a semitone from Eb (up), A a tone from G and a semitone from Bb.
    r.set(SCALE, 11.0);
    assert_eq!(r.play(71), Some(70));
    assert_eq!(r.play(62), Some(63));
    assert_eq!(r.play(69), Some(70));
}

#[test]
fn a_project_without_a_scale_lets_every_note_through() {
    for rule in [NEAREST, DOWN, UP, BLOCK] {
        let mut r = Rig::new(&[(RULE, rule as f32)]);
        r.scale = PitchSet::EMPTY;
        for key in 0..=127u8 {
            assert_eq!(r.play(key), Some(key));
        }
    }
}

#[test]
fn the_project_source_follows_any_set_of_pitch_classes() {
    // Every key, every rule, on A major and on a single pitch class: as the reference search.
    for (scale, classes) in [
        (A_MAJOR, vec![1u8, 2, 4, 6, 8, 9, 11]),
        (PitchSet::from_bits(1 << 7), vec![7]),
    ] {
        for rule in [NEAREST, DOWN, UP, BLOCK] {
            let mut r = Rig::new(&[(RULE, rule as f32)]);
            r.scale = scale;
            for key in 0..=127u8 {
                let want = on_keyboard(reference(key, &classes, rule));
                assert_eq!(r.play(key), want, "rule {rule}, key {key}");
            }
        }
    }
}

// ---------------------------------------------------------------- the device's scale

#[test]
fn every_scale_of_the_device_on_every_root_matches_the_reference_search() {
    for (mode, (name, degrees)) in MODES.iter().enumerate() {
        for root in 0..12u8 {
            let classes: Vec<u8> = degrees.iter().map(|d| (root + d) % 12).collect();
            assert_eq!(
                mode_set(root, mode),
                PitchSet::from_bits(classes.iter().fold(0, |b, c| b | 1 << c)),
                "{name} on {root}"
            );
            for rule in [NEAREST, DOWN, UP, BLOCK] {
                let r = Rig::new(&[
                    (SOURCE, FROM_DEVICE as f32),
                    (ROOT, f32::from(root)),
                    (SCALE, mode as f32),
                    (RULE, rule as f32),
                ]);
                for key in 0..=127u8 {
                    let want = on_keyboard(reference(key, &classes, rule));
                    // The project's scale (C major) plays no part.
                    assert_eq!(
                        r.lattice.target(key, C_MAJOR),
                        want,
                        "{name} on {root}, rule {rule}, key {key}"
                    );
                }
            }
        }
    }
    // Through the stream too, on one of them: D dorian, nearest.
    let mut r = Rig::new(&[(SOURCE, FROM_DEVICE as f32), (ROOT, 2.0), (SCALE, 4.0)]);
    let classes = [2u8, 4, 5, 7, 9, 11, 0];
    for key in 40..90u8 {
        assert_eq!(r.play(key), on_keyboard(reference(key, &classes, NEAREST)));
    }
}

// ---------------------------------------------------------------- the matrix

#[test]
fn the_matrix_moves_its_cells_blocks_one_and_leaves_the_others() {
    // Identity by default: every note passes.
    let mut r = Rig::new(&[(SOURCE, FROM_MATRIX as f32)]);
    for key in 0..=127u8 {
        assert_eq!(r.play(key), Some(key));
    }
    // C# blocked, D# to E, F# to C (six either way: down), C to B (the B below), G to D (five
    // down).
    r.set(map_param(1), BLOCKED as f32);
    r.set(map_param(3), 4.0);
    r.set(map_param(6), 0.0);
    r.set(map_param(0), 11.0);
    r.set(map_param(7), 2.0);
    r.log.clear();
    // A blocked note writes neither its note-on nor its note-off.
    r.block(&[(0, on(1, 61, 0.8))], 64);
    r.block(&[(0, off(1, 61))], 64);
    assert!(r.log.is_empty(), "{:?}", r.log);
    assert_eq!(r.play(63), Some(64));
    assert_eq!(r.play(66), Some(60));
    assert_eq!(r.play(60), Some(59));
    assert_eq!(r.play(67), Some(62));
    for key in [62u8, 64, 65, 68, 69, 70, 71] {
        assert_eq!(r.play(key), Some(key), "{key} unchanged");
    }
    // The rule does not apply to the matrix.
    r.set(RULE, BLOCK as f32);
    assert_eq!(r.play(62), Some(62));
    // At the keyboard's ends: C0's cell to B goes below 0, not played.
    assert_eq!(r.play(0), None);
}

// ---------------------------------------------------------------- transposition and range

#[test]
fn the_transposition_comes_after_the_scale_and_a_note_off_the_keyboard_is_not_played() {
    let mut r = Rig::new(&[(TRANSPOSE, 24.0)]);
    assert_eq!(r.play(61), Some(84), "C#4 to C4, then two octaves up");
    assert_eq!(r.play(103), Some(127));
    assert_eq!(r.play(104), Some(127), "G#7, a tie, down to G7");
    assert_eq!(r.play(105), None, "A7 + 24 = 129");
    r.set(TRANSPOSE, -24.0);
    assert_eq!(r.play(25), Some(0));
    assert_eq!(r.play(23), None);
    // Clamped to the range: ±24 at most.
    r.set(TRANSPOSE, 100.0);
    assert_eq!(r.play(60), Some(84));
    // Up from the top: B is the scale's last key below 128 in C major; 127 (G) stays, and with
    // `up` the A above 127 is off the keyboard.
    let mut r = Rig::new(&[(RULE, UP as f32)]);
    r.scale = PitchSet::from_bits(1 << 9);
    assert_eq!(r.play(127), None);
    let mut r = Rig::new(&[(RULE, DOWN as f32)]);
    r.scale = PitchSet::from_bits(1 << 9);
    assert_eq!(r.play(5), None, "no A below F0");
    assert!(r.sounding().is_empty());
}

#[test]
fn velocity_and_fine_tuning_are_kept() {
    let mut r = Rig::new(&[]);
    r.block(
        &[(
            5,
            Event::NoteOn {
                offset: 0,
                voice: VoiceId::new(3).unwrap(),
                key: 61,
                velocity: 0.42,
                tune_cents: 37.0,
            },
        )],
        64,
    );
    assert_eq!(
        r.log,
        [Out::On {
            frame: 5,
            voice: 60,
            key: 60,
            velocity: 0.42,
            tune: 37.0
        }]
    );
    // A velocity that is not finite comes out at 0, one above 1 at 1.
    r.block(&[(0, on(4, 62, f32::NAN)), (1, on(5, 64, 9.0))], 64);
    assert!(matches!(r.log[1], Out::On { velocity: 0.0, .. }));
    assert!(matches!(r.log[2], Out::On { velocity: 1.0, .. }));
}

// ---------------------------------------------------------------- held notes

#[test]
fn a_scale_changed_under_held_notes_ends_them_at_the_key_they_play() {
    for change in ["project", "device", "matrix", "rule", "transpose", "source"] {
        let mut r = Rig::new(&[]);
        // C# and F# held in C major: they play C and F.
        r.block(&[(0, on(1, 61, 0.8)), (0, on(2, 66, 0.8))], 64);
        match change {
            "project" => r.scale = A_MAJOR,
            "device" => {
                r.set(SOURCE, FROM_DEVICE as f32);
                r.set(ROOT, 1.0);
            }
            "matrix" => {
                r.set(SOURCE, FROM_MATRIX as f32);
                r.set(map_param(1), 7.0);
            }
            "rule" => r.set(RULE, UP as f32),
            "transpose" => r.set(TRANSPOSE, 3.0),
            _ => r.set(SOURCE, FROM_MATRIX as f32),
        }
        // A note received after the change follows it; the held ones end at their keys.
        r.block(&[(0, on(3, 61, 0.8))], 64);
        r.block(&[(0, off(1, 61)), (1, off(2, 66)), (2, off(3, 61))], 64);
        assert_eq!(&r.ons()[..2], [60, 65], "{change}");
        let third = r.ons()[2];
        let want = match change {
            "project" | "device" | "source" => 61,
            "matrix" => 55,
            "rule" => 62,
            _ => 63,
        };
        assert_eq!(third, want, "{change}");
        assert_eq!(r.offs(), [60, 65, want], "{change}");
        assert!(r.sounding().is_empty(), "{change}");
    }
}

#[test]
fn two_received_notes_on_one_key_end_once_at_the_last_release() {
    // C and C# both play C: one note-on, and one note-off at the last of the two releases,
    // whichever it is.
    for first in [60u8, 61] {
        let second = if first == 60 { 61 } else { 60 };
        let mut r = Rig::new(&[]);
        r.block(&[(0, on(1, 60, 0.8)), (3, on(2, 61, 0.5))], 64);
        assert_eq!(r.ons(), [60]);
        let (v_first, v_second) = if first == 60 { (1, 2) } else { (2, 1) };
        r.block(&[(0, off(v_first, first))], 64);
        assert!(r.offs().is_empty(), "C still held by {second}");
        r.block(&[(7, off(v_second, second))], 64);
        assert_eq!(r.offs(), [60]);
        assert!(matches!(r.log.last(), Some(Out::Off { frame: 135, .. })));
        assert!(r.sounding().is_empty());
    }
    // Three notes on C (B# does not exist: C, C#, and C with the rule `down` from C#), released
    // in any order: one note-off, at the third.
    let mut r = Rig::new(&[(RULE, DOWN as f32)]);
    r.block(
        &[
            (0, on(1, 60, 0.8)),
            (0, on(2, 61, 0.8)),
            (0, on(3, 60, 0.8)),
        ],
        64,
    );
    r.block(&[(0, off(2, 61)), (1, off(3, 60))], 64);
    assert!(r.offs().is_empty());
    r.block(&[(0, off(1, 60))], 64);
    assert_eq!((r.ons(), r.offs()), (vec![60], vec![60]));
    // And the key plays again after.
    assert_eq!(r.play(61), Some(60));
}

#[test]
fn a_repeated_voice_ends_its_note_first() {
    // A note-on on a voice Lattice holds (a broken guarantee): its note ends, the new one
    // starts, 2 events, the declared bound per input.
    let mut r = Rig::new(&[]);
    r.block(&[(0, on(1, 60, 1.0)), (10, on(1, 64, 1.0))], 64);
    assert_eq!(r.log.len(), 3);
    assert_eq!((r.ons(), r.offs()), (vec![60, 64], vec![60]));
    r.block(&[(0, off(1, 64))], 64);
    assert!(r.sounding().is_empty());
    assert_eq!(DESCRIPTOR.midi_output.unwrap().per_input, 2);
    assert_eq!(DESCRIPTOR.midi_output.unwrap().per_block, 0);
}

#[test]
fn a_note_off_of_a_voice_not_held_writes_nothing() {
    let mut r = Rig::new(&[]);
    r.block(&[(0, off(9, 60))], 64);
    assert!(r.log.is_empty());
}

#[test]
fn a_stop_ends_every_note_and_the_next_ones_play() {
    let mut r = Rig::new(&[]);
    r.block(
        &[
            (0, on(1, 60, 1.0)),
            (0, on(2, 61, 1.0)),
            (0, on(3, 64, 1.0)),
        ],
        64,
    );
    r.block(&[(4, Event::AllNotesOff { offset: 0 })], 64);
    assert_eq!(r.log.last(), Some(&Out::All), "the stop goes on");
    // The late note-offs of the notes the stop ended write nothing.
    let before = r.log.len();
    r.block(&[(0, off(1, 60)), (0, off(2, 61)), (0, off(3, 64))], 64);
    assert_eq!(r.log.len(), before);
    assert!(r.sounding().is_empty());
    // C plays again, once, and ends.
    assert_eq!(r.play(61), Some(60));
}

#[test]
fn a_full_table_plays_no_more_and_its_extra_note_offs_end_nothing() {
    let mut r = Rig::new(&[]);
    r.scale = PitchSet::EMPTY;
    let notes: Vec<(u32, Event)> = (0..HELD as u32)
        .map(|n| (0, on(n, (n % 128) as u8, 1.0)))
        .collect();
    r.block(&notes, 64);
    assert_eq!(r.ons().len(), HELD);
    r.block(&[(0, on(500, 3, 1.0))], 64);
    assert_eq!(r.ons().len(), HELD, "the table is full");
    r.block(&[(0, off(500, 3))], 64);
    assert!(r.offs().is_empty(), "the refused note ends nothing");
    let offs: Vec<(u32, Event)> = (0..HELD as u32)
        .map(|n| (0, off(n, (n % 128) as u8)))
        .collect();
    r.block(&offs, 64);
    assert!(r.sounding().is_empty());
}

#[test]
fn a_reset_with_a_full_table_allocates_nothing_and_leaves_it_empty() {
    let mut r = Rig::new(&[]);
    let notes: Vec<(u32, Event)> = (0..HELD as u32).map(|n| (0, on(n, 61, 1.0))).collect();
    r.block(&notes, 64);
    assert_eq!(r.ons(), [60]);
    // The engine calls it on the audio thread when Lattice is switched (#219).
    let lattice = &mut r.lattice;
    assert_no_alloc(|| lattice.reset());
    r.log.clear();
    // The note-offs of the old table write nothing, and every key starts afresh.
    let offs: Vec<(u32, Event)> = (0..HELD as u32).map(|n| (0, off(n, 61))).collect();
    r.block(&offs, 64);
    assert!(r.log.is_empty());
    let again: Vec<(u32, Event)> = (0..HELD as u32)
        .map(|n| (0, on(n, (n % 128) as u8, 1.0)))
        .collect();
    r.scale = PitchSet::EMPTY;
    r.block(&again, 64);
    assert_eq!(r.ons().len(), HELD, "the whole table again");
}

#[test]
fn many_resets_wrap_the_generation_and_keep_the_table_sound() {
    let mut r = Rig::new(&[]);
    for _ in 0..1000 {
        r.lattice.reset();
    }
    r.block(&[(0, on(1, 61, 1.0))], 64);
    r.lattice.reset();
    r.block(&[(0, on(1, 61, 1.0))], 64);
    r.block(&[(0, off(1, 61))], 64);
    assert_eq!((r.ons(), r.offs()), (vec![60, 60], vec![60]));
}

#[test]
fn the_generation_wrap_wipes_the_tables_and_nothing_old_holds_again() {
    // Notes held at generation 1 (C, played by two voices), then cleared; the counter is moved
    // to its last value and cleared once more: it wraps to 1, where the old entries were.
    let mut r = Rig::new(&[]);
    r.lattice.set_generation_for_tests(1);
    r.block(&[(0, on(1, 61, 1.0)), (0, on(2, 60, 1.0))], 64);
    r.lattice.reset();
    r.lattice.set_generation_for_tests(u32::MAX);
    let lattice = &mut r.lattice;
    assert_no_alloc(|| lattice.reset());
    r.log.clear();
    // The old voices' note-offs end nothing, and C starts again at once.
    r.block(&[(0, off(1, 61)), (0, off(2, 60))], 64);
    assert!(r.log.is_empty(), "{:?}", r.log);
    assert_eq!(r.play(61), Some(60));
    assert_eq!((r.ons(), r.offs()), (vec![60], vec![60]));
}

#[test]
fn near_an_end_of_the_keyboard_nearest_picks_the_scale_note_on_it() {
    // Only A: from C#0 (1) the A below would be -3, off the keyboard, so A0 (9) plays; from G9
    // (127) the A above would be 129, so the A below (117) plays. `down` and `up` keep their
    // direction and find none.
    let mut r = Rig::new(&[]);
    r.scale = PitchSet::from_bits(1 << 9);
    assert_eq!(r.play(1), Some(9));
    assert_eq!(r.play(127), Some(117));
    r.set(RULE, DOWN as f32);
    assert_eq!(r.play(1), None);
    r.set(RULE, UP as f32);
    assert_eq!(r.play(127), None);
}

// ---------------------------------------------------------------- sweeps and descriptor

#[test]
fn every_parameter_swept_over_its_range_keeps_the_output_sound() {
    for p in DESCRIPTOR.params {
        let steps = 12;
        for i in 0..=steps {
            let value = p.min + (p.max - p.min) * f64::from(i) / f64::from(steps);
            for source in [FROM_PROJECT, FROM_DEVICE, FROM_MATRIX] {
                let mut r = Rig::new(&[(SOURCE, source as f32), (p.id.0, value as f32)]);
                let mut events = Vec::new();
                for n in 0..40u32 {
                    events.push((n, on(n, (20 + n * 2) as u8, 0.7)));
                }
                r.block(&events, 64);
                // A setting changed mid-note: every note still ends.
                r.set(p.id.0, (p.max - value + p.min) as f32);
                r.set(SOURCE, ((source + 1) % 3) as f32);
                let offs: Vec<(u32, Event)> = (0..40u32)
                    .map(|n| (n, off(n, (20 + n * 2) as u8)))
                    .collect();
                r.block(&offs, 64);
                assert!(r.sounding().is_empty(), "{} = {value}", p.key);
                assert_eq!(r.ons().len(), r.offs().len(), "{} = {value}", p.key);
            }
        }
    }
    // Values out of range and non-finite are clamped or ignored.
    let mut r = Rig::new(&[
        (SOURCE, f32::NAN),
        (RULE, 1e9),
        (TRANSPOSE, f32::NEG_INFINITY),
        (map_param(4), -1e9),
        (ParamId(999).0, 1.0),
    ]);
    r.block(&[(0, on(1, 60, 0.5))], 64);
    r.block(&[(0, off(1, 60))], 64);
    assert!(r.sounding().is_empty());
}

#[test]
fn the_descriptor_is_valid_and_names_lattice() {
    DESCRIPTOR.validate().unwrap();
    assert_eq!(
        (DESCRIPTOR.kind, DESCRIPTOR.name),
        ("faulk.lattice", "Lattice")
    );
    assert_eq!(DESCRIPTOR.kind_id.0, 12);
    assert_eq!(DESCRIPTOR.params.len(), 17);
    // The device follows the project's scale to its nearest note by default.
    let default = |id: u32| DESCRIPTOR.param(ParamId(id)).unwrap().default;
    assert_eq!(default(SOURCE), f64::from(FROM_PROJECT));
    assert_eq!(default(RULE), f64::from(NEAREST));
    // The matrix starts as the identity.
    for pc in 0..12 {
        assert_eq!(default(map_param(pc)), f64::from(pc));
    }
    const { assert!(DESCRIPTOR.memory_bytes > 0) };
}
