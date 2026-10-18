//! The project document (project model § 1.2 and § 1.3).
//!
//! What makes the music is here; what belongs to the screen or the machine is in
//! [`crate::SessionState`], never in these types. Every durable entity is keyed by a
//! stable id in a map of its parent; the order of tracks, scenes and devices is a
//! fractional [`OrderKey`] carried by each element. A [`Project`] read from text is
//! checked by [`Project::validate`] before use.

use crate::blob::Hash;
use crate::ids::{ClipId, DeviceId, Id, LaneId, PlacementId, ProjectId, SceneId, TrackId};
use crate::order::{OrderKey, by_order};
use crate::serde_util::{Rgb, db, finite, finite_map, unique_map, unique_set};
use engine::time::{Frame, LoopRegion, Quantize, SampleRate, Tempo, Tick, TimeSignature};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The on-disk format version this crate reads and writes (§ 2.6).
pub const FORMAT_VERSION: u32 = 1;

/// The built-in instrument of audio tracks: the clip player.
pub const CLIP_PLAYER: &str = "faulk.clip_player";

/// The first thing a reader takes from `project.json`: its format version alone, every
/// other field ignored. [`Project`] refuses unknown fields, so a newer format would fail
/// with an opaque serde error; reading this first tells "migrate" or "newer than this
/// build" (`format` other than [`FORMAT_VERSION`]) apart from "corrupt".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct FormatProbe {
    pub format: u32,
}

/// The project document: what makes the music, saved and versioned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// On-disk format version, for migrations ([`FORMAT_VERSION`]).
    pub format: u32,
    /// Stable identity, created once with the project and never changed ("save as"
    /// makes a new one). Required: a project without it is refused on read.
    pub id: ProjectId,
    /// Display name: trimmed, not empty.
    pub name: String,
    /// The tempo map, sorted by tick. Exactly one point at tick 0 in the MVP (constant
    /// tempo); the map shape keeps the format ready for tempo changes.
    pub tempo: Vec<TempoPoint>,
    /// The time-signature map, sorted by tick. Exactly one point at tick 0 in the MVP.
    pub meter: Vec<MeterPoint>,
    /// Saved transport settings.
    pub transport: TransportSettings,
    /// The rows of the launch grid.
    #[serde(deserialize_with = "unique_map")]
    pub scenes: BTreeMap<SceneId, Scene>,
    /// The master bus.
    pub master: Mixer,
    /// The tracks.
    #[serde(deserialize_with = "unique_map")]
    pub tracks: BTreeMap<TrackId, Track>,
}

/// A tempo from a tick on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TempoPoint {
    pub tick: Tick,
    pub bpm: Tempo,
}

/// A time signature from a tick on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterPoint {
    pub tick: Tick,
    pub signature: TimeSignature,
}

/// The transport settings saved with the project (§ 1.3). Play state and the playhead
/// are ephemeral and never saved.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportSettings {
    /// The launch quantization, global to the project (founder's decision 8): one bar by
    /// default.
    pub launch_quantize: Quantize,
    /// The loop region `[start, end)` in ticks, when one is set.
    pub loop_region: Option<LoopRegion>,
    /// Whether the transport loops over `loop_region`.
    pub loop_enabled: bool,
    pub metronome: Metronome,
}

impl Default for TransportSettings {
    fn default() -> Self {
        Self {
            launch_quantize: Quantize::Bar,
            loop_region: None,
            loop_enabled: false,
            metronome: Metronome::default(),
        }
    }
}

/// The metronome settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metronome {
    pub enabled: bool,
    /// Level in dB; `-inf` is written `null`.
    #[serde(with = "db")]
    pub gain_db: f64,
    /// Bars counted in before recording.
    pub count_in_bars: u8,
}

impl Default for Metronome {
    fn default() -> Self {
        Self {
            enabled: false,
            gain_db: 0.0,
            count_in_bars: 1,
        }
    }
}

/// A row of the launch grid. A tempo per scene comes after the MVP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub name: String,
    pub order: OrderKey,
}

/// A track: its chain of devices, its clips, its arrangement and its automation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub name: String,
    pub color: Option<Rgb>,
    pub order: OrderKey,
    pub kind: TrackKind,
    pub mixer: Mixer,
    pub output: Output,
    /// The chain: at most one instrument, first, then effects, ordered by
    /// [`Device::order`].
    #[serde(deserialize_with = "unique_map")]
    pub devices: BTreeMap<DeviceId, Device>,
    #[serde(deserialize_with = "unique_map")]
    pub clips: BTreeMap<ClipId, Clip>,
    /// This track's arrangement.
    #[serde(deserialize_with = "unique_map")]
    pub placements: BTreeMap<PlacementId, Placement>,
    #[serde(deserialize_with = "unique_map")]
    pub automation: BTreeMap<LaneId, AutomationLane>,
}

/// What a track's clips hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// MIDI clips, played by the track's instrument.
    Midi,
    /// Audio clips, played by the clip player ([`CLIP_PLAYER`]).
    Audio,
}

/// Where a track's signal goes. Only the master in the MVP; groups and returns later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    Master,
}

/// Gain, pan and mute of a track or of the master. Solo is not here: it is session state
/// (founder's decision 10). Sends come after the MVP.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mixer {
    /// Gain in dB; `-inf` (silence) is written `null`.
    #[schemars(with = "Option<f64>")]
    #[serde(with = "db")]
    pub gain_db: f64,
    /// Pan from -1 (left) to 1 (right).
    #[schemars(with = "f64")]
    #[serde(with = "finite")]
    pub pan: f64,
    pub mute: bool,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
        }
    }
}

/// A link of a track's chain. Its display name is its type's single English name, never
/// translated nor stored here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub order: OrderKey,
    pub role: DeviceRole,
    pub enabled: bool,
    pub source: DeviceSource,
    /// Parameter id → value in the unit the device declares.
    #[schemars(with = "BTreeMap<String, f64>")]
    #[serde(with = "finite_map")]
    pub params: BTreeMap<String, f64>,
    /// Opaque instrument state, stored as a blob.
    pub state: Option<Hash>,
}

/// What a device does in the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    Instrument,
    Effect,
}

/// Where a device comes from. Third-party plugins come after the MVP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DeviceSource {
    /// A device built into Faulk, by type id (`"faulk.osc_synth"`) and version.
    Builtin { id: String, version: u32 },
    /// A module of the Workshop, by name and the hash of its compiled code.
    Module { name: String, hash: Hash },
}

/// A clip: the content. A slot puts it in the launch grid, a [`Placement`] in the
/// arrangement. Copying a clip makes a new, independent clip (founder's decision 11); two
/// placements of the same clip are linked clips.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub name: String,
    pub color: Option<Rgb>,
    /// The scene of the launch-grid slot the clip sits in.
    pub slot: Option<SceneId>,
    /// Reserved for a per-clip launch quantization after the MVP: absent means the
    /// project setting. The MVP never writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_quantize: Option<Quantize>,
    pub content: ClipContent,
}

/// A clip's content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipContent {
    /// Musical time: length and loop in ticks.
    Midi {
        length: Tick,
        looping: Option<TickLoop>,
        #[serde(deserialize_with = "unique_set")]
        notes: BTreeSet<Note>,
    },
    /// Source time: a range `[start, start + frames)` of an audio file, in frames. No
    /// length in ticks is stored: it is computed at the project's tempo.
    Audio {
        media: AudioMedia,
        #[schemars(with = "u64")]
        start: Frame,
        #[schemars(with = "u64")]
        frames: Frame,
        /// Clip gain in dB; `-inf` is written `null`.
        #[schemars(with = "Option<f64>")]
        #[serde(with = "db")]
        gain_db: f64,
        looping: Option<FrameLoop>,
    },
}

/// An audio file referenced by its blob, with its metadata in clear so the project reads,
/// validates and merges without decoding the audio (§ 2.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioMedia {
    pub hash: Hash,
    #[schemars(with = "u32")]
    pub sample_rate: SampleRate,
    pub channels: u16,
    /// Length of the whole file in frames.
    #[schemars(with = "u64")]
    pub frames: Frame,
    /// The file's original name.
    pub name: String,
}

/// A MIDI clip's loop `[start, end)` in ticks, within `[0, length]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TickLoop {
    pub start: Tick,
    pub end: Tick,
}

/// An audio clip's loop `[start, end)` in source frames, relative to the clip's start,
/// within `[0, frames]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameLoop {
    #[schemars(with = "u64")]
    pub start: Frame,
    #[schemars(with = "u64")]
    pub end: Frame,
}

/// A note: a value without an id (CT-4). Its order compares fields in declaration order:
/// (start, pitch, length, velocity, cents, chance, repeat).
///
/// The last three fields are optional in the file: a default value is not written, and an
/// absent one reads as the default, so a note has exactly one representation. `chance` and
/// `repeat` are stored and edited here; the clip player reads them with #192.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// Position from the start of the clip, in ticks.
    pub start: Tick,
    /// MIDI key, 0 to 127.
    #[schemars(range(min = 0, max = 127))]
    pub pitch: u8,
    /// Duration in ticks, at least 1.
    pub length: Tick,
    /// Velocity, 1 to 127.
    #[schemars(range(min = 1, max = 127))]
    pub velocity: u8,
    /// Fine tuning in cents, -50 to 50 (0 by default).
    #[serde(default, skip_serializing_if = "is_default_cents")]
    #[schemars(range(min = -50, max = 50))]
    pub cents: i8,
    /// Probability that the note plays, in percent, 0 to 100 (100 by default).
    #[serde(
        default = "Note::default_chance",
        skip_serializing_if = "is_default_chance"
    )]
    #[schemars(range(min = 0, max = 100))]
    pub chance: u8,
    /// How many times the note repeats, 1 to 16 (1 by default).
    #[serde(
        default = "Note::default_repeat",
        skip_serializing_if = "is_default_repeat"
    )]
    #[schemars(range(min = 1, max = 16))]
    pub repeat: u8,
}

fn is_default_cents(v: &i8) -> bool {
    *v == 0
}

fn is_default_chance(v: &u8) -> bool {
    *v == Note::DEFAULT_CHANCE
}

fn is_default_repeat(v: &u8) -> bool {
    *v == Note::DEFAULT_REPEAT
}

impl Note {
    /// Fine tuning at most this far from the key, in cents, either way.
    pub const MAX_CENTS: i8 = 50;
    /// A note plays every time by default.
    pub const DEFAULT_CHANCE: u8 = 100;
    /// A note plays once by default.
    pub const DEFAULT_REPEAT: u8 = 1;
    /// Most repeats of a note.
    pub const MAX_REPEAT: u8 = 16;

    fn default_chance() -> u8 {
        Self::DEFAULT_CHANCE
    }

    fn default_repeat() -> u8 {
        Self::DEFAULT_REPEAT
    }

    /// A note with the default fine tuning, chance and repeat.
    pub const fn new(start: Tick, pitch: u8, length: Tick, velocity: u8) -> Self {
        Self {
            start,
            pitch,
            length,
            velocity,
            cents: 0,
            chance: Self::DEFAULT_CHANCE,
            repeat: Self::DEFAULT_REPEAT,
        }
    }

    /// Whether every field is in its range (position and length aside, which depend on
    /// the clip): pitch at most 127, velocity 1 to 127, cents -50 to 50, chance at most
    /// 100, repeat 1 to 16.
    pub fn fields_in_range(&self) -> bool {
        self.pitch <= 127
            && (1..=127).contains(&self.velocity)
            && (-Self::MAX_CENTS..=Self::MAX_CENTS).contains(&self.cents)
            && self.chance <= 100
            && (1..=Self::MAX_REPEAT).contains(&self.repeat)
    }
}

/// A clip set in the arrangement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub clip: ClipId,
    pub start: Tick,
    pub span: Span,
}

/// The extent of a placement: in ticks for a MIDI clip, in source frames for an audio
/// clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Span {
    Ticks {
        length: Tick,
        offset: Tick,
    },
    Frames {
        #[schemars(with = "u64")]
        length: Frame,
        #[schemars(with = "u64")]
        offset: Frame,
    },
}

/// A parameter's curve over time, in a track's arrangement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomationLane {
    pub target: AutomationTarget,
    /// Points by tick: at most one point per tick.
    #[serde(deserialize_with = "unique_map")]
    pub points: BTreeMap<Tick, AutomationPoint>,
}

/// What a lane automates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AutomationTarget {
    /// A parameter of a device of the same track.
    Device { device: DeviceId, param: String },
    /// A parameter of the track's mixer.
    Mixer { param: MixerParam },
}

/// An automatable mixer parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MixerParam {
    Gain,
    Pan,
}

/// A value at a tick and the shape of the transition to the next point.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomationPoint {
    #[schemars(with = "f64")]
    #[serde(with = "finite")]
    pub value: f64,
    pub shape: Shape,
}

/// The shape of the transition from a point to the next one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Linear,
    Step,
}

impl Project {
    /// A valid empty project: 120 BPM, 4/4, launch quantization one bar, no loop.
    pub fn new(id: ProjectId, name: impl Into<String>) -> Self {
        Self {
            format: FORMAT_VERSION,
            id,
            name: name.into(),
            tempo: vec![TempoPoint {
                tick: Tick::ZERO,
                bpm: Tempo::clamped(Tempo::DEFAULT_BPM),
            }],
            meter: vec![MeterPoint {
                tick: Tick::ZERO,
                signature: TimeSignature::COMMON,
            }],
            transport: TransportSettings::default(),
            scenes: BTreeMap::new(),
            master: Mixer::default(),
            tracks: BTreeMap::new(),
        }
    }

    /// The project's tempo: the first point of the map (the only one in the MVP).
    pub fn tempo(&self) -> Tempo {
        self.tempo
            .first()
            .map_or(Tempo::clamped(Tempo::DEFAULT_BPM), |p| p.bpm)
    }

    /// The project's time signature: the first point of the map (the only one in the MVP).
    pub fn time_signature(&self) -> TimeSignature {
        self.meter
            .first()
            .map_or(TimeSignature::COMMON, |p| p.signature)
    }

    /// Whether an entity of the project already has this id: the project itself, a scene,
    /// a track, a device, a clip, a placement or a lane. Ids are unique across all of
    /// them ([`Project::validate`] refuses a duplicate), so a command that brings an
    /// entity back with its old identifier checks this first.
    pub fn contains_id(&self, id: &Id) -> bool {
        self.id.0 == *id
            || self.scenes.keys().any(|k| k.0 == *id)
            || self.tracks.iter().any(|(k, t)| {
                k.0 == *id
                    || t.devices.keys().any(|k| k.0 == *id)
                    || t.clips.keys().any(|k| k.0 == *id)
                    || t.placements.keys().any(|k| k.0 == *id)
                    || t.automation.keys().any(|k| k.0 == *id)
            })
    }

    /// The tracks in display order (order key, then id).
    pub fn tracks_in_order(&self) -> Vec<(&TrackId, &Track)> {
        by_order(&self.tracks, |t| &t.order)
    }

    /// How many clips, placements and lanes the whole project has: the counts the
    /// engine's slot tables bound project-wide ([`crate::limits::MAX_CLIPS`]…).
    pub fn counts(&self) -> Counts {
        self.tracks.values().fold(Counts::default(), |c, t| Counts {
            clips: c.clips + t.clips.len(),
            placements: c.placements + t.placements.len(),
            lanes: c.lanes + t.automation.len(),
        })
    }

    /// The scenes in display order (order key, then id).
    pub fn scenes_in_order(&self) -> Vec<(&SceneId, &Scene)> {
        by_order(&self.scenes, |s| &s.order)
    }
}

/// Counts of a project's entities (see [`Project::counts`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub clips: usize,
    pub placements: usize,
    pub lanes: usize,
}

impl Track {
    /// An empty track with the default mixer, routed to the master.
    pub fn new(kind: TrackKind, name: impl Into<String>, order: OrderKey) -> Self {
        Self {
            name: name.into(),
            color: None,
            order,
            kind,
            mixer: Mixer::default(),
            output: Output::Master,
            devices: BTreeMap::new(),
            clips: BTreeMap::new(),
            placements: BTreeMap::new(),
            automation: BTreeMap::new(),
        }
    }

    /// The chain in order (order key, then id).
    pub fn devices_in_order(&self) -> Vec<(&DeviceId, &Device)> {
        by_order(&self.devices, |d| &d.order)
    }
}

impl ClipContent {
    /// Whether this is a MIDI clip.
    pub fn is_midi(&self) -> bool {
        matches!(self, Self::Midi { .. })
    }
}
