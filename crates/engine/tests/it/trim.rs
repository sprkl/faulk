//! Trim, `faulk.trim` (#693): the note stream through the device contract, every `process`
//! under the allocation detector, the output buffer sized from the descriptor's bounds (and
//! never overflowed). Lengths in frames for tempo and free mode at several tempos, signatures
//! and rates, the release trigger, key tracking, overlapping notes, all-notes-off, reset,
//! stops and jumps of the transport, the overflow of the note table, parameter sweeps and
//! block-size invariance. The conformance run on every device of the registry is in
//! `osc_synth.rs`; the native/WASM parity is `crates/engine-wasm/tests/trim_parity.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::device::{AudioIo, BlockCtx, Device, Event, EventBuffer, Transport, VoiceId};
use engine::devices::trim::{
    self, CAPACITY, DESCRIPTOR, DIVISION, FEEL, FREE, GATE, KEY_TRACK, MODE, RELEASE, TIME,
    TRIGGER, Trim,
};
use engine::msg::ParamId;
use engine::time::{Tempo, TimeSignature};

const SR: f32 = 48_000.0;

fn trim_with(rate: f32, params: &[(u32, f32)]) -> Trim {
    let mut t = Trim::new(rate);
    for (id, v) in params {
        t.set_param(ParamId(*id), *v);
    }
    t.reset();
    t
}

fn voice(v: u32) -> VoiceId {
    VoiceId::new(v).unwrap()
}

fn on(v: u32, key: u8) -> Event {
    Event::NoteOn {
        offset: 0,
        voice: voice(v),
        key,
        velocity: 0.8,
        tune_cents: 0.0,
    }
}

fn off(v: u32, key: u8) -> Event {
    Event::NoteOff {
        offset: 0,
        voice: voice(v),
        key,
    }
}

fn transport(bpm: f64, sig: (u8, u8), playing: bool) -> Transport {
    Transport {
        playing,
        tempo: Tempo::new(bpm).unwrap(),
        signature: TimeSignature::new(sig.0, sig.1).unwrap(),
        position_ticks: 0.0,
        ..Transport::default()
    }
}

/// The transport of a block at `frame` with the clock running from frame 0 at `bpm`.
fn running(bpm: f64, sig: (u8, u8), frame: u64) -> Transport {
    Transport {
        position_ticks: frame as f64 * bpm * 16.0 / f64::from(SR),
        ..transport(bpm, sig, true)
    }
}

/// Renders `total` frames in blocks of `size` at `rate`, the input events at their absolute
/// frames, the transport of each block from `clock` (its position as given). Returns the
/// output events at their absolute frames (their offsets set to 0). The output buffer has the capacity the host gives
/// it (`MidiOutput::capacity` of the block's input) and must never overflow.
fn render_with(
    t: &mut Trim,
    rate: f32,
    total: u64,
    size: usize,
    clock: &dyn Fn(u64) -> Transport,
    input: &[(u64, Event)],
) -> Vec<(u64, Event)> {
    let bounds = DESCRIPTOR.midi_output.unwrap();
    let mut left = vec![0.0f32; size];
    let mut right = vec![0.0f32; size];
    let mut result = Vec::new();
    let mut start = 0u64;
    while start < total {
        let frames = (size as u64).min(total - start) as usize;
        let local: Vec<Event> = input
            .iter()
            .filter(|(at, _)| (start..start + frames as u64).contains(at))
            .map(|(at, e)| e.at((at - start) as u32))
            .collect();
        let mut out = EventBuffer::try_with_capacity(bounds.capacity(local.len())).unwrap();
        let ctx = BlockCtx {
            sample_rate: rate,
            frames: frames as u32,
            transport: clock(start),
        };
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: &mut left[..frames],
                right: &mut right[..frames],
                events_out: &mut out,
            };
            t.process(&ctx, &local, &mut io);
            out.finish();
        });
        assert_eq!(
            out.overflowed(),
            0,
            "the declared bounds hold (block at {start})"
        );
        let mut last = 0;
        for e in out.events() {
            assert!(e.offset() >= last, "sorted");
            assert!((e.offset() as usize) < frames, "inside the block");
            last = e.offset();
            // The absolute frame says where it is: the offset is cleared, so that streams cut
            // into different blocks compare.
            result.push((start + u64::from(e.offset()), e.at(0)));
        }
        start += frames as u64;
    }
    result
}

/// [`render_with`] at 120 BPM in 4/4, the clock running from tick 0.
fn render(t: &mut Trim, total: u64, size: usize, input: &[(u64, Event)]) -> Vec<(u64, Event)> {
    let clock = |frame: u64| Transport {
        position_ticks: frame as f64 * 1920.0 / f64::from(SR),
        ..transport(120.0, (4, 4), true)
    };
    render_with(t, SR, total, size, &clock, input)
}

/// The output notes as (start, end, key, velocity), matched by voice id.
fn notes(stream: &[(u64, Event)]) -> Vec<(u64, u64, u8, f32)> {
    let mut open: Vec<(u32, u64, u8, f32)> = Vec::new();
    let mut done = Vec::new();
    for (at, e) in stream {
        match *e {
            Event::NoteOn {
                voice,
                key,
                velocity,
                ..
            } => {
                assert!(
                    !open.iter().any(|o| o.0 == voice.get()),
                    "a voice id is unique among the notes held"
                );
                open.push((voice.get(), *at, key, velocity));
            }
            Event::NoteOff { voice, .. } => {
                let i = open
                    .iter()
                    .position(|o| o.0 == voice.get())
                    .expect("a note-off ends a held note");
                let (_, start, key, vel) = open.swap_remove(i);
                done.push((start, *at, key, vel));
            }
            Event::AllNotesOff { .. } => {
                for (_, start, key, vel) in open.drain(..) {
                    done.push((start, *at, key, vel));
                }
            }
            Event::ParamValue { .. } => panic!("Trim writes notes only"),
        }
    }
    assert!(open.is_empty(), "no note left held: {open:?}");
    done.sort_by_key(|n| (n.0, n.1));
    done
}

// ---------------------------------------------------------------- lengths

#[test]
fn an_eighth_at_120_bpm_ends_every_note_250_ms_after_its_start_whatever_its_input_length() {
    for rate in [44_100.0f32, 48_000.0, 96_000.0] {
        let quarter_second = (rate / 4.0) as u64;
        let clock = |frame: u64| Transport {
            position_ticks: frame as f64 * 1920.0 / f64::from(rate),
            ..transport(120.0, (4, 4), true)
        };
        let input = [
            (100, on(1, 60)),
            (101, off(1, 60)),
            (50_000, on(2, 62)),
            (55_000, off(2, 62)),
            (100_000, on(3, 64)),
            (100_000 + 3 * rate as u64, off(3, 64)),
        ];
        let mut streams = Vec::new();
        for size in [1usize, 37, 128, 2048] {
            let mut t = trim_with(rate, &[(DIVISION, trim::DIV_1_8 as f32)]);
            let total = 100_000 + 4 * rate as u64;
            let stream = render_with(&mut t, rate, total, size, &clock, &input);
            let n = notes(&stream);
            assert_eq!(n.len(), 3, "{rate} {size}");
            for (start, end, ..) in &n {
                assert_eq!(end - start, quarter_second, "{rate} Hz, blocks of {size}");
            }
            assert_eq!(
                n.iter().map(|n| n.0).collect::<Vec<_>>(),
                [100, 50_000, 100_000]
            );
            streams.push(stream);
        }
        assert!(
            streams.windows(2).all(|w| w[0] == w[1]),
            "the same stream whatever the block size"
        );
    }
}

/// The length of one note under `params` at `bpm` and `sig`, at 48 kHz.
fn one_length(params: &[(u32, f32)], bpm: f64, sig: (u8, u8), key: u8) -> u64 {
    let mut t = trim_with(SR, params);
    let clock = move |frame: u64| running(bpm, sig, frame);
    let stream = render_with(
        &mut t,
        SR,
        3_000_000,
        2048,
        &clock,
        &[(10, on(1, key)), (20, off(1, key))],
    );
    let n = notes(&stream);
    assert_eq!(n.len(), 1);
    n[0].1 - n[0].0
}

/// A case of the tempo lengths: division, feel, gate, tempo, signature, frames.
type Case = (u32, u32, f32, f64, (u8, u8), u64);

#[test]
fn tempo_lengths_in_ticks_become_frames_at_the_tempo_and_the_signature() {
    use trim::{
        DIV_1_2, DIV_1_4, DIV_1_8, DIV_1_16, DIV_1_64, DIV_1_BAR, DIV_2_BARS, DIV_4_BARS, DOTTED,
        STRAIGHT, TRIPLET,
    };
    // (division, feel, gate, bpm, signature, frames at 48 kHz, worked by hand)
    let cases: &[Case] = &[
        // 480 ticks at 174 BPM: 480 / (174 × 16) s = 0.172 413 8 s = 8 275.86 frames.
        (DIV_1_8, STRAIGHT, 100.0, 174.0, (4, 4), 8_276),
        (DIV_1_4, STRAIGHT, 100.0, 60.0, (4, 4), 48_000),
        // 60 ticks at 999 BPM: 60 / 15 984 s = 180.18 frames.
        (DIV_1_64, STRAIGHT, 100.0, 999.0, (4, 4), 180),
        // A bar of 3/4 and of 6/8: 2 880 ticks, 1.5 s at 120 BPM.
        (DIV_1_BAR, STRAIGHT, 100.0, 120.0, (3, 4), 72_000),
        (DIV_1_BAR, STRAIGHT, 100.0, 120.0, (6, 8), 72_000),
        // Two bars of 7/8: 6 720 ticks, 3.5 s.
        (DIV_2_BARS, STRAIGHT, 100.0, 120.0, (7, 8), 168_000),
        // A quarter triplet: 640 ticks, 1/3 s; a dotted eighth: 720 ticks, 0.375 s.
        (DIV_1_4, TRIPLET, 100.0, 120.0, (4, 4), 16_000),
        (DIV_1_8, DOTTED, 100.0, 120.0, (4, 4), 18_000),
        // Gate 1 % of a sixteenth: 2.4 ticks, 1.25 ms; 200 % of a half: 3 840 ticks, 2 s.
        (DIV_1_16, STRAIGHT, 1.0, 120.0, (4, 4), 60),
        (DIV_1_2, STRAIGHT, 200.0, 120.0, (4, 4), 96_000),
        // Gate 50 % of a sixteenth at 120: 120 ticks, 62.5 ms.
        (DIV_1_16, STRAIGHT, 50.0, 120.0, (4, 4), 3_000),
        // Four bars of 4/4 at 20 BPM: 15 360 ticks, 48 s.
        (DIV_4_BARS, STRAIGHT, 100.0, 20.0, (4, 4), 2_304_000),
    ];
    for &(division, feel, gate, bpm, sig, frames) in cases {
        let params = [
            (DIVISION, division as f32),
            (FEEL, feel as f32),
            (GATE, gate),
        ];
        assert_eq!(
            one_length(&params, bpm, sig, 60),
            frames,
            "division {division} feel {feel} gate {gate} at {bpm} BPM in {sig:?}"
        );
    }
}

#[test]
fn free_lengths_are_the_time_in_frames() {
    for (seconds, frames) in [(0.001f32, 48u64), (0.25, 12_000), (10.0, 480_000)] {
        let params = [(MODE, FREE as f32), (TIME, seconds)];
        assert_eq!(one_length(&params, 120.0, (4, 4), 60), frames, "{seconds}");
        // The tempo does not matter in free mode.
        assert_eq!(one_length(&params, 61.3, (7, 8), 60), frames, "{seconds}");
    }
}

#[test]
fn key_tracking_doubles_the_length_one_octave_down_at_plus_100() {
    let base = [(MODE, FREE as f32), (TIME, 0.25)];
    let with = |amount: f32, key: u8| {
        let mut p = base.to_vec();
        p.push((KEY_TRACK, amount));
        one_length(&p, 120.0, (4, 4), key)
    };
    // factor = 2^(key_track / 100 × (60 − pitch) / 12)
    assert_eq!(with(100.0, 60), 12_000);
    assert_eq!(with(100.0, 48), 24_000);
    assert_eq!(with(100.0, 36), 48_000);
    assert_eq!(with(100.0, 72), 6_000);
    assert_eq!(with(-100.0, 48), 6_000);
    assert_eq!(with(-100.0, 72), 24_000);
    assert_eq!(with(50.0, 36), 24_000);
    assert_eq!(with(0.0, 0), 12_000);
    // A fine tuning counts in the pitch: 48 + 1200 cents is 60.
    let mut t = trim_with(SR, &[(MODE, FREE as f32), (TIME, 0.25), (KEY_TRACK, 100.0)]);
    let tuned = Event::NoteOn {
        offset: 0,
        voice: voice(1),
        key: 48,
        velocity: 1.0,
        tune_cents: 1200.0,
    };
    let n = notes(&render(&mut t, 100_000, 512, &[(0, tuned)]));
    assert_eq!(n[0].1 - n[0].0, 12_000);
}

// ---------------------------------------------------------------- trigger

#[test]
fn on_release_the_note_starts_at_the_frame_of_the_received_note_off() {
    let mut t = trim_with(
        SR,
        &[(TRIGGER, RELEASE as f32), (DIVISION, trim::DIV_1_8 as f32)],
    );
    let input = [
        (1_000, on(7, 60)),
        (30_001, off(7, 60)),
        (40_000, on(8, 64)),
    ];
    let stream = render(&mut t, 100_000, 333, &input);
    // Nothing before the release; the held key 64 is never released: nothing for it.
    assert!(stream.iter().all(|(at, _)| *at >= 30_001));
    let n = notes(&stream);
    assert_eq!(n, vec![(30_001, 42_001, 60, 0.8)]);
    assert_eq!(t.held(), 1);
}

#[test]
fn a_shorter_note_is_lengthened_and_a_longer_one_cut() {
    let mut t = trim_with(SR, &[(DIVISION, trim::DIV_1_16 as f32)]);
    let input = [
        (0, on(1, 60)),
        (10, off(1, 60)),
        (20_000, on(2, 60)),
        (90_000, off(2, 60)),
    ];
    let n = notes(&render(&mut t, 120_000, 128, &input));
    assert_eq!(n, vec![(0, 6_000, 60, 0.8), (20_000, 26_000, 60, 0.8)]);
}

#[test]
fn overlapping_notes_of_one_key_and_a_reused_voice_each_keep_their_length() {
    let mut t = trim_with(SR, &[(DIVISION, trim::DIV_1_8 as f32)]);
    // The live input reuses voice 61 for every press of key 60.
    let input = [
        (0, on(61, 60)),
        (100, off(61, 60)),
        (200, on(61, 60)),
        (300, off(61, 60)),
        (400, on(61, 60)),
        // A chord of 8 notes at one frame.
        (5_000, on(1, 48)),
        (5_000, on(2, 52)),
        (5_000, on(3, 55)),
        (5_000, on(4, 59)),
        (5_000, on(5, 62)),
        (5_000, on(6, 65)),
        (5_000, on(7, 69)),
        (5_000, on(8, 72)),
    ];
    let stream = render(&mut t, 50_000, 64, &input);
    let n = notes(&stream);
    assert_eq!(n.len(), 11);
    for (start, end, ..) in &n {
        assert_eq!(end - start, 12_000);
    }
    assert_eq!(n[0].0, 0);
    assert_eq!(n[1].0, 200);
    assert_eq!(n[2].0, 400);
}

#[test]
fn an_end_and_a_start_on_the_same_frame_come_out_end_first() {
    let mut t = trim_with(SR, &[(MODE, FREE as f32), (TIME, 0.01)]);
    let stream = render(&mut t, 2_000, 100, &[(0, on(1, 60)), (480, on(2, 60))]);
    let at_480: Vec<_> = stream.iter().filter(|(at, _)| *at == 480).collect();
    assert!(matches!(at_480[0].1, Event::NoteOff { .. }), "{at_480:?}");
    assert!(matches!(at_480[1].1, Event::NoteOn { .. }), "{at_480:?}");
}

// ---------------------------------------------------------------- endings

#[test]
fn all_notes_off_is_passed_on_and_nothing_ends_after_it() {
    for trigger in [trim::START, RELEASE] {
        let mut t = trim_with(
            SR,
            &[
                (DIVISION, trim::DIV_1_BAR as f32),
                (TRIGGER, trigger as f32),
            ],
        );
        let input = [
            (0, on(1, 60)),
            (10, off(1, 60)),
            (20, on(2, 62)),
            (5_000, Event::AllNotesOff { offset: 0 }),
            (6_000, off(2, 62)),
        ];
        let stream = render(&mut t, 200_000, 256, &input);
        let last = stream.last().unwrap();
        assert_eq!(*last, (5_000, Event::AllNotesOff { offset: 0 }));
        assert_eq!((t.sounding(), t.held()), (0, 0));
        notes(&stream);
    }
}

#[test]
fn reset_forgets_every_note_and_emits_nothing_after() {
    let mut t = trim_with(SR, &[(DIVISION, trim::DIV_1_BAR as f32)]);
    render(&mut t, 1_000, 128, &[(0, on(1, 60))]);
    assert_eq!(t.sounding(), 1);
    t.reset();
    assert_eq!(t.sounding(), 0);
    assert!(render(&mut t, 200_000, 2048, &[]).is_empty());
}

#[test]
fn reset_with_both_tables_full_takes_constant_time_and_allocates_nothing() {
    // The engine resets a MIDI effect on the audio thread at each bypass toggle (#219).
    for trigger in [trim::START, RELEASE] {
        let mut t = trim_with(
            SR,
            &[
                (DIVISION, trim::DIV_4_BARS as f32),
                (TRIGGER, trigger as f32),
            ],
        );
        let input: Vec<_> = (0..CAPACITY as u32)
            .flat_map(|i| {
                [
                    (u64::from(i), on(i + 1, 60)),
                    (u64::from(i), off(i + 1, 60)),
                ]
            })
            .chain((0..CAPACITY as u32).map(|i| (200 + u64::from(i), on(1_000 + i, 64))))
            .collect();
        render(&mut t, 1_024, 1_024, &input);
        assert_eq!(t.sounding(), CAPACITY);
        assert_no_alloc(|| t.reset());
        assert_eq!((t.sounding(), t.held()), (0, 0));
    }
}

#[test]
fn a_stop_ends_every_note_at_its_first_frame_and_fires_no_held_key() {
    let mut t = trim_with(SR, &[(DIVISION, trim::DIV_4_BARS as f32)]);
    let clock = |frame: u64| Transport {
        position_ticks: frame as f64 * 1920.0 / f64::from(SR),
        ..transport(120.0, (4, 4), frame < 10_240)
    };
    let input = [(0, on(1, 60)), (100, on(2, 64))];
    let stream = render_with(&mut t, SR, 30_000, 512, &clock, &input);
    let n = notes(&stream);
    assert_eq!(n, vec![(0, 10_240, 60, 0.8), (100, 10_240, 64, 0.8)]);

    // Release mode: the stop's note-offs (the schedule releases its notes) fire nothing.
    let mut t = trim_with(SR, &[(TRIGGER, RELEASE as f32)]);
    let input = [(0, on(1, 60)), (10_240, off(1, 60))];
    let stream = render_with(&mut t, SR, 30_000, 512, &clock, &input);
    assert!(stream.is_empty(), "{stream:?}");
    // Played again after the stop, it works.
    let replay = |frame: u64| Transport {
        playing: frame >= 30_000,
        ..running(120.0, (4, 4), frame.saturating_sub(30_000))
    };
    let input = [(30_000, on(3, 60)), (31_000, off(3, 60))];
    let stream = render_with(&mut t, SR, 60_000, 512, &replay, &input);
    assert_eq!(notes(&stream), vec![(31_000, 37_000, 60, 0.8)]);
}

#[test]
fn a_jump_while_playing_releases_like_any_note_off_and_keeps_the_notes_sounding() {
    // A loop wrap at frame 4 096: the position goes back to 0.
    let clock = |frame: u64| {
        let f = if frame >= 4_096 { frame - 4_096 } else { frame };
        Transport {
            position_ticks: f as f64 * 1920.0 / f64::from(SR),
            ..transport(120.0, (4, 4), true)
        }
    };
    // Release mode: the note-off the schedule sends with the wrap (a note that ends on the
    // loop's end, or one the loop cuts) fires the note at the wrap.
    let mut t = trim_with(SR, &[(TRIGGER, RELEASE as f32)]);
    let input = [(0, on(1, 60)), (4_096, off(1, 60))];
    let n = notes(&render_with(&mut t, SR, 20_000, 512, &clock, &input));
    assert_eq!(n, vec![(4_096, 10_096, 60, 0.8)]);

    let mut t = trim_with(SR, &[(DIVISION, trim::DIV_1_4 as f32)]);
    let input = [(0, on(1, 60)), (4_096, off(1, 60))];
    let n = notes(&render_with(&mut t, SR, 40_000, 512, &clock, &input));
    assert_eq!(n, vec![(0, 24_000, 60, 0.8)]);
}

#[test]
fn a_full_table_ends_the_oldest_note_at_the_frame_of_the_new_one() {
    for trigger in [trim::START, RELEASE] {
        let mut t = trim_with(
            SR,
            &[
                (TRIGGER, trigger as f32),
                (DIVISION, trim::DIV_4_BARS as f32),
            ],
        );
        let mut input = Vec::new();
        for i in 0..(CAPACITY as u32 + 3) {
            input.push((u64::from(i) * 10, on(i + 1, (i % 128) as u8)));
            if trigger == RELEASE {
                input.push((u64::from(i) * 10 + 5, off(i + 1, (i % 128) as u8)));
            }
        }
        // One block holds them all: the declared bounds still hold (render checks it).
        let stream = render(&mut t, 2048, 2048, &input);
        let first = if trigger == RELEASE { 5 } else { 0 };
        let ends: Vec<u64> = stream
            .iter()
            .filter(|(_, e)| matches!(e, Event::NoteOff { .. }))
            .map(|(at, _)| *at)
            .collect();
        let overflow_at = CAPACITY as u64 * 10 + first;
        assert_eq!(ends, vec![overflow_at, overflow_at + 10, overflow_at + 20]);
        assert_eq!(t.sounding(), CAPACITY);
    }
    // The table of held keys: the oldest key is forgotten.
    let mut t = trim_with(SR, &[(TRIGGER, RELEASE as f32)]);
    let input: Vec<_> = (0..(CAPACITY as u32 + 1))
        .map(|i| (u64::from(i), on(i + 1, 60)))
        .chain([(500, off(1, 60)), (501, off(2, 60))])
        .collect();
    let n = notes(&render(&mut t, 20_000, 1024, &input));
    assert_eq!(n, vec![(501, 6_501, 60, 0.8)]);
}

#[test]
fn a_parameter_change_while_notes_sound_keeps_their_ends() {
    let mut t = trim_with(SR, &[(DIVISION, trim::DIV_1_8 as f32)]);
    let set = |id: u32, value: f32| Event::ParamValue {
        offset: 0,
        id: ParamId(id),
        value,
    };
    let input = [
        (0, on(1, 60)),
        (100, set(DIVISION, trim::DIV_4_BARS as f32)),
        (100, set(MODE, FREE as f32)),
        (100, set(KEY_TRACK, 100.0)),
        (100, set(TRIGGER, RELEASE as f32)),
        (200, off(1, 60)),
        // Pressed in release mode, released in start mode: still plays at its release.
        (300, on(2, 60)),
        (400, set(TRIGGER, trim::START as f32)),
        (500, off(2, 60)),
    ];
    let n = notes(&render(&mut t, 50_000, 77, &input));
    assert_eq!(n, vec![(0, 12_000, 60, 0.8), (500, 5_300, 60, 0.8)]);
}

#[test]
fn every_parameter_swept_to_its_bounds_and_wild_values_never_leaves_a_note_held() {
    let mut rng = 0x0693_u64;
    let mut next = move || {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let wild = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30];
    let mut input = Vec::new();
    let mut frame = 0u64;
    let mut held: Vec<(u32, u8)> = Vec::new();
    for i in 0..3_000u32 {
        frame += next() % 200;
        let p = &DESCRIPTOR.params[(next() % DESCRIPTOR.params.len() as u64) as usize];
        let value = match next() % 6 {
            0 => p.min as f32,
            1 => p.max as f32,
            2 => wild[(next() % 5) as usize],
            _ => (p.min + (p.max - p.min) * (next() % 1000) as f64 / 1000.0) as f32,
        };
        // Free lengths kept short so that every note ends within the run.
        let value = if p.id.0 == TIME {
            value.min(0.5)
        } else {
            value
        };
        let value = if p.id.0 == DIVISION {
            value.min(trim::DIV_1_2 as f32)
        } else {
            value
        };
        input.push((
            frame,
            Event::ParamValue {
                offset: 0,
                id: p.id,
                value,
            },
        ));
        if !held.is_empty() && next() % 2 == 0 {
            let (v, k) = held.swap_remove((next() % held.len() as u64) as usize);
            input.push((frame, off(v, k)));
        } else {
            let k = (next() % 128) as u8;
            input.push((frame, on(i + 1, k)));
            held.push((i + 1, k));
        }
    }
    // Lengths at most 0.5 s or a half note at 120 (gate 200 %: 2 s), times key tracking up to
    // 2^5: the run goes on long enough for every note to end.
    let total = frame + 100 * SR as u64;
    let mut streams = Vec::new();
    for size in [1usize << 11, 333] {
        let mut t = Trim::new(SR);
        let stream = render(&mut t, total, size, &input);
        notes(&stream);
        assert_eq!(t.sounding(), 0);
        streams.push(stream);
    }
    assert_eq!(
        streams[0], streams[1],
        "the same stream whatever the block size"
    );
}

#[test]
fn non_finite_and_unknown_parameters_are_ignored() {
    let mut t = Trim::new(SR);
    for p in DESCRIPTOR.params {
        t.set_param(p.id, f32::NAN);
        t.set_param(p.id, f32::INFINITY);
    }
    t.set_param(ParamId(99), 1.0);
    // Still the defaults: a sixteenth at 120 BPM.
    let n = notes(&render(&mut t, 20_000, 128, &[(0, on(1, 60))]));
    assert_eq!(n[0].1, 6_000);
}

#[test]
fn the_descriptor_declares_the_contract_s_midi_effect() {
    use engine::device::DeviceRole;
    assert_eq!(DESCRIPTOR.validate(), Ok(()));
    assert_eq!((DESCRIPTOR.kind, DESCRIPTOR.name), ("faulk.trim", "Trim"));
    assert_eq!(DESCRIPTOR.role, DeviceRole::MidiEffect);
    assert!(DESCRIPTOR.memory_bytes >= size_of::<Trim>() as u64);
    let keys: Vec<_> = DESCRIPTOR.params.iter().map(|p| p.key).collect();
    assert_eq!(
        keys,
        [
            "mode",
            "time",
            "division",
            "feel",
            "gate",
            "trigger",
            "key_track"
        ]
    );
    assert!(DESCRIPTOR.params.iter().all(|p| p.smoothing_seconds == 0.0));
}

#[test]
fn an_end_due_before_a_later_event_of_the_block_comes_out_at_its_own_frame() {
    let mut t = trim_with(SR, &[(MODE, FREE as f32), (TIME, 0.001)]);
    // One block of 2048: the note ends at 148, the next event is at 1 000.
    let stream = render(&mut t, 2048, 2048, &[(100, on(1, 60)), (1_000, on(2, 62))]);
    assert_eq!(
        notes(&stream),
        vec![(100, 148, 60, 0.8), (1_000, 1_048, 62, 0.8)]
    );
}

// ---------------------------------------------------------------- in the engine

mod engine_level {
    use engine::OSC_SYNTH;
    use engine::devices::trim;
    use engine::engine::cycle::Cycle;
    use engine::msg::{
        ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state, encode_stream,
    };
    use engine::time::{Quantize, Tempo, Tick, TimeSignature};

    const RATE: f32 = 48_000.0;
    const T: TrackSlot = TrackSlot(0);
    const FX: DeviceSlot = DeviceSlot(0);

    struct Run {
        cycle: Cycle,
        seq: u32,
    }

    impl Run {
        /// Trim (4 bars at 120 BPM: 8 s) before the synth (release 10 ms), the clock running,
        /// one note played and released at once: Trim holds it.
        fn new() -> Self {
            let mut run = Run {
                cycle: Cycle::new(RATE).unwrap(),
                seq: 0,
            };
            run.send(&[
                M::Reset { epoch: 1 },
                M::SetTransport {
                    tempo: Tempo::new(120.0).unwrap(),
                    signature: TimeSignature::default(),
                    launch_quantize: Quantize::default(),
                },
                M::TrackCreate {
                    track: T,
                    kind: TrackKind::Midi,
                },
                M::DeviceInsert {
                    track: T,
                    device: FX,
                    kind: trim::DESCRIPTOR.kind_id,
                    state: encode_state(&[(ParamId(trim::DIVISION), trim::DIV_4_BARS as f32)]),
                },
                M::DeviceInsert {
                    track: T,
                    device: DeviceSlot(1),
                    kind: OSC_SYNTH.kind_id,
                    state: encode_state(&[(ParamId(4), 0.01)]),
                },
                M::Play { from: Tick(0) },
                M::NoteOn {
                    track: T,
                    pitch: 60,
                    velocity: 120,
                    at: None,
                },
                M::NoteOff {
                    track: T,
                    pitch: 60,
                    at: None,
                },
            ]);
            run
        }

        fn send(&mut self, msgs: &[M]) {
            let bytes = encode_stream(1, self.seq, msgs).unwrap();
            self.seq = self.seq.wrapping_add(msgs.len() as u32);
            self.cycle.submit(&bytes).unwrap();
        }

        /// Peak of the next `blocks` blocks of 128 frames.
        fn peak(&mut self, blocks: usize) -> f32 {
            let mut peak = 0f32;
            for _ in 0..blocks {
                let n = self.cycle.render(128);
                for s in self.cycle.left()[..n]
                    .iter()
                    .chain(&self.cycle.right()[..n])
                {
                    peak = peak.max(s.abs());
                }
            }
            peak
        }
    }

    /// The note sounds for a second (Trim holds it), then `action` runs; 50 ms later the
    /// track is silent.
    fn ends_after(action: &[M]) {
        let mut run = Run::new();
        assert!(
            run.peak(375) > 0.05,
            "Trim holds the note after its note-off"
        );
        run.send(action);
        run.peak(19);
        let after = run.peak(20);
        assert!(after < 1e-4, "{action:?}: still sounding, peak {after}");
    }

    #[test]
    fn a_stop_ends_the_notes_trim_holds() {
        ends_after(&[M::Stop]);
    }

    #[test]
    fn switching_trim_off_ends_the_notes_it_holds() {
        ends_after(&[M::DeviceSetEnabled {
            track: T,
            device: FX,
            enabled: false,
        }]);
    }

    #[test]
    fn removing_trim_ends_the_notes_it_holds() {
        ends_after(&[M::DeviceRemove {
            track: T,
            device: FX,
        }]);
    }

    #[test]
    fn a_parameter_change_keeps_the_note_to_its_end_and_no_further() {
        let mut run = Run::new();
        run.peak(375);
        run.send(&[M::ParamSet {
            track: T,
            device: FX,
            param: ParamId(trim::DIVISION),
            value: trim::DIV_1_64 as f32,
        }]);
        // Still sounding: its 8 s were set when it started.
        assert!(run.peak(375 * 6) > 0.05);
        // Sounding to 8 s, then the release (10 ms) and silence.
        assert!(run.peak(370) > 0.05);
        run.peak(15);
        assert!(run.peak(20) < 1e-4);
        // A note played afterwards gets the new length (1/64: 31 ms).
        run.send(&[M::NoteOn {
            track: T,
            pitch: 64,
            velocity: 120,
            at: None,
        }]);
        assert!(run.peak(2) > 0.01);
        // 31 ms and the release: silent from 53 ms on.
        run.peak(18);
        assert!(run.peak(20) < 1e-4, "a 1/64 note ends after 31 ms");
    }
}
