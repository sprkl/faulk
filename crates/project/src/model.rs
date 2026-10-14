//! The project document (project model § 1.2 and § 1.3).
//!
//! What makes the music is here; what belongs to the screen or the machine is in
//! [`crate::SessionState`], never in these types. Every durable entity is keyed by a
//! stable id in a map of its parent; the order of tracks, scenes and devices is a
//! fractional [`OrderKey`] carried by each element. A [`Project`] read from text is
//! checked by [`Project::validate`] before use.

use crate::blob::Hash;
use crate::ids::{ClipId, DeviceId, LaneId, PlacementId, ProjectId, SceneId, TrackId};
use crate::order::{OrderKey, by_order};
use crate::serde_util::{Rgb, db, finite, finite_map, unique_map, unique_set};
use engine::time::{Frame, LoopRegion, Quantize, SampleRate, Tempo, Tick, TimeSignature};
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// MIDI clips, played by the track's instrument.
    Midi,
    /// Audio clips, played by the clip player ([`CLIP_PLAYER`]).
    Audio,
}

/// Where a track's signal goes. Only the master in the MVP; groups and returns later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    Master,
}

/// Gain, pan and mute of a track or of the master. Solo is not here: it is session state
/// (founder's decision 10). Sends come after the MVP.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mixer {
    /// Gain in dB; `-inf` (silence) is written `null`.
    #[serde(with = "db")]
    pub gain_db: f64,
    /// Pan from -1 (left) to 1 (right).
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub order: OrderKey,
    pub role: DeviceRole,
    pub enabled: bool,
    pub source: DeviceSource,
    /// Parameter id → value in the unit the device declares.
    #[serde(with = "finite_map")]
    pub params: BTreeMap<String, f64>,
    /// Opaque instrument state, stored as a blob.
    pub state: Option<Hash>,
}

/// What a device does in the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    Instrument,
    Effect,
}

/// Where a device comes from. Third-party plugins come after the MVP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
        start: Frame,
        frames: Frame,
        /// Clip gain in dB; `-inf` is written `null`.
        #[serde(with = "db")]
        gain_db: f64,
        looping: Option<FrameLoop>,
    },
}

/// An audio file referenced by its blob, with its metadata in clear so the project reads,
/// validates and merges without decoding the audio (§ 2.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioMedia {
    pub hash: Hash,
    pub sample_rate: SampleRate,
    pub channels: u16,
    /// Length of the whole file in frames.
    pub frames: Frame,
    /// The file's original name.
    pub name: String,
}

/// A MIDI clip's loop `[start, end)` in ticks, within `[0, length]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickLoop {
    pub start: Tick,
    pub end: Tick,
}

/// An audio clip's loop `[start, end)` in source frames, relative to the clip's start,
/// within `[0, frames]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameLoop {
    pub start: Frame,
    pub end: Frame,
}

/// A note: a value without an id (CT-4). Its order compares fields in declaration order:
/// (start, pitch, length, velocity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub start: Tick,
    pub pitch: u8,
    pub length: Tick,
    pub velocity: u8,
}

/// A clip set in the arrangement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub clip: ClipId,
    pub start: Tick,
    pub span: Span,
}

/// The extent of a placement: in ticks for a MIDI clip, in source frames for an audio
/// clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Span {
    Ticks { length: Tick, offset: Tick },
    Frames { length: Frame, offset: Frame },
}

/// A parameter's curve over time, in a track's arrangement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationLane {
    pub target: AutomationTarget,
    /// Points by tick: at most one point per tick.
    #[serde(deserialize_with = "unique_map")]
    pub points: BTreeMap<Tick, AutomationPoint>,
}

/// What a lane automates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AutomationTarget {
    /// A parameter of a device of the same track.
    Device { device: DeviceId, param: String },
    /// A parameter of the track's mixer.
    Mixer { param: MixerParam },
}

/// An automatable mixer parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MixerParam {
    Gain,
    Pan,
}

/// A value at a tick and the shape of the transition to the next point.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationPoint {
    #[serde(with = "finite")]
    pub value: f64,
    pub shape: Shape,
}

/// The shape of the transition from a point to the next one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

    /// The tracks in display order (order key, then id).
    pub fn tracks_in_order(&self) -> Vec<(&TrackId, &Track)> {
        by_order(&self.tracks, |t| &t.order)
    }

    /// The scenes in display order (order key, then id).
    pub fn scenes_in_order(&self) -> Vec<(&SceneId, &Scene)> {
        by_order(&self.scenes, |s| &s.order)
    }
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
