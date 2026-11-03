//! Device contract (CT-5): the test devices of `engine::device::testing` rendered offline,
//! asserting on the signal, under the allocation detector.

use assert_no_alloc::assert_no_alloc;
use engine::device::testing::{
    self, CONFORMANCE_MAX_FRAMES, Chain, GAIN, SCENARIO, SCENARIO_BLOCKS, SCENARIO_FRAMES,
    Scheduled, TONE, TRANSPOSE, Target, TestGain, TestTone, TestTranspose, gain, tone, transpose,
};
use engine::device::{
    AudioIo, BlockCtx, DescriptorError, Device, DeviceDescriptor, DeviceRole, DeviceSetup, Event,
    EventBuffer, Factory, MAX_DESCRIPTION_LEN, MAX_KEY_LEN, MAX_PARAMS_PER_DEVICE, MidiOutput,
    NamedKey, ParamDescriptor, Scale, Transport, Unit, VoiceId, builtin, builtin_descriptors,
};
use engine::msg::{DeviceKind, ParamId};
use engine::{OSC_SYNTH, Param, Synth};

const SR: f32 = 48_000.0;

fn setup() -> DeviceSetup {
    DeviceSetup {
        sample_rate: SR,
        max_block_frames: 2048,
    }
}

fn ctx(frames: usize) -> BlockCtx {
    BlockCtx {
        sample_rate: SR,
        frames: frames as u32,
        transport: Transport::default(),
    }
}

fn voice(id: u32) -> VoiceId {
    VoiceId::new(id).unwrap()
}

fn on(offset: u32, id: u32, key: u8, tune_cents: f32) -> Event {
    Event::NoteOn {
        offset,
        voice: voice(id),
        key,
        velocity: 1.0,
        tune_cents,
    }
}

fn off(offset: u32, id: u32) -> Event {
    Event::NoteOff {
        offset,
        voice: voice(id),
        key: 0,
    }
}

fn pv(offset: u32, id: ParamId, value: f32) -> Event {
    Event::ParamValue { offset, id, value }
}

/// Renders `blocks` blocks of `frames` frames through `device`, `events[b]` in block `b`,
/// with `input` as the input of every frame; returns the left channel (checked equal to the
/// right one), under the allocation detector.
fn run(
    device: &mut dyn Device,
    frames: usize,
    blocks: usize,
    events: &[(usize, Event)],
    input: f32,
) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * blocks];
    let mut right = vec![0.0f32; frames * blocks];
    let mut scratch = EventBuffer::try_with_capacity(16).unwrap();
    let mut block_events = Vec::with_capacity(events.len());
    for b in 0..blocks {
        block_events.clear();
        block_events.extend(events.iter().filter(|(eb, _)| *eb == b).map(|(_, e)| *e));
        let (l, r) = (
            &mut out[b * frames..(b + 1) * frames],
            &mut right[b * frames..(b + 1) * frames],
        );
        l.fill(input);
        r.fill(input);
        let c = ctx(frames);
        assert_no_alloc(|| {
            let mut io = AudioIo {
                left: l,
                right: r,
                events_out: &mut scratch,
            };
            device.process(&c, &block_events, &mut io);
        });
    }
    assert_eq!(out, right, "the test devices are mono");
    out
}

fn assert_clean(signal: &[f32]) {
    for (i, s) in signal.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is not finite: {s}");
        assert!(
            *s == 0.0 || s.abs() >= f32::MIN_POSITIVE,
            "sample {i} is denormal"
        );
    }
}

/// Frequency from rising zero crossings.
fn frequency(signal: &[f32]) -> f32 {
    let crossings: Vec<usize> = signal
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, _)| i)
        .collect();
    let periods = (crossings.len() - 1) as f32;
    let samples = (crossings[crossings.len() - 1] - crossings[0]) as f32;
    SR * periods / samples
}

// ---------------------------------------------------------------- descriptors

#[test]
fn every_descriptor_is_valid() {
    for d in builtin_descriptors().chain([&TONE, &GAIN, &TRANSPOSE]) {
        assert_eq!(d.validate(), Ok(()), "{}", d.kind);
    }
}

#[test]
fn builtin_kinds_and_numbers_are_unique() {
    let all: Vec<_> = builtin_descriptors().collect();
    for (i, a) in all.iter().enumerate() {
        for b in &all[..i] {
            assert_ne!(a.kind, b.kind);
            assert_ne!(a.kind_id, b.kind_id);
        }
        assert_eq!(builtin(a.kind_id).map(|d| d.kind), Some(a.kind));
    }
    assert!(builtin(DeviceKind(0)).is_none());
    // Pinned: the number of a kind never changes.
    assert_eq!(OSC_SYNTH.kind_id, DeviceKind(1));
    assert_eq!(OSC_SYNTH.kind, "faulk.osc_synth");
    assert_eq!(OSC_SYNTH.name, "Oscillator");
}

#[test]
fn validate_rejects_each_broken_rule() {
    let base_param = ParamDescriptor {
        id: ParamId(0),
        key: "cutoff",
        description: "Test parameter cutoff.",
        unit: Unit::Hertz,
        min: 20.0,
        max: 20_000.0,
        default: 1_000.0,
        scale: Scale::Log,
        step: 0.0,
        smoothing_seconds: 0.01,
        values: &[],
    };
    let base = DeviceDescriptor {
        kind: "faulk.filter",
        kind_id: DeviceKind(9),
        version: 1,
        name: "Filter",
        role: DeviceRole::Effect,
        memory_bytes: 64,
        midi_output: None,
        named_keys: &[],
        params: &[],
    };
    let with = |params: Vec<ParamDescriptor>| DeviceDescriptor {
        params: Box::leak(params.into_boxed_slice()),
        ..base
    };
    let one = |p: ParamDescriptor| with(vec![p]);
    assert_eq!(one(base_param).validate(), Ok(()));

    let cases: Vec<(DeviceDescriptor, DescriptorError)> = vec![
        (
            DeviceDescriptor {
                kind: "filter",
                ..base
            },
            DescriptorError::BadKind,
        ),
        (
            DeviceDescriptor {
                kind: "faulk.Filter",
                ..base
            },
            DescriptorError::BadKind,
        ),
        (
            DeviceDescriptor {
                kind: "faulk.9x",
                ..base
            },
            DescriptorError::BadKind,
        ),
        (
            DeviceDescriptor {
                kind: Box::leak(format!("faulk.{}", "a".repeat(MAX_KEY_LEN)).into_boxed_str()),
                ..base
            },
            DescriptorError::BadKind,
        ),
        (
            DeviceDescriptor { name: "", ..base },
            DescriptorError::BadName,
        ),
        (
            DeviceDescriptor {
                name: " Filter",
                ..base
            },
            DescriptorError::BadName,
        ),
        (
            DeviceDescriptor {
                name: "Filtré",
                ..base
            },
            DescriptorError::BadName,
        ),
        (
            DeviceDescriptor {
                kind_id: DeviceKind(0),
                ..base
            },
            DescriptorError::BadKindId,
        ),
        (
            DeviceDescriptor {
                role: DeviceRole::MidiEffect,
                ..base
            },
            DescriptorError::BadMidiOutput,
        ),
        (
            DeviceDescriptor {
                role: DeviceRole::MidiEffect,
                midi_output: Some(MidiOutput::default()),
                ..base
            },
            DescriptorError::BadMidiOutput,
        ),
        (
            DeviceDescriptor {
                midi_output: Some(MidiOutput {
                    per_input: 1,
                    per_block: 0,
                }),
                ..base
            },
            DescriptorError::BadMidiOutput,
        ),
        (
            DeviceDescriptor {
                named_keys: &[NamedKey {
                    key: 128,
                    name: "X",
                }],
                ..base
            },
            DescriptorError::BadNamedKeys,
        ),
        (
            DeviceDescriptor {
                named_keys: &[
                    NamedKey { key: 5, name: "A" },
                    NamedKey { key: 5, name: "B" },
                ],
                ..base
            },
            DescriptorError::BadNamedKeys,
        ),
        (
            DeviceDescriptor {
                named_keys: &[NamedKey { key: 5, name: " A" }],
                ..base
            },
            DescriptorError::BadNamedKeys,
        ),
        (
            DeviceDescriptor {
                named_keys: &[
                    NamedKey { key: 6, name: "A" },
                    NamedKey { key: 5, name: "B" },
                ],
                ..base
            },
            DescriptorError::BadNamedKeys,
        ),
        (
            DeviceDescriptor {
                named_keys: &[NamedKey { key: 5, name: "" }],
                ..base
            },
            DescriptorError::BadNamedKeys,
        ),
        (
            DeviceDescriptor {
                named_keys: &[NamedKey {
                    key: 5,
                    name: "Café",
                }],
                ..base
            },
            DescriptorError::BadNamedKeys,
        ),
        (
            DeviceDescriptor {
                role: DeviceRole::Effect,
                named_keys: &[NamedKey { key: 5, name: "A" }],
                ..base
            },
            DescriptorError::BadNamedKeys,
        ),
        (
            with(vec![base_param; MAX_PARAMS_PER_DEVICE + 1]),
            DescriptorError::TooManyParams,
        ),
        (
            with(vec![
                base_param,
                ParamDescriptor {
                    key: "other",
                    ..base_param
                },
            ]),
            DescriptorError::DuplicateParam(0),
        ),
        (
            with(vec![
                base_param,
                ParamDescriptor {
                    id: ParamId(1),
                    ..base_param
                },
            ]),
            DescriptorError::DuplicateParam(1),
        ),
        (
            one(ParamDescriptor {
                key: "Cutoff",
                ..base_param
            }),
            DescriptorError::BadKey(0),
        ),
        (
            one(ParamDescriptor {
                key: "cutoff.hz",
                ..base_param
            }),
            DescriptorError::BadKey(0),
        ),
        (
            one(ParamDescriptor {
                key: "",
                ..base_param
            }),
            DescriptorError::BadKey(0),
        ),
        (
            one(ParamDescriptor {
                description: "",
                ..base_param
            }),
            DescriptorError::BadDescription(0),
        ),
        (
            one(ParamDescriptor {
                description: " padded ",
                ..base_param
            }),
            DescriptorError::BadDescription(0),
        ),
        (
            one(ParamDescriptor {
                description: Box::leak("x".repeat(MAX_DESCRIPTION_LEN + 1).into_boxed_str()),
                ..base_param
            }),
            DescriptorError::BadDescription(0),
        ),
        (
            one(ParamDescriptor {
                min: 30_000.0,
                ..base_param
            }),
            DescriptorError::BadRange(0),
        ),
        (
            one(ParamDescriptor {
                max: f64::NAN,
                ..base_param
            }),
            DescriptorError::BadRange(0),
        ),
        (
            one(ParamDescriptor {
                min: 0.0,
                ..base_param
            }),
            DescriptorError::BadScale(0),
        ),
        (
            one(ParamDescriptor {
                step: -1.0,
                ..base_param
            }),
            DescriptorError::BadStep(0),
        ),
        (
            one(ParamDescriptor {
                step: 7.0,
                smoothing_seconds: 0.0,
                ..base_param
            }),
            DescriptorError::BadStep(0),
        ),
        (
            one(ParamDescriptor {
                default: 5.0,
                ..base_param
            }),
            DescriptorError::BadDefault(0),
        ),
        (
            one(ParamDescriptor {
                scale: Scale::Linear,
                min: 0.0,
                max: 10.0,
                default: 0.5,
                step: 1.0,
                smoothing_seconds: 0.0,
                ..base_param
            }),
            DescriptorError::BadDefault(0),
        ),
        (
            one(ParamDescriptor {
                smoothing_seconds: -0.1,
                ..base_param
            }),
            DescriptorError::BadSmoothing(0),
        ),
        (
            one(ParamDescriptor {
                smoothing_seconds: 11.0,
                ..base_param
            }),
            DescriptorError::BadSmoothing(0),
        ),
        (
            one(ParamDescriptor {
                scale: Scale::Linear,
                min: 0.0,
                max: 2.0,
                default: 0.0,
                step: 1.0,
                ..base_param
            }),
            DescriptorError::BadSmoothing(0),
        ),
        (
            one(ParamDescriptor {
                scale: Scale::Linear,
                min: 0.0,
                max: 2.0,
                default: 0.0,
                step: 1.0,
                smoothing_seconds: 0.0,
                values: &["a", "b"],
                ..base_param
            }),
            DescriptorError::BadValues(0),
        ),
        (
            one(ParamDescriptor {
                scale: Scale::Linear,
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step: 1.0,
                smoothing_seconds: 0.0,
                values: &["a", "B"],
                ..base_param
            }),
            DescriptorError::BadValues(0),
        ),
    ];
    for (i, (d, expected)) in cases.iter().enumerate() {
        assert_eq!(d.validate(), Err(*expected), "case {i}");
    }
}

#[test]
fn osc_synth_descriptor_matches_the_synth() {
    let synth = Synth::new(SR);
    let names = ["waveform", "attack", "decay", "sustain", "release", "gain"];
    assert_eq!(OSC_SYNTH.params.len(), names.len());
    for (p, name) in OSC_SYNTH.params.iter().zip(names) {
        let param = Param::from_id(p.id.0).expect("the id is a synth parameter");
        assert_eq!(p.key, name);
        assert_eq!(p.default as f32, synth.param(param), "default of {name}");
        // The range is the synth's clamp.
        let mut s = Synth::new(SR);
        s.set_param(param, (p.min - 1.0) as f32);
        assert_eq!(s.param(param), p.min as f32, "min of {name}");
        s.set_param(param, (p.max + 100.0) as f32);
        assert_eq!(s.param(param), p.max as f32, "max of {name}");
        // A stepped value is rounded to the step, as `sanitize` does.
        if p.step > 0.0 {
            for v in [0.4f32, 0.6, 1.49, 2.7] {
                s.set_param(param, v);
                assert_eq!(Some(s.param(param)), p.sanitize(v), "{name} at {v}");
            }
        }
    }
    assert_eq!(
        OSC_SYNTH.params[0].values,
        ["sine", "saw", "square", "triangle"]
    );
}

#[test]
fn sanitize_and_smoothing_frames() {
    let p = &GAIN.params[0];
    assert_eq!(p.sanitize(f32::NAN), None);
    assert_eq!(p.sanitize(f32::INFINITY), None);
    assert_eq!(p.sanitize(5.0), Some(2.0));
    assert_eq!(p.sanitize(-5.0), Some(0.0));
    assert_eq!(p.smoothing_frames(48_000.0), 480);
    assert_eq!(p.smoothing_frames(44_100.0), 441);
    assert_eq!(GAIN.params[1].smoothing_frames(48_000.0), 0);
    let semis = &TRANSPOSE.params[0];
    assert_eq!(semis.sanitize(2.4), Some(2.0));
    assert_eq!(semis.sanitize(-2.6), Some(-3.0));
}

#[test]
fn sanitize_in_f64_is_the_one_sanitize_widened_and_keeps_the_project_precision() {
    // The formula `sanitize` had before `sanitize_f64` (DV-8): `sanitize` must still give
    // exactly its result, for every parameter of every built-in device, at the bounds, the
    // default and values off the step.
    fn legacy(p: &ParamDescriptor, value: f32) -> Option<f32> {
        if !value.is_finite() {
            return None;
        }
        let mut v = f64::from(value).clamp(p.min, p.max);
        if p.step > 0.0 {
            v = (p.min + ((v - p.min) / p.step).round() * p.step).min(p.max);
        }
        Some(v as f32)
    }
    let mut checked = 0;
    for descriptor in builtin_descriptors().chain([&GAIN, &TRANSPOSE, &TONE]) {
        for p in descriptor.params {
            let span = (p.max - p.min) as f32;
            let mut values = vec![
                p.min as f32,
                p.max as f32,
                p.default as f32,
                p.min as f32 - 1.0,
                p.max as f32 + 1.0,
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::MAX,
                f32::MIN,
            ];
            for i in 0..=40 {
                values.push(p.min as f32 + span * (i as f32) / 37.0 - 0.013);
            }
            for v in values {
                assert_eq!(p.sanitize(v), legacy(p, v), "{} at {v}", p.key);
                assert_eq!(
                    p.sanitize_f64(f64::from(v)).map(|x| x as f32),
                    legacy(p, v),
                    "{} at {v}",
                    p.key
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 500, "{checked}");
    // In f64 a value keeps its precision: 0.1 is not widened from an f32.
    let attack = OSC_SYNTH.params.iter().find(|p| p.key == "attack").unwrap();
    assert_eq!(attack.sanitize_f64(0.1), Some(0.1));
    assert_eq!(attack.sanitize_f64(f64::NAN), None);
    assert_eq!(attack.sanitize_f64(1e300), Some(30.0));
    let waveform = &OSC_SYNTH.params[0];
    assert_eq!(waveform.sanitize_f64(2.6), Some(3.0));
}

// ---------------------------------------------------------------- events

#[test]
fn event_fits_in_24_bytes_and_voice_ids_have_24_bits() {
    assert!(size_of::<Event>() <= 24, "{} bytes", size_of::<Event>());
    assert_eq!(
        VoiceId::new(VoiceId::MAX).map(VoiceId::get),
        Some(0xFF_FFFF)
    );
    assert_eq!(VoiceId::new(VoiceId::MAX + 1), None);
}

#[test]
fn rank_puts_params_first_and_keeps_notes_in_issue_order() {
    let params = [pv(5, tone::GAIN, 1.0)];
    // Issue order: an off, an on, the same voice's off, a release.
    let notes = [
        off(5, 2),
        on(5, 1, 60, 0.0),
        off(5, 1),
        Event::AllNotesOff { offset: 5 },
    ];
    let mut merged = EventBuffer::try_with_capacity(16).unwrap();
    testing::merge(notes.iter(), params.iter(), &mut merged);
    assert_eq!(
        merged.events(),
        [params[0], notes[0], notes[1], notes[2], notes[3]]
    );
}

#[test]
fn a_note_on_and_off_of_one_voice_at_one_offset_never_sticks() {
    // A tap shorter than a block: both land at the block start, in issue order.
    let mut t = TestTone::new(&setup());
    let out = run(
        &mut t,
        128,
        2,
        &[(0, on(0, 1, 69, 0.0)), (0, off(0, 1))],
        0.0,
    );
    assert_eq!(t.active_voices(), 0);
    assert!(out.iter().all(|s| *s == 0.0));
    // The same through the whole chain (the MIDI effect passes them on in order).
    let script = [
        Scheduled {
            frame: 64,
            target: Target::Transpose,
            event: on(0, 1, 69, 0.0),
        },
        Scheduled {
            frame: 64,
            target: Target::Transpose,
            event: off(0, 1),
        },
    ];
    let mut chain = Chain::new(&setup()).unwrap();
    let mut l = vec![0.0f32; 1024];
    let mut r = vec![0.0f32; 1024];
    assert_no_alloc(|| chain.render(&script, &[128], &mut l, &mut r));
    assert_eq!(chain.tone.active_voices(), 0);
    assert!(l.iter().all(|s| *s == 0.0));
}

#[test]
fn a_release_after_a_note_on_at_one_offset_silences_it_and_before_does_not() {
    let release = Event::AllNotesOff { offset: 5 };
    let mut t = TestTone::new(&setup());
    let released = run(&mut t, 128, 2, &[(0, on(5, 1, 69, 0.0)), (0, release)], 0.0);
    assert!(released.iter().all(|s| *s == 0.0));
    let mut t = TestTone::new(&setup());
    let plays = run(&mut t, 128, 2, &[(0, release), (0, on(5, 1, 69, 0.0))], 0.0);
    assert!(plays[..5].iter().all(|s| *s == 0.0));
    assert_eq!(plays[5], 0.125, "a note-on issued after the release plays");
}

// ---------------------------------------------------------------- the instrument

#[test]
fn a_note_starts_and_stops_at_its_exact_offset() {
    for (frames, k, m) in [
        (64usize, 37usize, 1_000usize),
        (128, 0, 127),
        (512, 511, 513),
        (2048, 1_234, 3_000),
    ] {
        let mut t = TestTone::new(&setup());
        let blocks = 4_096usize.div_ceil(frames).max(2);
        let events = [
            (k / frames, on((k % frames) as u32, 1, 69, 0.0)),
            (m / frames, off((m % frames) as u32, 1)),
        ];
        let out = run(&mut t, frames, blocks, &events, 0.0);
        assert_clean(&out);
        assert!(out[..k].iter().all(|s| *s == 0.0), "silence before {k}");
        // cos(0) at the onset: velocity 1 × gain 0.5 × 0.25.
        assert_eq!(out[k], 0.125, "onset at {k} (block {frames})");
        assert!(out[m..].iter().all(|s| *s == 0.0), "silence from {m}");
        assert!(out[m - 1] != 0.0 || out[m - 2] != 0.0, "sound until {m}");
    }
}

#[test]
fn pitch_follows_key_and_fine_tuning() {
    for (cents, expected) in [
        (0.0f32, 440.0f32),
        (50.0, 440.0 * 2f32.powf(50.0 / 1200.0)),
        (-100.0, 415.3047),
    ] {
        let mut t = TestTone::new(&setup());
        let out = run(&mut t, 128, 375, &[(0, on(0, 1, 69, cents))], 0.0);
        let f = frequency(&out);
        assert!(
            (f - expected).abs() < 0.05,
            "{cents} cents: {f} Hz, expected {expected}"
        );
    }
}

#[test]
fn a_note_off_ends_the_note_of_its_voice_not_of_its_key() {
    let mut t = TestTone::new(&setup());
    // Two notes on one key, two voices: ending voice 1 leaves voice 2.
    let events = [
        (0, on(0, 1, 60, 0.0)),
        (0, on(0, 2, 60, 0.0)),
        (1, off(0, 1)),
    ];
    let _ = run(&mut t, 128, 2, &events, 0.0);
    assert_eq!(t.active_voices(), 1);
}

#[test]
fn a_stepped_parameter_changes_at_its_exact_offset() {
    // `octave` doubles the phase increment from frame k on: before k the render matches a
    // render without the event, from k it differs.
    let k = 300usize;
    let mut a = TestTone::new(&setup());
    let plain = run(&mut a, 128, 4, &[(0, on(0, 1, 69, 0.0))], 0.0);
    let mut b = TestTone::new(&setup());
    let events = [
        (0, on(0, 1, 69, 0.0)),
        (k / 128, pv((k % 128) as u32, tone::OCTAVE, 1.0)),
    ];
    let shifted = run(&mut b, 128, 4, &events, 0.0);
    // Frame k still plays the phase reached with the old increment; k + 1 differs.
    assert_eq!(plain[..=k], shifted[..=k]);
    assert_ne!(plain[k + 1], shifted[k + 1]);
}

// ---------------------------------------------------------------- the effect and smoothing

#[test]
fn a_smoothed_parameter_ramps_linearly_to_its_target_in_the_declared_time() {
    // DC input 1.0 through the gain: the output is the gain curve itself.
    let n = GAIN.params[0].smoothing_frames(SR) as usize; // 480
    for frames in [1usize, 64, 128, 2048] {
        let k = 1_000usize;
        let mut g = TestGain::new(&setup());
        let blocks = (k + 2 * n).div_ceil(frames) + 1;
        let out = run(
            &mut g,
            frames,
            blocks,
            &[(k / frames, pv((k % frames) as u32, gain::GAIN, 0.5))],
            1.0,
        );
        assert!(
            out[..=k].iter().all(|s| *s == 1.0),
            "unchanged up to the event's frame"
        );
        for i in 0..=n {
            let expected = 1.0 - 0.5 * i as f32 / n as f32;
            assert!(
                (out[k + i] - expected).abs() < 1e-6,
                "frame k+{i}: {}",
                out[k + i]
            );
        }
        assert_eq!(out[k + n], 0.5, "target reached exactly {n} frames later");
        assert!(out[k + n..].iter().all(|s| *s == 0.5));
    }
}

#[test]
fn a_new_value_during_a_ramp_starts_from_where_the_value_is() {
    let mut g = TestGain::new(&setup());
    let events = [(0, pv(0, gain::GAIN, 0.0)), (0, pv(240, gain::GAIN, 1.0))];
    let out = run(&mut g, 2048, 1, &events, 1.0);
    assert!((out[240] - 0.5).abs() < 1e-6, "mid-ramp value {}", out[240]);
    assert!((out[240 + 240] - 0.75).abs() < 1e-6);
    assert_eq!(out[240 + 480], 1.0);
}

#[test]
fn an_unsmoothed_parameter_applies_at_its_exact_offset() {
    let mut g = TestGain::new(&setup());
    let events = [(1, pv(17, gain::MUTE, 1.0)), (2, pv(100, gain::MUTE, 0.0))];
    let out = run(&mut g, 128, 3, &events, 1.0);
    assert_eq!(out[128 + 16], 1.0);
    assert!(out[128 + 17..256 + 100].iter().all(|s| *s == 0.0));
    assert_eq!(out[256 + 100], 1.0);
}

#[test]
fn set_param_ramps_from_the_next_block_and_reset_snaps() {
    let mut g = TestGain::new(&setup());
    let _ = run(&mut g, 128, 1, &[], 1.0);
    g.set_param(gain::GAIN, 0.0);
    let out = run(&mut g, 128, 1, &[], 1.0);
    assert_eq!(out[0], 1.0, "frame 0 of the next block is the ramp's start");
    assert!((out[1] - (1.0 - 1.0 / 480.0)).abs() < 1e-6);
    g.reset();
    let out = run(&mut g, 128, 1, &[], 1.0);
    assert!(out.iter().all(|s| *s == 0.0), "reset snaps to the target");
    // Out of range, NaN and unknown ids.
    g.set_param(gain::GAIN, f32::NAN);
    g.set_param(ParamId(77), 1.0);
    g.set_param(gain::GAIN, 9.0);
    g.reset();
    let out = run(&mut g, 128, 1, &[], 1.0);
    assert!(out.iter().all(|s| *s == 2.0), "clamped to the max");
}

// ---------------------------------------------------------------- the MIDI effect

fn transpose_block(t: &mut TestTranspose, events: &[Event], frames: usize) -> Vec<Event> {
    let mut out =
        EventBuffer::try_with_capacity(TRANSPOSE.midi_output.unwrap().capacity(events.len()))
            .unwrap();
    let mut l = vec![0.25f32; frames];
    let mut r = vec![0.25f32; frames];
    let c = ctx(frames);
    assert_no_alloc(|| {
        let mut io = AudioIo {
            left: &mut l,
            right: &mut r,
            events_out: &mut out,
        };
        t.process(&c, events, &mut io);
    });
    out.finish();
    assert_eq!(out.overflowed(), 0);
    assert!(
        l.iter().chain(&r).all(|s| *s == 0.25),
        "a MIDI effect leaves the audio untouched"
    );
    out.events().to_vec()
}

#[test]
fn the_midi_effect_transposes_and_tunes_notes_and_keeps_their_voices() {
    let mut t = TestTranspose::new(&setup());
    let input = [
        on(3, 1, 60, 10.0),
        pv(10, transpose::SEMITONES, 7.0),
        pv(10, transpose::CENTS, -30.0),
        on(10, 2, 60, 10.0),
        off(20, 1),
        on(30, 3, 125, 0.0),
        Event::AllNotesOff { offset: 40 },
    ];
    let out = transpose_block(&mut t, &input, 64);
    assert_eq!(
        out,
        [
            on(3, 1, 60, 10.0),
            on(10, 2, 67, -20.0),
            Event::NoteOff {
                offset: 20,
                voice: voice(1),
                key: 7 // `key` is informational: the input's 0, transposed
            },
            on(30, 3, 127, -30.0),
            Event::AllNotesOff { offset: 40 },
        ]
    );
}

#[test]
fn the_chain_plays_the_midi_effects_notes_through_the_instrument() {
    // An octave up through the MIDI effect and an octave down on the instrument: key 57
    // sounds at 220 Hz.
    let script = [
        Scheduled {
            frame: 0,
            target: Target::Transpose,
            event: pv(0, transpose::SEMITONES, 12.0),
        },
        Scheduled {
            frame: 0,
            target: Target::Transpose,
            event: on(0, 1, 57, 0.0),
        },
        Scheduled {
            frame: 0,
            target: Target::Tone,
            event: pv(0, tone::OCTAVE, -1.0),
        },
    ];
    let mut chain = Chain::new(&setup()).unwrap();
    let mut l = vec![0.0f32; 48_000];
    let mut r = vec![0.0f32; 48_000];
    let dropped = assert_no_alloc(|| chain.render(&script, &[128], &mut l, &mut r));
    assert_eq!(dropped, 0);
    assert_eq!(l[0], 0.125, "the note starts at frame 0");
    let f = frequency(&l);
    assert!((f - 220.0).abs() < 0.05, "{f} Hz");
}

// ---------------------------------------------------------------- the event buffer

#[test]
fn event_buffer_transitions() {
    // Capacity 1: only the reserved slot.
    let mut b = EventBuffer::try_with_capacity(0).unwrap();
    assert_eq!(b.capacity(), 1);
    assert!(!b.push(on(0, 1, 60, 0.0)));
    assert_eq!(b.overflowed(), 1);
    b.finish();
    assert_eq!(b.events(), [Event::AllNotesOff { offset: 0 }]);

    // Capacity 4: three pushes, the fourth refused; offsets kept sorted.
    let mut b = EventBuffer::try_with_capacity(4).unwrap();
    b.finish();
    assert!(b.is_empty(), "finish without overflow adds nothing");
    assert!(b.push(on(10, 1, 60, 0.0)));
    assert!(b.push(on(4, 2, 62, 0.0)), "an earlier offset is raised");
    assert_eq!(b.events()[1].offset(), 10);
    assert!(b.push(on(12, 3, 64, 0.0)));
    assert!(!b.push(off(13, 1)), "the reserved slot is not for pushes");
    assert_eq!(b.overflowed(), 1);
    b.finish();
    assert_eq!(b.len(), 4);
    assert_eq!(b.events()[3], Event::AllNotesOff { offset: 12 });
    // finish twice, push after finish.
    b.finish();
    assert_eq!(b.len(), 4);
    assert!(!b.push(off(20, 2)));
    assert_eq!(b.overflowed(), 2);
    b.finish();
    assert_eq!(b.len(), 4);
    // clear resets the content and the count.
    b.clear();
    assert!(b.is_empty());
    assert_eq!(b.overflowed(), 0);
    assert!(b.push(on(0, 1, 60, 0.0)));
}

#[test]
fn an_overflowed_output_releases_the_notes_it_started() {
    // The reserved release lands after the note-ons at its offset, also once merged.
    let mut out = EventBuffer::try_with_capacity(3).unwrap();
    assert!(out.push(on(7, 1, 60, 0.0)));
    assert!(out.push(on(7, 2, 64, 0.0)));
    assert!(!out.push(off(9, 1)));
    out.finish();
    let params = [pv(7, tone::GAIN, 1.0)];
    let mut merged = EventBuffer::try_with_capacity(16).unwrap();
    testing::merge(out.events().iter(), params.iter(), &mut merged);
    let mut t = TestTone::new(&setup());
    let events: Vec<(usize, Event)> = merged.events().iter().map(|e| (0, *e)).collect();
    let rendered = run(&mut t, 64, 2, &events, 0.0);
    assert_eq!(t.active_voices(), 0, "no note left stuck");
    assert!(rendered.iter().all(|s| *s == 0.0));
}

// ---------------------------------------------------------------- whole chain, rt safety

#[test]
fn the_render_does_not_depend_on_the_block_size_and_never_allocates() {
    let render = |blocks: &[usize]| {
        let mut chain = Chain::new(&setup()).unwrap();
        let mut l = vec![0.0f32; SCENARIO_FRAMES];
        let mut r = vec![0.0f32; SCENARIO_FRAMES];
        let dropped = assert_no_alloc(|| chain.render(&SCENARIO, blocks, &mut l, &mut r));
        assert_eq!(dropped, 0);
        assert_eq!(l, r);
        l
    };
    let reference = render(&SCENARIO_BLOCKS);
    assert_clean(&reference);
    assert!(
        reference.iter().any(|s| s.abs() > 0.1),
        "the scenario makes sound"
    );
    for blocks in [
        &[1usize][..],
        &[64],
        &[128],
        &[512],
        &[2048],
        &[3, 1000, 17],
    ] {
        assert_eq!(render(blocks), reference, "blocks {blocks:?}");
    }
    // Mute from 9999 to 10500 (exact offsets through the chain).
    assert!(reference[9_999..10_500].iter().all(|s| *s == 0.0));
    assert!(reference[9_998] != 0.0 && reference[10_500] != 0.0);
}

#[test]
fn several_seconds_of_events_never_allocate() {
    // The scenario looped over 5 s of audio at 128 frames, events every few hundred frames.
    let mut script = Vec::new();
    for rep in 0..10u64 {
        for s in SCENARIO {
            script.push(Scheduled {
                frame: s.frame + rep * SCENARIO_FRAMES as u64,
                ..s
            });
        }
    }
    let mut chain = Chain::new(&setup()).unwrap();
    let mut l = vec![0.0f32; 10 * SCENARIO_FRAMES];
    let mut r = vec![0.0f32; 10 * SCENARIO_FRAMES];
    let dropped = assert_no_alloc(|| chain.render(&script, &[128], &mut l, &mut r));
    assert_eq!(dropped, 0);
    assert_clean(&l);
}

#[test]
fn every_test_device_passes_the_conformance_run() {
    let factories: [(Factory, &DeviceDescriptor); 3] = [
        (TestTone::build, &TONE),
        (TestGain::build, &GAIN),
        (TestTranspose::build, &TRANSPOSE),
    ];
    for (factory, desc) in factories {
        for rate in [44_100.0f32, 48_000.0, 96_000.0, 1.0, f32::NAN] {
            let setup = DeviceSetup {
                sample_rate: rate,
                max_block_frames: CONFORMANCE_MAX_FRAMES,
            };
            let mut device = factory(&setup).expect("built");
            assert_eq!(device.descriptor().kind, desc.kind);
            let mut out =
                EventBuffer::try_with_capacity(desc.midi_output.unwrap_or_default().capacity(64))
                    .unwrap();
            for seed in 0..40u64 {
                let result = assert_no_alloc(|| {
                    testing::conformance(device.as_mut(), rate, seed, 50, &mut out)
                });
                assert_eq!(result, Ok(()), "{} at {rate} Hz, seed {seed}", desc.kind);
            }
        }
    }
}

#[test]
fn the_scenario_fills_its_output() {
    let mut out = vec![0.0f32; 2 * SCENARIO_FRAMES];
    assert_eq!(testing::scenario(&mut out), 2 * SCENARIO_FRAMES);
    assert_eq!(out[..SCENARIO_FRAMES], out[SCENARIO_FRAMES..]);
    assert_eq!(testing::scenario(&mut out[..10]), 0);
}
