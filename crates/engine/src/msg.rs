//! Project → engine synchronisation contract (CT-3, `docs/specs/02-decoupage-mvp.md` § 2.3).
//!
//! This module declares the types of the contract, the binary header of the control
//! stream and the growth mechanism ([`Table`]); it holds no engine logic. The payload
//! codecs, `Engine::prepare`, `Engine::apply` and the rings are MO-1's; the channel of each
//! target (helper thread + `rtrb`, prepare Worker + shared memory) is MO-2's. Until then
//! nothing in the engine consumes these types.
//!
//! # The three message kinds
//!
//! 1. [`ControlMsg`], what the project sends to the engine: produced by the command layer
//!    (`crates/commands`, in Rust, never in TypeScript), encoded in the binary format below,
//!    carried from the page to the *prepare side* (a Worker in web mode, the helper's control
//!    thread in Boost mode). Every structure it names is an engine **slot** (`u32` index)
//!    assigned by the project, which keeps the correspondence with its stable identifiers
//!    (§ 2.4): the engine never sees a `TrackId`.
//! 2. [`RtMsg`], what `Engine::prepare(ControlMsg)` produces **off the audio path** (it may
//!    allocate) and `Engine::apply(RtMsg)` consumes **on the audio path** (it never
//!    allocates nor frees): a Rust value, never serialized, moved through a lock-free
//!    single-producer single-consumer ring. Natively the ring is `rtrb`; in the web build
//!    the same Rust ring lives inside the shared linear memory, pushed by the prepare
//!    Worker's instance and popped by the worklet's instance (CT-3's measured choice, § 2.3).
//!    Only the prepare side pushes on this ring; the live input has its own queue.
//! 3. [`Feedback`], what the engine reports to the page about 30 times per second:
//!    playhead, transport state, active clips, meters, load, counters, the notes recorded
//!    since the last report. A fixed little-endian layout written by the audio thread into
//!    a region the page reads without stopping the engine, or sent as bytes by the helper.
//!
//! # From `ControlMsg` to `RtMsg`
//!
//! `prepare` turns each control message into **at most one** rt message (the garbage
//! invariant below depends on it):
//!
//! | `ControlMsg` | `RtMsg` |
//! |---|---|
//! | `Reset { epoch }` | `Swap(Everything)`: a fresh engine state, the old one is one garbage item |
//! | `SetTransport`, `SetMetronome`, `SetMaster` | the same, copied |
//! | `TrackCreate` | `Swap(Track)`, or `Swap(Tracks)` first when the track table is full |
//! | `TrackRemove` | `Remove(Track)`: the track owns its devices and buffers, one garbage item |
//! | `TrackSetMix/Solo/Arm/Monitor`, `DeviceSetEnabled`, `ParamSet`, `SceneSetSlot`, `ClipAudioSet` | the same, copied |
//! | `DeviceInsert` | `Swap(Device)` (built by `prepare`) |
//! | `DeviceRemove` | `Remove(Device)` |
//! | `ClipNotesUpload` (last chunk) | `Swap(ClipNotes)`: the whole table, built from the chunks |
//! | `AutomationLaneUpload` (last chunk) | `Swap(LanePoints)` |
//! | `TrackPlacementsUpload` (last chunk) | `Swap(TrackPlacements)` |
//! | `ClipRemove`, `AutomationLaneRemove`, `AudioSourceRelease` | `Remove(…)` |
//! | `AudioSourceDeclare` | `Swap(AudioSource)`: the streaming buffers, built by `prepare` (AU-3) |
//! | `LaunchClip/Scene`, `StopTrack/All`, `BackToArrangement` | `Launch(…)` |
//! | `Play`, `Stop`, `Seek`, `SetLoop`, `SetRecording` | `Transport(…)` |
//! | `NoteOn`, `NoteOff`, `AllNotesOff` | the live queue, never `prepare` |
//!
//! **Backpressure, no loss.** When the rt ring is full, the prepare side keeps the rt
//! message in its own ordered queue (it may allocate: it is off the audio path) and
//! pushes it as soon as a slot frees; control messages are never dropped nor reordered.
//! That queue is bounded too ([`limits::PENDING_QUEUE_CAPACITY`]): beyond it the engine
//! is not consuming (audio context suspended, device lost) and `prepare` returns
//! [`MsgError::EngineBusy`] to the project, which shows the stalled engine, stops sending,
//! and resyncs with a `Reset` once the engine runs again. In web mode the page knows its
//! `AudioContext` is suspended (background tab) and stops sending before that point. The
//! memory of the tables parked in the queue is bounded only by the fallible allocation of
//! `prepare` ([`MsgError::OutOfMemory`]), never by a trap.
//!
//! **Chunked uploads.** Notes, points and placements arrive in chunks (`first`, `total`)
//! staged on the prepare side per slot. A chunk whose `first` is not the staged length, or
//! whose `total` differs from the staged one, restarts the staging from that chunk and is
//! counted as rejected; the table is built and swapped only when `first + len == total`.
//! `Reset` and the `Remove` of the slot discard its staging. A partial upload never reaches
//! the audio side.
//!
//! # Growth without a visible cap (decision 7)
//!
//! The engine has no fixed capacities. Every table ([`Table`]) is a boxed slice with a
//! length: pushing into a full table fails with [`TableFull`] and never allocates. Two
//! kinds of tables, two growth paths:
//!
//! - **Content tables** (the notes of a clip, the points of a lane, the placements of a
//!   track) are immutable snapshots of the project's data: `prepare` builds the whole table
//!   from the upload and `apply` swaps a pointer. They never grow in place: an edit is a new
//!   upload. The cost on the audio thread does not depend on the table's size.
//! - **Runtime tables** (tracks, audio sources: state the audio thread mutates) are
//!   **slot arrays**: every slot exists (`len == capacity`, built by
//!   [`Table::try_filled`]), an empty slot holds `None`, and a message addresses a slot
//!   directly (the project assigns slots, reuses freed ones, never above the limit). When
//!   a message names a slot at or beyond the capacity, `prepare` first builds the next
//!   table, empty, with the capacity doubled until the slot fits; `apply` moves the entries
//!   into it ([`Table::take_from`]: one pointer per slot, bounded by the limit of that
//!   table, measured by `msg_cost` and by the e2e suite), swaps, and pushes the old table
//!   to [`Garbage`]. A slot beyond the capacity in `apply` is a contract violation: the
//!   message is skipped and counted, never a panic.
//!
//! Every allocation of `prepare` is **fallible**: [`Table::try_with_capacity`] and
//! [`try_box`], never `Box::new` nor `handle_alloc_error`. A refused allocation
//! (`memory.grow` refused, machine out of memory) is [`MsgError::OutOfMemory`], reported
//! to the project and shown, never a trap nor a silence.
//!
//! Nothing is ever freed on the audio thread: what `apply` replaces or removes goes back
//! as **one** [`Garbage`] item per rt message on the second ring, and the prepare side drops
//! it. Invariant that keeps `apply` from ever dropping: the garbage ring holds one slot
//! more than the rt ring ([`limits::GARBAGE_RING_CAPACITY`]), only `prepare` pushes on the
//! rt ring, each rt message yields at most one garbage item, and `prepare` drains the
//! garbage ring before each push. After a drain at most `RT_RING_CAPACITY` messages are in
//! flight; the audio side may apply one of them and push its garbage before the push that
//! follows the drain lands, so the garbage ring never holds more than
//! `RT_RING_CAPACITY + 1` items: there is always a free slot for `apply`.
//!
//! Above the tables stand the **safety limits** of [`limits`], very high and checked by the
//! project before a message is sent ([`MsgError::LimitExceeded`]): the engine itself never
//! fails in real time. They are counts; the memory they imply, every table at its limit and
//! doubled during a growth, plus the PCM budget of the audio tracks, the rings and the
//! fragmentation of a linear memory that never shrinks, stays under the 4 GiB of wasm32
//! with margin (test `limits_fit_under_the_wasm32_cap`). The memory of the devices is not
//! a count: each device kind declares its size (CT-5's descriptor) and the project refuses
//! an insert that would take the declared total past [`limits::ENGINE_MEMORY_BUDGET_BYTES`].
//!
//! # Linear memory (web)
//!
//! The module reserves [`limits::WASM_INITIAL_PAGES`] and grows only in `prepare` (the
//! Worker), never in `process`. The memory is shared: a `memory.grow` leaves the worklet's
//! existing views valid (they keep their length); the worklet re-takes its views at the
//! start of a block when `memory.buffer` changed, a constant-cost comparison.
//!
//! # Live input
//!
//! Clip and automation events never cross a bounded queue: the engine reads them in their
//! tables while rendering. Only the live input (MIDI, keyboard: [`ControlMsg::NoteOn`],
//! [`ControlMsg::NoteOff`], [`ControlMsg::AllNotesOff`]) is queued, in
//! [`limits::LIVE_QUEUE_CAPACITY`] slots, far beyond what a human produces between two
//! blocks. On overflow the event is dropped and the producer increments [`LiveOverflow`];
//! the audio side, at the start of every block, first drains the live queue and applies
//! its events, then reads the counter and, when it moved, releases every live note (the
//! helper's order, `crates/helper/src/audio.rs`: a note-on queued before the lost note-off
//! is applied before the release, never after it), so no note stays
//! stuck even if nothing else is ever sent. The count is reported in [`Feedback`].
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
//!   and counts it ([`Feedback`]`.rejected`). A wrong version, a reserved bit set or a
//!   `len` over the limit is a corrupt stream: [`MsgError::UnsupportedVersion`],
//!   [`MsgError::Corrupt`], [`MsgError::TooLarge`].
//! - `epoch` is the engine generation held by the page. Only [`ControlMsg::Reset`] may
//!   carry a new epoch: the engine adopts it and discards every staging of the previous
//!   one, provided the epoch is **newer** (wrapping comparison, `(epoch - current) as i16 > 0`:
//!   a `Reset` of a past epoch that was duplicated or replayed is dropped and counted, so a
//!   late `Reset` never rolls the engine back); any other message whose epoch is not the
//!   current one is dropped and counted ([`MsgError::StaleEpoch`]), equality only. When the
//!   page switches engine (web ↔ helper) or comes back to an engine it had suspended, it
//!   increments the epoch and resends the whole project after a `Reset`: a chunked upload
//!   cut by the switch never completes in the old epoch, and a message of the old epoch
//!   still in flight on either channel is dropped.
//! - Payloads larger than [`limits::MAX_CONTROL_MSG_BYTES`] are split by the sender.
//! - The PCM of the audio tracks never goes through a message: AU-3 streams it into buffers
//!   that [`ControlMsg::AudioSourceDeclare`] only declares.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::time::{Frame, LoopRegion, Quantize, Tempo, Tick, TimeSignature};

/// Version of the `ControlMsg` binary format ([`Header::version`]).
pub const MSG_VERSION: u8 = 1;

/// Version of the [`Feedback`] layout.
pub const FEEDBACK_VERSION: u8 = 1;

/// Size of a [`Header`] in bytes.
pub const HEADER_BYTES: usize = 16;

/// Alignment of a message start in a stream, in bytes.
pub const MSG_ALIGN: usize = 4;

/// The safety limits: very high, enforced by the project before a message is sent, never
/// reached by a human, and sized together under the wasm32 cap (see the module doc).
pub mod limits {
    /// Tracks in a project.
    pub const MAX_TRACKS: u32 = 4096;
    /// Devices in one track's chain.
    pub const MAX_DEVICES_PER_TRACK: u32 = 64;
    /// Clips in a project (grid and arrangement together).
    pub const MAX_CLIPS: u32 = 65_536;
    /// Placements in a project, every track together.
    pub const MAX_PLACEMENTS: u32 = 1_048_576;
    /// Placements in one track's arrangement.
    pub const MAX_PLACEMENTS_PER_TRACK: u32 = 65_536;
    /// Notes in a project, every clip together.
    pub const MAX_NOTES: u32 = 16_777_216;
    /// Notes in one clip.
    pub const MAX_NOTES_PER_CLIP: u32 = 1_048_576;
    /// Automation lanes in a project.
    pub const MAX_LANES: u32 = 65_536;
    /// Automation points in a project, every lane together.
    pub const MAX_AUTOMATION_POINTS: u32 = 16_777_216;
    /// Automation points in one lane.
    pub const MAX_POINTS_PER_LANE: u32 = 1_048_576;
    /// Audio sources (files) declared to the engine.
    pub const MAX_AUDIO_SOURCES: u32 = 4096;
    /// Scenes of the launch grid.
    pub const MAX_SCENES: u32 = 1024;
    /// Memory the streaming buffers of the audio tracks may hold together (AU-3).
    pub const PCM_BUDGET_BYTES: u64 = 1 << 30;
    /// Memory the project lets the engine's tables and devices take together, from the
    /// sizes the device descriptors declare (CT-5) and the tables' sizes.
    pub const ENGINE_MEMORY_BUDGET_BYTES: u64 = (3 << 30) - WASM_RESERVED_BYTES;
    /// Memory kept out of the budgets for the module's code, data, stacks and rings.
    pub const WASM_RESERVED_BYTES: u64 = 64 << 20;
    /// Payload of one `ControlMsg`; larger uploads are chunked.
    pub const MAX_CONTROL_MSG_BYTES: u32 = 16 << 20;
    /// Slots of the live input queue (notes played live).
    pub const LIVE_QUEUE_CAPACITY: usize = 4096;
    /// Slots of the `RtMsg` ring and of the garbage ring.
    pub const RT_RING_CAPACITY: usize = 4096;
    /// Slots of the garbage ring: one more than the rt ring (module doc, the invariant).
    pub const GARBAGE_RING_CAPACITY: usize = RT_RING_CAPACITY + 1;
    /// Rt messages the prepare side parks when the ring is full; beyond, `EngineBusy`.
    pub const PENDING_QUEUE_CAPACITY: usize = 4096;
    /// Most `RtMsg` applied at the start of one block; the rest wait for the next block.
    pub const MAX_APPLY_PER_BLOCK: usize = 128;
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ClipSlot(pub u32);

/// Index of a placement (a clip in the arrangement) in the engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Note {
    pub start: Tick,
    pub length: Tick,
    pub pitch: u8,
    pub velocity: u8,
}

/// Shape of the transition after an automation point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Curve {
    #[default]
    Step = 0,
    Linear = 1,
}

/// A point of an automation lane; at most one per tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct AutomationPoint {
    pub tick: Tick,
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

/// A clip placed in a track's arrangement: start in ticks, the clip's own offset and the
/// length played, in ticks for a MIDI clip, in frames for an audio clip ([`Span`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub placement: PlacementSlot,
    pub clip: ClipSlot,
    pub start: Tick,
    pub span: Span,
}

/// The extent of a placement: offset into the clip and length, in the clip's unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Ticks { offset: Tick, length: Tick },
    Frames { offset: Frame, length: Frame },
}

impl Default for Span {
    fn default() -> Self {
        Span::Ticks {
            offset: Tick(0),
            length: Tick(0),
        }
    }
}

// ---------------------------------------------------------------- ControlMsg

/// What the project sends to the engine. One variant per MVP action, including the ones
/// whose implementation comes later (automation, audio sources, recording, metronome): the
/// list is the contract.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlMsg {
    /// Forgets everything (tracks, clips, sources, lanes, staging) and adopts `epoch`.
    /// Sent before a full resend, on load and on every engine switch.
    Reset {
        epoch: u16,
    },
    SetTransport {
        tempo: Tempo,
        signature: TimeSignature,
        launch_quantize: Quantize,
    },
    /// Metronome (TR-2): on or off, its level, bars of count-in before a recording.
    SetMetronome {
        enabled: bool,
        volume: f32,
        count_in_bars: u8,
    },
    /// Master mix (MX-1).
    SetMaster {
        gain: f32,
        mute: bool,
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
    /// Arms a track (RE-1): the live input and the recording reach it.
    TrackArm {
        track: TrackSlot,
        armed: bool,
    },
    /// Monitors the live input through the track's chain (RE-1).
    TrackSetMonitor {
        track: TrackSlot,
        monitor: bool,
    },
    /// The track the live input goes to when no track is armed (RE-1).
    SelectTrack {
        track: Option<TrackSlot>,
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
    /// The table is swapped in once the last chunk arrived (module doc, chunked uploads).
    ClipNotesUpload {
        clip: ClipSlot,
        length: Tick,
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
        frames: Frame,
    },
    AudioSourceRelease {
        source: SourceSlot,
    },
    /// Makes a clip an audio clip: a range of a source.
    ClipAudioSet {
        clip: ClipSlot,
        source: SourceSlot,
        start: Frame,
        length: Frame,
    },
    // arrangement
    /// The arrangement of one track, in chunks like the notes.
    TrackPlacementsUpload {
        track: TrackSlot,
        first: u32,
        total: u32,
        placements: Vec<Placement>,
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
    /// Every track plays its arrangement again (AR-2).
    BackToArrangement,
    // transport
    Play {
        from: Tick,
    },
    Stop,
    Seek {
        tick: Tick,
    },
    SetLoop {
        region: Option<LoopRegion>,
    },
    /// Recording on or off (RE-2); the take comes back through `Feedback::recorded`.
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
    SetMetronome = 2,
    SetMaster = 3,
    TrackCreate = 10,
    TrackRemove = 11,
    TrackSetMix = 12,
    TrackSetSolo = 13,
    TrackArm = 14,
    TrackSetMonitor = 15,
    SelectTrack = 16,
    DeviceInsert = 20,
    DeviceRemove = 21,
    DeviceSetEnabled = 22,
    ParamSet = 23,
    ClipNotesUpload = 30,
    ClipRemove = 31,
    AudioSourceDeclare = 40,
    AudioSourceRelease = 41,
    ClipAudioSet = 42,
    TrackPlacementsUpload = 50,
    AutomationLaneUpload = 60,
    AutomationLaneRemove = 61,
    SceneSetSlot = 70,
    LaunchClip = 71,
    LaunchScene = 72,
    StopTrack = 73,
    StopAll = 74,
    BackToArrangement = 75,
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
    pub const ALL: [Tag; 37] = [
        Tag::Reset,
        Tag::SetTransport,
        Tag::SetMetronome,
        Tag::SetMaster,
        Tag::TrackCreate,
        Tag::TrackRemove,
        Tag::TrackSetMix,
        Tag::TrackSetSolo,
        Tag::TrackArm,
        Tag::TrackSetMonitor,
        Tag::SelectTrack,
        Tag::DeviceInsert,
        Tag::DeviceRemove,
        Tag::DeviceSetEnabled,
        Tag::ParamSet,
        Tag::ClipNotesUpload,
        Tag::ClipRemove,
        Tag::AudioSourceDeclare,
        Tag::AudioSourceRelease,
        Tag::ClipAudioSet,
        Tag::TrackPlacementsUpload,
        Tag::AutomationLaneUpload,
        Tag::AutomationLaneRemove,
        Tag::SceneSetSlot,
        Tag::LaunchClip,
        Tag::LaunchScene,
        Tag::StopTrack,
        Tag::StopAll,
        Tag::BackToArrangement,
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
    /// The tag written in the header of this message (exhaustive: a new variant without a
    /// tag does not compile).
    pub fn tag(&self) -> Tag {
        match self {
            ControlMsg::Reset { .. } => Tag::Reset,
            ControlMsg::SetTransport { .. } => Tag::SetTransport,
            ControlMsg::SetMetronome { .. } => Tag::SetMetronome,
            ControlMsg::SetMaster { .. } => Tag::SetMaster,
            ControlMsg::TrackCreate { .. } => Tag::TrackCreate,
            ControlMsg::TrackRemove { .. } => Tag::TrackRemove,
            ControlMsg::TrackSetMix { .. } => Tag::TrackSetMix,
            ControlMsg::TrackSetSolo { .. } => Tag::TrackSetSolo,
            ControlMsg::TrackArm { .. } => Tag::TrackArm,
            ControlMsg::TrackSetMonitor { .. } => Tag::TrackSetMonitor,
            ControlMsg::SelectTrack { .. } => Tag::SelectTrack,
            ControlMsg::DeviceInsert { .. } => Tag::DeviceInsert,
            ControlMsg::DeviceRemove { .. } => Tag::DeviceRemove,
            ControlMsg::DeviceSetEnabled { .. } => Tag::DeviceSetEnabled,
            ControlMsg::ParamSet { .. } => Tag::ParamSet,
            ControlMsg::ClipNotesUpload { .. } => Tag::ClipNotesUpload,
            ControlMsg::ClipRemove { .. } => Tag::ClipRemove,
            ControlMsg::AudioSourceDeclare { .. } => Tag::AudioSourceDeclare,
            ControlMsg::AudioSourceRelease { .. } => Tag::AudioSourceRelease,
            ControlMsg::ClipAudioSet { .. } => Tag::ClipAudioSet,
            ControlMsg::TrackPlacementsUpload { .. } => Tag::TrackPlacementsUpload,
            ControlMsg::AutomationLaneUpload { .. } => Tag::AutomationLaneUpload,
            ControlMsg::AutomationLaneRemove { .. } => Tag::AutomationLaneRemove,
            ControlMsg::SceneSetSlot { .. } => Tag::SceneSetSlot,
            ControlMsg::LaunchClip { .. } => Tag::LaunchClip,
            ControlMsg::LaunchScene { .. } => Tag::LaunchScene,
            ControlMsg::StopTrack { .. } => Tag::StopTrack,
            ControlMsg::StopAll => Tag::StopAll,
            ControlMsg::BackToArrangement => Tag::BackToArrangement,
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
        fn chunk(
            what: &'static str,
            first: u32,
            len: usize,
            total: u32,
            limit: u32,
        ) -> Result<(), MsgError> {
            below(what, u64::from(total), u64::from(limit) + 1)?;
            let end = u64::from(first) + len as u64;
            if end > u64::from(total) {
                return Err(MsgError::LimitExceeded {
                    what,
                    limit: u64::from(total),
                    requested: end,
                });
            }
            Ok(())
        }
        let track = |t: &TrackSlot| below("tracks", t.0.into(), MAX_TRACKS.into());
        let clip = |c: &ClipSlot| below("clips", c.0.into(), MAX_CLIPS.into());
        let source = |s: &SourceSlot| below("audio sources", s.0.into(), MAX_AUDIO_SOURCES.into());
        let scene = |s: &SceneSlot| below("scenes", s.0.into(), MAX_SCENES.into());
        let lane = |l: &LaneSlot| below("lanes", l.0.into(), MAX_LANES.into());
        let device = |d: &DeviceSlot| {
            below(
                "devices per track",
                d.0.into(),
                MAX_DEVICES_PER_TRACK.into(),
            )
        };
        match self {
            ControlMsg::TrackCreate { track: t, .. }
            | ControlMsg::TrackRemove { track: t }
            | ControlMsg::TrackSetMix { track: t, .. }
            | ControlMsg::TrackSetSolo { track: t, .. }
            | ControlMsg::TrackArm { track: t, .. }
            | ControlMsg::TrackSetMonitor { track: t, .. }
            | ControlMsg::StopTrack { track: t }
            | ControlMsg::NoteOn { track: t, .. }
            | ControlMsg::NoteOff { track: t, .. }
            | ControlMsg::AllNotesOff { track: t } => track(t),
            ControlMsg::SelectTrack { track: t } => t.as_ref().map_or(Ok(()), track),
            ControlMsg::DeviceInsert {
                track: t,
                device: d,
                ..
            }
            | ControlMsg::DeviceRemove {
                track: t,
                device: d,
            }
            | ControlMsg::DeviceSetEnabled {
                track: t,
                device: d,
                ..
            }
            | ControlMsg::ParamSet {
                track: t,
                device: d,
                ..
            } => {
                track(t)?;
                device(d)
            }
            ControlMsg::ClipNotesUpload {
                clip: c,
                first,
                total,
                notes,
                ..
            } => {
                clip(c)?;
                chunk(
                    "notes per clip",
                    *first,
                    notes.len(),
                    *total,
                    MAX_NOTES_PER_CLIP,
                )
            }
            ControlMsg::ClipRemove { clip: c } => clip(c),
            ControlMsg::AudioSourceDeclare { source: s, .. }
            | ControlMsg::AudioSourceRelease { source: s } => source(s),
            ControlMsg::ClipAudioSet {
                clip: c, source: s, ..
            } => {
                clip(c)?;
                source(s)
            }
            ControlMsg::TrackPlacementsUpload {
                track: t,
                first,
                total,
                placements,
            } => {
                track(t)?;
                for p in placements {
                    below("placements", p.placement.0.into(), MAX_PLACEMENTS.into())?;
                    clip(&p.clip)?;
                }
                chunk(
                    "placements per track",
                    *first,
                    placements.len(),
                    *total,
                    MAX_PLACEMENTS_PER_TRACK,
                )
            }
            ControlMsg::AutomationLaneUpload {
                lane: l,
                track: t,
                first,
                total,
                points,
                ..
            } => {
                lane(l)?;
                track(t)?;
                chunk(
                    "points per lane",
                    *first,
                    points.len(),
                    *total,
                    MAX_POINTS_PER_LANE,
                )
            }
            ControlMsg::AutomationLaneRemove { lane: l } => lane(l),
            ControlMsg::SceneSetSlot {
                track: t,
                scene: s,
                clip: c,
            } => {
                track(t)?;
                scene(s)?;
                c.as_ref().map_or(Ok(()), clip)
            }
            ControlMsg::LaunchClip { track: t, scene: s } => {
                track(t)?;
                scene(s)
            }
            ControlMsg::LaunchScene { scene: s } => scene(s),
            ControlMsg::Reset { .. }
            | ControlMsg::SetTransport { .. }
            | ControlMsg::SetMetronome { .. }
            | ControlMsg::SetMaster { .. }
            | ControlMsg::StopAll
            | ControlMsg::BackToArrangement
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
    /// A message of another engine generation than the current one, dropped.
    StaleEpoch { message: u16, current: u16 },
    /// A safety limit the project must refuse before sending.
    LimitExceeded {
        what: &'static str,
        limit: u64,
        requested: u64,
    },
    /// `prepare` could not get the memory (allocation or `memory.grow` refused).
    OutOfMemory { bytes: usize },
    /// The rt ring is full: the prepare side queues the message and retries, never blocks.
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

    /// Reads a header from the start of `bytes` and checks it against the current `epoch`
    /// (a `Reset` of a newer epoch is accepted: it is how the engine adopts one; a `Reset`
    /// of the current or an older epoch is as stale as any other message). `bytes` must
    /// hold the whole message: a payload shorter than `len` is [`MsgError::Truncated`].
    ///
    /// The checks go in this order: header size, version, reserved bits, `len`, tag,
    /// epoch, payload size.
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
        let raw_tag = u16::from_le_bytes([bytes[2], bytes[3]]);
        let tag = Tag::from_u16(raw_tag).ok_or(MsgError::UnknownTag {
            tag: raw_tag,
            skip: Header::frame_len(len),
        })?;
        let epoch = u16::from_le_bytes([bytes[4], bytes[5]]);
        let newer = epoch.wrapping_sub(current_epoch) as i16 > 0;
        let accepted = if tag == Tag::Reset {
            // A Reset of the current epoch is a duplicate: as stale as a past one.
            newer
        } else {
            epoch == current_epoch
        };
        if !accepted {
            return Err(MsgError::StaleEpoch {
                message: epoch,
                current: current_epoch,
            });
        }
        if bytes.len() < HEADER_BYTES + len as usize {
            return Err(MsgError::Truncated {
                needed: HEADER_BYTES + len as usize,
                available: bytes.len(),
            });
        }
        let seq = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
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

// ---------------------------------------------------------------- fallible allocation

/// Boxes a value through a fallible allocation: `prepare`'s only way to box, so that a
/// refused `memory.grow` is a typed error and never a trap (`Box::new` aborts).
pub fn try_box<T>(value: T) -> Result<Box<T>, MsgError> {
    let mut slot = Vec::new();
    slot.try_reserve_exact(1)
        .map_err(|_| MsgError::OutOfMemory {
            bytes: size_of::<T>(),
        })?;
    slot.push(value);
    let mut boxed: Box<[T]> = slot.into_boxed_slice();
    let ptr = boxed.as_mut_ptr();
    core::mem::forget(boxed);
    // SAFETY: `ptr` is the start of a boxed slice of exactly one `T`, whose ownership
    // `forget` just released; a `Box<T>` has the same layout as a `Box<[T; 1]>` allocation.
    Ok(unsafe { Box::from_raw(ptr) })
}

// ---------------------------------------------------------------- RtMsg and growth

/// A table of the engine: a boxed slice and a length. Built off the audio path, it never
/// allocates afterwards; when full, `prepare` builds a larger one and `apply` swaps them.
#[derive(Debug)]
pub struct Table<T> {
    items: Box<[T]>,
    len: usize,
}

/// `Table::push` on a full table, giving the item back untouched (nothing is dropped on
/// the audio path): the caller asks `prepare` for a larger table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableFull<T>(pub T);

impl<T: Default> Table<T> {
    /// Allocates a table of `capacity` slots, or fails typed when the memory is refused.
    pub fn try_with_capacity(capacity: usize) -> Result<Self, MsgError> {
        let mut items = Vec::new();
        items
            .try_reserve_exact(capacity)
            .map_err(|_| MsgError::OutOfMemory {
                bytes: capacity.saturating_mul(size_of::<T>()),
            })?;
        items.resize_with(capacity, T::default);
        Ok(Self {
            items: items.into_boxed_slice(),
            len: 0,
        })
    }

    /// The capacity `prepare` gives the next table: double, at least 1.
    pub fn next_capacity(&self) -> usize {
        (self.capacity() * 2).max(1)
    }

    /// A slot array: `capacity` slots that all exist (`len == capacity`), each holding
    /// `T::default()`; the runtime tables are built this way.
    pub fn try_filled(capacity: usize) -> Result<Self, MsgError> {
        let mut table = Self::try_with_capacity(capacity)?;
        table.len = capacity;
        Ok(table)
    }

    /// Appends the items of `old` (a growing list) to this (larger) list, on the audio
    /// path: one move per item, no allocation, `old` keeps its storage (emptied) and goes
    /// to the garbage. Items beyond this capacity stay in `old` (never happens when
    /// `prepare` doubled it).
    pub fn take_from(&mut self, old: &mut Table<T>) {
        let n = old.len.min(self.items.len() - self.len);
        for (dst, src) in self.items[self.len..self.len + n]
            .iter_mut()
            .zip(old.items[..n].iter_mut())
        {
            *dst = core::mem::take(src);
        }
        self.len += n;
        old.len -= n;
        if old.len > 0 {
            old.items.rotate_left(n);
        }
    }

    /// Moves the slots of `old` (a slot array) into the first slots of this (larger) slot
    /// array, on the audio path: one move per slot of `old`, empty ones included, no
    /// allocation; `old` keeps its storage (every slot back to the default) and goes to the
    /// garbage. Slots beyond this capacity stay in `old` (never happens when `prepare`
    /// doubled it).
    pub fn take_slots_from(&mut self, old: &mut Table<T>) {
        let n = old.len.min(self.items.len());
        for (dst, src) in self.items[..n].iter_mut().zip(old.items[..n].iter_mut()) {
            *dst = core::mem::take(src);
        }
        if n < old.len {
            old.items.rotate_left(n);
        }
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

    /// Appends without allocating; `Err(TableFull(item))` leaves the table unchanged and
    /// gives the item back.
    pub fn push(&mut self, item: T) -> Result<(), TableFull<T>> {
        if self.is_full() {
            return Err(TableFull(item));
        }
        self.items[self.len] = item;
        self.len += 1;
        Ok(())
    }
}

/// An opaque engine structure built by `prepare` (a track with its devices, a device, the
/// whole engine state): the concrete types are MO-3's and CT-5's.
pub type Opaque = Box<dyn core::any::Any + Send>;

/// Which engine structure a [`Swap`] or a [`RtMsg::Remove`] targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    /// The whole engine state (`Reset`).
    Everything,
    /// The runtime table of the tracks (grown when full).
    Tracks,
    Track(TrackSlot),
    Device(TrackSlot, DeviceSlot),
    /// The notes of one clip (content snapshot).
    ClipNotes(ClipSlot),
    /// The points of one lane (content snapshot).
    LanePoints(LaneSlot),
    /// The arrangement of one track (content snapshot).
    TrackPlacements(TrackSlot),
    /// The runtime table of the audio sources (grown when full).
    AudioSources,
    AudioSource(SourceSlot),
}

/// A structure `prepare` built, carried into `apply` by [`RtMsg::Swap`]. Runtime tables
/// hold one boxed entry per slot ([`Opaque`]); content tables hold the values.
#[derive(Debug)]
pub enum Prepared {
    Everything(Opaque),
    Tracks(Table<Option<Opaque>>),
    Track(Opaque),
    Device(Opaque),
    ClipNotes(Table<Note>),
    LanePoints(Table<AutomationPoint>),
    TrackPlacements(Table<Placement>),
    AudioSources(Table<Option<Opaque>>),
    AudioSource(Opaque),
}

/// What `apply` sends back on the garbage ring, one per rt message, dropped on the
/// prepare side only. A removed track carries its devices and buffers inside its box.
#[derive(Debug)]
pub enum Garbage {
    Everything(Opaque),
    Tracks(Table<Option<Opaque>>),
    Track(Opaque),
    Device(Opaque),
    ClipNotes(Table<Note>),
    LanePoints(Table<AutomationPoint>),
    TrackPlacements(Table<Placement>),
    AudioSources(Table<Option<Opaque>>),
    AudioSource(Opaque),
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
    Play { from: Tick },
    Stop,
    Seek { tick: Tick },
    SetLoop { region: Option<LoopRegion> },
    SetRecording { recording: bool },
}

/// Launch order carried to the audio thread (executed at the next quantization boundary).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMsg {
    Clip { track: TrackSlot, scene: SceneSlot },
    Scene { scene: SceneSlot },
    StopTrack { track: TrackSlot },
    StopAll,
    BackToArrangement,
}

/// What `prepare` hands to `apply`: a Rust value moved through the ring, never serialized.
/// `offset` is the frame in the next block where the message takes effect (0: block start).
#[derive(Debug)]
pub enum RtMsg {
    /// Replaces a structure (a grown table, a new device); the old one goes to [`Garbage`].
    Swap(Swap),
    /// Removes a structure; what it held goes to [`Garbage`].
    Remove(TableKind),
    SetTransport {
        tempo: Tempo,
        signature: TimeSignature,
        launch_quantize: Quantize,
    },
    SetMetronome {
        enabled: bool,
        volume: f32,
        count_in_bars: u8,
    },
    SetMaster {
        gain: f32,
        mute: bool,
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
    TrackSetMonitor {
        track: TrackSlot,
        monitor: bool,
    },
    SelectTrack {
        track: Option<TrackSlot>,
    },
    DeviceSetEnabled {
        track: TrackSlot,
        device: DeviceSlot,
        enabled: bool,
    },
    ParamSet {
        track: TrackSlot,
        device: DeviceSlot,
        param: ParamId,
        value: f32,
        offset: u16,
    },
    SceneSetSlot {
        track: TrackSlot,
        scene: SceneSlot,
        clip: Option<ClipSlot>,
    },
    ClipAudioSet {
        clip: ClipSlot,
        source: SourceSlot,
        start: Frame,
        length: Frame,
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

/// Counter of the live events dropped because the live queue was full, incremented by the
/// producer, read by the audio side at the start of every block: when it moved, the audio
/// side releases every live note (no stuck note, whatever is sent afterwards).
#[derive(Debug, Default)]
pub struct LiveOverflow(AtomicU32);

impl LiveOverflow {
    pub const fn new() -> Self {
        Self(AtomicU32::new(0))
    }

    /// Producer side: one more event lost.
    pub fn record(&self) {
        self.0.fetch_add(1, Ordering::Release);
    }

    /// Audio side: the total so far; a change since the last block means a release is due.
    pub fn total(&self) -> u32 {
        self.0.load(Ordering::Acquire)
    }
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

/// State of a clip slot of the launch grid in [`Feedback`]; only the slots that are not
/// stopped are reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClipState {
    Queued = 1,
    Playing = 2,
    Stopping = 3,
}

/// A grid slot that is queued, playing or stopping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct ActiveClip {
    pub track: TrackSlot,
    pub scene: SceneSlot,
    pub state: ClipState,
}

/// Size of an [`ActiveClip`] on the wire.
pub const ACTIVE_CLIP_BYTES: usize = 8;

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

/// A note the engine recorded since the last report (RE-2): the page turns the take into
/// a clip with one undoable command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct RecordedNote {
    pub track: TrackSlot,
    pub note: Note,
}

/// Size of the fixed part of a [`Feedback`] on the wire.
pub const FEEDBACK_HEADER_BYTES: usize = 56;

/// What the engine reports to the page, about 30 times per second. Fixed little-endian
/// layout, written by the audio thread into a region sized by [`Feedback::size_for`]:
///
/// ```text
/// offset size field
///      0    1 version = FEEDBACK_VERSION
///      1    1 transport (TransportState)
///      2    2 epoch
///      4    4 seq (seqlock: the writer makes it odd before the body and even after;
///             a reader takes an even seq, the body, then the same seq again, or re-reads)
///      8    8 playhead_tick (i64)
///     16    8 playhead_frame (u64)
///     24    4 cpu_load (f32, 0..1, render time over block time)
///     28    4 live_overflows (u32, cumulative)
///     32    4 rejected (u32, cumulative: unknown tags, stale epochs, bad chunks)
///     36    4 disk_underruns (u32, cumulative, AU-3)
///     40    4 tracks (u32)
///     44    4 active_clips (u32, at most 2 per track: one playing, one queued)
///     48    4 recorded (u32, notes recorded since the last report, drained)
///     52    4 reserved
///     56      TrackMeter × tracks (16 bytes each)
///          then ActiveClip × active_clips (track u32, scene u16, state u8, pad u8)
///          then RecordedNote × recorded (track u32, pad u32, start i64, length i64,
///               pitch u8, velocity u8, pad ×6: 32 bytes on the wire)
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feedback {
    pub transport: TransportState,
    pub epoch: u16,
    pub seq: u32,
    pub playhead_tick: Tick,
    pub playhead_frame: Frame,
    pub cpu_load: f32,
    pub live_overflows: u32,
    pub rejected: u32,
    pub disk_underruns: u32,
    pub meters: Vec<TrackMeter>,
    pub active_clips: Vec<ActiveClip>,
    pub recorded: Vec<RecordedNote>,
}

impl Feedback {
    /// Bytes of a report for `tracks` tracks, `active_clips` grid slots and `recorded` notes.
    pub fn size_for(tracks: usize, active_clips: usize, recorded: usize) -> usize {
        FEEDBACK_HEADER_BYTES
            + tracks * TRACK_METER_BYTES
            + active_clips * ACTIVE_CLIP_BYTES
            + recorded * RECORDED_NOTE_WIRE_BYTES
    }

    /// The region the audio side writes into: sized for the limits of the moment, resized
    /// by `prepare` when the track table grows.
    pub fn max_size_for(tracks: usize) -> usize {
        Feedback::size_for(tracks, 2 * tracks, MAX_RECORDED_PER_REPORT)
    }
}

/// Size of a [`RecordedNote`] on the wire (padded).
pub const RECORDED_NOTE_WIRE_BYTES: usize = 32;

/// Most recorded notes one report carries; the rest wait for the next one.
pub const MAX_RECORDED_PER_REPORT: usize = 1024;
