//! Property tests: generated valid projects pass `validate()` and survive a serde round
//! trip unchanged, byte for byte on the second write.

use project::*;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
struct TrackSpec {
    midi: bool,
    name: String,
    gain: Option<f64>,
    pan: f64,
    mute: bool,
    effects: Vec<(String, f64)>,
    clips: Vec<ClipSpec>,
    points: Vec<(i64, f64, bool)>,
}

#[derive(Debug, Clone)]
struct ClipSpec {
    length: i64,
    notes: Vec<(i64, u8, i64, u8)>,
    slot: bool,
    placed_at: Option<i64>,
}

fn name() -> impl Strategy<Value = String> {
    "[A-Za-z0-9éà]([A-Za-z0-9 éà]{0,20}[A-Za-z0-9éà])?"
}

fn clip_spec() -> impl Strategy<Value = ClipSpec> {
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

fn track_spec() -> impl Strategy<Value = TrackSpec> {
    (
        any::<bool>(),
        name(),
        prop::option::of(-120.0f64..12.0),
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

fn build(seed: u128, name: String, tracks: Vec<TrackSpec>, scenes: usize) -> Project {
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
        track.automation.insert(
            LaneId::generate(&mut ids),
            AutomationLane {
                target,
                points: spec
                    .points
                    .into_iter()
                    .map(|(t, value, step)| {
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn generated_projects_are_valid_and_round_trip(
        seed in any::<u128>(),
        name in name(),
        tracks in prop::collection::vec(track_spec(), 0..6),
        scenes in 0usize..5,
    ) {
        let project = build(seed, name, tracks, scenes);
        prop_assert_eq!(project.validate(), Ok(()));
        let json = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(&back, &project);
        prop_assert_eq!(serde_json::to_string(&back).unwrap(), json);
        prop_assert_eq!(back.validate(), Ok(()));
    }

    #[test]
    fn sessions_round_trip(
        solo in prop::collection::btree_set(any::<u64>(), 0..10),
        input in prop::option::of(name()),
    ) {
        let mut ids = IdGenerator::new(1, 1);
        let all: Vec<TrackId> = solo.iter().map(|_| TrackId::generate(&mut ids)).collect();
        let session = SessionState {
            selected_track: all.first().cloned(),
            solo: all.into_iter().collect(),
            midi_input: input,
        };
        let json = serde_json::to_string(&session).unwrap();
        prop_assert_eq!(serde_json::from_str::<SessionState>(&json).unwrap(), session);
    }
}

proptest! {
    /// The slot table against a naive model: every assign gives the lowest slot no live id
    /// holds, ids keep their slots, a full table refuses, and releasing everything leaves
    /// the empty table.
    #[test]
    fn slot_table_gives_the_lowest_free_slot(ops in prop::collection::vec((any::<bool>(), 0u8..40), 0..300)) {
        const LIMIT: u32 = 24;
        let mut table = SlotTable::<u8>::default();
        let mut model: BTreeMap<u8, u32> = BTreeMap::new();
        for (assign, id) in ops {
            if assign {
                let expected = model.get(&id).copied().or_else(|| {
                    let used: BTreeSet<u32> = model.values().copied().collect();
                    (0..LIMIT).find(|s| !used.contains(s))
                });
                prop_assert_eq!(table.assign(&id, LIMIT), expected);
                if let Some(slot) = expected {
                    model.insert(id, slot);
                }
            } else {
                prop_assert_eq!(table.release(&id), model.remove(&id));
            }
            prop_assert_eq!(table.len(), model.len());
            prop_assert_eq!(table.span(), model.values().max().map_or(0, |m| m + 1));
            for (id, slot) in &model {
                prop_assert_eq!(table.get(id), Some(*slot));
                prop_assert_eq!(table.id_at(*slot), Some(id));
            }
        }
        for id in model.keys() {
            table.release(id);
        }
        prop_assert_eq!(table, SlotTable::default());
    }
}
