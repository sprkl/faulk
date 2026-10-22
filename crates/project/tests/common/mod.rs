//! Generators shared by the integration tests: valid projects of every shape the model has.
#![allow(dead_code)]

pub mod decorate;
pub mod reference;
pub mod rules;

use project::*;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct TrackSpec {
    pub midi: bool,
    pub name: String,
    pub gain: Option<f64>,
    pub pan: f64,
    pub mute: bool,
    pub effects: Vec<(String, f64)>,
    pub clips: Vec<ClipSpec>,
    pub points: Vec<(i64, f64, bool)>,
}

#[derive(Debug, Clone)]
pub struct ClipSpec {
    pub length: i64,
    pub notes: Vec<(i64, u8, i64, u8)>,
    pub slot: bool,
    pub placed_at: Option<i64>,
}

pub fn name() -> impl Strategy<Value = String> {
    "[A-Za-z0-9éà]([A-Za-z0-9 éà]{0,20}[A-Za-z0-9éà])?"
}

pub fn clip_spec() -> impl Strategy<Value = ClipSpec> {
    (
        1i64..100_000,
        prop::collection::vec((0i64..100_000, 0u8..=127, 1i64..10_000, 1u8..=127), 0..30),
        any::<bool>(),
        prop::option::of(0i64..1_000_000),
    )
        .prop_map(|(length, notes, slot, placed_at)| ClipSpec {
            length,
            // Keep each note inside the clip.
            notes: notes
                .into_iter()
                .map(|(s, p, l, v)| {
                    let s = s % length;
                    (s, p, l.min(length - s), v)
                })
                .collect(),
            slot,
            placed_at,
        })
}

pub fn track_spec() -> impl Strategy<Value = TrackSpec> {
    (
        any::<bool>(),
        name(),
        prop::option::of(MIN_GAIN_DB..=MAX_GAIN_DB),
        -1.0f64..=1.0,
        any::<bool>(),
        prop::collection::vec(("[a-z][a-z0-9_]{0,10}", -1e6f64..1e6), 0..4),
        prop::collection::vec(clip_spec(), 0..4),
        prop::collection::vec((0i64..1_000_000, -1e3f64..1e3, any::<bool>()), 0..20),
    )
        .prop_map(
            |(midi, name, gain, pan, mute, effects, clips, points)| TrackSpec {
                midi,
                name,
                gain,
                pan,
                mute,
                effects,
                clips,
                points,
            },
        )
}

pub fn build(seed: u128, name: String, tracks: Vec<TrackSpec>, scenes: usize) -> Project {
    let mut ids = IdGenerator::new(1_700_000_000_000, seed);
    let mut project = Project::new(ProjectId::generate(&mut ids), name);
    let mut scene_ids = Vec::new();
    let mut order: Option<OrderKey> = None;
    for _ in 0..scenes {
        let key = OrderKey::between(order.as_ref(), None).unwrap();
        let id = SceneId::generate(&mut ids);
        project.scenes.insert(
            id.clone(),
            Scene {
                name: String::new(),
                order: key.clone(),
            },
        );
        scene_ids.push(id);
        order = Some(key);
    }
    let mut order: Option<OrderKey> = None;
    for spec in tracks {
        let key = OrderKey::between(None, order.as_ref()).unwrap();
        order = Some(key.clone());
        let kind = if spec.midi {
            TrackKind::Midi
        } else {
            TrackKind::Audio
        };
        let mut track = Track::new(kind, spec.name, key);
        track.mixer = Mixer {
            gain_db: spec.gain.unwrap_or(f64::NEG_INFINITY),
            pan: spec.pan,
            mute: spec.mute,
        };
        let instrument = DeviceId::generate(&mut ids);
        track.devices.insert(
            instrument,
            Device {
                order: OrderKey::first(),
                role: DeviceRole::Instrument,
                enabled: true,
                source: DeviceSource::Builtin {
                    id: if spec.midi {
                        "faulk.osc_synth".into()
                    } else {
                        CLIP_PLAYER.into()
                    },
                    version: 1,
                },
                params: BTreeMap::new(),
                state: None,
            },
        );
        let mut last = OrderKey::first();
        let mut first_effect = None;
        for (param, value) in spec.effects {
            last = OrderKey::between(Some(&last), None).unwrap();
            let id = DeviceId::generate(&mut ids);
            first_effect.get_or_insert((id.clone(), param.clone()));
            track.devices.insert(
                id,
                Device {
                    order: last.clone(),
                    role: DeviceRole::Effect,
                    enabled: true,
                    source: DeviceSource::Builtin {
                        id: "faulk.filter".into(),
                        version: 1,
                    },
                    params: BTreeMap::from([(param, value)]),
                    state: None,
                },
            );
        }
        for (i, c) in spec.clips.into_iter().enumerate() {
            let id = ClipId::generate(&mut ids);
            let slot = (c.slot && i < scene_ids.len()).then(|| scene_ids[i].clone());
            let content = if spec.midi {
                ClipContent::Midi {
                    length: Tick(c.length),
                    looping: None,
                    notes: c
                        .notes
                        .iter()
                        .map(|&(s, p, l, v)| Note {
                            // The optional fields take every value of their range.
                            cents: (i16::from(p) % 101 - 50) as i8,
                            chance: v % 101,
                            repeat: 1 + p % 16,
                            ..Note::new(Tick(s), p, Tick(l), v)
                        })
                        .collect::<BTreeSet<_>>(),
                }
            } else {
                ClipContent::Audio {
                    media: AudioMedia {
                        hash: Hash([c.length as u8; 32]),
                        sample_rate: SampleRate::HZ_44100,
                        channels: 2,
                        frames: Frame(c.length as u64 * 2),
                        name: "take.wav".into(),
                    },
                    start: Frame(0),
                    frames: Frame(c.length as u64),
                    gain_db: 0.0,
                    looping: None,
                }
            };
            if let Some(at) = c.placed_at {
                let span = if spec.midi {
                    Span::Ticks {
                        length: Tick(c.length),
                        offset: Tick(0),
                    }
                } else {
                    Span::Frames {
                        length: Frame(c.length as u64),
                        offset: Frame(0),
                    }
                };
                track.placements.insert(
                    PlacementId::generate(&mut ids),
                    Placement {
                        clip: id.clone(),
                        start: Tick(at),
                        span,
                    },
                );
            }
            track.clips.insert(
                id,
                Clip {
                    name: String::new(),
                    color: None,
                    slot,
                    launch_quantize: None,
                    content,
                },
            );
        }
        let target = match first_effect {
            Some((device, param)) => AutomationTarget::Device { device, param },
            None => AutomationTarget::Mixer {
                param: MixerParam::Gain,
            },
        };
        // A gain lane holds levels: its points are spread over the shared range (#269).
        let gain_lane = target
            == AutomationTarget::Mixer {
                param: MixerParam::Gain,
            };
        track.automation.insert(
            LaneId::generate(&mut ids),
            AutomationLane {
                target,
                points: spec
                    .points
                    .into_iter()
                    .map(|(t, value, step)| {
                        // The generated values are in -1e3..1e3: a linear map keeps them
                        // finite and inside the range.
                        let value = if gain_lane {
                            MIN_GAIN_DB + (value + 1e3) / 2e3 * (MAX_GAIN_DB - MIN_GAIN_DB)
                        } else {
                            value
                        };
                        let shape = if step { Shape::Step } else { Shape::Linear };
                        (Tick(t), AutomationPoint { value, shape })
                    })
                    .collect(),
            },
        );
        project.tracks.insert(TrackId::generate(&mut ids), track);
    }
    project
}
