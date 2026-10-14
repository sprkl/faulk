//! Project → engine synchronisation contract (CT-3, `docs/specs/02-decoupage-mvp.md` § 2.3).
//!
//! This module declares the types of the contract and the binary header; it holds no
//! engine logic. The payload codecs, `Engine::prepare`, `Engine::apply` and the rings are
//! MO-1's; the channel of each target (helper thread + `rtrb`, worklet + shared memory) is
//! MO-2's. Until then nothing in the engine consumes these types.
//!
//! # The three message kinds
//!
//! 1. [`ControlMsg`], what the project sends to the engine: created by the command layer
//!    (`crates/commands`, in Rust, never in TypeScript), encoded in the binary format below,
//!    carried from the page to the *prepare side* (a Worker in web mode, the helper's control
//!    thread in Boost mode). Every structure it names is an engine **slot** (`u32` index)
//!    assigned by the project, which keeps the correspondence with its stable identifiers
//!    (§ 2.4): the engine never sees a `TrackId`.
//! 2. [`RtMsg`], what `Engine::prepare(ControlMsg)` produces **off the audio path** (it may
//!    allocate) and `Engine::apply(RtMsg)` consumes **on the audio path** (it never
//!    allocates): a Rust value, never serialized, moved through a lock-free single-producer
//!    single-consumer ring. Natively the ring is `rtrb`; in the web build the same Rust ring
//!    lives inside the shared linear memory, written by the prepare Worker's instance and
//!    read by the worklet's instance (CT-3's measured choice, § 2.3).
//! 3. [`Feedback`], what the engine reports to the page about 30 times per second:
//!    playhead, transport state, clip states, meters, load, counters. A fixed
//!    little-endian layout written by the audio thread into a region the page reads
//!    without stopping the engine, or sent as bytes by the helper.
//!
//! # Growth without a visible cap (decision 7)
//!
//! The engine has no fixed capacities. Every table ([`Table`]) is a boxed slice with a
//! length: pushing into a full table fails with [`TableFull`] and never allocates. The cycle:
//!
//! 1. `prepare` finds the table full → it builds the next one off the audio path, with twice
//!    the capacity, through a **fallible** allocation (`try_reserve`, never
//!    `handle_alloc_error`): a refused allocation (`memory.grow` refused, machine out of
//!    memory) is [`MsgError::OutOfMemory`], reported to the project and shown, never a trap
//!    nor a silence;
//! 2. the next table travels in [`RtMsg::Swap`] as a [`Prepared`] value;
//! 3. `apply` moves the items of the old table into the new one ([`Table::take_from`],
//!    no allocation), swaps the two in one block, and pushes the old table as [`Garbage`]
//!    on the second ring;
//! 4. the prepare side drops the garbage. Nothing is ever freed on the audio thread.
//!
//! Invariant that keeps `apply` from ever dropping: the garbage ring has the capacity of the
//! rt ring ([`limits::RT_RING_CAPACITY`]) and `prepare` drains the garbage ring before
//! each push, so for every `RtMsg` in flight there is a free garbage slot.
//!
//! Above the tables stand the **safety limits** of [`limits`], very high and checked by the
//! project before a message is sent ([`MsgError::LimitExceeded`]): the engine itself never
//! fails in real time. They are sized so that the whole engine, every table at its limit
//! and doubled during a growth, plus the PCM budget of the audio tracks, stays under the
//! 4 GiB of wasm32 (test `limits_fit_under_the_wasm32_cap`).
//!
//! # Linear memory (web)
//!
//! The module reserves an initial memory and grows it only in `prepare` (the Worker),
//! never in `process`. The memory is shared: a `memory.grow` leaves the worklet's existing
//! views valid (they keep their length); the worklet re-takes its views at the start of a
//! block when `memory.buffer` changed, a constant-cost comparison.
//!
//! # Live input
//!
//! Clip and automation events never cross a bounded queue: the engine reads them in their
//! tables while rendering. Only the live input (MIDI, keyboard: [`ControlMsg::NoteOn`],
//! [`ControlMsg::NoteOff`]) is queued, in [`limits::LIVE_QUEUE_CAPACITY`] slots, far beyond
//! what a human produces between two blocks. On overflow the event is dropped and counted
//! ([`Feedback`]`.live_overflows`); the producer then sends [`RtMsg::AllNotesOff`] for that
//! track as its first message once a slot frees, so no note stays stuck.
//!
//! # Binary format of `ControlMsg`
//!
//! Little-endian everywhere, whatever the host. A message is a [`Header`] of
//! [`HEADER_BYTES`] bytes followed by `len` payload bytes; the next message starts at the
//! next multiple of [`MSG_ALIGN`]. A stream holds any number of messages back to back.
//!
//! ```text
//! offset  size  field
//!      0     1  version   = MSG_VERSION
//!      1     1  flags     = 0 (reserved, must be zero)
//!      2     2  tag       = Tag, the ControlMsg variant
//!      4     2  epoch     = engine generation (see below)
//!      6     2  reserved  = 0
//!      8     4  len       = payload length in bytes, <= MAX_CONTROL_MSG_BYTES
//!     12     4  seq       = sequence number, +1 per message, wraps
//! ```
//!
//! - An unknown `tag` does not stop the stream: the receiver skips the message by its `len`
//!   and counts it ([`Feedback`]`.rejected`). A wrong version or a `len` over the limit is
//!   a corrupt stream: [`MsgError::UnsupportedVersion`], [`MsgError::TooLarge`].
//! - `epoch` is the engine generation held by the page. When the page switches engine (web
//!   ↔ helper, or a worklet restarted after an error), it increments the epoch and resends
//!   the whole project to the new engine with the new epoch; a message of an older epoch
//!   still in flight is dropped ([`MsgError::StaleEpoch`]), so an engine switch in the
//!   middle of a message sequence never applies half of it to the wrong engine.
//! - Payloads larger than [`limits::MAX_CONTROL_MSG_BYTES`] are split by the sender: notes
//!   and automation points go in chunks ([`ControlMsg::ClipNotesUpload`],
//!   [`ControlMsg::AutomationLaneUpload`] carry `first` and `total`).
//! - The PCM of the audio tracks never goes through a message: AU-3 streams it into buffers
//!   that [`ControlMsg::AudioSourceDeclare`] only declares.

/// Version of the `ControlMsg` binary format ([`Header::version`]).
pub const MSG_VERSION: u8 = 1;

/// Version of the [`Feedback`] layout.
pub const FEEDBACK_VERSION: u8 = 1;

/// Size of a [`Header`] in bytes.
pub const HEADER_BYTES: usize = 16;

/// Alignment of a message start in a stream, in bytes.
pub const MSG_ALIGN: usize = 4;

/// Position of a message in the project's time, in ticks (`engine::time::Tick`, 960 per
/// quarter note, CT-2). The wire representation, so that this module does not depend on
/// CT-2's merge; the project converts.
pub type TickRaw = i64;

/// Position in an audio file, in frames (`engine::time::Frame`, CT-2); wire representation.
pub type FrameRaw = u64;

/// The safety limits: very high, enforced by the project before a message is sent, never
/// reached by a human, and sized together under the wasm32 cap (see the module doc).
pub mod limits {
    /// Tracks in a project.
    pub const MAX_TRACKS: u32 = 4096;
    /// Devices in one track's chain.
    pub const MAX_DEVICES_PER_TRACK: u32 = 64;
    /// Clips in a project (grid and arrangement together).
    pub const MAX_CLIPS: u32 = 65_536;
    /// Placements in the arrangement.
    pub const MAX_PLACEMENTS: u32 = 1_048_576;
    /// Notes in a project, every clip together.
    pub const MAX_NOTES: u32 = 16_777_216;
    /// Notes in one clip.
    pub const MAX_NOTES_PER_CLIP: u32 = 1_048_576;
    /// Automation lanes in a project.
    pub const MAX_LANES: u32 = 65_536;
    /// Automation points in a project, every lane together.
    pub const MAX_AUTOMATION_POINTS: u32 = 16_777_216;
    /// Audio sources (files) declared to the engine.
    pub const MAX_AUDIO_SOURCES: u32 = 4096;
    /// Scenes of the launch grid.
    pub const MAX_SCENES: u32 = 1024;
    /// Memory the streaming buffers of the audio tracks may hold together (AU-3).
    pub const PCM_BUDGET_BYTES: u64 = 1 << 30;
    /// Payload of one `ControlMsg`; larger uploads are chunked.
    pub const MAX_CONTROL_MSG_BYTES: u32 = 16 << 20;
    /// Slots of the live input queue (notes played live).
    pub const LIVE_QUEUE_CAPACITY: usize = 4096;
    /// Slots of the `RtMsg` ring and of the garbage ring.
    pub const RT_RING_CAPACITY: usize = 4096;
    /// Most `RtMsg` applied at the start of one block; the rest wait for the next block.
    pub const MAX_APPLY_PER_BLOCK: usize = 256;
    /// Largest render block, in frames (Web Audio allows up to 2048).
    pub const MAX_BLOCK_FRAMES: usize = 2048;
    /// Initial size of the web module's linear memory, in 64 KiB pages (16 MiB).
    pub const WASM_INITIAL_PAGES: u32 = 256;
    /// The cap of a wasm32 linear memory: 65 536 pages of 64 KiB.
    pub const WASM_MAX_BYTES: u64 = 1 << 32;
}

// ---------------------------------------------------------------- slots and payload types

/// Index of a track in the engine, assigned by the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackSlot(pub u32);

/// Position of a device in a track's chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceSlot(pub u16);

/// Index of a clip in the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipSlot(pub u32);

/// Index of a placement (a clip in the arrangement) in the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlacementSlot(pub u32);

/// Index of a scene of the launch grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SceneSlot(pub u16);

/// Index of an automation lane in the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LaneSlot(pub u32);

/// Index of an audio source (a file's PCM, streamed by AU-3) in the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceSlot(pub u32);

/// Identifier of a parameter inside a device (`Param::id` for the synth).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParamId(pub u32);

/// Kind of a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TrackKind {
    Midi = 0,
    Audio = 1,
}

/// Kind of a device, as the device registry of CT-5 numbers them (`"faulk.osc_synth"` → id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceKind(pub u32);

/// A note of a MIDI clip, as the project stores it (no identifier, CT-4 decides one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Note {
    pub start: TickRaw,
    pub length: TickRaw,
    pub pitch: u8,
    pub velocity: u8,
}

/// Shape of the transition after an automation point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Curve {
    Step = 0,
    Linear = 1,
}

/// A point of an automation lane; at most one per tick.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct AutomationPoint {
    pub tick: TickRaw,
    pub value: f32,
    pub curve: Curve,
}

/// What an automation lane drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationTarget {
    DeviceParam { device: DeviceSlot, param: ParamId },
    MixerGain,
    MixerPan,
}

/// Loop region `[start, end)` in ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRaw {
    pub start: TickRaw,
    pub end: TickRaw,
}

/// Launch quantization, as the project encodes `engine::time::Quantize` (CT-2) on the wire:
/// `0` none, otherwise the length in ticks.
pub type QuantizeRaw = u32;

// ---------------------------------------------------------------- ControlMsg

/// What the project sends to the engine. One variant per MVP action, including the ones
/// whose implementation comes later (automation, audio sources): the list is the contract.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlMsg {
    /// Forgets everything: tracks, clips, sources, lanes. Sent before a full resend.
    Reset,
    /// Tempo (BPM, 20–999), time signature, launch quantization.
    SetTransport {
        tempo_bpm: f64,
        numerator: u8,
        denominator: u8,
        launch_quantize: QuantizeRaw,
    },
    // tracks
    TrackCreate {
        track: TrackSlot,
        kind: TrackKind,
    },
    TrackRemove {
        track: TrackSlot,
    },
    TrackSetMix {
        track: TrackSlot,
        gain: f32,
        pan: f32,
        mute: bool,
    },
    /// Solo is session state (decision 10) but the engine renders it.
    TrackSetSolo {
        track: TrackSlot,
        solo: bool,
    },
    /// Arms a track: the live input also reaches it.
    TrackArm {
        track: TrackSlot,
        armed: bool,
    },
    // devices
    DeviceInsert {
        track: TrackSlot,
        device: DeviceSlot,
        kind: DeviceKind,
        /// Opaque initial state of the device (its parameters, CT-5).
        state: Vec<u8>,
    },
    DeviceRemove {
        track: TrackSlot,
        device: DeviceSlot,
    },
    DeviceSetEnabled {
        track: TrackSlot,
        device: DeviceSlot,
        enabled: bool,
    },
    /// A parameter set by hand or by an agent (not by automation).
    ParamSet {
        track: TrackSlot,
        device: DeviceSlot,
        param: ParamId,
        value: f32,
    },
    // MIDI clips
    /// The notes of a clip, in chunks: `first` is the index of `notes[0]` among `total`.
    /// The engine swaps the clip's table in once the last chunk arrived.
    ClipNotesUpload {
        clip: ClipSlot,
        length: TickRaw,
        looped: bool,
        first: u32,
        total: u32,
        notes: Vec<Note>,
    },
    ClipRemove {
        clip: ClipSlot,
    },
    // audio clips and sources
    /// Declares a source whose PCM AU-3 streams into the engine's buffers.
    AudioSourceDeclare {
        source: SourceSlot,
        sample_rate: u32,
        channels: u8,
        frames: FrameRaw,
    },
    AudioSourceRelease {
        source: SourceSlot,
    },
    /// Makes a clip an audio clip: a range of a source.
    ClipAudioSet {
        clip: ClipSlot,
        source: SourceSlot,
        start: FrameRaw,
        length: FrameRaw,
    },
    // arrangement
    ClipPlace {
        track: TrackSlot,
        placement: PlacementSlot,
        clip: ClipSlot,
        start: TickRaw,
        offset: TickRaw,
        length: TickRaw,
    },
    ClipUnplace {
        placement: PlacementSlot,
    },
    // automation
    /// The points of a lane, in chunks like the notes.
    AutomationLaneUpload {
        lane: LaneSlot,
        track: TrackSlot,
        target: AutomationTarget,
        first: u32,
        total: u32,
        points: Vec<AutomationPoint>,
    },
    AutomationLaneRemove {
        lane: LaneSlot,
    },
    // launch grid
    SceneSetSlot {
        track: TrackSlot,
        scene: SceneSlot,
        clip: Option<ClipSlot>,
    },
    LaunchClip {
        track: TrackSlot,
        scene: SceneSlot,
    },
    LaunchScene {
        scene: SceneSlot,
    },
    StopTrack {
        track: TrackSlot,
    },
    StopAll,
    // transport
    Play {
        from: TickRaw,
    },
    Stop,
    Seek {
        tick: TickRaw,
    },
    SetLoop {
        region: Option<LoopRaw>,
    },
    SetRecording {
        recording: bool,
    },
    // live input (the only messages that go through the bounded live queue)
    NoteOn {
        track: TrackSlot,
        pitch: u8,
        velocity: u8,
    },
    NoteOff {
        track: TrackSlot,
        pitch: u8,
    },
    AllNotesOff {
        track: TrackSlot,
    },
}

/// The `tag` of a [`Header`]: one value per [`ControlMsg`] variant, stable across versions
/// (a new variant takes a new number, a removed one leaves a hole).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Tag {
    Reset = 0,
    SetTransport = 1,
    TrackCreate = 10,
    TrackRemove = 11,
    TrackSetMix = 12,
    TrackSetSolo = 13,
    TrackArm = 14,
    DeviceInsert = 20,
    DeviceRemove = 21,
    DeviceSetEnabled = 22,
    ParamSet = 23,
    ClipNotesUpload = 30,
    ClipRemove = 31,
    AudioSourceDeclare = 40,
    AudioSourceRelease = 41,
    ClipAudioSet = 42,
    ClipPlace = 50,
    ClipUnplace = 51,
    AutomationLaneUpload = 60,
    AutomationLaneRemove = 61,
    SceneSetSlot = 70,
    LaunchClip = 71,
    LaunchScene = 72,
    StopTrack = 73,
    StopAll = 74,
    Play = 80,
    Stop = 81,
    Seek = 82,
    SetLoop = 83,
    SetRecording = 84,
    NoteOn = 90,
    NoteOff = 91,
    AllNotesOff = 92,
}

impl Tag {
    /// Every tag, in declaration order (for exhaustive tests and generated tables).
    pub const ALL: [Tag; 33] = [
        Tag::Reset,
        Tag::SetTransport,
        Tag::TrackCreate,
        Tag::TrackRemove,
        Tag::TrackSetMix,
        Tag::TrackSetSolo,
        Tag::TrackArm,
        Tag::DeviceInsert,
        Tag::DeviceRemove,
        Tag::DeviceSetEnabled,
        Tag::ParamSet,
        Tag::ClipNotesUpload,
        Tag::ClipRemove,
        Tag::AudioSourceDeclare,
        Tag::AudioSourceRelease,
        Tag::ClipAudioSet,
        Tag::ClipPlace,
        Tag::ClipUnplace,
        Tag::AutomationLaneUpload,
        Tag::AutomationLaneRemove,
        Tag::SceneSetSlot,
        Tag::LaunchClip,
        Tag::LaunchScene,
        Tag::StopTrack,
        Tag::StopAll,
        Tag::Play,
        Tag::Stop,
        Tag::Seek,
        Tag::SetLoop,
        Tag::SetRecording,
        Tag::NoteOn,
        Tag::NoteOff,
        Tag::AllNotesOff,
    ];

    /// The tag of a wire value, `None` for a number this version does not know.
    pub fn from_u16(value: u16) -> Option<Tag> {
        Tag::ALL.iter().copied().find(|tag| *tag as u16 == value)
    }

    /// Whether the message goes through the live input queue rather than `prepare`.
    pub fn is_live(self) -> bool {
        matches!(self, Tag::NoteOn | Tag::NoteOff | Tag::AllNotesOff)
    }
}

impl ControlMsg {
    /// The tag written in the header of this message.
    pub fn tag(&self) -> Tag {
        match self {
            ControlMsg::Reset => Tag::Reset,
            ControlMsg::SetTransport { .. } => Tag::SetTransport,
            ControlMsg::TrackCreate { .. } => Tag::TrackCreate,
            ControlMsg::TrackRemove { .. } => Tag::TrackRemove,
            ControlMsg::TrackSetMix { .. } => Tag::TrackSetMix,
            ControlMsg::TrackSetSolo { .. } => Tag::TrackSetSolo,
            ControlMsg::TrackArm { .. } => Tag::TrackArm,
            ControlMsg::DeviceInsert { .. } => Tag::DeviceInsert,
            ControlMsg::DeviceRemove { .. } => Tag::DeviceRemove,
            ControlMsg::DeviceSetEnabled { .. } => Tag::DeviceSetEnabled,
            ControlMsg::ParamSet { .. } => Tag::ParamSet,
            ControlMsg::ClipNotesUpload { .. } => Tag::ClipNotesUpload,
            ControlMsg::ClipRemove { .. } => Tag::ClipRemove,
            ControlMsg::AudioSourceDeclare { .. } => Tag::AudioSourceDeclare,
            ControlMsg::AudioSourceRelease { .. } => Tag::AudioSourceRelease,
            ControlMsg::ClipAudioSet { .. } => Tag::ClipAudioSet,
            ControlMsg::ClipPlace { .. } => Tag::ClipPlace,
            ControlMsg::ClipUnplace { .. } => Tag::ClipUnplace,
            ControlMsg::AutomationLaneUpload { .. } => Tag::AutomationLaneUpload,
            ControlMsg::AutomationLaneRemove { .. } => Tag::AutomationLaneRemove,
            ControlMsg::SceneSetSlot { .. } => Tag::SceneSetSlot,
            ControlMsg::LaunchClip { .. } => Tag::LaunchClip,
            ControlMsg::LaunchScene { .. } => Tag::LaunchScene,
            ControlMsg::StopTrack { .. } => Tag::StopTrack,
            ControlMsg::StopAll => Tag::StopAll,
            ControlMsg::Play { .. } => Tag::Play,
            ControlMsg::Stop => Tag::Stop,
            ControlMsg::Seek { .. } => Tag::Seek,
            ControlMsg::SetLoop { .. } => Tag::SetLoop,
            ControlMsg::SetRecording { .. } => Tag::SetRecording,
            ControlMsg::NoteOn { .. } => Tag::NoteOn,
            ControlMsg::NoteOff { .. } => Tag::NoteOff,
            ControlMsg::AllNotesOff { .. } => Tag::AllNotesOff,
        }
    }

    /// Checks the message against the safety limits, as the project does before sending.
    /// Only the fields a limit bounds are checked; the payload codecs check the rest.
    pub fn check_limits(&self) -> Result<(), MsgError> {
        use limits::*;
        fn below(what: &'static str, value: u64, limit: u64) -> Result<(), MsgError> {
            if value < limit {
                Ok(())
            } else {
                Err(MsgError::LimitExceeded {
                    what,
                    limit,
                    requested: value,
                })
            }
        }
        match self {
            ControlMsg::TrackCreate { track, .. }
            | ControlMsg::TrackRemove { track }
            | ControlMsg::TrackSetMix { track, .. }
            | ControlMsg::TrackSetSolo { track, .. }
            | ControlMsg::TrackArm { track, .. }
            | ControlMsg::StopTrack { track }
            | ControlMsg::NoteOn { track, .. }
            | ControlMsg::NoteOff { track, .. }
            | ControlMsg::AllNotesOff { track } => {
                below("tracks", track.0.into(), MAX_TRACKS.into())
            }
            ControlMsg::DeviceInsert { track, device, .. }
            | ControlMsg::DeviceRemove { track, device }
            | ControlMsg::DeviceSetEnabled { track, device, .. }
            | ControlMsg::ParamSet { track, device, .. } => {
                below("tracks", track.0.into(), MAX_TRACKS.into())?;
                below(
                    "devices per track",
                    device.0.into(),
                    MAX_DEVICES_PER_TRACK.into(),
                )
            }
            ControlMsg::ClipNotesUpload {
                clip, first, total, ..
            } => {
                below("clips", clip.0.into(), MAX_CLIPS.into())?;
                below(
                    "notes per clip",
                    u64::from(*first) + u64::from(*total),
                    u64::from(MAX_NOTES_PER_CLIP) + 1,
                )
            }
            ControlMsg::ClipRemove { clip } => below("clips", clip.0.into(), MAX_CLIPS.into()),
            ControlMsg::AudioSourceDeclare { source, .. }
            | ControlMsg::AudioSourceRelease { source } => {
                below("audio sources", source.0.into(), MAX_AUDIO_SOURCES.into())
            }
            ControlMsg::ClipAudioSet { clip, source, .. } => {
                below("clips", clip.0.into(), MAX_CLIPS.into())?;
                below("audio sources", source.0.into(), MAX_AUDIO_SOURCES.into())
            }
            ControlMsg::ClipPlace {
                track,
                placement,
                clip,
                ..
            } => {
                below("tracks", track.0.into(), MAX_TRACKS.into())?;
                below("placements", placement.0.into(), MAX_PLACEMENTS.into())?;
                below("clips", clip.0.into(), MAX_CLIPS.into())
            }
            ControlMsg::ClipUnplace { placement } => {
                below("placements", placement.0.into(), MAX_PLACEMENTS.into())
            }
            ControlMsg::AutomationLaneUpload {
                lane,
                track,
                first,
                total,
                ..
            } => {
                below("lanes", lane.0.into(), MAX_LANES.into())?;
                below("tracks", track.0.into(), MAX_TRACKS.into())?;
                below(
                    "automation points",
                    u64::from(*first) + u64::from(*total),
                    u64::from(MAX_AUTOMATION_POINTS) + 1,
                )
            }
            ControlMsg::AutomationLaneRemove { lane } => {
                below("lanes", lane.0.into(), MAX_LANES.into())
            }
            ControlMsg::SceneSetSlot { track, scene, .. } | ControlMsg::LaunchClip { track, scene } => {
                below("tracks", track.0.into(), MAX_TRACKS.into())?;
                below("scenes", scene.0.into(), MAX_SCENES.into())
            }
            ControlMsg::LaunchScene { scene } => below("scenes", scene.0.into(), MAX_SCENES.into()),
            ControlMsg::Reset
            | ControlMsg::SetTransport { .. }
            | ControlMsg::StopAll
            | ControlMsg::Play { .. }
            | ControlMsg::Stop
            | ControlMsg::Seek { .. }
            | ControlMsg::SetLoop { .. }
            | ControlMsg::SetRecording { .. } => Ok(()),
        }
    }
}

// ---------------------------------------------------------------- header

/// The 16-byte header of a `ControlMsg` on the wire (layout in the module doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub tag: Tag,
    pub epoch: u16,
    pub len: u32,
    pub seq: u32,
}

/// What can go wrong with a message, typed: the engine never panics on input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgError {
    /// Fewer bytes than a header, or than `len` announces.
    Truncated { needed: usize, available: usize },
    /// A `version` this build does not read.
    UnsupportedVersion(u8),
    /// A `tag` this build does not know: skipped by `len`, counted, the stream goes on.
    UnknownTag { tag: u16, skip: usize },
    /// Reserved bits set: a corrupt stream.
    Corrupt,
    /// `len` above `MAX_CONTROL_MSG_BYTES`: a corrupt stream.
    TooLarge(u32),
    /// A message of a previous engine generation, dropped.
    StaleEpoch { message: u16, current: u16 },
    /// A safety limit the project must refuse before sending.
    LimitExceeded {
        what: &'static str,
        limit: u64,
        requested: u64,
    },
    /// `prepare` could not get the memory (allocation or `memory.grow` refused).
    OutOfMemory { bytes: usize },
    /// The rt ring is full: `prepare` must retry later, it never blocks.
    EngineBusy,
}

impl Header {
    /// Writes the header, little-endian.
    pub fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut out = [0u8; HEADER_BYTES];
        out[0] = self.version;
        out[2..4].copy_from_slice(&(self.tag as u16).to_le_bytes());
        out[4..6].copy_from_slice(&self.epoch.to_le_bytes());
        out[8..12].copy_from_slice(&self.len.to_le_bytes());
        out[12..16].copy_from_slice(&self.seq.to_le_bytes());
        out
    }

    /// Reads a header from the start of `bytes` and checks it against the current `epoch`.
    ///
    /// The checks go in this order: size, version, reserved bits, `len`, epoch, then the tag
    /// (an unknown tag comes last so that its `skip` is trustworthy).
    pub fn decode(bytes: &[u8], current_epoch: u16) -> Result<Header, MsgError> {
        if bytes.len() < HEADER_BYTES {
            return Err(MsgError::Truncated {
                needed: HEADER_BYTES,
                available: bytes.len(),
            });
        }
        let version = bytes[0];
        if version != MSG_VERSION {
            return Err(MsgError::UnsupportedVersion(version));
        }
        if bytes[1] != 0 || bytes[6] != 0 || bytes[7] != 0 {
            return Err(MsgError::Corrupt);
        }
        let len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if len > limits::MAX_CONTROL_MSG_BYTES {
            return Err(MsgError::TooLarge(len));
        }
        let epoch = u16::from_le_bytes([bytes[4], bytes[5]]);
        if epoch != current_epoch {
            return Err(MsgError::StaleEpoch {
                message: epoch,
                current: current_epoch,
            });
        }
        let seq = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let raw_tag = u16::from_le_bytes([bytes[2], bytes[3]]);
        let tag = Tag::from_u16(raw_tag).ok_or(MsgError::UnknownTag {
            tag: raw_tag,
            skip: Header::frame_len(len),
        })?;
        Ok(Header {
            version,
            tag,
            epoch,
            len,
            seq,
        })
    }

    /// Bytes from the start of this message to the start of the next one.
    pub fn frame_len(len: u32) -> usize {
        (HEADER_BYTES + len as usize).next_multiple_of(MSG_ALIGN)
    }
}

// ---------------------------------------------------------------- RtMsg and growth

/// A table of the engine: a boxed slice and a length. Built off the audio path, it never
/// allocates afterwards; when full, `prepare` builds a larger one and `apply` swaps them.
#[derive(Debug)]
pub struct Table<T> {
    items: Box<[T]>,
    len: usize,
}

/// `Table::push` on a full table: the caller asks `prepare` for a larger one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableFull;

impl<T: Default + Copy> Table<T> {
    /// Allocates a table of `capacity` slots, or fails typed when the memory is refused.
    pub fn try_with_capacity(capacity: usize) -> Result<Self, MsgError> {
        let mut items = Vec::new();
        items
            .try_reserve_exact(capacity)
            .map_err(|_| MsgError::OutOfMemory {
                bytes: capacity.saturating_mul(size_of::<T>()),
            })?;
        items.resize(capacity, T::default());
        Ok(Self {
            items: items.into_boxed_slice(),
            len: 0,
        })
    }

    /// The capacity `prepare` gives the next table: double, at least 1.
    pub fn next_capacity(&self) -> usize {
        (self.capacity() * 2).max(1)
    }
}

impl<T> Table<T> {
    pub fn capacity(&self) -> usize {
        self.items.len()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        self.len == self.items.len()
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len]
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.items[..self.len]
    }

    /// Appends without allocating; `Err(TableFull)` leaves the table unchanged.
    pub fn push(&mut self, item: T) -> Result<(), TableFull> {
        if self.is_full() {
            return Err(TableFull);
        }
        self.items[self.len] = item;
        self.len += 1;
        Ok(())
    }

    /// Moves the items of `old` into this (larger) table, on the audio path: no allocation,
    /// `old` keeps its storage and goes to the garbage. Items beyond this capacity stay in
    /// `old` (never happens when `prepare` doubled the capacity).
    pub fn take_from(&mut self, old: &mut Table<T>) {
        let n = old.len.min(self.items.len() - self.len);
        for (dst, src) in self.items[self.len..self.len + n]
            .iter_mut()
            .zip(old.items[..n].iter_mut())
        {
            core::mem::swap(dst, src);
        }
        self.len += n;
        old.len -= n;
        if old.len > 0 {
            old.items.rotate_left(n);
        }
    }
}

/// Which engine structure a [`Swap`] replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    Tracks,
    /// The notes of one clip.
    ClipNotes(ClipSlot),
    /// The points of one lane.
    LanePoints(LaneSlot),
    Placements,
    AudioSources,
    /// A device built by `prepare` (insert or replace).
    Device(TrackSlot, DeviceSlot),
}

/// A structure `prepare` built, carried into `apply` by [`RtMsg::Swap`]. The entry types
/// are placeholders that MO-1, MO-3 and AU-3 fill in (adding fields to an entry changes
/// nothing in this contract); a device is a boxed trait object of CT-5.
#[derive(Debug)]
pub enum Prepared {
    Tracks(Table<TrackEntry>),
    ClipNotes(Table<Note>),
    LanePoints(Table<AutomationPoint>),
    Placements(Table<PlacementEntry>),
    AudioSources(Table<SourceEntry>),
    Device(Box<dyn core::any::Any + Send>),
}

/// What `apply` sends back on the garbage ring, dropped on the prepare side only.
#[derive(Debug)]
pub enum Garbage {
    Tracks(Table<TrackEntry>),
    ClipNotes(Table<Note>),
    LanePoints(Table<AutomationPoint>),
    Placements(Table<PlacementEntry>),
    AudioSources(Table<SourceEntry>),
    Device(Box<dyn core::any::Any + Send>),
}

/// Placeholder entry of the track table (MO-3 fills it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackEntry {
    pub slot: u32,
}

/// Placeholder entry of the placements table (AR-1 fills it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlacementEntry {
    pub slot: u32,
}

/// Placeholder entry of the audio sources table (AU-3 fills it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceEntry {
    pub slot: u32,
}

/// Replaces one engine structure in one block.
#[derive(Debug)]
pub struct Swap {
    pub kind: TableKind,
    pub next: Prepared,
}

/// Transport order carried to the audio thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportMsg {
    Play { from: TickRaw },
    Stop,
    Seek { tick: TickRaw },
    SetLoop { region: Option<LoopRaw> },
    SetRecording { recording: bool },
}

/// Launch order carried to the audio thread (executed at the next quantization boundary).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMsg {
    Clip { track: TrackSlot, scene: SceneSlot },
    Scene { scene: SceneSlot },
    StopTrack { track: TrackSlot },
    StopAll,
}

/// What `prepare` hands to `apply`: a Rust value moved through the ring, never serialized.
/// `offset` is the frame in the next block where the message takes effect (0: block start).
#[derive(Debug)]
pub enum RtMsg {
    /// Replaces a structure (a grown table, a new device); the old one goes to [`Garbage`].
    Swap(Swap),
    /// Removes a structure; what it held goes to [`Garbage`].
    Remove(TableKind),
    ParamSet {
        track: TrackSlot,
        device: DeviceSlot,
        param: ParamId,
        value: f32,
        offset: u16,
    },
    TrackSetMix {
        track: TrackSlot,
        gain: f32,
        pan: f32,
        mute: bool,
    },
    TrackSetSolo {
        track: TrackSlot,
        solo: bool,
    },
    TrackArm {
        track: TrackSlot,
        armed: bool,
    },
    DeviceSetEnabled {
        track: TrackSlot,
        device: DeviceSlot,
        enabled: bool,
    },
    SceneSetSlot {
        track: TrackSlot,
        scene: SceneSlot,
        clip: Option<ClipSlot>,
    },
    ClipAudioSet {
        clip: ClipSlot,
        source: SourceSlot,
        start: FrameRaw,
        length: FrameRaw,
    },
    SetTransport {
        tempo_bpm: f64,
        numerator: u8,
        denominator: u8,
        launch_quantize: QuantizeRaw,
    },
    Transport(TransportMsg),
    Launch(LaunchMsg),
    /// Live note, from the live queue.
    NoteOn {
        track: TrackSlot,
        pitch: u8,
        velocity: u8,
        offset: u16,
    },
    NoteOff {
        track: TrackSlot,
        pitch: u8,
        offset: u16,
    },
    AllNotesOff {
        track: TrackSlot,
    },
}

// ---------------------------------------------------------------- Feedback

/// State of the transport in [`Feedback`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum TransportState {
    #[default]
    Stopped = 0,
    Playing = 1,
    Recording = 2,
}

/// State of a clip slot of the launch grid in [`Feedback`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ClipState {
    #[default]
    Empty = 0,
    Stopped = 1,
    Queued = 2,
    Playing = 3,
    Stopping = 4,
}

/// Meters of one track, linear amplitude.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct TrackMeter {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
}

/// Size of a [`TrackMeter`] on the wire.
pub const TRACK_METER_BYTES: usize = 16;

/// Size of the fixed part of a [`Feedback`] on the wire.
pub const FEEDBACK_HEADER_BYTES: usize = 48;

/// What the engine reports to the page, about 30 times per second. Fixed little-endian
/// layout, written by the audio thread into a region sized by [`Feedback::size_for`]:
///
/// ```text
/// offset size field
///      0    1 version = FEEDBACK_VERSION
///      1    1 transport (TransportState)
///      2    2 epoch
///      4    4 seq (+1 per report; a reader that sees it change while reading re-reads)
///      8    8 playhead_tick (i64)
///     16    8 playhead_frame (u64)
///     24    4 cpu_load (f32, 0..1, render time over block time)
///     28    4 live_overflows (u32, cumulative)
///     32    4 rejected (u32, cumulative: unknown tags, stale epochs)
///     36    4 disk_underruns (u32, cumulative, AU-3)
///     40    4 tracks (u32)
///     44    4 clip_slots (u32) = tracks × scenes
///     48      TrackMeter × tracks (16 bytes each)
///          then ClipState × clip_slots (1 byte each), padded to 4
/// ```
///
/// Recorded notes are not part of it: they go through the record take (RE epic), which
/// returns them to the page outside the 30 Hz report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feedback {
    pub transport: TransportState,
    pub epoch: u16,
    pub seq: u32,
    pub playhead_tick: TickRaw,
    pub playhead_frame: FrameRaw,
    pub cpu_load: f32,
    pub live_overflows: u32,
    pub rejected: u32,
    pub disk_underruns: u32,
    pub meters: Vec<TrackMeter>,
    pub clips: Vec<ClipState>,
}

impl Feedback {
    /// Bytes of a report for `tracks` tracks and `clip_slots` grid slots.
    pub fn size_for(tracks: usize, clip_slots: usize) -> usize {
        (FEEDBACK_HEADER_BYTES + tracks * TRACK_METER_BYTES + clip_slots).next_multiple_of(4)
    }
}
