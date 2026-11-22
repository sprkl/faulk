//! Scatter (#691): the choices (direction, count, interval, in semitones and in the scale's
//! degrees), the chance draw and the choice at equal seed (restarted by a reset and a start of
//! the transport), the cycle order, no stuck note (a setting changed mid-note, a stop, a reset,
//! a repeated voice, a full table), the output bounds, a sweep of every parameter, and the audio
//! path under the allocation detector. The render through the synth, native and WASM, is in
//! `crates/test-support/tests/it/scatter.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::{
    AudioIo, BlockCtx, Device, DeviceSetup, Event, EventBuffer, PitchSet, Transport, VoiceId,
};
use engine::devices::draw::song_tick;
use engine::devices::scatter::{
    AROUND, CHANCE, CHOICES, CYCLE, DESCRIPTOR, DIRECTION, DOWN, HELD, IN_SCALE, INTERVAL, MODE,
    ORDER, SEED, Scatter, UP, scale_step, steps,
};
use engine::msg::ParamId;
use std::collections::{BTreeMap, BTreeSet};

const RATE: f32 = 48_000.0;

/// F natural minor: F G A♭ B♭ C D♭ E♭.
const F_MINOR: PitchSet = PitchSet::from_bits(
    (1 << 5) | (1 << 7) | (1 << 8) | (1 << 10) | (1 << 0) | (1 << 1) | (1 << 3),
);

/// An event out of Scatter, at its absolute frame.
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

/// Drives a Scatter with events at absolute frames, cut in blocks of 128 frames.
struct Rig {
    scatter: Scatter,
    out: EventBuffer,
    frame: u64,
    playing: bool,
    scale: PitchSet,
    /// Frames per block.
    block: u64,
    /// The song position of frame 0, in ticks.
    origin: f64,
    log: Vec<Out>,
}

/// The ticks of one frame at the default tempo.
fn ticks_per_frame() -> f64 {
    Transport::default().tempo.bpm() * 16.0 / f64::from(RATE)
}

impl Rig {
    fn new(params: &[(u32, f32)]) -> Rig {
        let setup = DeviceSetup {
            sample_rate: RATE,
            ..DeviceSetup::default()
        };
        let mut scatter = Scatter::new(&setup).unwrap();
        for (id, value) in params {
            // On the audio thread too (device.rs): under the detector.
            assert_no_alloc(|| scatter.set_param(ParamId(*id), *value));
        }
        assert_no_alloc(|| scatter.reset());
        Rig {
            scatter,
            out: EventBuffer::try_with_capacity(DESCRIPTOR.midi_output.unwrap().capacity(512))
                .unwrap(),
            frame: 0,
            playing: true,
            scale: PitchSet::EMPTY,
            block: 128,
            origin: 0.0,
            log: Vec::new(),
        }
    }

    fn set(&mut self, id: u32, value: f32) {
        let scatter = &mut self.scatter;
        assert_no_alloc(|| scatter.set_param(ParamId(id), value));
    }

    /// The device's reset, on the audio thread: under the detector.
    fn reset(&mut self) {
        let scatter = &mut self.scatter;
        assert_no_alloc(|| scatter.reset());
    }

    /// Renders up to `until` (absolute frame), applying `events` (absolute frame, event).
    fn run(&mut self, events: &[(u64, Event)], until: u64) {
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
            assert!(block.len() <= 512);
            let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
            let ctx = BlockCtx {
                sample_rate: RATE,
                frames: frames as u32,
                transport: Transport {
                    playing: self.playing,
                    scale: self.scale,
                    position_ticks: self.origin + self.frame as f64 * ticks_per_frame(),
                    ..Transport::default()
                },
            };
            self.out.clear();
            let (scatter, out) = (&mut self.scatter, &mut self.out);
            assert_no_alloc(|| {
                let mut io = AudioIo {
                    left: &mut l,
                    right: &mut r,
                    events_out: out,
                };
                scatter.process(&ctx, &block, &mut io);
            });
            assert_eq!(self.out.overflowed(), 0, "the output never overflows");
            assert!(
                self.out.events().len() <= 2 * block.len(),
                "at most per_input events per input"
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

    /// The keys of the note-ons out, in order.
    fn keys(&self) -> Vec<u8> {
        self.log
            .iter()
            .filter_map(|o| match *o {
                Out::On { key, .. } => Some(key),
                _ => None,
            })
            .collect()
    }

    /// The notes still sounding: every note-on matched by a note-off at its voice and key, an
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

/// `n` short notes, one every 100 frames, keys from `keys` in turn, each ended before the next.
fn melody(n: usize, keys: &[u8]) -> Vec<(u64, Event)> {
    let mut events = Vec::new();
    for i in 0..n {
        let key = keys[i % keys.len()];
        let at = i as u64 * 100;
        let voice = (i % 50) as u32 + 1;
        events.push((at, on(voice, key)));
        events.push((at + 50, off(voice, key)));
    }
    events
}

/// The keys received by `melody(n, keys)`, in order.
fn melody_keys(n: usize, keys: &[u8]) -> Vec<u8> {
    (0..n).map(|i| keys[i % keys.len()]).collect()
}

/// Renders `melody(n, keys)` and returns the keys out.
fn scatter(params: &[(u32, f32)], n: usize, keys: &[u8], scale: PitchSet) -> Vec<u8> {
    let mut r = Rig::new(params);
    r.scale = scale;
    r.run(&melody(n, keys), n as u64 * 100 + 128);
    assert!(r.sounding().is_empty(), "nothing left sounding");
    r.keys()
}

const CHROMATIC: [u8; 25] = [
    48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71,
    72,
];

// ---------------------------------------------------------------- the criteria

#[test]
fn chance_100_one_choice_12_up_moves_every_note_an_octave_up_and_chance_0_changes_nothing() {
    let all = [
        (CHANCE, 100.0),
        (CHOICES, 1.0),
        (INTERVAL, 12.0),
        (DIRECTION, UP as f32),
    ];
    let mut r = Rig::new(&all);
    let tuned = Event::NoteOn {
        offset: 0,
        voice: VoiceId::new(7).unwrap(),
        key: 60,
        velocity: 0.4,
        tune_cents: -31.5,
    };
    r.run(&[(10, tuned), (300, off(7, 60))], 512);
    assert_eq!(
        r.log,
        [
            Out::On {
                frame: 10,
                voice: 7,
                key: 72,
                velocity: 0.4,
                tune: -31.5
            },
            Out::Off {
                frame: 300,
                voice: 7,
                key: 72
            }
        ],
        "same frame, voice, velocity and tuning, an octave up; the note-off follows"
    );
    let keys = scatter(&all, 200, &CHROMATIC, PitchSet::EMPTY);
    assert_eq!(
        keys,
        melody_keys(200, &CHROMATIC)
            .iter()
            .map(|k| k + 12)
            .collect::<Vec<_>>()
    );

    // Chance 0: the stream goes through event for event, in every mode.
    for mode in [0.0, IN_SCALE as f32] {
        let events = melody(300, &CHROMATIC);
        let mut r = Rig::new(&[(CHANCE, 0.0), (MODE, mode), (CHOICES, 24.0)]);
        r.scale = F_MINOR;
        r.run(&events, 300 * 100 + 128);
        let expected: Vec<Out> = events
            .iter()
            .map(|(f, e)| match *e {
                Event::NoteOn { voice, key, .. } => Out::On {
                    frame: *f,
                    voice: voice.get(),
                    key,
                    velocity: 0.75,
                    tune: 0.0,
                },
                Event::NoteOff { voice, key, .. } => Out::Off {
                    frame: *f,
                    voice: voice.get(),
                    key,
                },
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(r.log, expected);
    }
}

#[test]
fn chance_50_moves_about_half_of_1000_notes_the_same_way_at_equal_seed() {
    let keys_in = [60u8, 63, 67, 70];
    for seed in [0.0, 1.0, 42.0, 1000.0] {
        let params = [
            (CHANCE, 50.0),
            (CHOICES, 4.0),
            (INTERVAL, 5.0),
            (SEED, seed),
        ];
        let a = scatter(&params, 1000, &keys_in, PitchSet::EMPTY);
        let input = melody_keys(1000, &keys_in);
        let moved = a.iter().zip(&input).filter(|(o, i)| o != i).count();
        assert!((450..=550).contains(&moved), "seed {seed}: {moved} moved");
        assert_eq!(
            a,
            scatter(&params, 1000, &keys_in, PitchSet::EMPTY),
            "same seed, same sequence"
        );
    }
    let s0 = scatter(
        &[(CHANCE, 50.0), (SEED, 0.0)],
        1000,
        &keys_in,
        PitchSet::EMPTY,
    );
    let s1 = scatter(
        &[(CHANCE, 50.0), (SEED, 1.0)],
        1000,
        &keys_in,
        PitchSet::EMPTY,
    );
    assert_ne!(s0, s1, "another seed, another draw");
}

/// `melody(n, keys)` moved `frames` later.
fn later(n: usize, keys: &[u8], frames: u64) -> Vec<(u64, Event)> {
    melody(n, keys)
        .into_iter()
        .map(|(f, e)| (f + frames, e))
        .collect()
}

#[test]
fn a_note_draws_by_its_place_in_the_song_whatever_the_start_the_blocks_or_a_live_note() {
    let params = [(CHANCE, 50.0), (CHOICES, 6.0), (INTERVAL, 2.0), (SEED, 7.0)];
    let keys_in = [62u8, 65, 69];
    let n = 64;
    let first = scatter(&params, n, &keys_in, PitchSet::EMPTY);
    let input = melody_keys(n, &keys_in);
    assert_ne!(first, input, "some notes move");
    // Played again, from the start: the same draw.
    assert_eq!(first, scatter(&params, n, &keys_in, PitchSet::EMPTY));
    // Played from the middle (the 33rd note's place): the notes from there move as they did.
    let mut r = Rig::new(&params);
    let skip = 32;
    r.origin = (skip as u64 * 100) as f64 * ticks_per_frame();
    let rest: Vec<(u64, Event)> = melody(n, &keys_in)
        .into_iter()
        .filter(|(f, _)| *f >= skip as u64 * 100)
        .map(|(f, e)| (f - skip as u64 * 100, e))
        .collect();
    r.run(&rest, (n - skip) as u64 * 100 + 128);
    assert_eq!(r.keys(), first[skip..], "a play from the middle");
    // Other cuts of the blocks: the same.
    for block in [1, 37, 97, 512] {
        let mut r = Rig::new(&params);
        r.block = block;
        r.run(&melody(n, &keys_in), n as u64 * 100 + 128);
        assert_eq!(r.keys(), first, "blocks of {block}");
    }
    // A live note between two of them (another key, at its own place): the others draw as
    // before.
    let mut r = Rig::new(&params);
    let mut events = melody(n, &keys_in);
    events.push((1_030, on(99, 40)));
    events.push((1_060, off(99, 40)));
    r.run(&events, n as u64 * 100 + 128);
    let mut keys = r.keys();
    keys.remove(11);
    assert_eq!(keys, first, "a live note shifts nothing");
    // The same notes later in the song (another place): another draw.
    let mut r = Rig::new(&params);
    r.run(&later(n, &keys_in, 100_000), 100_000 + n as u64 * 100 + 128);
    assert_ne!(r.keys(), first, "another place, another draw");
    // A reset changes nothing of a draw while playing.
    let mut r = Rig::new(&params);
    r.run(&melody(n / 2, &keys_in), (n / 2) as u64 * 100);
    r.reset();
    r.run(&later(n, &keys_in, 0)[n..], n as u64 * 100 + 128);
    assert_eq!(r.keys(), first);
}

#[test]
fn stopped_the_draw_counts_the_notes_from_the_last_reset() {
    let params = [(CHANCE, 50.0), (CHOICES, 6.0), (INTERVAL, 2.0), (SEED, 3.0)];
    let keys_in = [60u8, 64, 67];
    let n = 64;
    let run = |r: &mut Rig| {
        let start = r.frame;
        r.run(&later(n, &keys_in, start), start + n as u64 * 100 + 128);
    };
    let mut r = Rig::new(&params);
    r.playing = false;
    run(&mut r);
    let first = r.keys();
    assert_ne!(first, melody_keys(n, &keys_in), "some notes move");
    // Later, still stopped: the count goes on, another draw.
    run(&mut r);
    assert_ne!(r.keys()[n..], first[..]);
    // After a reset: the same again, wherever the playhead stands.
    r.reset();
    r.origin = 12_345.0;
    run(&mut r);
    assert_eq!(r.keys()[2 * n..], first[..]);
    // Playing does not use the count: the same melody playing gives the draw of its places.
    let playing = scatter(&params, n, &keys_in, PitchSet::EMPTY);
    assert_ne!(playing, first);
}

#[test]
fn the_cycle_order_restarts_at_a_start_of_the_transport_and_at_a_reset() {
    let params = [
        (CHANCE, 100.0),
        (CHOICES, 3.0),
        (INTERVAL, 4.0),
        (DIRECTION, UP as f32),
        (ORDER, CYCLE as f32),
    ];
    let mut r = Rig::new(&params);
    r.run(&melody(2, &[60]), 300);
    // Stopped, then started: the turn restarts.
    r.playing = false;
    r.run(&[], r.frame + 128);
    r.playing = true;
    let start = r.frame;
    r.run(&later(2, &[60], start), start + 300);
    r.reset();
    let start = r.frame;
    r.run(&later(2, &[60], start), start + 300);
    assert_eq!(r.keys(), [64, 68, 64, 68, 64, 68]);
}

#[test]
fn in_scale_mode_in_f_minor_no_note_comes_out_of_the_scale() {
    for interval in 1..=12 {
        for choices in [1.0, 2.0, 7.0, 24.0] {
            for direction in [UP, DOWN, AROUND] {
                for order in [0.0, CYCLE as f32] {
                    let base = [
                        (MODE, IN_SCALE as f32),
                        (INTERVAL, interval as f32),
                        (CHOICES, choices),
                        (DIRECTION, direction as f32),
                        (ORDER, order),
                        (SEED, interval as f32),
                    ];
                    // 1 000 chromatic notes, all moved: every one lands in F minor.
                    let mut all = base.to_vec();
                    all.push((CHANCE, 100.0));
                    let keys = scatter(&all, 1000, &CHROMATIC, F_MINOR);
                    assert_eq!(keys.len(), 1000);
                    for k in &keys {
                        assert!(F_MINOR.contains(*k), "{k} off F minor ({base:?})");
                    }
                    // 1 000 notes of the scale, half moved: still in it.
                    let in_scale: Vec<u8> = (36..96).filter(|k| F_MINOR.contains(*k)).collect();
                    let mut half = base.to_vec();
                    half.push((CHANCE, 50.0));
                    for k in scatter(&half, 1000, &in_scale, F_MINOR) {
                        assert!(F_MINOR.contains(k), "{k} off F minor ({base:?})");
                    }
                }
            }
        }
    }
}

#[test]
fn scale_steps_count_degrees_from_the_degree_at_or_below() {
    // F4 = 65: two degrees up is A♭4, one down E♭4; F♯4 counts from F4.
    assert_eq!(scale_step(65, 2, F_MINOR), 68);
    assert_eq!(scale_step(66, 2, F_MINOR), 68);
    assert_eq!(scale_step(65, -1, F_MINOR), 63);
    // Seven degrees are an octave.
    assert_eq!(scale_step(65, 7, F_MINOR), 77);
    assert_eq!(scale_step(65, -7, F_MINOR), 53);
    // C4 = 60 is a degree; B3 = 59 counts from B♭3.
    assert_eq!(scale_step(60, 1, F_MINOR), 61);
    assert_eq!(scale_step(59, 0, F_MINOR), 58);
    // Below the lowest class of the octave (C is F minor's lowest, 0): wraps to the octave below.
    let d_major = PitchSet::from_bits(
        (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 9) | (1 << 11) | (1 << 1),
    );
    assert_eq!(
        scale_step(60, 0, d_major),
        59,
        "C4 counts from B3 in D major"
    );
    // No scale: semitones.
    assert_eq!(scale_step(60, 5, PitchSet::EMPTY), 65);
    // Without a project scale, scale mode reads semitones.
    let keys = scatter(
        &[
            (MODE, IN_SCALE as f32),
            (CHANCE, 100.0),
            (CHOICES, 1.0),
            (INTERVAL, 3.0),
            (DIRECTION, UP as f32),
        ],
        10,
        &[60],
        PitchSet::EMPTY,
    );
    assert_eq!(keys, vec![63; 10]);
}

#[test]
fn the_choices_lie_up_down_or_around_one_interval_apart() {
    assert_eq!(
        (1..=5).map(|k| steps(k, AROUND)).collect::<Vec<_>>(),
        [1, -1, 2, -2, 3]
    );
    assert_eq!(steps(3, UP), 3);
    assert_eq!(steps(3, DOWN), -3);
    for (direction, choices, interval, expected) in [
        (UP, 1, 12, vec![12]),
        (UP, 3, 5, vec![5, 10, 15]),
        (DOWN, 2, 7, vec![-14, -7]),
        (AROUND, 1, 12, vec![12]),
        (AROUND, 2, 12, vec![-12, 12]),
        (AROUND, 5, 2, vec![-4, -2, 2, 4, 6]),
        (UP, 24, 1, (1..=24).collect()),
        (AROUND, 24, 1, (-12..=12).filter(|s| *s != 0).collect()),
    ] {
        let keys = scatter(
            &[
                (CHANCE, 100.0),
                (CHOICES, choices as f32),
                (INTERVAL, interval as f32),
                (DIRECTION, direction as f32),
                (SEED, 3.0),
            ],
            2000,
            &[64],
            PitchSet::EMPTY,
        );
        let offsets: BTreeSet<i32> = keys.iter().map(|k| i32::from(*k) - 64).collect();
        assert_eq!(
            offsets.into_iter().collect::<Vec<_>>(),
            expected,
            "{direction} {choices} {interval}: every choice drawn, none other"
        );
    }
}

#[test]
fn the_cycle_order_takes_each_choice_in_turn() {
    let keys = scatter(
        &[
            (CHANCE, 100.0),
            (CHOICES, 3.0),
            (INTERVAL, 4.0),
            (DIRECTION, UP as f32),
            (ORDER, CYCLE as f32),
        ],
        7,
        &[60],
        PitchSet::EMPTY,
    );
    assert_eq!(keys, [64, 68, 72, 64, 68, 72, 64]);
    // A note that stays (lost its draw) does not take a turn.
    let keys = scatter(
        &[
            (CHANCE, 50.0),
            (CHOICES, 3.0),
            (INTERVAL, 4.0),
            (DIRECTION, UP as f32),
            (ORDER, CYCLE as f32),
        ],
        300,
        &[60],
        PitchSet::EMPTY,
    );
    let moved: Vec<u8> = keys.into_iter().filter(|k| *k != 60).collect();
    for (i, k) in moved.iter().enumerate() {
        assert_eq!(*k, [64, 68, 72][i % 3]);
    }
}

#[test]
fn a_choice_off_the_keyboard_leaves_the_note_as_received() {
    let keys = scatter(
        &[
            (CHANCE, 100.0),
            (CHOICES, 1.0),
            (INTERVAL, 12.0),
            (DIRECTION, UP as f32),
        ],
        3,
        &[120, 115, 127],
        PitchSet::EMPTY,
    );
    assert_eq!(keys, [120, 127, 127]);
    let keys = scatter(
        &[
            (CHANCE, 100.0),
            (CHOICES, 1.0),
            (INTERVAL, 12.0),
            (DIRECTION, DOWN as f32),
        ],
        2,
        &[5, 12],
        PitchSet::EMPTY,
    );
    assert_eq!(keys, [5, 0]);
    // In the scale, the note stays on its own degree: F♯8 (126) on F8, off F minor no more.
    let up = [
        (MODE, IN_SCALE as f32),
        (CHANCE, 100.0),
        (CHOICES, 1.0),
        (INTERVAL, 12.0),
        (DIRECTION, UP as f32),
    ];
    assert_eq!(scatter(&up, 2, &[126, 125], F_MINOR), [125, 125]);
    // Down from C-1 (0) in D major: its degree, B, is below 0, so the next one up, C♯ (1).
    let d_major = PitchSet::from_bits(
        (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 9) | (1 << 11) | (1 << 1),
    );
    let mut down = up;
    down[4] = (DIRECTION, DOWN as f32);
    assert_eq!(scatter(&down, 2, &[0, 3], d_major), [1, 2]);
}

// ---------------------------------------------------------------- no stuck note

#[test]
fn each_note_off_comes_out_at_its_note_on_key_whatever_changed_in_between() {
    let params: [(u32, [f32; 3]); 7] = [
        (CHANCE, [0.0, 50.0, 100.0]),
        (CHOICES, [1.0, 9.0, 24.0]),
        (INTERVAL, [1.0, 7.0, 12.0]),
        (DIRECTION, [0.0, 1.0, 2.0]),
        (MODE, [0.0, 1.0, 0.0]),
        (ORDER, [0.0, 1.0, 0.0]),
        (SEED, [0.0, 500.0, 1000.0]),
    ];
    let mut r = Rig::new(&[(CHANCE, 100.0), (CHOICES, 24.0), (INTERVAL, 1.0)]);
    r.scale = F_MINOR;
    let mut frame = 0;
    for round in 0..30u32 {
        // Ten notes held, the settings changed under them, then released at their received keys.
        let events: Vec<(u64, Event)> = (0..10u32)
            .map(|i| {
                (
                    frame + u64::from(i),
                    on(i + 1, 50 + (i * 3 + round) as u8 % 40),
                )
            })
            .collect();
        r.run(&events, frame + 200);
        let (id, values) = params[round as usize % params.len()];
        r.set(id, values[round as usize % 3]);
        r.scale = if round % 2 == 0 {
            PitchSet::EMPTY
        } else {
            F_MINOR
        };
        let offs: Vec<(u64, Event)> = (0..10u32)
            .map(|i| {
                (
                    frame + 200 + u64::from(i),
                    off(i + 1, 50 + (i * 3 + round) as u8 % 40),
                )
            })
            .collect();
        frame += 400;
        r.run(&offs, frame);
        assert!(r.sounding().is_empty(), "round {round}");
    }
}

#[test]
fn a_stop_or_a_reset_ends_everything_and_the_late_note_offs_end_nothing_twice() {
    let all = [
        (CHANCE, 100.0),
        (CHOICES, 1.0),
        (INTERVAL, 7.0),
        (DIRECTION, UP as f32),
    ];
    // A stop: the all-notes-off goes on; the note-offs after it pass at the received key and
    // voice (the instrument has no such note any more: harmless).
    let mut r = Rig::new(&all);
    let all_off = Event::AllNotesOff { offset: 0 };
    r.run(
        &[
            (0, on(1, 60)),
            (0, on(2, 62)),
            (100, all_off),
            (200, off(1, 60)),
        ],
        300,
    );
    assert_eq!(r.log[2], Out::All { frame: 100 });
    assert_eq!(
        r.log[3],
        Out::Off {
            frame: 200,
            voice: 1,
            key: 60
        }
    );
    // A reset: the table is empty, the next notes draw from the start.
    let mut r = Rig::new(&all);
    r.run(&[(0, on(1, 60))], 128);
    r.reset();
    r.run(
        &[(200, off(1, 60)), (300, on(1, 60)), (400, off(1, 60))],
        512,
    );
    assert_eq!(
        r.log[1..],
        [
            Out::Off {
                frame: 200,
                voice: 1,
                key: 60
            },
            Out::On {
                frame: 300,
                voice: 1,
                key: 67,
                velocity: 0.75,
                tune: 0.0
            },
            Out::Off {
                frame: 400,
                voice: 1,
                key: 67
            },
        ]
    );
}

#[test]
fn a_repeated_voice_ends_its_held_note_first() {
    let mut r = Rig::new(&[
        (CHANCE, 100.0),
        (CHOICES, 1.0),
        (INTERVAL, 5.0),
        (DIRECTION, UP as f32),
    ]);
    r.run(&[(0, on(3, 60)), (50, on(3, 62)), (100, off(3, 62))], 256);
    assert_eq!(
        r.log,
        [
            Out::On {
                frame: 0,
                voice: 3,
                key: 65,
                velocity: 0.75,
                tune: 0.0
            },
            Out::Off {
                frame: 50,
                voice: 3,
                key: 65
            },
            Out::On {
                frame: 50,
                voice: 3,
                key: 67,
                velocity: 0.75,
                tune: 0.0
            },
            Out::Off {
                frame: 100,
                voice: 3,
                key: 67
            },
        ]
    );
}

#[test]
fn a_note_beyond_a_full_table_goes_through_unchanged_and_ends() {
    let mut r = Rig::new(&[
        (CHANCE, 100.0),
        (CHOICES, 1.0),
        (INTERVAL, 1.0),
        (DIRECTION, UP as f32),
    ]);
    let n = HELD as u32 + 3;
    let ons: Vec<(u64, Event)> = (0..n)
        .map(|i| (u64::from(i / 4), on(i + 1, (i % 100) as u8)))
        .collect();
    r.run(&ons, 128);
    let keys = r.keys();
    for (i, k) in keys.iter().enumerate() {
        let received = (i % 100) as u8;
        let expected = if i < HELD { received + 1 } else { received };
        assert_eq!(*k, expected, "note {i}");
    }
    let offs: Vec<(u64, Event)> = (0..n)
        .map(|i| (200 + u64::from(i / 4), off(i + 1, (i % 100) as u8)))
        .collect();
    r.run(&offs, 512);
    assert!(r.sounding().is_empty());
    // The table is free again.
    r.run(&[(600, on(1, 10)), (700, off(1, 10))], 800);
    assert_eq!(r.keys().last(), Some(&11));
    assert!(r.sounding().is_empty());
}

// ---------------------------------------------------------------- robustness

#[test]
fn every_parameter_at_every_extreme_keeps_the_stream_bounded_and_whole() {
    let extremes = [
        -1e9,
        -1.0,
        0.0,
        0.5,
        1.0,
        12.0,
        24.0,
        100.0,
        1000.0,
        1e9,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ];
    for p in DESCRIPTOR.params {
        for &value in &extremes {
            let mut r = Rig::new(&[(CHANCE, 100.0), (MODE, IN_SCALE as f32)]);
            r.scale = F_MINOR;
            r.set(p.id.0, value);
            // The value also arrives as an event, mid-block.
            let pv = Event::ParamValue {
                offset: 0,
                id: p.id,
                value,
            };
            let mut events = melody(40, &[0, 1, 60, 126, 127]);
            events.push((1234, pv));
            r.run(&events, 40 * 100 + 128);
            assert_eq!(r.keys().len(), 40, "{} = {value}", p.key);
            assert!(r.sounding().is_empty(), "{} = {value}", p.key);
        }
    }
}

#[test]
fn the_descriptor_is_valid_and_declares_its_memory() {
    DESCRIPTOR.validate().unwrap();
    assert_eq!(DESCRIPTOR.name, "Scatter");
    assert_eq!(DESCRIPTOR.kind, "faulk.scatter");
    assert!(DESCRIPTOR.memory_bytes as usize >= size_of::<Scatter>() + HELD * 12);
    let keys: Vec<_> = DESCRIPTOR.params.iter().map(|p| p.key).collect();
    assert_eq!(
        keys,
        [
            "chance",
            "choices",
            "interval",
            "direction",
            "mode",
            "order",
            "seed"
        ]
    );
    assert!(DESCRIPTOR.params.iter().all(|p| !p.description.is_empty()));
}

/// A clip's note at tick `T` plays on the frame its position falls in (`floor`, as the engine's
/// transport does): its place is `T` from any block start, at every tempo and rate where a
/// frame is shorter than a tick, the slowest and the fastest included.
#[test]
fn a_clip_note_is_placed_on_its_own_tick_from_any_block_start() {
    for (bpm, rate) in [
        (120.0, 48_000.0),
        (999.0, 48_000.0),
        (999.0, 22_050.0),
        (999.0, 16_000.0),
        (20.0, 8_000.0),
        (174.0, 44_100.0),
    ] {
        let tpf = bpm * 16.0 / rate;
        let samples_per_tick = 1.0 / tpf;
        for tick in (0..20_000i64).step_by(7) {
            let frame = (tick as f64 * samples_per_tick).floor() as i64;
            for start in [0i64, 1, frame / 3, frame - 1, frame].map(|s| s.clamp(0, frame)) {
                let position = start as f64 * tpf;
                let at = (frame - start) as usize;
                assert_eq!(
                    song_tick(position, at, tpf),
                    tick,
                    "{bpm} BPM at {rate} Hz, tick {tick} from frame {start}"
                );
            }
        }
    }
}
