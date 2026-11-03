//! The reference projects of the folder format: built here with fixed ids, and committed as
//! folders under `tests/reference/` (bless them with `FAULK_BLESS=1 cargo test -p project
//! --test it folder_reference::`). Their layout is the one of `Layout::REFERENCE`.

use project::file::*;
use project::*;
use std::collections::{BTreeMap, BTreeSet};

/// Lanes of 5 points or more are blobs, lanes of 2 or fewer text: the thresholds of the
/// real format scaled down so that a reference folder holds both forms.
pub const LAYOUT: Layout = Layout {
    blob_above: 4,
    text_below: 2,
};

pub fn id(n: u8) -> project::ids::Id {
    // ULIDs of fixed digits: "01HF" + 21 zeros + one Crockford digit.
    let digit = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ"[usize::from(n) % 32] as char;
    project::ids::Id::parse(&format!("01HF{}{digit}", "0".repeat(21))).unwrap()
}

fn point(value: f64, shape: Shape) -> AutomationPoint {
    AutomationPoint { value, shape }
}

fn note(start: i64, pitch: u8, length: i64, velocity: u8) -> Note {
    Note::new(Tick(start), pitch, Tick(length), velocity)
}

pub fn minimal() -> Project {
    Project::new(ProjectId(id(1)), "Minimal")
}

pub fn song() -> Project {
    let mut p = Project::new(ProjectId(id(1)), "Song \"one\"");
    p.transport.loop_region = LoopRegion::new(Tick(0), Tick(15360));
    p.transport.loop_enabled = true;
    p.transport.metronome.enabled = true;
    p.transport.metronome.gain_db = -6.0;
    p.master.gain_db = -1.5;
    let scene_a = SceneId(id(2));
    let scene_b = SceneId(id(3));
    p.scenes.insert(
        scene_a.clone(),
        Scene {
            name: "Intro".into(),
            order: OrderKey::first(),
        },
    );
    p.scenes.insert(
        scene_b.clone(),
        Scene {
            name: "Verse".into(),
            order: OrderKey::between(Some(&OrderKey::first()), None).unwrap(),
        },
    );

    // A MIDI track: a synth, a filter, two clips, an arrangement, two lanes.
    let mut bass = Track::new(TrackKind::Midi, "Bass", OrderKey::first());
    bass.color = Some(Rgb::new(0x22, 0x88, 0xcc));
    bass.mixer = Mixer {
        gain_db: -3.0,
        pan: -0.25,
        mute: false,
    };
    let synth = DeviceId(id(10));
    let filter = DeviceId(id(11));
    bass.devices.insert(
        synth,
        Device {
            order: OrderKey::first(),
            role: DeviceRole::Instrument,
            enabled: true,
            source: DeviceSource::Builtin {
                id: "faulk.osc_synth".into(),
                version: 1,
            },
            params: BTreeMap::from([("attack".into(), 0.01), ("wave".into(), 2.0)]),
            state: None,
            preset: None,
        },
    );
    bass.devices.insert(
        filter.clone(),
        Device {
            order: OrderKey::between(Some(&OrderKey::first()), None).unwrap(),
            role: DeviceRole::Effect,
            enabled: true,
            source: DeviceSource::Builtin {
                id: "faulk.filter".into(),
                version: 1,
            },
            params: BTreeMap::from([("cutoff".into(), 800.0)]),
            state: None,
            // Set from a preset of the project (#191), kept in `presets/`.
            preset: Some(Box::new(project::preset::DevicePreset {
                location: project::preset::PresetLocation::Project,
                id: id(60).to_string(),
                name: "Warm".into(),
            })),
        },
    );
    let riff = ClipId(id(20));
    let fill = ClipId(id(21));
    bass.clips.insert(
        riff.clone(),
        Clip {
            name: "Riff".into(),
            color: None,
            slot: Some(scene_a),
            launch_quantize: None,
            content: ClipContent::Midi {
                length: Tick(3840),
                looping: None,
                notes: BTreeSet::from([
                    note(0, 36, 240, 110),
                    note(0, 48, 240, 96),
                    note(480, 36, 240, 100),
                    note(960, 43, 120, 100),
                ]),
            },
        },
    );
    bass.clips.insert(
        fill.clone(),
        Clip {
            name: "Fill".into(),
            color: Some(Rgb::new(255, 0, 128)),
            slot: Some(scene_b),
            launch_quantize: None,
            content: ClipContent::Midi {
                length: Tick(1920),
                looping: Some(TickLoop {
                    start: Tick(480),
                    end: Tick(1920),
                }),
                notes: BTreeSet::from([
                    Note {
                        chance: 50,
                        ..note(480, 36, 240, 100)
                    },
                    Note {
                        cents: -12,
                        repeat: 4,
                        ..note(960, 40, 120, 64)
                    },
                ]),
            },
        },
    );
    bass.placements.insert(
        PlacementId(id(30)),
        Placement {
            clip: riff.clone(),
            start: Tick(0),
            span: Span::Ticks {
                length: Tick(15360),
                offset: Tick(0),
            },
        },
    );
    bass.placements.insert(
        PlacementId(id(31)),
        Placement {
            clip: fill,
            start: Tick(15360),
            span: Span::Ticks {
                length: Tick(1920),
                offset: Tick(480),
            },
        },
    );
    // A text lane (3 points: the example of § 2.3) and a lane in a blob (6 points).
    bass.automation.insert(
        LaneId(id(40)),
        AutomationLane {
            target: AutomationTarget::Device {
                device: filter,
                param: "cutoff".into(),
            },
            points: BTreeMap::from([
                (Tick(0), point(800.0, Shape::Linear)),
                (Tick(3840), point(2400.0, Shape::Linear)),
                (Tick(7680), point(2400.0, Shape::Step)),
            ]),
        },
    );
    bass.automation.insert(
        LaneId(id(41)),
        AutomationLane {
            target: AutomationTarget::Mixer {
                param: MixerParam::Pan,
            },
            points: (0..6)
                .map(|i| (Tick(i * 960), point(-1.0 + 0.4 * i as f64, Shape::Linear)))
                .collect(),
        },
    );
    p.tracks.insert(TrackId(id(50)), bass);

    // An audio track: the clip player, one clip of a sample, silence as its gain.
    let mut audio = Track::new(
        TrackKind::Audio,
        "Drums",
        OrderKey::between(Some(&OrderKey::first()), None).unwrap(),
    );
    audio.color = Some(Rgb::new(0xe0, 0x70, 0x40));
    audio.mixer.gain_db = f64::NEG_INFINITY;
    audio.devices.insert(
        DeviceId(id(12)),
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
            preset: None,
        },
    );
    let take = ClipId(id(22));
    audio.clips.insert(
        take.clone(),
        Clip {
            name: "Loop".into(),
            color: None,
            slot: None,
            launch_quantize: None,
            content: ClipContent::Audio {
                media: AudioMedia {
                    hash: Hash([0xab; 32]),
                    sample_rate: SampleRate::HZ_44100,
                    channels: 2,
                    frames: Frame(441_000),
                    name: "drums.wav".into(),
                },
                start: Frame(0),
                frames: Frame(88_200),
                gain_db: f64::NEG_INFINITY,
                looping: Some(FrameLoop {
                    start: Frame(0),
                    end: Frame(88_200),
                }),
            },
        },
    );
    audio.placements.insert(
        PlacementId(id(32)),
        Placement {
            clip: take,
            start: Tick(3840),
            span: Span::Frames {
                length: Frame(44_100),
                offset: Frame(1_000),
            },
        },
    );
    p.tracks.insert(TrackId(id(51)), audio);
    p.presets.insert(
        project::preset::PresetId(id(60)),
        project::preset::Preset {
            device: "faulk.filter".into(),
            version: 1,
            name: "Warm".into(),
            category: Some("Tone".into()),
            params: BTreeMap::from([("cutoff".into(), 800.0), ("resonance".into(), 0.3)]),
        },
    );
    p
}

pub fn all() -> Vec<(&'static str, Project)> {
    vec![("minimal", minimal()), ("song", song())]
}
