//! Project → engine synchronisation contract (CT-3, `docs/specs/02-decoupage-mvp.md` § 2.3),
//! and its payload codecs (MO-1, [`codec`] module doc).
//!
//! This module declares the types of the contract, the binary format of the control stream
//! and the growth mechanism ([`Table`], [`Put`], [`PagedPut`]). The two sides of the cycle,
//! `Preparer::prepare` and `Engine::apply`, live in [`crate::engine`] (MO-1); the channel
//! of each target (helper thread + `rtrb`, prepare Worker + shared memory) is MO-2's.
//!
//! # The three message kinds
//!
//! 1. [`ControlMsg`], what the project sends to the engine: produced by the command layer
//!    (`crates/commands`, in Rust, never in TypeScript), encoded in the binary format below,
//!    carried from the page to the *prepare side* (a Worker in web mode, the helper's control
//!    thread in Boost mode). Every structure it names is an engine **slot** (`u32` index)
//!    assigned by the project, which keeps the correspondence with its stable identifiers
//!    (§ 2.4): the engine never sees a `TrackId`.
//! 2. [`RtMsg`], what `prepare` (the spec's `Engine::prepare`, [`crate::engine::Preparer`])
//!    produces **off the audio path** (it may allocate, fallibly) and `apply`
//!    ([`crate::engine::Engine::apply`]) consumes **on the audio path** (it never
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
//! invariant below depends on it); a growth travels inside the message that needs it:
//!
//! | `ControlMsg` | `RtMsg` |
//! |---|---|
//! | `Reset { epoch }` | `Reset`: a fresh engine state, the old one is one garbage item (its tracks first fade out in the graph and come back through the retire ring, MO-3) |
//! | `SetTransport`, `SetMetronome`, `SetMaster`, `SetScale` | the same, copied |
//! | `TrackCreate` | `TrackPut`: the new track, and the grown track table when the slot is beyond it (a track it replaces fades out, then comes back through the retire ring, MO-3) |
//! | `TrackRemove` | `TrackRemove`: the track owns its devices and tables; it fades out in the graph, then comes back through the retire ring (MO-3), not the garbage ring |
//! | `TrackSetMix/Solo/Arm/Monitor`, `HoldMix`, `SelectTrack`, `DeviceSetEnabled`, `ParamSet` | the same, copied |
//! | `DeviceInsert` | `DevicePut`: the device built by `prepare` with its initial state, and the grown device row |
//! | `DeviceRemove` | `DeviceRemove` |
//! | `DeviceMove` | `DeviceMove`: the device moved to its free slot, and the grown device row (#800) |
//! | `ClipNotesUpload` (last chunk), `ClipAudioSet` | `ClipPut`: the whole clip, and the page or the grown directory of the clip table |
//! | `AutomationLaneUpload` (last chunk) | `LanePut`, paged like the clips |
//! | `TrackPlacementsUpload` (last chunk) | `PlacementsSet`: the whole arrangement of the track |
//! | `SceneSetSlot` | `SceneSet`, and the grown scene row of the track |
//! | `AudioSourceDeclare` | `SourcePut` (its resident segments come later, `SegmentPut`, from the disk planner: [`crate::audio_source`]) |
//! | `ClipRemove`, `AutomationLaneRemove`, `AudioSourceRelease` | `ClipRemove`, `LaneRemove`, `SourceRemove` |
//! | `LaunchClip/Scene`, `StopTrack/All`, `BackToArrangement`, `BackTrackToArrangement` | `Launch(…)` |
//! | `Play`, `Stop`, `Seek`, `SetLoop`, `SetRecording` | `Transport(…)` |
//! | `AllSoundOff` | `AllSoundOff` |
//! | `NoteOn`, `NoteOff`, `AllNotesOff` | a [`LiveEvent`] on the live queue, never `prepare` |
//!
//! Every rt message takes effect at the start of the block that applies it (a `ParamSet`
//! is `Device::set_param`, offset 0 of the block, CT-5); sample-accurate events are MO-4's.
//!
//! **Backpressure, no loss.** When the rt ring is full, the prepare side keeps the rt
//! message in its own ordered queue (off the audio path; reserved whole when the channel is
//! built) and pushes it as soon as a slot frees: a new message goes behind the queue
//! whenever the queue is not empty, so control messages are never dropped nor reordered.
//! The host calls `Preparer::flush` regularly (MO-2's control thread), so the queue drains
//! even when the project sends nothing. That queue is bounded too
//! ([`limits::PENDING_QUEUE_CAPACITY`]): beyond it the engine is not consuming (audio
//! context suspended, device lost) and `prepare` returns [`MsgError::EngineBusy`] to the
//! project, before anything is prepared or committed; the project shows the stalled engine,
//! stops sending, and resyncs with a `Reset` once the engine runs again. In web mode the
//! page knows its `AudioContext` is suspended (background tab) and stops sending before
//! that point. The memory of the tables parked in the queue is bounded only by the
//! fallible allocation of `prepare` ([`MsgError::OutOfMemory`]), never by a trap.
//!
//! **Chunked uploads.** Notes, points and placements arrive in chunks (`first`, `total`)
//! staged on the prepare side per slot (the whole `total` reserved at the first chunk). A
//! chunk whose `first` is not the staged length, or whose `total` differs from the staged
//! one, is counted as rejected and drops the staging; when it is a first chunk
//! (`first == 0`) the staging restarts from it. The table is built and swapped only when
//! `first + len == total`. `Reset` and the `Remove` of the slot (of the track, for its
//! placements) discard its staging. A partial upload never reaches the audio side.
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
//! - **Runtime tables** (state the audio thread mutates) are **slot arrays**: every slot
//!   exists (`len == capacity`, built by [`Table::try_filled`]), an empty slot holds
//!   `None`, and a message addresses a slot directly (the project assigns slots, reuses
//!   freed ones, never above the limit). The prepare side keeps a shadow of their
//!   capacities. When a message names a slot at or beyond the capacity, `prepare` builds
//!   the next table, empty, with the capacity doubled until the slot fits, and sends it
//!   **in the same rt message** as the item ([`Put`]); `apply` moves the entries into it
//!   ([`Table::take_slots_from`]: one pointer per slot), swaps, puts the item, and sends
//!   the old table to the garbage. The tracks (4 096), the sources (4 096), and per track
//!   the device row (1 024) and the scene row (4 096) grow this way: a growth moves at most
//!   4 096 pointers, the case CT-3 measured. The clips (1 048 576) and the lanes (65 536)
//!   are too large for that: they are **paged** ([`PagedPut`], `crate::engine::Paged`), a
//!   directory of pages of [`PAGE_SLOTS`] slots; `prepare` sends a page the first time one
//!   of its slots is used, and a larger directory when the page is beyond it, so a growth
//!   moves at most `limit / PAGE_SLOTS` (1 024) pointers.
//!
//! A slot beyond the table in `apply` is a contract violation: the item goes to the
//! garbage and the message is counted as skipped. A "grown" table that is not larger than
//! the live one is one too: the swap is refused (moving into it would lose live entries),
//! that table goes to the garbage, the item is still put when its slot fits, and the
//! message is counted. Never a panic.
//!
//! Every allocation of `prepare` is **fallible**: [`Table::try_with_capacity`] and
//! [`try_box`], never `Box::new` nor `handle_alloc_error`. A refused allocation
//! (`memory.grow` refused, machine out of memory) is [`MsgError::OutOfMemory`], reported
//! to the project and shown, never a trap nor a silence; `prepare` allocates everything
//! before it commits anything, so the refused message can be sent again as is.
//!
//! Nothing is ever freed on the audio thread: what `apply` replaces or removes goes back
//! as **one** [`Garbage`] item per rt message on the second ring, and the prepare side drops
//! it. Invariant that keeps `apply` from ever dropping: the garbage ring holds one slot
//! more than the rt ring ([`limits::GARBAGE_RING_CAPACITY`]), only `prepare` pushes on the
//! rt ring, each rt message yields at most one garbage item, and `prepare` drains the
//! garbage ring before each push. After a drain at most `RT_RING_CAPACITY` messages are in
//! flight; the audio side may apply one of them and push its garbage before the push that
//! follows the drain lands, so the garbage ring never holds more than
//! `RT_RING_CAPACITY + 1` items: there is always a free slot for `apply`. Should it ever be
//! full, `apply` leaks the item and counts it rather than free it.
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
//! [`ControlMsg::NoteOff`], [`ControlMsg::AllNotesOff`], carried as [`LiveEvent`]s) is
//! queued, in
//! [`limits::LIVE_QUEUE_CAPACITY`] slots, far beyond what a human produces between two
//! blocks. On overflow the event is dropped and the producer increments [`LiveOverflow`];
//! the audio side, at the start of every block, first drains the live queue and applies
//! its events, then reads the counter and, when it moved, releases every live note (the
//! helper's order, `crates/helper/src/audio.rs`: a note-on queued before the lost note-off
//! is applied before the release, never after it), so no note stays
//! stuck even if nothing else is ever sent. The count is reported in [`Feedback`].
//!
//! **Stamps (#499).** A live note on or off may carry `at`, the frame of the host's audio
//! clock at which it was played (web: the audio context's frame). The host tells the engine
//! the host frame of each block (`Engine::set_host_frame`, the worklet's `currentFrame`
//! through `fk_render_at`); `Engine::begin_block` turns the stamp into an engine sample, and
//! the recorder places the note there ([`crate::recorder`]). A live note still sounds at the
//! start of the block that takes it: by then its stamp is in the past, so the stamp adds no
//! latency. In the binary format the stamp is a `u64` appended to the payload only when
//! present: an unstamped note is the bytes it was before stamps existed.
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

use crate::engine::{Clip, DeviceEntry, Lane, Source, State, Track};
use crate::time::{Frame, LoopRegion, Quantize, SampleRate, Tempo, Tick, TimeSignature};

pub mod codec;
pub use codec::{
    NOTE_WIRE_BYTES, PLACEMENT_WIRE_BYTES, POINT_WIRE_BYTES, STATE_PAIR_BYTES, encode_state,
    encode_stream, state_pairs,
};

/// Version of the `ControlMsg` binary format ([`Header::version`]).
pub const MSG_VERSION: u8 = 1;

/// Version of the [`Feedback`] layout.
pub const FEEDBACK_VERSION: u8 = 11;

/// Size of a [`Header`] in bytes.
pub const HEADER_BYTES: usize = 16;

/// Alignment of a message start in a stream, in bytes.
pub const MSG_ALIGN: usize = 4;

/// The safety limits: very high, enforced by the project before a message is sent, never
/// reached by a human, and sized together under the wasm32 cap (see the module doc).
pub mod limits {
    /// Tracks in a project.
    pub const MAX_TRACKS: u32 = 4096;
    /// The largest shift of a take's notes (`ControlMsg::SetRecording`), in microseconds,
    /// either way: above the commands' bound (1000 ms of latency plus 500 ms of offset).
    pub const MAX_RECORD_SHIFT_US: i32 = 2_000_000;
    /// Devices in one track's chain.
    pub const MAX_DEVICES_PER_TRACK: u32 = 1024;
    /// Clips in a project (grid and arrangement together).
    pub const MAX_CLIPS: u32 = 1_048_576;
    /// Placements in a project, every track together.
    pub const MAX_PLACEMENTS: u32 = 1_048_576;
    /// Placements in one track's arrangement.
    pub const MAX_PLACEMENTS_PER_TRACK: u32 = 131_072;
    /// Notes in a project, every clip together.
    pub const MAX_NOTES: u32 = 16_777_216;
    /// Notes in one clip.
    pub const MAX_NOTES_PER_CLIP: u32 = 1_048_576;
    /// Automation lanes in a project.
    pub const MAX_LANES: u32 = 65_536;
    /// Automation points in a project, every lane together.
    pub const MAX_AUTOMATION_POINTS: u32 = 16_777_216;
    /// Automation points in one lane.
    pub const MAX_POINTS_PER_LANE: u32 = 16_777_216;
    /// Audio sources (files) declared to the engine.
    pub const MAX_AUDIO_SOURCES: u32 = 4096;
    /// Scenes of the launch grid.
    pub const MAX_SCENES: u32 = 4096;
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
    /// The longest delay the graph gives a track to align it with the others, and the
    /// longest bypass line of a device, in frames (#749, `crate::graph` > Latency): 85 ms at
    /// 192 kHz, 341 ms at 48 kHz (sixteen Ceilings at their longest look-ahead and 192 kHz). A
    /// chain whose latency goes beyond is compensated up to it.
    /// It sizes the graph's compensation ring once ([`crate::latency::Compensation`]), not a
    /// buffer per track.
    pub const MAX_COMPENSATION_FRAMES: u32 = 16_383;
    /// Initial size of the web module's linear memory, in 64 KiB pages (16 MiB).
    pub const WASM_INITIAL_PAGES: u32 = 256;
    /// The cap of a wasm32 linear memory: 65 536 pages of 64 KiB.
    pub const WASM_MAX_BYTES: u64 = 1 << 32;
}

// ---------------------------------------------------------------- slots and payload types

/// Index of a track in the engine, assigned by the project.
///
/// [`TrackSlot::MASTER`] is not a track: it addresses the master's chain of effects (#364) in
/// the five device messages (`DeviceInsert`, `DeviceRemove`, `DeviceMove`, `DeviceSetEnabled`,
/// `ParamSet`),
/// and the master's automation lanes (#378) in `AutomationLaneUpload`, and nowhere else. The project never assigns it (tracks stay below
/// [`limits::MAX_TRACKS`]), so a message of a track that names it finds no track and is
/// refused or skipped, as a slot nobody holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackSlot(pub u32);

impl TrackSlot {
    /// The master's chain, in the device messages (see the type).
    pub const MASTER: TrackSlot = TrackSlot(u32::MAX);

    /// Whether this is [`TrackSlot::MASTER`].
    pub const fn is_master(self) -> bool {
        self.0 == u32::MAX
    }
}

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SourceSlot(pub u32);

/// Identifier of a parameter inside a device (`Param::id` for the synth).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize), serde(transparent))]
pub struct ParamId(pub u32);

/// Kind of a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TrackKind {
    Midi = 0,
    Audio = 1,
}

/// Kind of a device, as the device registry of CT-5 numbers them (`"faulk.osc_synth"` → id,
/// [`crate::device::DeviceDescriptor::kind_id`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize), serde(transparent))]
pub struct DeviceKind(pub u32);

/// A note of a MIDI clip, as the project stores it (no identifier, CT-4 decides one).
/// `cents` (fine tuning, -50 to 50), `chance` (percent, 0 to 100) and `repeat` (1 to 16)
/// are the project's optional fields (CL-1, #225): the clip player (CL-2) plays `cents`;
/// `chance` and `repeat` are carried for #192.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Note {
    pub start: Tick,
    pub length: Tick,
    pub pitch: u8,
    pub velocity: u8,
    pub cents: i8,
    pub chance: u8,
    pub repeat: u8,
}

impl Note {
    /// Largest `cents` magnitude.
    pub const MAX_CENTS: i8 = 50;
    /// Largest `chance` (always plays).
    pub const MAX_CHANCE: u8 = 100;
    /// Largest `repeat`.
    pub const MAX_REPEAT: u8 = 16;
}

impl Default for Note {
    /// A note at tick 0, no length, key 0, velocity 0, no fine tuning, always played,
    /// once: the defaults of the optional fields are the project's.
    fn default() -> Self {
        Note {
            start: Tick::ZERO,
            length: Tick::ZERO,
            pitch: 0,
            velocity: 0,
            cents: 0,
            chance: Self::MAX_CHANCE,
            repeat: 1,
        }
    }
}

/// Shape of the transition after an automation point (`crate::automation`, #881): the value
/// held, or the straight line to the next point, as is or warped (`EaseIn` starts slowly and
/// ends fast, `EaseOut` the other way round, `Smooth` is slow at both ends).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Curve {
    #[default]
    Step = 0,
    Linear = 1,
    EaseIn = 2,
    EaseOut = 3,
    Smooth = 4,
}

/// A point of an automation lane; at most one per tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct AutomationPoint {
    pub tick: Tick,
    /// In the target's unit: the parameter's value, the pan (-1 to 1), or the gain in dB
    /// (a straight line between two points follows the fader's travel, `crate::fader`).
    pub value: f32,
    pub curve: Curve,
}

/// What an automation lane drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationTarget {
    DeviceParam {
        device: DeviceSlot,
        param: ParamId,
    },
    /// The track's gain; points in dB.
    MixerGain,
    /// The track's pan, -1 (left) to 1 (right).
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
    /// The project's scale (#690), as a set of pitch classes (bit `c` for pitch class `c`, 0 =
    /// C; bits above the twelfth ignored; 0: no scale). The MIDI effects that work "in the
    /// scale" read it in `device::Transport::scale`. A `Reset` clears it.
    SetScale {
        pitch_classes: u16,
    },
    /// Metronome (TR-2): on or off, its level (a linear gain, 1.0 the reference level of the
    /// click, 0 to 4; the project's `gain_db` is converted by the sender), bars of count-in
    /// before a recording.
    SetMetronome {
        enabled: bool,
        volume: f32,
        count_in_bars: u8,
    },
    /// Master mix (MX-1): linear gain, pan from -1 to 1, mute.
    SetMaster {
        gain: f32,
        pan: f32,
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
    /// A manual gesture on a fader or a pan that left the value as it was (#417): the project
    /// drops such a gesture as unchanged, so no `TrackSetMix` carries it, and a `TrackSetMix`
    /// that moves neither field looks like a mute, which holds nothing. This message says the
    /// intent: it holds the lanes that drive the fields it names (`Some`, with the fader's value
    /// the project holds, which the engine takes again) on `track` (`TrackSlot::MASTER` for the
    /// master) and gives the strip's field back to its fader, as a gesture that moves the value
    /// does (AM-2). It never touches the mute nor a field it does not name.
    HoldMix {
        track: TrackSlot,
        gain: Option<f32>,
        pan: Option<f32>,
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
    /// The selected track (RE-1): the commands layer routes the live input to it and to the
    /// armed tracks, so the engine only stores it.
    SelectTrack {
        track: Option<TrackSlot>,
    },
    // devices
    DeviceInsert {
        track: TrackSlot,
        device: DeviceSlot,
        kind: DeviceKind,
        /// Initial state of the device: its parameter values, `(param id, value)` pairs
        /// ([`codec::encode_state`]).
        state: Vec<u8>,
    },
    DeviceRemove {
        track: TrackSlot,
        device: DeviceSlot,
    },
    /// Moves the device in `from` to the free slot `to` without building it again (#800).
    /// The instance keeps its state (tails, envelopes, filter states, its fade and its enabled
    /// flag), so a device that shifts in its chain when another is inserted or removed in
    /// front of it goes on sounding. Nothing is built, dropped or faded. `from` must hold a
    /// device and `to` must be free. Only an audio effect moves: a MIDI effect (its output is
    /// sized from its place, #219) and an instrument (it plays first after the MIDI effects)
    /// are refused ([`MsgError::BadPayload`], as an occupied `to` is). The devices removed
    /// around it keep their place in the chain it left ([`crate::graph`] > Device fades).
    DeviceMove {
        track: TrackSlot,
        from: DeviceSlot,
        to: DeviceSlot,
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
    /// `looping` is the clip's loop `[start, end)` within `[0, length]` (CL-2), `None`
    /// for a clip that plays once.
    ClipNotesUpload {
        clip: ClipSlot,
        length: Tick,
        looping: Option<LoopRegion>,
        first: u32,
        total: u32,
        notes: Vec<Note>,
    },
    ClipRemove {
        clip: ClipSlot,
    },
    // audio clips and sources
    /// Declares a source whose PCM the disk side reads into the engine's buffers
    /// ([`crate::audio_source`], AU-3). `media` is the SHA-256 of its blob: the disk side's
    /// key to the file (`blobs/<hash>` or its PCM cache); the audio side ignores it.
    AudioSourceDeclare {
        source: SourceSlot,
        sample_rate: u32,
        channels: u8,
        frames: Frame,
        media: [u8; 32],
    },
    AudioSourceRelease {
        source: SourceSlot,
    },
    /// Makes a clip an audio clip: a range `[start, start + length)` of a source, its gain
    /// (linear, `0.0` for silence) and its loop (`[start, end)` in source frames relative to
    /// the clip's start, within `[0, length]`; `None`: played once).
    ClipAudioSet {
        clip: ClipSlot,
        source: SourceSlot,
        start: Frame,
        length: Frame,
        gain: f32,
        looping: Option<(Frame, Frame)>,
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
    /// One track plays its arrangement again; the others are left alone (AR-2).
    BackTrackToArrangement {
        track: TrackSlot,
    },
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
    /// `shift_us`: how much earlier the notes of the take that starts are placed, in
    /// microseconds (negative: later), the output latency the host estimates plus the manual
    /// offset (#560). The recorder moves each note back on its own clock, where it knows the
    /// loop wraps ([`crate::recorder`]); clamped to [`limits::MAX_RECORD_SHIFT_US`] either way.
    /// Ignored when `recording` is false and while a take runs.
    SetRecording {
        recording: bool,
        shift_us: i32,
    },
    // live input (the only messages that go through the bounded live queue)
    /// A note played live. `at` is the host frame at which it was played (#499; module doc,
    /// live input), `None` when the host gives no time: it then counts from the start of the
    /// block that takes it.
    NoteOn {
        track: TrackSlot,
        pitch: u8,
        velocity: u8,
        at: Option<u64>,
    },
    /// A live note released; `at` as for [`ControlMsg::NoteOn`].
    NoteOff {
        track: TrackSlot,
        pitch: u8,
        at: Option<u64>,
    },
    AllNotesOff {
        track: TrackSlot,
    },
    /// Cuts every sound of the engine at once: the output fades out over one fade
    /// ([`crate::graph::FADE_SECONDS`]), then every device is reset (voices, tails) and
    /// the output resumes, silent. Not a live event: it goes through `prepare` and the rt
    /// ring, like a transport order.
    AllSoundOff,
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
    SetScale = 4,
    TrackCreate = 10,
    TrackRemove = 11,
    TrackSetMix = 12,
    TrackSetSolo = 13,
    TrackArm = 14,
    TrackSetMonitor = 15,
    SelectTrack = 16,
    HoldMix = 17,
    DeviceInsert = 20,
    DeviceRemove = 21,
    DeviceSetEnabled = 22,
    ParamSet = 23,
    DeviceMove = 24,
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
    BackTrackToArrangement = 76,
    Play = 80,
    Stop = 81,
    Seek = 82,
    SetLoop = 83,
    SetRecording = 84,
    NoteOn = 90,
    NoteOff = 91,
    AllNotesOff = 92,
    AllSoundOff = 93,
}

impl Tag {
    /// Every tag, in declaration order (for exhaustive tests and generated tables).
    pub const ALL: [Tag; 42] = [
        Tag::Reset,
        Tag::SetTransport,
        Tag::SetMetronome,
        Tag::SetMaster,
        Tag::SetScale,
        Tag::TrackCreate,
        Tag::TrackRemove,
        Tag::TrackSetMix,
        Tag::TrackSetSolo,
        Tag::TrackArm,
        Tag::TrackSetMonitor,
        Tag::SelectTrack,
        Tag::HoldMix,
        Tag::DeviceInsert,
        Tag::DeviceRemove,
        Tag::DeviceSetEnabled,
        Tag::ParamSet,
        Tag::DeviceMove,
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
        Tag::BackTrackToArrangement,
        Tag::Play,
        Tag::Stop,
        Tag::Seek,
        Tag::SetLoop,
        Tag::SetRecording,
        Tag::NoteOn,
        Tag::NoteOff,
        Tag::AllNotesOff,
        Tag::AllSoundOff,
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
            ControlMsg::SetScale { .. } => Tag::SetScale,
            ControlMsg::TrackCreate { .. } => Tag::TrackCreate,
            ControlMsg::TrackRemove { .. } => Tag::TrackRemove,
            ControlMsg::TrackSetMix { .. } => Tag::TrackSetMix,
            ControlMsg::TrackSetSolo { .. } => Tag::TrackSetSolo,
            ControlMsg::TrackArm { .. } => Tag::TrackArm,
            ControlMsg::TrackSetMonitor { .. } => Tag::TrackSetMonitor,
            ControlMsg::SelectTrack { .. } => Tag::SelectTrack,
            ControlMsg::HoldMix { .. } => Tag::HoldMix,
            ControlMsg::DeviceInsert { .. } => Tag::DeviceInsert,
            ControlMsg::DeviceRemove { .. } => Tag::DeviceRemove,
            ControlMsg::DeviceSetEnabled { .. } => Tag::DeviceSetEnabled,
            ControlMsg::ParamSet { .. } => Tag::ParamSet,
            ControlMsg::DeviceMove { .. } => Tag::DeviceMove,
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
            ControlMsg::BackTrackToArrangement { .. } => Tag::BackTrackToArrangement,
            ControlMsg::Play { .. } => Tag::Play,
            ControlMsg::Stop => Tag::Stop,
            ControlMsg::Seek { .. } => Tag::Seek,
            ControlMsg::SetLoop { .. } => Tag::SetLoop,
            ControlMsg::SetRecording { .. } => Tag::SetRecording,
            ControlMsg::NoteOn { .. } => Tag::NoteOn,
            ControlMsg::NoteOff { .. } => Tag::NoteOff,
            ControlMsg::AllNotesOff { .. } => Tag::AllNotesOff,
            ControlMsg::AllSoundOff => Tag::AllSoundOff,
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
            | ControlMsg::BackTrackToArrangement { track: t }
            | ControlMsg::NoteOn { track: t, .. }
            | ControlMsg::NoteOff { track: t, .. }
            | ControlMsg::AllNotesOff { track: t } => track(t),
            ControlMsg::SelectTrack { track: t } => t.as_ref().map_or(Ok(()), track),
            // The master's strip is addressed by `TrackSlot::MASTER` here too.
            ControlMsg::HoldMix { track: t, .. } => {
                if t.is_master() {
                    Ok(())
                } else {
                    track(t)
                }
            }
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
                // The master's chain is addressed by `TrackSlot::MASTER`, in these four
                // messages, `DeviceMove` and `HoldMix` only: any other message of a track refuses it as a slot past
                // the limit.
                if !t.is_master() {
                    track(t)?;
                }
                device(d)
            }
            ControlMsg::DeviceMove {
                track: t, from, to, ..
            } => {
                if !t.is_master() {
                    track(t)?;
                }
                device(from)?;
                device(to)
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
                // A lane of the master (#378) names the master's row.
                if !t.is_master() {
                    track(t)?;
                }
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
            | ControlMsg::SetScale { .. }
            | ControlMsg::StopAll
            | ControlMsg::AllSoundOff
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
    /// The rt ring and the prepare side's queue are full: the engine is not consuming.
    EngineBusy,
    /// A known tag whose payload is malformed or out of range (codec module doc): the
    /// message is skipped by its length and counted, the stream goes on.
    BadPayload { tag: u16 },
    /// A `DeviceInsert` of a kind no factory builds: skipped and counted.
    UnknownDevice(DeviceKind),
    /// A message naming a track, or a device of a track, that does not exist on the
    /// prepare side: skipped and counted.
    NoSuchTarget,
}

impl MsgError {
    /// Whether the error stops the stream ([`Preparer::submit`](crate::engine::Preparer::submit)
    /// returns it): a corrupt stream, a refused allocation, a stalled engine. The others are
    /// rejections of one message, skipped and counted while the stream goes on.
    pub fn stops_the_stream(&self) -> bool {
        matches!(
            self,
            MsgError::Truncated { .. }
                | MsgError::UnsupportedVersion(_)
                | MsgError::Corrupt
                | MsgError::TooLarge(_)
                | MsgError::OutOfMemory { .. }
                | MsgError::EngineBusy
        )
    }

    /// A stable negative number for a C ABI (the web module's exports): one per variant.
    pub fn code(&self) -> i32 {
        match self {
            MsgError::Truncated { .. } => -1,
            MsgError::UnsupportedVersion(_) => -2,
            MsgError::UnknownTag { .. } => -3,
            MsgError::Corrupt => -4,
            MsgError::TooLarge(_) => -5,
            MsgError::StaleEpoch { .. } => -6,
            MsgError::LimitExceeded { .. } => -7,
            MsgError::OutOfMemory { .. } => -8,
            MsgError::EngineBusy => -9,
            MsgError::BadPayload { .. } => -10,
            MsgError::UnknownDevice(_) => -11,
            MsgError::NoSuchTarget => -12,
        }
    }
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
    /// A full table of the items of `items` (a content table built by `prepare`): its
    /// storage becomes the table's, without a copy when its capacity is its length.
    pub fn from_vec(items: Vec<T>) -> Self {
        let items = items.into_boxed_slice();
        let len = items.len();
        Self { items, len }
    }

    /// Empties a table of plain values on the audio path: no drop, no free.
    pub fn clear(&mut self)
    where
        T: Copy,
    {
        self.len = 0;
    }

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

/// A runtime table: a slot array (`len == capacity`), an empty slot holds `None`.
pub type Slots<T> = Table<Option<T>>;

/// Puts `value` in slot `slot` of a runtime table, growing it first when `grown` is set:
/// `apply` moves the entries of the current table into `grown` (one move per slot), swaps,
/// then puts the value. The growth and the item travel in **one** rt message, so a
/// `ControlMsg` still yields at most one `RtMsg` (module doc).
#[derive(Debug)]
pub struct Put<T> {
    pub slot: u32,
    pub value: T,
    pub grown: Option<Slots<T>>,
}

/// What one `apply` of a [`Put`] or a remove leaves for the garbage: the replaced table
/// (emptied by the move) and the replaced value, or the value `apply` could not place (a
/// slot beyond the table, a contract violation). One garbage item whatever happened.
#[derive(Debug)]
pub struct Spare<T> {
    pub table: Option<Slots<T>>,
    pub item: Option<T>,
}

impl<T> Spare<T> {
    pub const NONE: Spare<T> = Spare {
        table: None,
        item: None,
    };

    /// Nothing to free: `apply` pushes no garbage item.
    pub fn is_empty(&self) -> bool {
        self.table.is_none() && self.item.is_none()
    }
}

/// Slots of one page of a paged runtime table (clips, lanes).
pub const PAGE_SLOTS: usize = 1024;

/// A page of a paged runtime table: [`PAGE_SLOTS`] slots.
pub type Page<T> = Slots<T>;

/// What a [`PagedPut`] may carry to make room: a larger directory of pages, the page of
/// the slot. Boxed so that the rt message stays small; `apply` takes its parts out and
/// sends the box back to the garbage holding what they replaced.
#[derive(Debug)]
pub struct PagedGrowth<T> {
    pub directory: Option<Slots<Page<T>>>,
    pub page: Option<Page<T>>,
}

/// Puts `value` in slot `slot` of a paged runtime table (clips, lanes): the large tables
/// grow by pages, so a growth moves one pointer per page, never one per slot.
#[derive(Debug)]
pub struct PagedPut<T> {
    pub slot: u32,
    pub value: T,
    pub growth: Option<Box<PagedGrowth<T>>>,
}

/// What one `apply` of a [`PagedPut`] or a paged remove leaves for the garbage: the growth
/// box (holding the replaced directory, or a page or directory `apply` could not use) and
/// the replaced or refused value.
#[derive(Debug)]
pub struct PagedSpare<T> {
    pub growth: Option<Box<PagedGrowth<T>>>,
    pub item: Option<T>,
}

impl<T> PagedSpare<T> {
    pub const NONE: PagedSpare<T> = PagedSpare {
        growth: None,
        item: None,
    };

    pub fn is_empty(&self) -> bool {
        self.growth.is_none() && self.item.is_none()
    }
}

/// What `apply` sends back on the garbage ring, at most one per rt message, dropped on the
/// prepare side only. A removed track carries its devices and its tables inside its box.
#[derive(Debug)]
pub enum Garbage {
    /// The whole previous state (`Reset`).
    State(Box<State>),
    Tracks(Spare<Box<Track>>),
    Devices(Spare<Box<DeviceEntry>>),
    Clips(PagedSpare<Box<Clip>>),
    Lanes(PagedSpare<Box<Lane>>),
    Sources(Spare<Box<Source>>),
    /// A replaced or evicted resident segment, or a refused one (AU-3).
    Segment(Option<Box<crate::audio_source::Segment>>),
    /// A replaced or detached ring, or a refused one (AU-3).
    Ring(Option<crate::audio_source::Shared<crate::audio_source::Ring>>),
    /// A replaced arrangement of a track (or the refused new one).
    Placements(Table<Placement>),
    /// A replaced (grown) scene row of a track.
    Scenes(Slots<ClipSlot>),
}

/// Transport order carried to the audio thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportMsg {
    Play { from: Tick },
    Stop,
    Seek { tick: Tick },
    SetLoop { region: Option<LoopRegion> },
    SetRecording { recording: bool, shift_us: i32 },
}

/// Launch order carried to the audio thread (executed at the next quantization boundary).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMsg {
    Clip { track: TrackSlot, scene: SceneSlot },
    Scene { scene: SceneSlot },
    StopTrack { track: TrackSlot },
    StopAll,
    BackToArrangement,
    BackTrackToArrangement { track: TrackSlot },
}

/// What `prepare` hands to `apply`: a Rust value moved through the ring, never serialized.
/// Every variant takes effect at the start of the block that applies it.
#[derive(Debug)]
pub enum RtMsg {
    /// A fresh engine state; the old one goes to [`Garbage::State`].
    Reset(Box<State>),
    /// A new sample rate (#236, `Preparer::set_sample_rate`): the clock (its playhead kept),
    /// the metronome, the limiter and the graph's fades follow it; the devices built for it
    /// come after, one `DevicePut` each. Not a `ControlMsg`: the rate is the host's.
    SetSampleRate(SampleRate),
    SetTransport {
        tempo: Tempo,
        signature: TimeSignature,
        launch_quantize: Quantize,
    },
    SetScale(crate::device::PitchSet),
    SetMetronome {
        enabled: bool,
        volume: f32,
        count_in_bars: u8,
    },
    SetMaster {
        gain: f32,
        pan: f32,
        mute: bool,
    },
    /// A new track (its devices, scenes and placements empty), in its slot.
    TrackPut(Put<Box<Track>>),
    TrackRemove(TrackSlot),
    TrackSetMix {
        track: TrackSlot,
        gain: f32,
        pan: f32,
        mute: bool,
    },
    HoldMix {
        track: TrackSlot,
        gain: Option<f32>,
        pan: Option<f32>,
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
    /// A device built by `prepare`, in its slot of the track's chain.
    DevicePut {
        track: TrackSlot,
        put: Put<Box<DeviceEntry>>,
    },
    DeviceRemove {
        track: TrackSlot,
        device: DeviceSlot,
    },
    /// The device of `from` moved to the free slot `to` (#800), after the row is grown to
    /// `grown` when `to` is beyond it: no device is built, dropped or faded, it keeps its
    /// state. The automation lanes name a slot and stay: the project uploads the moved
    /// device's lanes again. Anything else kept per slot outside [`DeviceEntry`] (none today)
    /// must move with it in the same step.
    DeviceMove {
        track: TrackSlot,
        from: DeviceSlot,
        to: DeviceSlot,
        grown: Option<Slots<Box<DeviceEntry>>>,
        /// The same device built again (#800), when `to` is a slot a device with a latency
        /// line left: if that device still holds the slot's timing (#761), `apply` puts this
        /// one there instead (held, then faded in, as a device put there is) and the moved one
        /// leaves from `from` (the values waiting for `from` in the lines before it are
        /// forgotten, as for a removal: the project uploads the moved device's lanes again);
        /// otherwise it goes to the garbage.
        spare: Option<Box<DeviceEntry>>,
    },
    DeviceSetEnabled {
        track: TrackSlot,
        device: DeviceSlot,
        enabled: bool,
    },
    /// A parameter set by hand or by an agent: `Device::set_param`, which behaves as a
    /// value at offset 0 of the block (CT-5).
    ParamSet {
        track: TrackSlot,
        device: DeviceSlot,
        param: ParamId,
        value: f32,
    },
    /// A clip's content (a completed notes upload, or an audio range), replacing the slot.
    ClipPut(PagedPut<Box<Clip>>),
    ClipRemove(ClipSlot),
    SourcePut(Put<Box<Source>>),
    SourceRemove(SourceSlot),
    /// A resident segment of a source (AU-3, [`crate::audio_source`]): put in its place
    /// `index`, or removed (`None`, an eviction). Only complete segments travel.
    SegmentPut {
        source: SourceSlot,
        index: u8,
        segment: Option<Box<crate::audio_source::Segment>>,
    },
    /// The ring of a port of a track (AU-3): attached, replaced or detached (`None`). A track
    /// whose `instance` is not this one (the slot was given to another track) keeps its own.
    StreamAttach {
        track: TrackSlot,
        instance: u32,
        port: u8,
        ring: Option<crate::audio_source::Shared<crate::audio_source::Ring>>,
    },
    /// The arrangement of a track (a completed upload), swapped by pointer.
    PlacementsSet {
        track: TrackSlot,
        placements: Table<Placement>,
    },
    LanePut(PagedPut<Box<Lane>>),
    LaneRemove(LaneSlot),
    /// A slot of a track's row of the launch grid, the row grown first when `grown` is set.
    SceneSet {
        track: TrackSlot,
        scene: SceneSlot,
        clip: Option<ClipSlot>,
        grown: Option<Slots<ClipSlot>>,
    },
    Transport(TransportMsg),
    /// `AllSoundOff` (see [`ControlMsg::AllSoundOff`]).
    AllSoundOff,
    Launch(LaunchMsg),
}

/// A note played live (MIDI, keyboard): the only events that cross a bounded queue, the
/// live queue, never `prepare` (module doc, live input).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveEvent {
    /// `at`: the engine sample at which the note was played (the host frame of the message
    /// converted by `Engine::begin_block`), `None` when unknown.
    NoteOn {
        track: TrackSlot,
        pitch: u8,
        velocity: u8,
        at: Option<u64>,
    },
    NoteOff {
        track: TrackSlot,
        pitch: u8,
        at: Option<u64>,
    },
    AllNotesOff {
        track: TrackSlot,
    },
}

impl Default for LiveEvent {
    fn default() -> Self {
        LiveEvent::AllNotesOff {
            track: TrackSlot(0),
        }
    }
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
/// stopped are reported (GR-1, `crate::launcher`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClipState {
    /// A launch waits for its quantization boundary, the transport runs.
    Queued = 1,
    /// The clip plays.
    Playing = 2,
    /// The clip plays and a stop (or a launch that leaves the track silent) waits for its
    /// boundary.
    Stopping = 3,
    /// A launch is armed: the transport is stopped, the clip starts at the first
    /// boundary once it plays.
    Armed = 4,
    /// The grid holds the track without a clip playing (after a stop, or when the clip ended):
    /// the arrangement is silent until `BackToArrangement`. Its `scene` is [`HELD_SCENE`].
    Held = 5,
}

/// The `scene` of a [`ClipState::Held`] entry: no cell.
pub const HELD_SCENE: SceneSlot = SceneSlot(u16::MAX);

impl ClipState {
    /// The state of a wire byte (`None` for a value this build does not know).
    pub fn from_byte(byte: u8) -> Option<ClipState> {
        match byte {
            1 => Some(ClipState::Queued),
            2 => Some(ClipState::Playing),
            3 => Some(ClipState::Stopping),
            4 => Some(ClipState::Armed),
            5 => Some(ClipState::Held),
            _ => None,
        }
    }
}

/// A grid slot that is queued, playing or stopping, or a track the grid holds ([`ClipState::Held`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct ActiveClip {
    pub track: TrackSlot,
    pub scene: SceneSlot,
    pub state: ClipState,
    /// The transport tick of the clip's position 0 for a clip that plays or stops (the page
    /// places a playhead in the clip from it); for a waiting launch, the boundary when the
    /// engine knows it for the order it reports (a newer order that still waits has none: 0).
    pub anchor: Tick,
}

/// Size of an [`ActiveClip`] on the wire.
pub const ACTIVE_CLIP_BYTES: usize = 16;

/// Meters of one track, linear amplitude.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct TrackMeter {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
    /// The largest gain reduction, in decibels (>= 0), that a device of the strip's chain took
    /// during the period (#469, version 4); 0 when none works.
    pub gain_reduction_db: f32,
}

/// Size of a [`TrackMeter`] on the wire.
pub const TRACK_METER_BYTES: usize = 20;

/// A note the engine recorded (RE-2, [`crate::recorder`]): its track slot, and the note with
/// its start in transport ticks and its played length; the page turns the take into clips
/// with one undoable command (`record.commit_take`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct RecordedNote {
    pub track: TrackSlot,
    pub note: Note,
}

/// Size of the fixed part of a [`Feedback`] on the wire.
pub const FEEDBACK_HEADER_BYTES: usize = 112;

/// Where the number of device readouts sits in a [`Feedback`] (#686, version 10): one `u32`.
pub const FEEDBACK_READOUTS_OFFSET: usize = 100;

/// Where the device latency sits in a [`Feedback`] (#749, version 10): one `u32`, followed by
/// the spectra count (version 11). One constant, so that the field moves in one place.
pub const FEEDBACK_LATENCY_OFFSET: usize = 104;

/// The bit of the latency word that says the compensation is capped (#749): a track needs a
/// longer delay than [`limits::MAX_COMPENSATION_FRAMES`]. The other bits are the frames.
pub const FEEDBACK_LATENCY_CAPPED: u32 = 1 << 31;

/// The latency word of a report: `frames` (saturated under [`FEEDBACK_LATENCY_CAPPED`]) and
/// the capped bit.
pub fn latency_word(frames: u32, capped: bool) -> u32 {
    frames.min(FEEDBACK_LATENCY_CAPPED - 1) | if capped { FEEDBACK_LATENCY_CAPPED } else { 0 }
}

/// Where the number of spectra sits in a [`Feedback`] (#156, version 11): one `u32`, the last
/// word of the header.
pub const FEEDBACK_SPECTRA_OFFSET: usize = 108;

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
///     24    4 cpu_load (f32, 0..1: the render time the host timed over the duration of the
///             blocks it timed in the period (`crate::host`, #486); -1 until the host timed a
///             block)
///     28    4 live_overflows (u32, cumulative)
///     32    4 rejected (u32, cumulative: unknown tags, stale epochs, bad chunks)
///     36    4 disk_underruns (u32, cumulative: blocks in which a voice had fewer frames from
///             disk than it needed, `crate::audio_source`, AU-3, version 7)
///     40    4 tracks (u32)
///     44    4 active_clips (u32, at most 2 per track: one playing or, with none to show, one held by the grid ([`ClipState::Held`]); one queued)
///     48    4 recorded (u32, the recorded notes this report carries: the last
///             min(record_total, MAX_RECORDED_PER_REPORT) notes of the take, version 5)
///     52    4 voices (u32, MO-5: the most voices sounding in the tracks' devices during the
///             period, `Device::voices` summed over the tracks, `crate::host`)
///     56    4 record_take (u32, the number of the current or last take, 0 before the first;
///             version 5)
///     60    4 record_total (u32, notes finished in that take: the notes carried are the
///             indices record_total - recorded .. record_total; version 5)
///     64    4 record_dropped (u32, notes of that take lost because the recorder's held
///             table was full; version 5)
///     68    4 record_flags (u32, bit 0: the recorder captures; once it is 0 with this
///             record_take, the take's notes are all in the report; version 5)
///     72    4 cpu_peak (f32, 0..1: the largest load of one peak window closed in the period;
///             with none closed (a coarse clock's window outlasts a period), the last window
///             closed, which can be under cpu_load; before any, cpu_load; -1 like cpu_load;
///             version 6, #486)
///     76    4 offline_sources (u32, declared audio sources the engine cannot read: blob
///             missing, PCM cache not ready, no disk; version 7, AU-3)
///     80    4 pcm_flags (u32, bit 0: something did not fit the PCM budget; version 7)
///     84    4 pcm_kib (u32, PCM in memory, rings and resident segments, in KiB; version 7)
///     88    4 silenced_frames (u32, cumulative since the channel was built, saturating: stereo
///             frames the output protection turned to silence because a sample was non-finite,
///             `Engine::limiter_silenced`; version 8, #667)
///     92    8 host_frame (u64: the host's frame at the report's playhead, the clock the host
///             gave `Engine::set_host_frame` (the web worklet's `currentFrame`) carried to the
///             sample the playhead (tick, frame above) sits at; a host that gives none (the
///             helper) has the engine's own sample counter; version 9, #733).
///             It keeps counting while the transport is stopped, where `playhead_frame` stops: `host_frame -
///             playhead_frame` is the host frame of position 0 only while the transport runs at one tempo,
///             with no loop wrap and no sample-rate change since the Play.
///    100    4 readouts (u32, the device readouts this report carries, at most
///             MAX_READOUTS_PER_REPORT; [`FEEDBACK_READOUTS_OFFSET`]; version 10, #686)
///    104    4 latency (u32: bits 0 to 30, latency_frames, the device latency of what is heard, in frames at
///             the engine's rate: the largest chain latency of the tracks plus the master's, which the graph
///             compensates (`crate::graph` > Latency), saturated at 2^31 - 1; bit 31 ([`FEEDBACK_LATENCY_CAPPED`]),
///             latency_capped: a track needs more than the compensation's limit and is no longer aligned;
///             [`FEEDBACK_LATENCY_OFFSET`]; version 10, #749)
///    108    4 spectra (u32, at most `analysis::MAX_SPECTRA`: the spectra this report carries;
///             [`FEEDBACK_SPECTRA_OFFSET`]; last word of the header; version 11, #156)
///    112      TrackMeter × (tracks + 1) (20 bytes each: peak L, peak R, RMS L, RMS R, then the
///             largest gain reduction in decibels of the strip's devices over the period, version 4):
///             one per track slot below `tracks` (zeros for an empty slot; peak and RMS after the
///             track's fader), then the output's meter after the master (version 2); the master's
///             reduction is that of its own chain
///          then ActiveClip × active_clips (track u32, scene u16, state u8, pad u8, anchor i64:
///             16 bytes; version 3)
///          then RecordedNote × recorded (track u32, pad u32, start i64, length i64,
///               pitch u8, velocity u8, pad ×6: 32 bytes on the wire)
///          then DeviceReadout × readouts (track u32, u32::MAX for the master's chain; device
///               slot u16; parameter id u16; value f32: 12 bytes; version 10): the live value
///               of a parameter a device moves by itself (`Device::readout`) after the last
///               block of the period, for each enabled device that reports one, in track
///               slot then device slot order, the master's chain last; past
///               MAX_READOUTS_PER_REPORT the rest is left out
///          then Spectrum × spectra (track slot u32 (`TrackSlot::MASTER` for the master's
///               chain), device slot u32, sample rate f32, then `analysis::BANDS` bands f32 in
///               dBFS: 396 bytes; the
///               spectrum of a device's last window, computed off the audio path one report
///               earlier, `crate::analysis`; version 11, #156)
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feedback {
    pub transport: TransportState,
    pub epoch: u16,
    pub seq: u32,
    pub playhead_tick: Tick,
    pub playhead_frame: Frame,
    /// The host's frame at the playhead (offset 92, version 9, #733).
    pub host_frame: u64,
    /// The device latency of what is heard, in frames ([`FEEDBACK_LATENCY_OFFSET`], version 10,
    /// #749).
    pub latency_frames: u32,
    /// The compensation is capped: a track is no longer aligned (bit 31 of the latency word).
    pub latency_capped: bool,
    pub cpu_load: f32,
    /// The largest load of one peak window closed in the period (`crate::host`); with a
    /// coarse clock a period may close none and carry the last window, which can be under
    /// `cpu_load`. -1 like it.
    pub cpu_peak: f32,
    pub live_overflows: u32,
    pub rejected: u32,
    pub disk_underruns: u32,
    /// Declared audio sources the engine cannot read (AU-3).
    pub offline_sources: u32,
    /// Bit 0: something did not fit the PCM budget (AU-3).
    pub pcm_flags: u32,
    /// PCM in memory, in KiB (AU-3).
    pub pcm_kib: u32,
    /// Stereo frames the output protection silenced because a sample was non-finite, since the
    /// channel was built (#667); the meters measure before the protection and skip such samples.
    pub silenced_frames: u32,
    pub meters: Vec<TrackMeter>,
    /// The output's meter after the master, last of the meters section on the wire.
    pub master: TrackMeter,
    pub voices: u32,
    pub active_clips: Vec<ActiveClip>,
    /// The last notes of the take, oldest first: indices `record_total - recorded.len()` on.
    pub recorded: Vec<RecordedNote>,
    pub record_take: u32,
    pub record_total: u32,
    pub record_dropped: u32,
    /// The recorder captures: while it does, the take may still grow.
    pub record_capturing: bool,
    /// The live values of the parameters devices move by themselves (#686).
    pub readouts: Vec<DeviceReadout>,
    /// The spectra of the analysed devices (#156, [`crate::analysis`]), at most
    /// [`crate::analysis::MAX_SPECTRA`].
    pub spectra: Vec<crate::analysis::Spectrum>,
}

impl Feedback {
    /// Bytes of a report for `tracks` tracks, `active_clips` grid slots, `recorded` notes,
    /// `readouts` device readouts and `spectra` spectra.
    pub fn size_for(
        tracks: usize,
        active_clips: usize,
        recorded: usize,
        readouts: usize,
        spectra: usize,
    ) -> usize {
        FEEDBACK_HEADER_BYTES
            + (tracks + 1) * TRACK_METER_BYTES
            + active_clips * ACTIVE_CLIP_BYTES
            + recorded * RECORDED_NOTE_WIRE_BYTES
            + readouts * DEVICE_READOUT_BYTES
            + spectra * crate::analysis::SPECTRUM_WIRE_BYTES
    }

    /// The region the audio side writes into: sized for the limits of the moment, resized
    /// by `prepare` when the track table grows.
    pub fn max_size_for(tracks: usize) -> usize {
        Feedback::size_for(
            tracks,
            2 * tracks,
            MAX_RECORDED_PER_REPORT,
            MAX_READOUTS_PER_REPORT,
            crate::analysis::MAX_SPECTRA,
        )
    }
}

/// The live value of a parameter a device moves by itself (`Device::readout`, #686): the
/// Filter's cutoff under its LFO and envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviceReadout {
    /// The device's track, [`TrackSlot::MASTER`] for the master's chain.
    pub track: TrackSlot,
    pub device: DeviceSlot,
    pub param: ParamId,
    /// In the parameter's unit.
    pub value: f32,
}

/// Size of a [`DeviceReadout`] on the wire.
pub const DEVICE_READOUT_BYTES: usize = 12;

/// Most device readouts one report carries.
pub const MAX_READOUTS_PER_REPORT: usize = 128;

/// Size of a [`RecordedNote`] on the wire (padded).
pub const RECORDED_NOTE_WIRE_BYTES: usize = 32;

/// Most recorded notes one report carries, and the recorder keeps: the last ones of the take
/// (`crate::recorder`), so a reader that falls further behind sees a gap in the indices.
pub const MAX_RECORDED_PER_REPORT: usize = 256;
