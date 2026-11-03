//! The deterministic generator of the synthetic heavy project (QA-7).
//!
//! Everything comes from one seed through SplitMix64 and one `IdGenerator` with a fixed
//! clock: the same profile and the same seed give the same project and, through the
//! writer, the same bytes (tested in `tests/it/load_bench.rs`). No clock, no `HashMap`
//! iteration, no entropy.

use project::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

/// The seed of the published numbers (the issue number).
pub const SEED: u64 = 130;

/// SplitMix64: small, fast, the same on every target.
#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (the modulo bias is irrelevant for a bench).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// A value in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// How big the generated project is.
#[derive(Clone, Debug)]
pub struct Profile {
    pub seed: u64,
    pub midi_tracks: usize,
    pub audio_tracks: usize,
    pub clips_per_midi_track: usize,
    /// Effects after the instrument of a MIDI track (the engine's test effect).
    pub effects: usize,
    pub notes_per_clip: usize,
    /// At most 10: synth sustain, gain, attack, decay, release, mixer gain and pan, the two
    /// effects' gain, the first effect's mute, in that order.
    pub lanes_per_midi_track: usize,
    /// At most 2: the mixer's gain and pan (an audio track has no other automatable target
    /// the engine receives).
    pub lanes_per_audio_track: usize,
    /// Spread over all the lanes (the first ones get the remainder).
    pub total_points: usize,
    pub clips_per_audio_track: usize,
    pub scenes: usize,
}

/// 16 bars of 4/4 at 960 ticks per quarter note.
pub const CLIP_TICKS: i64 = 16 * 4 * 960;
const AUDIO_RATE: u64 = 48_000;
const WAV_HEADER: u64 = 44;

impl Profile {
    /// The target of the issue: 100 tracks, 200 000 notes, 6 million automation points.
    pub fn heavy() -> Self {
        Self {
            seed: SEED,
            midi_tracks: 80,
            audio_tracks: 20,
            clips_per_midi_track: 10,
            effects: 2,
            notes_per_clip: 250,
            lanes_per_midi_track: 10,
            lanes_per_audio_track: 2,
            total_points: 6_000_000,
            clips_per_audio_track: 5,
            scenes: 8,
        }
    }

    /// The heavy profile with the devices the shipped web engine builds (no test effect), so
    /// that its engine stream runs in the browser's worklet as it is: 7 lanes a MIDI track,
    /// the same 6 million points.
    pub fn heavy_builtin() -> Self {
        Self {
            effects: 0,
            lanes_per_midi_track: 7,
            ..Self::heavy()
        }
    }

    /// A project small enough for the tests.
    pub fn small() -> Self {
        Self {
            seed: SEED,
            midi_tracks: 3,
            audio_tracks: 2,
            clips_per_midi_track: 3,
            effects: 2,
            notes_per_clip: 20,
            lanes_per_midi_track: 10,
            lanes_per_audio_track: 2,
            total_points: 3_000,
            clips_per_audio_track: 2,
            scenes: 2,
        }
    }

    pub fn tracks(&self) -> usize {
        self.midi_tracks + self.audio_tracks
    }

    pub fn notes(&self) -> usize {
        self.midi_tracks * self.clips_per_midi_track * self.notes_per_clip
    }

    pub fn lanes(&self) -> usize {
        self.midi_tracks * self.lanes_per_midi_track
            + self.audio_tracks * self.lanes_per_audio_track
    }

    pub fn audio_files(&self) -> usize {
        self.audio_tracks * self.clips_per_audio_track
    }
}

/// A generated audio file: a WAV header and noise, 16-bit stereo at 48 kHz.
#[derive(Clone, Debug)]
pub struct AudioFile {
    pub name: String,
    pub bytes: u64,
    pub seed: u64,
}

impl AudioFile {
    pub fn frames(&self) -> u64 {
        (self.bytes - WAV_HEADER) / 4
    }

    /// Streams the file into `out` and returns its SHA-256.
    pub fn write_to(&self, out: &mut impl Write) -> std::io::Result<Hash> {
        let mut hasher = Sha256::new();
        let data = self.frames() * 4;
        let mut head = Vec::with_capacity(WAV_HEADER as usize);
        head.extend_from_slice(b"RIFF");
        head.extend_from_slice(&((data + 36) as u32).to_le_bytes());
        head.extend_from_slice(b"WAVEfmt ");
        head.extend_from_slice(&16u32.to_le_bytes());
        head.extend_from_slice(&1u16.to_le_bytes());
        head.extend_from_slice(&2u16.to_le_bytes());
        head.extend_from_slice(&(AUDIO_RATE as u32).to_le_bytes());
        head.extend_from_slice(&((AUDIO_RATE * 4) as u32).to_le_bytes());
        head.extend_from_slice(&4u16.to_le_bytes());
        head.extend_from_slice(&16u16.to_le_bytes());
        head.extend_from_slice(b"data");
        head.extend_from_slice(&(data as u32).to_le_bytes());
        hasher.update(&head);
        out.write_all(&head)?;
        let mut rng = Rng(self.seed);
        let mut left = data as usize;
        let mut chunk = vec![0u8; 1 << 20];
        while left > 0 {
            let n = left.min(chunk.len());
            for word in chunk[..n].chunks_mut(8) {
                let bytes = rng.next_u64().to_le_bytes();
                word.copy_from_slice(&bytes[..word.len()]);
            }
            hasher.update(&chunk[..n]);
            out.write_all(&chunk[..n])?;
            left -= n;
        }
        Ok(Hash(hasher.finalize().into()))
    }
}

/// The audio files of the profile: `total_bytes` split evenly (each a multiple of 4 plus the
/// header).
pub fn audio_files(profile: &Profile, total_bytes: u64) -> Vec<AudioFile> {
    let n = profile.audio_files() as u64;
    if n == 0 {
        return Vec::new();
    }
    let each =
        ((total_bytes / n).saturating_sub(WAV_HEADER) / 4 * 4 + WAV_HEADER).max(WAV_HEADER + 4);
    (0..n)
        .map(|i| AudioFile {
            name: format!("take-{i:03}.wav"),
            bytes: each,
            seed: profile.seed ^ (0xA0D1_0000 + i),
        })
        .collect()
}

/// The values of one automation lane: a quantised random walk, one point every 5 to 35 ticks
/// (about 100 Hz at 120 BPM, a recorded gesture), a share of them steps.
pub fn lane_points(
    rng: &mut Rng,
    n: usize,
    target: &AutomationTarget,
) -> BTreeMap<Tick, AutomationPoint> {
    let mut v = rng.unit();
    let mut tick = 0i64;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        tick += 5 + rng.below(31) as i64;
        v = (v + (rng.unit() - 0.5) * 0.05).clamp(0.0, 1.0);
        let value = match target {
            AutomationTarget::Mixer {
                param: MixerParam::Gain,
            } => ((v * 60.0 - 60.0) * 100.0).round() / 100.0,
            AutomationTarget::Mixer {
                param: MixerParam::Pan,
            } => ((v * 2.0 - 1.0) * 1000.0).round() / 1000.0,
            _ => (v * 1000.0).round() / 1000.0,
        };
        let shape = if rng.below(100) < 15 {
            Shape::Step
        } else {
            Shape::Linear
        };
        out.push((Tick(tick), AutomationPoint { value, shape }));
    }
    out.into_iter().collect()
}

/// A MIDI clip's notes: unique, on a 32nd-note grid, inside the clip.
fn notes(rng: &mut Rng, n: usize) -> BTreeSet<Note> {
    let mut set = BTreeSet::new();
    while set.len() < n {
        let start = 120 * rng.below((CLIP_TICKS / 120) as u64) as i64;
        let length = (120 * (1 + rng.below(8)) as i64).min(CLIP_TICKS - start);
        let pitch = 36 + rng.below(61) as u8;
        let velocity = 30 + rng.below(98) as u8;
        let mut note = Note::new(Tick(start), pitch, Tick(length), velocity);
        if rng.below(100) == 0 {
            note.chance = 50;
        }
        if rng.below(200) == 0 {
            note.cents = rng.below(100) as i8 - 50;
            if note.cents == 0 {
                note.cents = 1;
            }
        }
        set.insert(note);
    }
    set
}

fn synth_device(order: OrderKey) -> Device {
    Device {
        order,
        role: DeviceRole::Instrument,
        enabled: true,
        source: DeviceSource::Builtin {
            id: "faulk.osc_synth".into(),
            version: 1,
        },
        params: [
            ("waveform", 1.0),
            ("attack", 0.01),
            ("decay", 0.2),
            ("sustain", 0.7),
            ("release", 0.3),
            ("gain", 0.8),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect(),
        state: None,
    }
}

fn effect_device(order: OrderKey) -> Device {
    Device {
        order,
        role: DeviceRole::Effect,
        enabled: true,
        source: DeviceSource::Builtin {
            id: "faulk.test_gain".into(),
            version: 1,
        },
        params: [("gain", 1.0), ("mute", 0.0)]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        state: None,
    }
}

/// Builds the project of `profile`; `audio` gives the hash of each audio file, in the order
/// of [`audio_files`].
pub fn build(profile: &Profile, audio: &[(AudioFile, Hash)]) -> Project {
    assert_eq!(
        audio.len(),
        profile.audio_files(),
        "one hash per audio file"
    );
    assert!(profile.lanes_per_midi_track <= 10 && profile.lanes_per_audio_track <= 2);
    let mut rng = Rng(profile.seed);
    let mut ids = IdGenerator::new(1_700_000_000_000, u128::from(profile.seed));
    let mut project = Project::new(ProjectId::generate(&mut ids), "Load bench 130");

    let mut scene_ids = Vec::new();
    let mut last: Option<OrderKey> = None;
    for i in 0..profile.scenes {
        let key = OrderKey::between(last.as_ref(), None).expect("order key");
        let id = SceneId::generate(&mut ids);
        project.scenes.insert(
            id.clone(),
            Scene {
                name: format!("Scene {}", i + 1),
                order: key.clone(),
            },
        );
        scene_ids.push(id);
        last = Some(key);
    }

    let lanes_total = profile.lanes();
    let base = profile.total_points / lanes_total.max(1);
    let extra = profile.total_points % lanes_total.max(1);
    let mut lane_index = 0usize;
    let next_points = |lane_index: &mut usize| {
        let n = base + usize::from(*lane_index < extra);
        *lane_index += 1;
        n
    };

    let mut track_key: Option<OrderKey> = None;
    let mut audio_next = 0usize;
    for t in 0..profile.tracks() {
        let midi = t < profile.midi_tracks;
        let key = OrderKey::between(track_key.as_ref(), None).expect("order key");
        track_key = Some(key.clone());
        let mut track = Track::new(
            if midi {
                TrackKind::Midi
            } else {
                TrackKind::Audio
            },
            format!("{} {}", if midi { "Synth" } else { "Audio" }, t + 1),
            key,
        );
        track.color = Some(Rgb::new((t * 37) as u8, (t * 91) as u8, (t * 17) as u8));
        track.mixer = Mixer {
            gain_db: -((rng.below(120)) as f64) / 10.0,
            pan: ((rng.unit() * 2.0 - 1.0) * 100.0).round() / 100.0,
            mute: false,
        };

        let mut order = OrderKey::first();
        let instrument = DeviceId::generate(&mut ids);
        let mut fx: Vec<DeviceId> = Vec::new();
        if midi {
            track
                .devices
                .insert(instrument.clone(), synth_device(order.clone()));
            for _ in 0..profile.effects {
                order = OrderKey::between(Some(&order), None).expect("order key");
                let id = DeviceId::generate(&mut ids);
                track
                    .devices
                    .insert(id.clone(), effect_device(order.clone()));
                fx.push(id);
            }
        } else {
            track.devices.insert(
                instrument.clone(),
                Device {
                    order: order.clone(),
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
        }

        if midi {
            for c in 0..profile.clips_per_midi_track {
                let id = ClipId::generate(&mut ids);
                track.clips.insert(
                    id.clone(),
                    Clip {
                        name: format!("Pattern {}", c + 1),
                        color: None,
                        slot: scene_ids.get(c).cloned(),
                        launch_quantize: None,
                        content: ClipContent::Midi {
                            length: Tick(CLIP_TICKS),
                            looping: None,
                            notes: notes(&mut rng, profile.notes_per_clip),
                        },
                    },
                );
                track.placements.insert(
                    PlacementId::generate(&mut ids),
                    Placement {
                        clip: id,
                        start: Tick(c as i64 * CLIP_TICKS),
                        span: Span::Ticks {
                            length: Tick(CLIP_TICKS),
                            offset: Tick(0),
                        },
                    },
                );
            }
        } else {
            for c in 0..profile.clips_per_audio_track {
                let (file, hash) = &audio[audio_next];
                audio_next += 1;
                let id = ClipId::generate(&mut ids);
                track.clips.insert(
                    id.clone(),
                    Clip {
                        name: format!("Take {}", c + 1),
                        color: None,
                        slot: scene_ids.get(c).cloned(),
                        launch_quantize: None,
                        content: ClipContent::Audio {
                            media: AudioMedia {
                                hash: *hash,
                                sample_rate: SampleRate::HZ_48000,
                                channels: 2,
                                frames: Frame(file.frames()),
                                name: file.name.clone(),
                            },
                            start: Frame(0),
                            frames: Frame(file.frames()),
                            gain_db: 0.0,
                            looping: None,
                        },
                    },
                );
                track.placements.insert(
                    PlacementId::generate(&mut ids),
                    Placement {
                        clip: id,
                        start: Tick(c as i64 * 20_000_000),
                        span: Span::Frames {
                            length: Frame(file.frames()),
                            offset: Frame(0),
                        },
                    },
                );
            }
        }

        let mut targets = vec![
            AutomationTarget::Mixer {
                param: MixerParam::Gain,
            },
            AutomationTarget::Mixer {
                param: MixerParam::Pan,
            },
        ];
        if midi {
            for p in ["sustain", "gain", "attack", "decay", "release"] {
                targets.push(AutomationTarget::Device {
                    device: instrument.clone(),
                    param: p.into(),
                });
            }
            for (f, p) in [(0, "gain"), (1, "gain"), (0, "mute")]
                .into_iter()
                .filter(|(f, _)| *f < fx.len())
            {
                targets.push(AutomationTarget::Device {
                    device: fx[f].clone(),
                    param: p.into(),
                });
            }
        }
        let lanes = if midi {
            profile.lanes_per_midi_track
        } else {
            profile.lanes_per_audio_track
        };
        assert!(
            targets.len() >= lanes,
            "the profile asks for more lanes than targets"
        );
        // Mixer lanes first for an audio track, so the two it has are the mixer's.
        for target in targets.into_iter().take(lanes) {
            let n = next_points(&mut lane_index);
            let points = lane_points(&mut rng, n, &target);
            track.automation.insert(
                LaneId::generate(&mut ids),
                AutomationLane { target, points },
            );
        }
        project.tracks.insert(TrackId::generate(&mut ids), track);
    }
    project
}

/// The whole pipeline in memory without touching the disk: the hashes come from streaming
/// the files into a sink.
pub fn generate(profile: &Profile, audio_bytes: u64) -> (Project, Vec<AudioFile>) {
    let files = audio_files(profile, audio_bytes);
    let with_hash: Vec<_> = files
        .iter()
        .map(|f| (f.clone(), f.write_to(&mut std::io::sink()).expect("sink")))
        .collect();
    (build(profile, &with_hash), files)
}
