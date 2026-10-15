//! The project document: serde round trip, refusal on read, the session kept apart, and
//! one failing case for each invariant of `validate()`.

use project::limits::*;
use project::*;
use std::collections::{BTreeMap, BTreeSet};

/// Ids of the fixture, kept to mutate it in the failing cases.
struct Fx {
    project: Project,
    scene: SceneId,
    midi: TrackId,
    audio: TrackId,
    synth: DeviceId,
    filter: DeviceId,
    player: DeviceId,
    midi_clip: ClipId,
    audio_clip: ClipId,
    lane: LaneId,
    placement: PlacementId,
}

fn hash(byte: u8) -> Hash {
    Hash([byte; 32])
}

fn tick(t: i64) -> Tick {
    Tick(t)
}

/// A project that uses every variant of the model.
fn fixture() -> Fx {
    let mut ids = IdGenerator::new(1_700_000_000_000, 42);
    let mut project = Project::new(ProjectId::generate(&mut ids), "Demo");
    project.transport.loop_region = LoopRegion::new(Tick(0), Tick(4 * 3840));
    project.transport.loop_enabled = true;
    project.transport.metronome = Metronome {
        enabled: true,
        gain_db: -6.0,
        count_in_bars: 2,
    };
    project.master.gain_db = -1.5;

    let scene = SceneId::generate(&mut ids);
    project.scenes.insert(
        scene.clone(),
        Scene {
            name: "Intro".into(),
            order: OrderKey::first(),
        },
    );
    let scene2 = SceneId::generate(&mut ids);
    project.scenes.insert(
        scene2,
        Scene {
            name: String::new(),
            order: OrderKey::between(Some(&OrderKey::first()), None).unwrap(),
        },
    );

    // MIDI track: synth + filter, a looping clip in the grid and in the arrangement,
    // one lane on the filter and one on the mixer.
    let midi = TrackId::generate(&mut ids);
    let mut track = Track::new(TrackKind::Midi, "Bass", OrderKey::first());
    track.color = Some(Rgb::new(255, 128, 0));
    track.mixer = Mixer {
        gain_db: f64::NEG_INFINITY,
        pan: -0.25,
        mute: true,
    };
    let synth = DeviceId::generate(&mut ids);
    track.devices.insert(
        synth.clone(),
        Device {
            order: OrderKey::first(),
            role: DeviceRole::Instrument,
            enabled: true,
            source: DeviceSource::Builtin {
                id: "faulk.osc_synth".into(),
                version: 1,
            },
            params: BTreeMap::from([("attack".into(), 0.01), ("waveform".into(), 2.0)]),
            state: Some(hash(1)),
        },
    );
    let filter = DeviceId::generate(&mut ids);
    track.devices.insert(
        filter.clone(),
        Device {
            order: OrderKey::between(Some(&OrderKey::first()), None).unwrap(),
            role: DeviceRole::Effect,
            enabled: false,
            source: DeviceSource::Module {
                name: "My filter".into(),
                hash: hash(2),
            },
            params: BTreeMap::from([("cutoff".into(), 800.0)]),
            state: None,
        },
    );
    let midi_clip = ClipId::generate(&mut ids);
    track.clips.insert(
        midi_clip.clone(),
        Clip {
            name: "Riff".into(),
            color: None,
            slot: Some(scene.clone()),
            launch_quantize: Some(Quantize::Beat),
            content: ClipContent::Midi {
                length: tick(3840),
                looping: Some(TickLoop {
                    start: tick(0),
                    end: tick(1920),
                }),
                notes: BTreeSet::from([
                    Note::new(tick(0), 36, tick(240), 110),
                    Note::new(tick(3600), 48, tick(240), 1),
                ]),
            },
        },
    );
    let placement = PlacementId::generate(&mut ids);
    track.placements.insert(
        placement.clone(),
        Placement {
            clip: midi_clip.clone(),
            start: tick(0),
            span: Span::Ticks {
                length: tick(7680),
                offset: tick(0),
            },
        },
    );
    let lane = LaneId::generate(&mut ids);
    track.automation.insert(
        lane.clone(),
        AutomationLane {
            target: AutomationTarget::Device {
                device: filter.clone(),
                param: "cutoff".into(),
            },
            points: BTreeMap::from([
                (
                    tick(0),
                    AutomationPoint {
                        value: 800.0,
                        shape: Shape::Linear,
                    },
                ),
                (
                    tick(3840),
                    AutomationPoint {
                        value: 2400.0,
                        shape: Shape::Step,
                    },
                ),
            ]),
        },
    );
    track.automation.insert(
        LaneId::generate(&mut ids),
        AutomationLane {
            target: AutomationTarget::Mixer {
                param: MixerParam::Pan,
            },
            points: BTreeMap::new(),
        },
    );
    project.tracks.insert(midi.clone(), track);

    // Audio track: the clip player, an audio clip and a frame placement.
    let audio = TrackId::generate(&mut ids);
    let mut track = Track::new(
        TrackKind::Audio,
        "Drums",
        OrderKey::between(Some(&OrderKey::first()), None).unwrap(),
    );
    let player = DeviceId::generate(&mut ids);
    track.devices.insert(
        player.clone(),
        Device {
            order: OrderKey::first(),
            role: DeviceRole::Instrument,
            enabled: true,
            source: DeviceSource::Builtin {
                id: CLIP_PLAYER.into(),
                version: 1,
            },
            params: BTreeMap::new(),
            state: None,
        },
    );
    let audio_clip = ClipId::generate(&mut ids);
    track.clips.insert(
        audio_clip.clone(),
        Clip {
            name: "Loop".into(),
            color: Some(Rgb::new(0, 0, 0)),
            slot: Some(scene.clone()),
            launch_quantize: None,
            content: ClipContent::Audio {
                media: AudioMedia {
                    hash: hash(3),
                    sample_rate: SampleRate::HZ_48000,
                    channels: 2,
                    frames: Frame(480_000),
                    name: "drums.wav".into(),
                },
                start: Frame(48_000),
                frames: Frame(96_000),
                gain_db: 0.0,
                looping: Some(FrameLoop {
                    start: Frame(0),
                    end: Frame(96_000),
                }),
            },
        },
    );
    track.placements.insert(
        PlacementId::generate(&mut ids),
        Placement {
            clip: audio_clip.clone(),
            start: tick(3840),
            span: Span::Frames {
                length: Frame(192_000),
                offset: Frame(0),
            },
        },
    );
    project.tracks.insert(audio.clone(), track);

    Fx {
        project,
        scene,
        midi,
        audio,
        synth,
        filter,
        player,
        midi_clip,
        audio_clip,
        lane,
        placement,
    }
}

#[test]
fn the_fixture_is_valid_and_round_trips() {
    let fx = fixture();
    assert_eq!(fx.project.validate(), Ok(()));
    let json = serde_json::to_string_pretty(&fx.project).unwrap();
    let back: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(back, fx.project);
    // Re-serializing gives the same bytes.
    assert_eq!(serde_json::to_string_pretty(&back).unwrap(), json);
    // -inf dB is null; the reserved per-clip quantization is written only when set.
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let midi = &value["tracks"][fx.midi.to_string()];
    assert_eq!(midi["mixer"]["gain_db"], serde_json::Value::Null);
    assert_eq!(
        midi["clips"][fx.midi_clip.to_string()]["launch_quantize"],
        "beat"
    );
    let audio = &value["tracks"][fx.audio.to_string()];
    assert!(
        audio["clips"][fx.audio_clip.to_string()]
            .get("launch_quantize")
            .is_none()
    );
    assert_eq!(value["transport"]["launch_quantize"], "bar");
}

#[test]
fn a_new_project_is_valid_with_the_defaults() {
    let p = Project::new(
        ProjectId::generate(&mut IdGenerator::for_tests()),
        "Untitled",
    );
    assert_eq!(p.validate(), Ok(()));
    assert_eq!(p.format, FORMAT_VERSION);
    assert_eq!(p.tempo().bpm(), 120.0);
    assert_eq!(p.time_signature(), TimeSignature::COMMON);
    assert_eq!(p.transport.launch_quantize, Quantize::Bar);
    assert!(p.transport.loop_region.is_none() && !p.transport.loop_enabled);
}

#[test]
fn reading_refuses_a_project_without_id_or_with_unknown_fields() {
    let fx = fixture();
    let mut value = serde_json::to_value(&fx.project).unwrap();
    value.as_object_mut().unwrap().remove("id");
    let err = serde_json::from_value::<Project>(value).unwrap_err();
    assert!(err.to_string().contains("missing field `id`"), "{err}");

    let mut value = serde_json::to_value(&fx.project).unwrap();
    value["solo"] = serde_json::json!([]);
    assert!(serde_json::from_value::<Project>(value).is_err());
}

/// Replaces `"<needle>"` by `"<needle>"` twice in a map: the text a bad line merge of
/// git could produce.
fn duplicate_entry(json: &str, key: &str) -> String {
    let start = json.find(&format!("\"{key}\":")).expect("key present");
    // The entry runs to the matching close of its value.
    let value_start = start + key.len() + 3;
    let bytes = json.as_bytes();
    let mut depth = 0i32;
    let mut end = value_start;
    for (i, &b) in bytes.iter().enumerate().skip(value_start) {
        match b {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                if depth == 0 {
                    end = i;
                    break;
                }
                depth -= 1;
            }
            b',' if depth == 0 => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let entry = &json[start..end];
    format!("{}{entry},{}", &json[..start], &json[start..])
}

#[test]
fn reading_refuses_duplicate_ids_and_two_points_on_one_tick() {
    let fx = fixture();
    let json = serde_json::to_string(&fx.project).unwrap();
    for key in [
        fx.midi.to_string(),
        fx.scene.to_string(),
        fx.synth.to_string(),
        fx.midi_clip.to_string(),
        fx.placement.to_string(),
        fx.lane.to_string(),
    ] {
        let bad = duplicate_entry(&json, &key);
        let err = serde_json::from_str::<Project>(&bad).unwrap_err();
        assert!(err.to_string().contains("duplicate key"), "{key}: {err}");
    }
    // Two points on tick 3840, written the same way or as two spellings of one number.
    let bad = duplicate_entry(&json, "3840");
    assert!(serde_json::from_str::<Project>(&bad).is_err());
    // Another spelling of tick 0 is refused too (as a duplicate or as a bad key), so a
    // second point cannot alias the first one.
    for alias in ["-0", "00", "+0", "0.0"] {
        let bad = json.replacen(
            "\"0\":{\"value\":800.0",
            &format!("\"0\":{{\"value\":800.0,\"shape\":\"step\"}},\"{alias}\":{{\"value\":800.0"),
            1,
        );
        assert_ne!(bad, json);
        assert!(serde_json::from_str::<Project>(&bad).is_err(), "{alias}");
    }
}

#[test]
fn writing_refuses_non_finite_numbers_and_writes_no_minus_zero() {
    let mut fx = fixture();
    fx.project.master.pan = -0.0;
    let json = serde_json::to_string(&fx.project).unwrap();
    assert!(json.contains("\"pan\":0.0"), "{json}");
    assert!(!json.contains("-0.0"));
    fx.project.master.gain_db = f64::NAN;
    assert!(serde_json::to_string(&fx.project).is_err());
    fx.project.master.gain_db = f64::INFINITY;
    assert!(serde_json::to_string(&fx.project).is_err());
}

#[test]
fn the_session_is_kept_apart_from_the_project() {
    let fx = fixture();
    let before = serde_json::to_string(&fx.project).unwrap();
    let value: serde_json::Value = serde_json::from_str(&before).unwrap();
    for key in ["solo", "selected_track", "midi_input", "session"] {
        assert!(value.get(key).is_none(), "{key} in the project");
    }
    let mut session = SessionState::default();
    session.solo.insert(fx.midi.clone());
    session.selected_track = Some(fx.audio.clone());
    session.midi_input = Some("Keystation 49".into());
    // The project's text does not move; the session has its own.
    assert_eq!(serde_json::to_string(&fx.project).unwrap(), before);
    let s = serde_json::to_string(&session).unwrap();
    assert_eq!(serde_json::from_str::<SessionState>(&s).unwrap(), session);

    // A session restored against a project without those tracks forgets them.
    let mut other = Project::new(fx.project.id.clone(), "Demo");
    other.tracks.clear();
    assert!(session.forget_missing_tracks(&other));
    assert!(session.solo.is_empty() && session.selected_track.is_none());
    assert_eq!(session.midi_input.as_deref(), Some("Keystation 49"));
    assert!(!session.forget_missing_tracks(&other));
    let mut kept = SessionState::default();
    kept.solo.insert(fx.midi.clone());
    assert!(!kept.forget_missing_tracks(&fx.project));
    assert_eq!(kept.solo.len(), 1);
}

#[test]
fn display_order_is_by_key_then_id() {
    let mut fx = fixture();
    // The fixture orders Bass (a0) then Drums (a1).
    let names: Vec<_> = fx
        .project
        .tracks_in_order()
        .iter()
        .map(|(_, t)| t.name.clone())
        .collect();
    assert_eq!(names, ["Bass", "Drums"]);
    // Same key on both (a line merge of two branches): the smaller id goes first.
    let key = OrderKey::first();
    for t in fx.project.tracks.values_mut() {
        t.order = key.clone();
    }
    assert_eq!(fx.project.validate(), Ok(()));
    let ids: Vec<_> = fx
        .project
        .tracks_in_order()
        .iter()
        .map(|(id, _)| (*id).clone())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    let chain: Vec<_> = fx.project.tracks[&fx.midi]
        .devices_in_order()
        .iter()
        .map(|(id, _)| (*id).clone())
        .collect();
    assert_eq!(chain, [fx.synth.clone(), fx.filter.clone()]);
    let _ = (fx.scene, fx.player);
}

// ---------------------------------------------------------------------------------------
// One failing case per invariant.
// ---------------------------------------------------------------------------------------

fn midi_track(fx: &mut Fx) -> &mut Track {
    fx.project.tracks.get_mut(&fx.midi).unwrap()
}

fn audio_track(fx: &mut Fx) -> &mut Track {
    fx.project.tracks.get_mut(&fx.audio).unwrap()
}

fn midi_content(fx: &mut Fx) -> &mut ClipContent {
    let id = fx.midi_clip.clone();
    &mut midi_track(fx).clips.get_mut(&id).unwrap().content
}

fn audio_content(fx: &mut Fx) -> &mut ClipContent {
    let id = fx.audio_clip.clone();
    &mut audio_track(fx).clips.get_mut(&id).unwrap().content
}

fn notes(fx: &mut Fx) -> &mut BTreeSet<Note> {
    match midi_content(fx) {
        ClipContent::Midi { notes, .. } => notes,
        _ => unreachable!(),
    }
}

fn note(start: i64, pitch: u8, length: i64, velocity: u8) -> Note {
    Note::new(Tick(start), pitch, Tick(length), velocity)
}

type Mutation = Box<dyn Fn(&mut Fx)>;

/// Every kind of violation, by name: the `match` has no wildcard, so a new kind does not
/// compile until it is named here, and `every_kind_has_a_failing_case` then requires a
/// case for it.
fn kind_name(kind: &ViolationKind) -> &'static str {
    use ViolationKind::*;
    match kind {
        Format(_) => "Format",
        EmptyName => "EmptyName",
        UntrimmedName => "UntrimmedName",
        NameTooLong => "NameTooLong",
        DuplicateId(_) => "DuplicateId",
        UnknownScene => "UnknownScene",
        SlotTaken => "SlotTaken",
        UnknownClip => "UnknownClip",
        UnknownDevice => "UnknownDevice",
        KindMismatch => "KindMismatch",
        BadKey => "BadKey",
        NoteOutsideClip => "NoteOutsideClip",
        BadNote => "BadNote",
        BadLength => "BadLength",
        BadRange => "BadRange",
        BadLoop => "BadLoop",
        LoopWithoutRegion => "LoopWithoutRegion",
        BadPosition => "BadPosition",
        TempoMap => "TempoMap",
        MeterMap => "MeterMap",
        NotFinite => "NotFinite",
        PanOutOfRange => "PanOutOfRange",
        TooManyInstruments => "TooManyInstruments",
        InstrumentNotFirst => "InstrumentNotFirst",
        WrongInstrument => "WrongInstrument",
        BadChannels => "BadChannels",
        CountIn => "CountIn",
        MissingInstrument => "MissingInstrument",
        BadSampleRate => "BadSampleRate",
        TooMany { .. } => "TooMany",
    }
}

const ALL_KINDS: [&str; 30] = [
    "MissingInstrument",
    "BadSampleRate",
    "Format",
    "EmptyName",
    "UntrimmedName",
    "NameTooLong",
    "DuplicateId",
    "UnknownScene",
    "SlotTaken",
    "UnknownClip",
    "UnknownDevice",
    "KindMismatch",
    "BadKey",
    "NoteOutsideClip",
    "BadNote",
    "BadLength",
    "BadRange",
    "BadLoop",
    "LoopWithoutRegion",
    "BadPosition",
    "TempoMap",
    "MeterMap",
    "NotFinite",
    "PanOutOfRange",
    "TooManyInstruments",
    "InstrumentNotFirst",
    "WrongInstrument",
    "BadChannels",
    "CountIn",
    "TooMany",
];

/// (description, mutation, expected kind name).
fn cases() -> Vec<(&'static str, Mutation, &'static str)> {
    let midi_note = |n: Note| -> Mutation {
        Box::new(move |fx: &mut Fx| {
            notes(fx).insert(n);
        })
    };
    vec![
        ("format 2", Box::new(|fx| fx.project.format = 2), "Format"),
        (
            "empty project name",
            Box::new(|fx| fx.project.name.clear()),
            "EmptyName",
        ),
        (
            "untrimmed track name",
            Box::new(|fx| midi_track(fx).name = " Bass".into()),
            "UntrimmedName",
        ),
        (
            "long scene name",
            Box::new(|fx| {
                let s = fx.scene.clone();
                fx.project.scenes.get_mut(&s).unwrap().name = "x".repeat(MAX_NAME_CHARS + 1);
            }),
            "NameTooLong",
        ),
        (
            "module with empty name",
            Box::new(|fx| {
                let f = fx.filter.clone();
                midi_track(fx).devices.get_mut(&f).unwrap().source = DeviceSource::Module {
                    name: String::new(),
                    hash: hash(9),
                };
            }),
            "EmptyName",
        ),
        (
            "track id reused as a scene id",
            Box::new(|fx| {
                let id = SceneId(fx.midi.0.clone());
                fx.project.scenes.insert(
                    id,
                    Scene {
                        name: "x".into(),
                        order: OrderKey::first(),
                    },
                );
            }),
            "DuplicateId",
        ),
        (
            "same clip id in two tracks",
            Box::new(|fx| {
                let id = fx.midi_clip.clone();
                let mut clip = midi_track(fx).clips[&id].clone();
                clip.slot = None;
                clip.content = audio_content(fx).clone();
                audio_track(fx).clips.insert(id, clip);
            }),
            "DuplicateId",
        ),
        (
            "project id reused as a lane id",
            Box::new(|fx| {
                let l = fx.lane.clone();
                let lane = midi_track(fx).automation.remove(&l).unwrap();
                let id = LaneId(fx.project.id.0.clone());
                midi_track(fx).automation.insert(id, lane);
            }),
            "DuplicateId",
        ),
        (
            "slot in a missing scene",
            Box::new(|fx| {
                let s = fx.scene.clone();
                fx.project.scenes.remove(&s);
            }),
            "UnknownScene",
        ),
        (
            "two clips in one slot",
            Box::new(|fx| {
                let id = fx.midi_clip.clone();
                let clip = midi_track(fx).clips[&id].clone();
                midi_track(fx)
                    .clips
                    .insert(ClipId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(), clip);
            }),
            "SlotTaken",
        ),
        (
            "placement of a missing clip",
            Box::new(|fx| {
                let p = fx.placement.clone();
                midi_track(fx).placements.get_mut(&p).unwrap().clip =
                    ClipId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
            }),
            "UnknownClip",
        ),
        (
            "placement of another track's clip",
            Box::new(|fx| {
                let p = fx.placement.clone();
                let other = fx.audio_clip.clone();
                midi_track(fx).placements.get_mut(&p).unwrap().clip = other;
            }),
            "UnknownClip",
        ),
        (
            "lane on a missing device",
            Box::new(|fx| {
                let f = fx.filter.clone();
                midi_track(fx).devices.remove(&f);
            }),
            "UnknownDevice",
        ),
        (
            "audio clip in a MIDI track",
            Box::new(|fx| {
                let c = audio_content(fx).clone();
                *midi_content(fx) = c;
                let p = fx.placement.clone();
                midi_track(fx).placements.remove(&p);
            }),
            "KindMismatch",
        ),
        (
            "frame span on a MIDI clip",
            Box::new(|fx| {
                let p = fx.placement.clone();
                midi_track(fx).placements.get_mut(&p).unwrap().span = Span::Frames {
                    length: Frame(10),
                    offset: Frame(0),
                };
            }),
            "KindMismatch",
        ),
        (
            "bad param id",
            Box::new(|fx| {
                let s = fx.synth.clone();
                midi_track(fx)
                    .devices
                    .get_mut(&s)
                    .unwrap()
                    .params
                    .insert("Cut Off".into(), 1.0);
            }),
            "BadKey",
        ),
        (
            "bad builtin id",
            Box::new(|fx| {
                let s = fx.synth.clone();
                midi_track(fx).devices.get_mut(&s).unwrap().source = DeviceSource::Builtin {
                    id: String::new(),
                    version: 1,
                };
            }),
            "BadKey",
        ),
        (
            "bad lane param",
            Box::new(|fx| {
                let l = fx.lane.clone();
                let f = fx.filter.clone();
                midi_track(fx).automation.get_mut(&l).unwrap().target = AutomationTarget::Device {
                    device: f,
                    param: "x y".into(),
                };
            }),
            "BadKey",
        ),
        (
            "note past the clip end",
            midi_note(note(3700, 60, 241, 100)),
            "NoteOutsideClip",
        ),
        (
            "note before the clip start",
            midi_note(note(-1, 60, 10, 100)),
            "NoteOutsideClip",
        ),
        ("pitch 128", midi_note(note(0, 128, 10, 100)), "BadNote"),
        ("velocity 0", midi_note(note(0, 60, 10, 0)), "BadNote"),
        ("velocity 128", midi_note(note(0, 60, 10, 128)), "BadNote"),
        ("note length 0", midi_note(note(0, 60, 0, 100)), "BadNote"),
        (
            "cents 51",
            midi_note(Note {
                cents: 51,
                ..note(0, 60, 10, 100)
            }),
            "BadNote",
        ),
        (
            "cents -51",
            midi_note(Note {
                cents: -51,
                ..note(0, 60, 10, 100)
            }),
            "BadNote",
        ),
        (
            "chance 101",
            midi_note(Note {
                chance: 101,
                ..note(0, 60, 10, 100)
            }),
            "BadNote",
        ),
        (
            "repeat 0",
            midi_note(Note {
                repeat: 0,
                ..note(0, 60, 10, 100)
            }),
            "BadNote",
        ),
        (
            "repeat 17",
            midi_note(Note {
                repeat: 17,
                ..note(0, 60, 10, 100)
            }),
            "BadNote",
        ),
        (
            "clip length 0",
            Box::new(|fx| {
                if let ClipContent::Midi {
                    length,
                    looping,
                    notes,
                } = midi_content(fx)
                {
                    *length = Tick(0);
                    *looping = None;
                    notes.clear();
                }
            }),
            "BadLength",
        ),
        (
            "clip length past the limit",
            Box::new(|fx| {
                if let ClipContent::Midi { length, .. } = midi_content(fx) {
                    *length = Tick(MAX_TICK + 1);
                }
            }),
            "BadLength",
        ),
        (
            "audio range of 0 frames",
            Box::new(|fx| {
                if let ClipContent::Audio {
                    frames, looping, ..
                } = audio_content(fx)
                {
                    *frames = Frame(0);
                    *looping = None;
                }
            }),
            "BadLength",
        ),
        (
            "audio range past the file",
            Box::new(|fx| {
                if let ClipContent::Audio { start, .. } = audio_content(fx) {
                    *start = Frame(400_000);
                }
            }),
            "BadRange",
        ),
        (
            "placement offset past the clip",
            Box::new(|fx| {
                let p = fx.placement.clone();
                midi_track(fx).placements.get_mut(&p).unwrap().span = Span::Ticks {
                    length: Tick(10),
                    offset: Tick(3840),
                };
            }),
            "BadRange",
        ),
        (
            "placement length 0",
            Box::new(|fx| {
                let p = fx.placement.clone();
                midi_track(fx).placements.get_mut(&p).unwrap().span = Span::Ticks {
                    length: Tick(0),
                    offset: Tick(0),
                };
            }),
            "BadLength",
        ),
        (
            "empty MIDI loop",
            Box::new(|fx| {
                if let ClipContent::Midi { looping, .. } = midi_content(fx) {
                    *looping = Some(TickLoop {
                        start: Tick(10),
                        end: Tick(10),
                    });
                }
            }),
            "BadLoop",
        ),
        (
            "MIDI loop past the clip",
            Box::new(|fx| {
                if let ClipContent::Midi { looping, .. } = midi_content(fx) {
                    *looping = Some(TickLoop {
                        start: Tick(0),
                        end: Tick(3841),
                    });
                }
            }),
            "BadLoop",
        ),
        (
            "audio loop past the range",
            Box::new(|fx| {
                if let ClipContent::Audio { looping, .. } = audio_content(fx) {
                    *looping = Some(FrameLoop {
                        start: Frame(0),
                        end: Frame(96_001),
                    });
                }
            }),
            "BadLoop",
        ),
        (
            "loop enabled without region",
            Box::new(|fx| fx.project.transport.loop_region = None),
            "LoopWithoutRegion",
        ),
        (
            "negative loop region",
            Box::new(|fx| fx.project.transport.loop_region = LoopRegion::new(Tick(-10), Tick(10))),
            "BadPosition",
        ),
        (
            "negative placement start",
            Box::new(|fx| {
                let p = fx.placement.clone();
                midi_track(fx).placements.get_mut(&p).unwrap().start = Tick(-1);
            }),
            "BadPosition",
        ),
        (
            "placement ending past the limit",
            Box::new(|fx| {
                let p = fx.placement.clone();
                midi_track(fx).placements.get_mut(&p).unwrap().start = Tick(MAX_TICK);
            }),
            "BadPosition",
        ),
        (
            "automation point past the limit",
            Box::new(|fx| {
                let l = fx.lane.clone();
                midi_track(fx)
                    .automation
                    .get_mut(&l)
                    .unwrap()
                    .points
                    .insert(
                        Tick(MAX_TICK + 1),
                        AutomationPoint {
                            value: 1.0,
                            shape: Shape::Step,
                        },
                    );
            }),
            "BadPosition",
        ),
        (
            "two tempo points",
            Box::new(|fx| {
                let p = TempoPoint {
                    tick: Tick(3840),
                    bpm: Tempo::clamped(90.0),
                };
                fx.project.tempo.push(p);
            }),
            "TempoMap",
        ),
        (
            "no tempo point",
            Box::new(|fx| fx.project.tempo.clear()),
            "TempoMap",
        ),
        (
            "tempo not at 0",
            Box::new(|fx| fx.project.tempo[0].tick = Tick(1)),
            "TempoMap",
        ),
        (
            "meter not at 0",
            Box::new(|fx| fx.project.meter[0].tick = Tick(1)),
            "MeterMap",
        ),
        (
            "NaN gain",
            Box::new(|fx| fx.project.master.gain_db = f64::NAN),
            "NotFinite",
        ),
        (
            "+inf clip gain",
            Box::new(|fx| {
                if let ClipContent::Audio { gain_db, .. } = audio_content(fx) {
                    *gain_db = f64::INFINITY;
                }
            }),
            "NotFinite",
        ),
        (
            "NaN param",
            Box::new(|fx| {
                let s = fx.synth.clone();
                midi_track(fx)
                    .devices
                    .get_mut(&s)
                    .unwrap()
                    .params
                    .insert("attack".into(), f64::NAN);
            }),
            "NotFinite",
        ),
        (
            "-inf automation value",
            Box::new(|fx| {
                let l = fx.lane.clone();
                midi_track(fx)
                    .automation
                    .get_mut(&l)
                    .unwrap()
                    .points
                    .insert(
                        Tick(1),
                        AutomationPoint {
                            value: f64::NEG_INFINITY,
                            shape: Shape::Linear,
                        },
                    );
            }),
            "NotFinite",
        ),
        (
            "NaN pan",
            Box::new(|fx| midi_track(fx).mixer.pan = f64::NAN),
            "NotFinite",
        ),
        (
            "pan 1.5",
            Box::new(|fx| midi_track(fx).mixer.pan = 1.5),
            "PanOutOfRange",
        ),
        (
            "audio track without the clip player",
            Box::new(|fx| {
                let p = fx.player.clone();
                audio_track(fx).devices.remove(&p);
            }),
            "MissingInstrument",
        ),
        (
            "audio file at 4 kHz",
            Box::new(|fx| {
                if let ClipContent::Audio { media, .. } = audio_content(fx) {
                    media.sample_rate = SampleRate::new(4_000).unwrap();
                }
            }),
            "BadSampleRate",
        ),
        (
            "audio placement ending past the limit",
            Box::new(|fx| {
                for p in audio_track(fx).placements.values_mut() {
                    p.start = Tick(MAX_TICK - 1);
                }
            }),
            "BadPosition",
        ),
        (
            "pan automation point at -2",
            Box::new(|fx| {
                let lane = AutomationLane {
                    target: AutomationTarget::Mixer {
                        param: MixerParam::Pan,
                    },
                    points: BTreeMap::from([(
                        Tick(0),
                        AutomationPoint {
                            value: -2.0,
                            shape: Shape::Step,
                        },
                    )]),
                };
                midi_track(fx)
                    .automation
                    .insert(LaneId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(), lane);
            }),
            "PanOutOfRange",
        ),
        (
            "two instruments",
            Box::new(|fx| {
                let f = fx.filter.clone();
                midi_track(fx).devices.get_mut(&f).unwrap().role = DeviceRole::Instrument;
            }),
            "TooManyInstruments",
        ),
        (
            "instrument after an effect",
            Box::new(|fx| {
                let s = fx.synth.clone();
                midi_track(fx).devices.get_mut(&s).unwrap().order = OrderKey::parse("a5").unwrap();
            }),
            "InstrumentNotFirst",
        ),
        (
            "clip player on a MIDI track",
            Box::new(|fx| {
                let s = fx.synth.clone();
                midi_track(fx).devices.get_mut(&s).unwrap().source = DeviceSource::Builtin {
                    id: CLIP_PLAYER.into(),
                    version: 1,
                };
            }),
            "WrongInstrument",
        ),
        (
            "synth on an audio track",
            Box::new(|fx| {
                let p = fx.player.clone();
                audio_track(fx).devices.get_mut(&p).unwrap().source = DeviceSource::Builtin {
                    id: "faulk.osc_synth".into(),
                    version: 1,
                };
            }),
            "WrongInstrument",
        ),
        (
            "clip player as an effect",
            Box::new(|fx| {
                let p = fx.player.clone();
                audio_track(fx).devices.get_mut(&p).unwrap().role = DeviceRole::Effect;
            }),
            "WrongInstrument",
        ),
        (
            "audio file with 0 channels",
            Box::new(|fx| {
                if let ClipContent::Audio { media, .. } = audio_content(fx) {
                    media.channels = 0;
                }
            }),
            "BadChannels",
        ),
        (
            "audio file with 65 channels",
            Box::new(|fx| {
                if let ClipContent::Audio { media, .. } = audio_content(fx) {
                    media.channels = MAX_CHANNELS + 1;
                }
            }),
            "BadChannels",
        ),
        (
            "count-in of 17 bars",
            Box::new(|fx| fx.project.transport.metronome.count_in_bars = MAX_COUNT_IN_BARS + 1),
            "CountIn",
        ),
        (
            "too many scenes",
            Box::new(|fx| {
                let mut ids = IdGenerator::new(1_800_000_000_000, 0);
                for _ in 0..MAX_SCENES {
                    fx.project.scenes.insert(
                        SceneId::generate(&mut ids),
                        Scene {
                            name: String::new(),
                            order: OrderKey::first(),
                        },
                    );
                }
            }),
            "TooMany",
        ),
        (
            "too many devices",
            Box::new(|fx| {
                let mut ids = IdGenerator::new(1_800_000_000_000, 0);
                let f = fx.filter.clone();
                let effect = midi_track(fx).devices[&f].clone();
                for _ in 0..MAX_DEVICES_PER_TRACK {
                    midi_track(fx)
                        .devices
                        .insert(DeviceId::generate(&mut ids), effect.clone());
                }
            }),
            "TooMany",
        ),
    ]
}

#[test]
fn each_invariant_has_a_failing_case() {
    for (what, mutate, expected) in cases() {
        let mut fx = fixture();
        mutate(&mut fx);
        let violations = fx.project.validate().expect_err(what);
        assert!(
            violations.iter().any(|v| kind_name(&v.kind) == expected),
            "{what}: expected {expected}, got {violations:?}"
        );
    }
}

#[test]
fn every_kind_has_a_failing_case() {
    let covered: BTreeSet<_> = cases().iter().map(|(_, _, k)| *k).collect();
    let all: BTreeSet<_> = ALL_KINDS.into_iter().collect();
    assert_eq!(covered, all);
}

#[test]
fn violations_carry_their_path_and_are_all_reported() {
    let mut fx = fixture();
    fx.project.format = 3;
    midi_track(&mut fx).mixer.pan = 2.0;
    let violations = fx.project.validate().unwrap_err();
    assert_eq!(violations.len(), 2, "{violations:?}");
    assert_eq!(violations[0].path, "format");
    assert_eq!(violations[1].path, format!("tracks/{}", fx.midi));
    assert_eq!(violations[1].kind, ViolationKind::PanOutOfRange);
}

#[test]
fn the_limits_hold_the_load_bench() {
    // QA-7: 100 tracks, 200 000 notes, 6 million automation points in a project.
    const { assert!(MAX_TRACKS >= 100) };
    const { assert!(MAX_NOTES >= 200_000 && MAX_NOTES_PER_CLIP >= 200_000) };
    const { assert!(MAX_POINTS >= 6_000_000 && MAX_POINTS_PER_LANE >= 6_000_000) };
    const { assert!(MAX_NOTES_PER_CLIP <= MAX_NOTES && MAX_POINTS_PER_LANE <= MAX_POINTS) };
}

#[test]
fn reading_refuses_a_duplicate_note() {
    let fx = fixture();
    let json = serde_json::to_string(&fx.project).unwrap();
    let note = r#"{"start":0,"pitch":36,"length":240,"velocity":110}"#;
    assert!(json.contains(note));
    let bad = json.replacen(note, &format!("{note},{note}"), 1);
    let err = serde_json::from_str::<Project>(&bad).unwrap_err();
    assert!(err.to_string().contains("duplicate value"), "{err}");
}

#[test]
fn the_format_is_read_before_the_strict_document() {
    // A newer format with a field this build does not know: the strict read fails, the
    // probe still reads the version, so the reader can say "newer format".
    let fx = fixture();
    let mut value = serde_json::to_value(&fx.project).unwrap();
    value["format"] = 2.into();
    value["tempo_curve"] = serde_json::json!([]);
    assert!(serde_json::from_value::<Project>(value.clone()).is_err());
    let probe: FormatProbe = serde_json::from_value(value).unwrap();
    assert_eq!(probe, FormatProbe { format: 2 });
}

#[test]
fn every_count_limit_has_a_failing_case() {
    // The fixture: 2 tracks, 2 scenes; the MIDI track has 2 devices (the synth with 2
    // parameters), 1 clip of 2 notes, 1 placement, 2 lanes (2 points on one); 2 notes and
    // 2 points in the whole project. Each limit set one below is refused at its path.
    let fx = fixture();
    let s = Limits::SAFETY;
    let midi = format!("tracks/{}", fx.midi);
    let cases: [(&str, Limits, String); 14] = [
        ("tracks", Limits { tracks: 1, ..s }, "tracks".into()),
        ("scenes", Limits { scenes: 1, ..s }, "scenes".into()),
        (
            "devices",
            Limits {
                devices_per_track: 1,
                ..s
            },
            midi.clone(),
        ),
        (
            "params",
            Limits {
                params_per_device: 1,
                ..s
            },
            format!("{midi}/devices/{}", fx.synth),
        ),
        (
            "clips",
            Limits {
                clips_per_track: 0,
                ..s
            },
            midi.clone(),
        ),
        (
            "placements",
            Limits {
                placements_per_track: 0,
                ..s
            },
            midi.clone(),
        ),
        (
            "lanes",
            Limits {
                lanes_per_track: 1,
                ..s
            },
            midi.clone(),
        ),
        (
            "notes per clip",
            Limits {
                notes_per_clip: 1,
                ..s
            },
            format!("{midi}/clips/{}", fx.midi_clip),
        ),
        (
            "points per lane",
            Limits {
                points_per_lane: 1,
                ..s
            },
            format!("{midi}/automation/{}", fx.lane),
        ),
        ("notes", Limits { notes: 1, ..s }, "tracks".into()),
        ("points", Limits { points: 1, ..s }, "tracks".into()),
        ("clips in the project", Limits { clips: 0, ..s }, "tracks".into()),
        (
            "placements in the project",
            Limits { placements: 0, ..s },
            "tracks".into(),
        ),
        ("lanes in the project", Limits { lanes: 1, ..s }, "tracks".into()),
    ];
    assert_eq!(fx.project.validate_with(&s), Ok(()));
    for (what, limits, path) in cases {
        let violations = fx.project.validate_with(&limits).expect_err(what);
        assert!(
            violations
                .iter()
                .any(|v| v.path == path && matches!(v.kind, ViolationKind::TooMany { .. })),
            "{what}: {violations:?}"
        );
    }
}

#[test]
fn a_default_note_writes_no_optional_field_and_a_file_without_them_reads_as_defaults() {
    let plain = note(480, 60, 240, 100);
    let json = serde_json::to_value(plain).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"start": 480, "pitch": 60, "length": 240, "velocity": 100})
    );
    assert_eq!(serde_json::from_value::<Note>(json).unwrap(), plain);
    assert_eq!(plain, Note::new(Tick(480), 60, Tick(240), 100));
    assert_eq!(
        (plain.cents, plain.chance, plain.repeat),
        (0, Note::DEFAULT_CHANCE, Note::DEFAULT_REPEAT)
    );
}

#[test]
fn the_optional_note_fields_round_trip_and_only_the_non_default_ones_are_written() {
    let tuned = Note {
        cents: -50,
        ..note(0, 60, 10, 100)
    };
    let json = serde_json::to_value(tuned).unwrap();
    assert_eq!(json["cents"], -50);
    assert!(json.get("chance").is_none() && json.get("repeat").is_none());
    let full = Note {
        cents: 50,
        chance: 0,
        repeat: 16,
        ..note(0, 60, 10, 100)
    };
    let json = serde_json::to_string(&full).unwrap();
    assert!(
        json.contains(r#""cents":50,"chance":0,"repeat":16"#),
        "{json}"
    );
    assert_eq!(serde_json::from_str::<Note>(&json).unwrap(), full);
    // Spelling the defaults out reads as the same note, written without them.
    let spelled: Note = serde_json::from_str(
        r#"{"start":0,"pitch":60,"length":10,"velocity":100,"cents":0,"chance":100,"repeat":1}"#,
    )
    .unwrap();
    assert_eq!(spelled, note(0, 60, 10, 100));
    // An unknown field is still refused, and a value out of the type is refused on read.
    assert!(
        serde_json::from_str::<Note>(r#"{"start":0,"pitch":60,"length":10,"velocity":100,"x":1}"#)
            .is_err()
    );
    assert!(
        serde_json::from_str::<Note>(
            r#"{"start":0,"pitch":60,"length":10,"velocity":100,"cents":300}"#
        )
        .is_err()
    );
}

#[test]
fn notes_differing_only_by_an_optional_field_are_two_notes() {
    let a = note(0, 60, 10, 100);
    let b = Note { chance: 50, ..a };
    let c = Note { repeat: 2, ..a };
    let d = Note { cents: 10, ..a };
    let set: BTreeSet<Note> = [a, b, c, d, a].into_iter().collect();
    assert_eq!(set.len(), 4);
}

#[test]
fn contains_id_finds_every_kind_of_entity_and_nothing_else() {
    let fx = fixture();
    let p = &fx.project;
    let mut every = vec![p.id.0.clone()];
    every.extend(p.scenes.keys().map(|k| k.0.clone()));
    for (id, t) in &p.tracks {
        every.push(id.0.clone());
        every.extend(t.devices.keys().map(|k| k.0.clone()));
        every.extend(t.clips.keys().map(|k| k.0.clone()));
        every.extend(t.placements.keys().map(|k| k.0.clone()));
        every.extend(t.automation.keys().map(|k| k.0.clone()));
    }
    // The fixture uses every kind of entity, each at least once.
    assert!(every.len() >= 14, "{}", every.len());
    for id in &every {
        assert!(p.contains_id(id), "{id}");
    }
    let mut ids = IdGenerator::new(1_800_000_000_000, 7);
    assert!(!p.contains_id(&ClipId::generate(&mut ids).0));
}
