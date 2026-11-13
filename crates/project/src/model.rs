//! The project document (project model § 1.2 and § 1.3).
//!
//! What makes the music is here; what belongs to the screen or the machine is in
//! [`crate::SessionState`], never in these types. Every durable entity is keyed by a
//! stable id in a map of its parent; the order of tracks, scenes and devices is a
//! fractional [`OrderKey`] carried by each element. A [`Project`] read from text is
//! checked by [`Project::validate`] before use.

use crate::blob::Hash;
use crate::ids::{
    ClipId, DeviceId, Id, LaneId, MarkerId, PlacementId, ProjectId, SceneId, TrackId,
};
use crate::order::{OrderKey, by_order};
use crate::preset::{DevicePreset, Preset, PresetId};
use crate::serde_util::{Rgb, db, finite, finite_map, unique_map, unique_set, unique_tick_map};
use engine::time::{Frame, LoopRegion, Quantize, SampleRate, Tempo, Tick, TimeSignature};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The on-disk format version this crate reads and writes (§ 2.6). Format 2 (#473): a
/// placement longer than a MIDI clip without a loop repeats the clip (format 1: silence).
/// Format 3 (#490): a track may hold a row `height`, so that a build of format 2, which
/// would refuse the field as corrupt, says the folder is from a newer build instead.
/// Format 4 (#191): a device may name the preset it was set from (`preset`), and the folder
/// may hold presets (`presets/<id>.json`); a build of format 3 would refuse the device field
/// as corrupt.
/// Format 5 (#695): the project may hold a key and scale (`scale`); a build of format 4 would
/// refuse the field as corrupt.
/// Format 6 (#219): a track's chain may hold MIDI effects (role `midi_effect`), before its
/// instrument; a build of format 5 would refuse the role as corrupt.
/// Format 7 (#881): an automation point may have the shape `ease_in`, `ease_out` or `smooth`
/// (a lane file's word, a lane blob's byte 2, 3 or 4); a build of format 6 would refuse it as
/// corrupt.
pub const FORMAT_VERSION: u32 = 7;

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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    /// The project's key and scale (#695). Absent (not written) when there is none: the project
    /// is chromatic and every tool behaves as without a scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<crate::scale::Scale>,
    /// The rows of the launch grid.
    #[serde(deserialize_with = "unique_map")]
    pub scenes: BTreeMap<SceneId, Scene>,
    // The master bus. Not a doc comment: a description beside its `$ref` would make the
    // generated TypeScript declare a copy of `Mixer`.
    pub master: Mixer,
    // The master's chain: effects only (#364), ordered by `Device::order`, run on the sum of
    // the tracks before the master's fader. Not a doc comment, like `master` above: a
    // description beside the map would make the schema inline a copy of `Device`.
    #[serde(default, deserialize_with = "unique_map")]
    #[schemars(required)]
    pub master_devices: BTreeMap<DeviceId, Device>,
    // The master's automation lanes (#378): the master's gain and pan, and the parameters of
    // the devices of `master_devices`. Not a doc comment, like `master_devices` above.
    #[serde(default, deserialize_with = "unique_map")]
    #[schemars(required)]
    pub master_automation: BTreeMap<LaneId, AutomationLane>,
    /// The tracks.
    #[serde(deserialize_with = "unique_map")]
    pub tracks: BTreeMap<TrackId, Track>,
    // The named section markers of the arrangement (#473), by id; the page shows them in
    // (tick, id) order. Absent from a project that has none, so its file is unchanged. Not a
    // doc comment, like `master_devices`: a description beside the map would inline a copy.
    #[serde(
        default,
        deserialize_with = "unique_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub markers: BTreeMap<MarkerId, Marker>,
    // The presets saved in the project (#191): one file each under `presets/`, so they travel
    // with the project. Absent from a project that has none, so its file is unchanged. Not a
    // doc comment, like `master_devices`: a description beside the map would inline a copy.
    #[serde(
        default,
        deserialize_with = "unique_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub presets: BTreeMap<PresetId, Preset>,
}

/// A named section marker: a point of the timeline the arrangement shows on its ruler and
/// the keyboard jumps to (#473).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    /// Where it stands on the timeline, in ticks.
    pub tick: Tick,
    /// Its name: trimmed, not empty, at most 200 characters.
    pub name: String,
}

/// A tempo from a tick on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TempoPoint {
    pub tick: Tick,
    pub bpm: Tempo,
}

/// A time signature from a tick on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeterPoint {
    pub tick: Tick,
    pub signature: TimeSignature,
}

/// The transport settings saved with the project (§ 1.3). Play state and the playhead
/// are ephemeral and never saved.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Metronome {
    pub enabled: bool,
    /// Level in dB; `-inf` is written `null`.
    #[serde(with = "db")]
    #[schemars(with = "Option<f64>")]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// The height of the track's row in the arrangement, in pixels, from
    /// [`MIN_TRACK_HEIGHT`] to [`MAX_TRACK_HEIGHT`] (#490). `None` is the page's default
    /// height (its density's). A presentation choice that belongs to the project, so that a
    /// producer finds it again on reopening: never read by the engine. Not written when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u16>,
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

/// The lowest finite gain of a track, the master, the metronome or a gain automation point,
/// in dB (MX-2, #269). Below it, a track, master or metronome gain is silence (`null`); a
/// gain automation point has no silence. Shared by the document
/// ([`crate::Project::validate`]) and the commands.
pub const MIN_GAIN_DB: f64 = -96.0;
/// The highest finite gain of a track, the master, the metronome or a gain automation point,
/// in dB (MX-2, #269).
pub const MAX_GAIN_DB: f64 = 6.0;

/// Lowest height of a track's row, in pixels (#490): the page's compact row.
pub const MIN_TRACK_HEIGHT: u16 = 28;
/// Highest height of a track's row, in pixels (#490).
pub const MAX_TRACK_HEIGHT: u16 = 176;

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
    /// The preset the device was last set from (#191), by `preset.load` or `preset.save`;
    /// absent when none, so that a device without one writes the same file as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<Box<DevicePreset>>,
}

/// What a device does in the chain. A track's chain is, in order, its MIDI effects, its
/// instrument (at most one), then its audio effects; the master's holds audio effects only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    /// Notes in, sound out: the sound of the track.
    Instrument,
    /// An audio effect: sound in, sound out, after the instrument.
    Effect,
    /// Notes in, notes out, before the instrument, on a MIDI track only (#219).
    MidiEffect,
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
    #[serde(deserialize_with = "unique_tick_map")]
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

/// The shape of the transition from a point to the next one (#881). `Linear` is a straight
/// line along the parameter's knob (in its unit, in the logarithm of the value for a
/// logarithmic parameter), along the fader's travel for a volume lane (#389); `Step`
/// holds the value until the next point; `EaseIn` (slow, then fast), `EaseOut` (fast, then
/// slow) and `Smooth` (an S, slow at both ends) follow the same line warped in time, by
/// `t²`, `t (2 − t)` and `t² (3 − 2t)` of the progress `t` (`engine::automation::warp`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    #[default]
    Linear,
    Step,
    EaseIn,
    EaseOut,
    Smooth,
}

impl Shape {
    /// Every shape, in the order the interface lists them.
    pub const ALL: [Shape; 5] = [
        Shape::Linear,
        Shape::Step,
        Shape::EaseIn,
        Shape::EaseOut,
        Shape::Smooth,
    ];

    /// The curve the engine plays for this shape.
    pub fn curve(self) -> engine::msg::Curve {
        use engine::msg::Curve;
        match self {
            Shape::Linear => Curve::Linear,
            Shape::Step => Curve::Step,
            Shape::EaseIn => Curve::EaseIn,
            Shape::EaseOut => Curve::EaseOut,
            Shape::Smooth => Curve::Smooth,
        }
    }

    /// The word of the shape in a lane file and in JSON (`linear`, `step`, `ease_in`,
    /// `ease_out`, `smooth`).
    pub fn keyword(self) -> &'static str {
        match self {
            Shape::Linear => "linear",
            Shape::Step => "step",
            Shape::EaseIn => "ease_in",
            Shape::EaseOut => "ease_out",
            Shape::Smooth => "smooth",
        }
    }

    /// The shape of a keyword ([`keyword`](Self::keyword)), if any.
    pub fn from_keyword(word: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|s| s.keyword() == word)
    }

    /// The byte of the shape in a lane's blob: 0 linear, 1 step, 2 ease in, 3 ease out, 4
    /// smooth.
    pub fn byte(self) -> u8 {
        match self {
            Shape::Linear => 0,
            Shape::Step => 1,
            Shape::EaseIn => 2,
            Shape::EaseOut => 3,
            Shape::Smooth => 4,
        }
    }

    /// The shape of a blob byte ([`byte`](Self::byte)), if any.
    pub fn from_byte(byte: u8) -> Option<Shape> {
        Shape::ALL.into_iter().find(|s| s.byte() == byte)
    }
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
            scale: None,
            scenes: BTreeMap::new(),
            master: Mixer::default(),
            master_devices: BTreeMap::new(),
            master_automation: BTreeMap::new(),
            tracks: BTreeMap::new(),
            markers: BTreeMap::new(),
            presets: BTreeMap::new(),
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
            || self.presets.keys().any(|k| k.0 == *id)
            || self.markers.keys().any(|k| k.0 == *id)
            || self.master_devices.keys().any(|k| k.0 == *id)
            || self.master_automation.keys().any(|k| k.0 == *id)
            || self.tracks.iter().any(|(k, t)| {
                k.0 == *id
                    || t.devices.keys().any(|k| k.0 == *id)
                    || t.clips.keys().any(|k| k.0 == *id)
                    || t.placements.keys().any(|k| k.0 == *id)
                    || t.automation.keys().any(|k| k.0 == *id)
            })
    }

    /// The lanes of a track (`Some`) or of the master (`None`), if the track exists.
    pub fn lanes(&self, owner: Option<&TrackId>) -> Option<&BTreeMap<LaneId, AutomationLane>> {
        match owner {
            None => Some(&self.master_automation),
            Some(t) => self.tracks.get(t).map(|t| &t.automation),
        }
    }

    /// [`Project::lanes`], to change them.
    pub fn lanes_mut(
        &mut self,
        owner: Option<&TrackId>,
    ) -> Option<&mut BTreeMap<LaneId, AutomationLane>> {
        match owner {
            None => Some(&mut self.master_automation),
            Some(t) => self.tracks.get_mut(t).map(|t| &mut t.automation),
        }
    }

    /// The owner of a lane: `Some(Some(track))` for a track's, `Some(None)` for the master's,
    /// `None` when no lane has this id.
    pub fn lane_owner(&self, lane: &LaneId) -> Option<Option<TrackId>> {
        if self.master_automation.contains_key(lane) {
            return Some(None);
        }
        self.tracks
            .iter()
            .find(|(_, t)| t.automation.contains_key(lane))
            .map(|(id, _)| Some(id.clone()))
    }

    /// The master's chain in order (order key, then id).
    pub fn master_devices_in_order(&self) -> Vec<(&DeviceId, &Device)> {
        by_order(&self.master_devices, |d| &d.order)
    }

    /// The device with this id, in a track's chain or in the master's. Device ids are
    /// unique across the project ([`Project::contains_id`]).
    pub fn device(&self, id: &DeviceId) -> Option<&Device> {
        self.master_devices
            .get(id)
            .or_else(|| self.tracks.values().find_map(|t| t.devices.get(id)))
    }

    /// [`Project::device`], to change it.
    pub fn device_mut(&mut self, id: &DeviceId) -> Option<&mut Device> {
        if self.master_devices.contains_key(id) {
            return self.master_devices.get_mut(id);
        }
        self.tracks.values_mut().find_map(|t| t.devices.get_mut(id))
    }

    /// The tracks in display order (order key, then id).
    pub fn tracks_in_order(&self) -> Vec<(&TrackId, &Track)> {
        by_order(&self.tracks, |t| &t.order)
    }

    /// How many clips, placements and lanes the whole project has: the counts the
    /// engine's slot tables bound project-wide ([`crate::limits::MAX_CLIPS`]…).
    pub fn counts(&self) -> Counts {
        let counts = Counts {
            lanes: self.master_automation.len(),
            ..Counts::default()
        };
        self.tracks.values().fold(counts, |c, t| Counts {
            clips: c.clips + t.clips.len(),
            placements: c.placements + t.placements.len(),
            lanes: c.lanes + t.automation.len(),
        })
    }

    /// The markers in timeline order (tick, then id).
    pub fn markers_in_order(&self) -> Vec<(&MarkerId, &Marker)> {
        let mut out: Vec<_> = self.markers.iter().collect();
        out.sort_by(|a, b| (a.1.tick, a.0).cmp(&(b.1.tick, b.0)));
        out
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
            height: None,
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
