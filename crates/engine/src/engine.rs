//! The prepare/apply cycle (MO-1, `docs/specs/02-decoupage-mvp.md` § 2.3): the two sides
//! of the engine channel that `crate::msg` declares.
//!
//! - [`Preparer`], the **prepare side** (a Worker in web mode, the helper's control thread
//!   in Boost mode): decodes the `ControlMsg` stream, turns each message into at most one
//!   [`RtMsg`] ([`Preparer::prepare`], which may allocate, fallibly), pushes it on the rt
//!   ring (or parks it in its ordered queue when the ring is full), routes the live notes
//!   to the live queue, and drops the [`Garbage`] the audio side sends back.
//! - [`Engine`], the **audio side**: at the start of each block ([`Engine::begin_block`])
//!   it applies the rt messages that arrived ([`Engine::apply`]: no allocation, no free,
//!   no lock, no panic), then takes the live events of the block, then checks the live
//!   overflow. What it replaces goes back as one garbage item per message.
//!
//! [`channel`] builds both sides, their rings and the initial [`State`]. The transport of
//! each target (the helper's thread, the web Worker on shared memory) is MO-2's; rendering
//! the tracks is MO-3's; routing events with their offsets MO-4's; the transport clock
//! TR-1's; launching LA's. [`State`] holds what they will read, nothing more.
//!
//! # The shadow
//!
//! The prepare side keeps a shadow of the runtime tables the audio side will hold once
//! everything sent is applied: the capacity of every slot array, which tracks and which
//! devices exist. It decides the growths from it (module doc of `crate::msg`) and refuses
//! a message naming a track or a device that does not exist ([`MsgError::NoSuchTarget`]).
//! `prepare` is atomic: every allocation happens first, the shadow and the staging are
//! changed only once they all succeeded, so a refused allocation
//! ([`MsgError::OutOfMemory`]) leaves the prepare side as it was.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use rtrb::{Consumer, Producer, PushError, RingBuffer};

use crate::device::{Device, DeviceSetup, Factory};
use crate::msg::{
    AutomationPoint, AutomationTarget, ClipSlot, ControlMsg, DeviceKind, DeviceSlot, Garbage,
    HEADER_BYTES, Header, LaneSlot, LaunchMsg, LiveEvent, LiveOverflow, MsgError, Note, PAGE_SLOTS,
    Page, PagedGrowth, PagedPut, PagedSpare, Placement, Put, RtMsg, SceneSlot, Slots, SourceSlot,
    Spare, Table, TrackKind, TrackSlot, TransportMsg, limits, state_pairs, try_box,
};
use crate::time::{Frame, LoopRegion, Quantize, Tempo, Tick, TimeSignature};

/// Initial capacities of the runtime tables: small, every table grows on demand
/// (decision 7), each kind up to its safety limit.
pub mod initial {
    pub const TRACKS: usize = 16;
    /// Pages of the clip directory (each page holds `msg::PAGE_SLOTS` clips).
    pub const CLIP_PAGES: usize = 4;
    /// Pages of the lane directory.
    pub const LANE_PAGES: usize = 1;
    pub const SOURCES: usize = 16;
    pub const DEVICES_PER_TRACK: usize = 4;
    pub const SCENES_PER_TRACK: usize = 8;
}

/// Resolves a device kind to the factory that builds it (the registry the host gives).
pub type Resolver = fn(DeviceKind) -> Option<Factory>;

/// The resolver of the built-in devices: none builds yet (DV-1 adds the synth).
pub fn builtin_factory(_kind: DeviceKind) -> Option<Factory> {
    None
}

// ---------------------------------------------------------------- the engine state

/// A device of a track's chain, as `prepare` built it.
pub struct DeviceEntry {
    pub device: Box<dyn Device>,
    pub enabled: bool,
}

impl core::fmt::Debug for DeviceEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceEntry")
            .field("kind", &self.device.descriptor().kind)
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// A launch asked for a track, executed at the next quantization boundary (LA's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchRequest {
    Clip(SceneSlot),
    Stop,
}

/// A track: its mix, its chain (a slot array of devices), its row of the launch grid (a
/// slot array of clips) and its arrangement (a content snapshot).
#[derive(Debug)]
pub struct Track {
    pub kind: TrackKind,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub monitor: bool,
    pub devices: Slots<Box<DeviceEntry>>,
    pub scenes: Slots<ClipSlot>,
    pub placements: Table<Placement>,
    /// The last launch asked for this track, not yet executed.
    pub launch: Option<LaunchRequest>,
}

/// What a clip plays.
#[derive(Debug)]
pub enum ClipContent {
    Notes(Table<Note>),
    Audio {
        source: SourceSlot,
        start: Frame,
        length: Frame,
    },
}

/// A clip: its content, its length and whether it loops (ticks, MIDI clips).
#[derive(Debug)]
pub struct Clip {
    pub length: Tick,
    pub looped: bool,
    pub content: ClipContent,
}

/// An automation lane: what it drives and its points (a content snapshot).
#[derive(Debug)]
pub struct Lane {
    pub track: TrackSlot,
    pub target: AutomationTarget,
    pub points: Table<AutomationPoint>,
}

/// A declared audio source; AU-3 adds its streaming buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub sample_rate: u32,
    pub channels: u8,
    pub frames: Frame,
}

/// Transport settings (`SetTransport`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TransportSettings {
    pub tempo: Tempo,
    pub signature: TimeSignature,
    pub launch_quantize: Quantize,
}

/// The metronome (`SetMetronome`, TR-2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metronome {
    pub enabled: bool,
    pub volume: f32,
    pub count_in_bars: u8,
}

/// The master mix (`SetMaster`, MX-1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Master {
    pub gain: f32,
    pub mute: bool,
}

impl Default for Master {
    fn default() -> Self {
        Master {
            gain: 1.0,
            mute: false,
        }
    }
}

/// What the transport orders asked; TR-1's clock reads and consumes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playback {
    pub playing: bool,
    pub recording: bool,
    pub loop_region: Option<LoopRegion>,
    /// The position the last `Play` or `Seek` asked for, not yet taken by the clock.
    pub cue: Option<Tick>,
}

/// The launches asked for the whole grid, not yet executed (LA's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridRequests {
    pub scene: Option<SceneSlot>,
    pub stop_all: bool,
    pub back_to_arrangement: bool,
}

/// A runtime table too large to move slot by slot (clips, lanes): a directory of pages of
/// [`PAGE_SLOTS`] slots. A page is allocated by `prepare` the first time one of its slots
/// is used; the directory grows like any slot array, so a growth moves one pointer per
/// page (at most `limit / PAGE_SLOTS`), never one per slot.
#[derive(Debug)]
pub struct Paged<T> {
    pages: Slots<Page<T>>,
}

impl<T> Paged<T> {
    /// An empty table whose directory holds `pages` pages, none allocated.
    pub fn try_new(pages: usize) -> Result<Self, MsgError> {
        Ok(Paged {
            pages: Table::try_filled(pages)?,
        })
    }

    /// Slots the directory can address.
    pub fn capacity(&self) -> usize {
        self.pages.capacity().saturating_mul(PAGE_SLOTS)
    }

    /// Pages the directory holds (allocated or not).
    pub fn directory_capacity(&self) -> usize {
        self.pages.capacity()
    }

    /// The value in `slot`, if any.
    pub fn get(&self, slot: u32) -> Option<&T> {
        let slot = slot as usize;
        self.pages
            .as_slice()
            .get(slot / PAGE_SLOTS)?
            .as_ref()?
            .as_slice()
            .get(slot % PAGE_SLOTS)?
            .as_ref()
    }

    /// The values held, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.pages
            .as_slice()
            .iter()
            .flatten()
            .flat_map(|page| page.as_slice().iter().flatten())
    }

    /// Puts a value on the audio path (see [`PagedPut`]): the larger directory first, then
    /// the page, then the value. Returns what is left over and whether the put was a
    /// contract violation (a directory not larger than the live one, a slot beyond it).
    fn put(&mut self, put: PagedPut<T>) -> (PagedSpare<T>, bool) {
        let PagedPut {
            slot,
            value,
            growth,
        } = put;
        let mut violation = false;
        let mut spare = PagedSpare::NONE;
        let index = slot as usize;
        if let Some(mut growth) = growth {
            if let Some(mut directory) = growth.directory.take() {
                if directory.capacity() > self.pages.capacity() {
                    directory.take_slots_from(&mut self.pages);
                    growth.directory = Some(core::mem::replace(&mut self.pages, directory));
                } else {
                    growth.directory = Some(directory);
                    violation = true;
                }
            }
            if let Some(page) = growth.page.take() {
                match self.pages.as_mut_slice().get_mut(index / PAGE_SLOTS) {
                    Some(entry @ None) => *entry = Some(page),
                    _ => growth.page = Some(page),
                }
            }
            spare.growth = Some(growth);
        }
        let entry = self
            .pages
            .as_mut_slice()
            .get_mut(index / PAGE_SLOTS)
            .and_then(Option::as_mut)
            .and_then(|page| page.as_mut_slice().get_mut(index % PAGE_SLOTS));
        match entry {
            Some(entry) => spare.item = entry.replace(value),
            None => {
                spare.item = Some(value);
                violation = true;
            }
        }
        (spare, violation)
    }

    /// Takes the value out of `slot` (`None` when empty or its page was never allocated);
    /// `Err` when the slot is beyond the directory.
    fn take(&mut self, slot: u32) -> Result<Option<T>, ()> {
        let index = slot as usize;
        match self.pages.as_mut_slice().get_mut(index / PAGE_SLOTS) {
            Some(Some(page)) => Ok(page
                .as_mut_slice()
                .get_mut(index % PAGE_SLOTS)
                .and_then(Option::take)),
            Some(None) => Ok(None),
            None => Err(()),
        }
    }
}

/// Everything the audio side holds about the project. Built by `prepare` (the initial one
/// by [`channel`], a fresh one for each `Reset`), mutated only by `apply`.
#[derive(Debug)]
pub struct State {
    pub transport: TransportSettings,
    pub metronome: Metronome,
    pub master: Master,
    pub playback: Playback,
    pub grid: GridRequests,
    pub selected: Option<TrackSlot>,
    pub tracks: Slots<Box<Track>>,
    pub clips: Paged<Box<Clip>>,
    pub lanes: Paged<Box<Lane>>,
    pub sources: Slots<Box<Source>>,
}

impl State {
    /// An empty state with the initial capacities, through fallible allocations.
    pub fn try_new() -> Result<Self, MsgError> {
        Ok(State {
            transport: TransportSettings::default(),
            metronome: Metronome::default(),
            master: Master::default(),
            playback: Playback::default(),
            grid: GridRequests::default(),
            selected: None,
            tracks: Table::try_filled(initial::TRACKS)?,
            clips: Paged::try_new(initial::CLIP_PAGES)?,
            lanes: Paged::try_new(initial::LANE_PAGES)?,
            sources: Table::try_filled(initial::SOURCES)?,
        })
    }

    /// The track in `slot`, if it exists.
    pub fn track(&self, slot: TrackSlot) -> Option<&Track> {
        self.tracks.as_slice().get(slot.0 as usize)?.as_deref()
    }

    /// The track in `slot`, if it exists, to mutate it.
    pub fn track_mut(&mut self, slot: TrackSlot) -> Option<&mut Track> {
        self.tracks
            .as_mut_slice()
            .get_mut(slot.0 as usize)?
            .as_deref_mut()
    }

    /// The device in `device` of track `track`, if it exists.
    pub fn device(&self, track: TrackSlot, device: DeviceSlot) -> Option<&DeviceEntry> {
        self.track(track)?
            .devices
            .as_slice()
            .get(device.0 as usize)?
            .as_deref()
    }

    /// The clip in `slot`, if it exists.
    pub fn clip(&self, slot: ClipSlot) -> Option<&Clip> {
        self.clips.get(slot.0).map(|c| &**c)
    }

    /// The lane in `slot`, if it exists.
    pub fn lane(&self, slot: LaneSlot) -> Option<&Lane> {
        self.lanes.get(slot.0).map(|l| &**l)
    }

    /// The source in `slot`, if it exists.
    pub fn source(&self, slot: SourceSlot) -> Option<&Source> {
        self.sources.as_slice().get(slot.0 as usize)?.as_deref()
    }
}

// ---------------------------------------------------------------- channel

/// Builds the two sides of the engine channel: the rt ring ([`limits::RT_RING_CAPACITY`]),
/// the garbage ring ([`limits::GARBAGE_RING_CAPACITY`]), the live queue
/// ([`limits::LIVE_QUEUE_CAPACITY`]), the overflow counter, the prepare side's queue
/// ([`limits::PENDING_QUEUE_CAPACITY`], reserved now) and the initial state. `setup` is
/// what devices are built for; `factories` resolves a `DeviceInsert`'s kind.
pub fn channel(setup: DeviceSetup, factories: Resolver) -> Result<(Preparer, Engine), MsgError> {
    channel_with(setup, factories, limits::RT_RING_CAPACITY)
}

/// [`channel`] with a smaller rt ring (and its garbage ring one slot larger), for tests of
/// the backpressure.
pub fn channel_with(
    setup: DeviceSetup,
    factories: Resolver,
    rt_capacity: usize,
) -> Result<(Preparer, Engine), MsgError> {
    let rt_capacity = rt_capacity.max(1);
    let (rt_tx, rt_rx) = RingBuffer::new(rt_capacity);
    let (gc_tx, gc_rx) = RingBuffer::new(rt_capacity + 1);
    let (live_tx, live_rx) = RingBuffer::new(limits::LIVE_QUEUE_CAPACITY);
    let overflow = Arc::new(LiveOverflow::new());
    let mut pending = VecDeque::new();
    pending
        .try_reserve_exact(limits::PENDING_QUEUE_CAPACITY)
        .map_err(|_| MsgError::OutOfMemory {
            bytes: limits::PENDING_QUEUE_CAPACITY * size_of::<RtMsg>(),
        })?;
    let preparer = Preparer {
        epoch: 0,
        setup,
        factories,
        rt: rt_tx,
        garbage: gc_rx,
        live: live_tx,
        overflow: Arc::clone(&overflow),
        pending,
        pending_capacity: limits::PENDING_QUEUE_CAPACITY,
        shadow: Shadow::try_new()?,
        notes: HashMap::new(),
        points: HashMap::new(),
        placements: HashMap::new(),
        rejected: 0,
        collected: 0,
    };
    let engine = Engine {
        state: try_box(State::try_new()?)?,
        rt: rt_rx,
        garbage: gc_tx,
        live: live_rx,
        overflow,
        overflow_seen: 0,
        live_block: Table::try_with_capacity(limits::LIVE_QUEUE_CAPACITY)?,
        release_live: false,
        applied: 0,
        skipped: 0,
        leaked: 0,
    };
    Ok((preparer, engine))
}

// ---------------------------------------------------------------- the prepare side

/// What the prepare side knows of a track it created.
#[derive(Debug)]
struct TrackShadow {
    /// One flag per slot of the track's device table: whether a device is there.
    devices: Vec<bool>,
    scenes_capacity: usize,
}

impl TrackShadow {
    fn try_new() -> Result<Self, MsgError> {
        Ok(TrackShadow {
            devices: try_vec(initial::DEVICES_PER_TRACK, false)?,
            scenes_capacity: initial::SCENES_PER_TRACK,
        })
    }
}

/// The runtime tables as the audio side will hold them once everything sent is applied.
#[derive(Debug)]
struct Shadow {
    /// One entry per slot of the track table.
    tracks: Vec<Option<TrackShadow>>,
    clips: PagedShadow,
    lanes: PagedShadow,
    sources: usize,
}

/// What the prepare side knows of a paged table: which pages it allocated, one flag per
/// page of the directory.
#[derive(Debug)]
struct PagedShadow {
    pages: Vec<bool>,
}

impl PagedShadow {
    fn try_new(pages: usize) -> Result<Self, MsgError> {
        Ok(PagedShadow {
            pages: try_vec(pages, false)?,
        })
    }

    /// The growth a put in `slot` needs, built (fallibly) but not committed; `limit` is the
    /// table's safety limit in slots.
    fn growth<T>(&self, slot: u32, limit: u32) -> Result<Option<Box<PagedGrowth<T>>>, MsgError> {
        let page = slot as usize / PAGE_SLOTS;
        let pages_limit = (limit as usize).div_ceil(PAGE_SLOTS);
        let directory = grow_for::<Page<T>>(self.pages.len(), page, pages_limit as u32)?;
        let needs_page = !self.pages.get(page).copied().unwrap_or(false);
        if directory.is_none() && !needs_page {
            return Ok(None);
        }
        let page = if needs_page {
            Some(Table::try_filled(PAGE_SLOTS)?)
        } else {
            None
        };
        Ok(Some(try_box(PagedGrowth { directory, page })?))
    }

    /// Reserves the flags of a growth (fallible, nothing changes).
    fn reserve<T>(&mut self, growth: &Option<Box<PagedGrowth<T>>>) -> Result<(), MsgError> {
        if let Some(directory) = growth.as_ref().and_then(|g| g.directory.as_ref()) {
            let more = directory.capacity().saturating_sub(self.pages.len());
            self.pages
                .try_reserve_exact(more)
                .map_err(|_| oom::<bool>(more))?;
        }
        Ok(())
    }

    /// Commits a growth whose flags were reserved: no allocation.
    fn commit<T>(&mut self, slot: u32, growth: &Option<Box<PagedGrowth<T>>>) {
        let Some(growth) = growth else { return };
        if let Some(directory) = &growth.directory {
            self.pages.resize(directory.capacity(), false);
        }
        if growth.page.is_some()
            && let Some(flag) = self.pages.get_mut(slot as usize / PAGE_SLOTS)
        {
            *flag = true;
        }
    }
}

impl Shadow {
    fn try_new() -> Result<Self, MsgError> {
        let mut tracks = Vec::new();
        tracks
            .try_reserve_exact(initial::TRACKS)
            .map_err(|_| oom::<Option<TrackShadow>>(initial::TRACKS))?;
        tracks.resize_with(initial::TRACKS, || None);
        Ok(Shadow {
            tracks,
            clips: PagedShadow::try_new(initial::CLIP_PAGES)?,
            lanes: PagedShadow::try_new(initial::LANE_PAGES)?,
            sources: initial::SOURCES,
        })
    }

    fn track(&self, slot: TrackSlot) -> Option<&TrackShadow> {
        self.tracks.get(slot.0 as usize)?.as_ref()
    }

    fn has_device(&self, track: TrackSlot, device: DeviceSlot) -> bool {
        self.track(track)
            .and_then(|t| t.devices.get(device.0 as usize).copied())
            .unwrap_or(false)
    }
}

/// A chunked upload being staged (module doc of `crate::msg`, chunked uploads).
#[derive(Debug)]
struct Stage<T> {
    total: u32,
    items: Vec<T>,
}

/// The prepare side of the channel (module doc). Off the audio path: it may allocate,
/// always fallibly.
pub struct Preparer {
    epoch: u16,
    setup: DeviceSetup,
    factories: Resolver,
    rt: Producer<RtMsg>,
    garbage: Consumer<Garbage>,
    live: Producer<LiveEvent>,
    overflow: Arc<LiveOverflow>,
    pending: VecDeque<RtMsg>,
    pending_capacity: usize,
    shadow: Shadow,
    notes: HashMap<u32, Stage<Note>>,
    points: HashMap<u32, Stage<AutomationPoint>>,
    placements: HashMap<u32, Stage<Placement>>,
    rejected: u32,
    collected: u64,
}

impl core::fmt::Debug for Preparer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Preparer")
            .field("epoch", &self.epoch)
            .field("pending", &self.pending.len())
            .field("rejected", &self.rejected)
            .finish_non_exhaustive()
    }
}

/// The allocation error of `n` items of `T`.
fn oom<T>(n: usize) -> MsgError {
    MsgError::OutOfMemory {
        bytes: n.saturating_mul(size_of::<T>()),
    }
}

fn try_vec<T: Clone>(n: usize, value: T) -> Result<Vec<T>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| oom::<T>(n))?;
    v.resize(n, value);
    Ok(v)
}

/// The capacity a slot array grows to so that `slot` fits: doubled until it does, never
/// above `limit` (the project keeps every slot below it).
fn grown_capacity(current: usize, slot: usize, limit: usize) -> usize {
    let mut capacity = current.max(1);
    while capacity <= slot {
        capacity = capacity.saturating_mul(2);
    }
    capacity.min(limit.max(slot + 1))
}

/// The next slot array when `slot` is beyond `current`, else `None`.
fn grow_for<T>(current: usize, slot: usize, limit: u32) -> Result<Option<Slots<T>>, MsgError> {
    if slot < current {
        return Ok(None);
    }
    Ok(Some(Table::try_filled(grown_capacity(
        current,
        slot,
        limit as usize,
    ))?))
}

impl Preparer {
    /// The epoch of the engine generation this side accepts.
    pub fn epoch(&self) -> u16 {
        self.epoch
    }

    /// Messages refused so far (unknown tags, stale epochs, bad payloads, limits, unknown
    /// devices, missing targets, bad chunks): `Feedback::rejected`.
    pub fn rejected(&self) -> u32 {
        self.rejected
    }

    /// Rt messages parked because the ring was full.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Garbage items dropped so far.
    pub fn collected(&self) -> u64 {
        self.collected
    }

    /// The live overflow counter this side increments (shared with the audio side).
    pub fn overflow(&self) -> &LiveOverflow {
        &self.overflow
    }

    /// Decodes and handles a stream of messages (the binary format of `crate::msg`).
    /// Unknown tags, stale epochs and rejected messages are skipped and counted; a corrupt
    /// stream, a refused allocation or a stalled engine stops it and is returned: the
    /// project shows it and resyncs with a `Reset`.
    pub fn submit(&mut self, stream: &[u8]) -> Result<(), MsgError> {
        let mut at = 0usize;
        while at < stream.len() {
            let bytes = &stream[at..];
            match Header::decode(bytes, self.epoch) {
                Ok(header) => {
                    let end = HEADER_BYTES + header.len as usize;
                    let outcome = ControlMsg::decode(&header, &bytes[HEADER_BYTES..end])
                        .and_then(|msg| self.handle(msg));
                    if let Err(error) = outcome {
                        if error.stops_the_stream() {
                            return Err(error);
                        }
                        self.rejected = self.rejected.saturating_add(1);
                    }
                    at = at.saturating_add(Header::frame_len(header.len));
                }
                Err(MsgError::UnknownTag { skip, .. }) => {
                    self.rejected = self.rejected.saturating_add(1);
                    at = at.saturating_add(skip);
                }
                Err(MsgError::StaleEpoch { .. }) => {
                    // The header is whole (the epoch is checked after the length), so its
                    // length says where the next message starts.
                    let len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
                    self.rejected = self.rejected.saturating_add(1);
                    at = at.saturating_add(Header::frame_len(len));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Handles one decoded message: a live note goes to the live queue (on overflow it is
    /// dropped and counted, [`LiveOverflow`]); anything else is prepared and sent. A
    /// rejection is returned to the caller (`submit` counts it).
    pub fn handle(&mut self, msg: ControlMsg) -> Result<(), MsgError> {
        msg.check_limits()?;
        if let Some(event) = live_event(&msg) {
            if self.live.push(event).is_err() {
                self.overflow.record();
            }
            return Ok(());
        }
        self.flush();
        if self.pending.len() >= self.pending_capacity {
            return Err(MsgError::EngineBusy);
        }
        if let Some(rt) = self.prepare(msg)? {
            self.send(rt);
        }
        Ok(())
    }

    /// Pushes what waited in the queue, in order, as far as the ring has room; drains the
    /// garbage ring before every push (the invariant of `crate::msg`).
    pub fn flush(&mut self) {
        self.collect();
        while let Some(msg) = self.pending.pop_front() {
            self.collect();
            if let Err(PushError::Full(refused)) = self.rt.push(msg) {
                self.pending.push_front(refused);
                break;
            }
        }
    }

    /// Drops what the audio side sent back; returns how many items.
    pub fn collect(&mut self) -> usize {
        let mut n = 0;
        while let Ok(garbage) = self.garbage.pop() {
            drop(garbage);
            n += 1;
        }
        self.collected += n as u64;
        n
    }

    /// Sends a prepared message: on the ring when nothing waits and it has room, else at
    /// the end of the queue (whose room `handle` checked: no allocation here).
    fn send(&mut self, msg: RtMsg) {
        if self.pending.is_empty() {
            self.collect();
            if let Err(PushError::Full(refused)) = self.rt.push(msg) {
                self.pending.push_back(refused);
            }
        } else {
            self.pending.push_back(msg);
        }
    }

    /// `Engine::prepare` of the spec: turns a control message into at most one rt message,
    /// off the audio path. It allocates what the audio side will need (tables, devices,
    /// grown slot arrays), fallibly, and commits its shadow and staging only once every
    /// allocation succeeded. `Ok(None)`: nothing reaches the audio side (a chunk staged, a
    /// live note, which [`Preparer::handle`] routes instead).
    pub fn prepare(&mut self, msg: ControlMsg) -> Result<Option<RtMsg>, MsgError> {
        let rt = match msg {
            ControlMsg::Reset { epoch } => {
                let state = try_box(State::try_new()?)?;
                let shadow = Shadow::try_new()?;
                self.epoch = epoch;
                self.shadow = shadow;
                self.notes.clear();
                self.points.clear();
                self.placements.clear();
                RtMsg::Reset(state)
            }
            ControlMsg::SetTransport {
                tempo,
                signature,
                launch_quantize,
            } => RtMsg::SetTransport {
                tempo,
                signature,
                launch_quantize,
            },
            ControlMsg::SetMetronome {
                enabled,
                volume,
                count_in_bars,
            } => RtMsg::SetMetronome {
                enabled,
                volume,
                count_in_bars,
            },
            ControlMsg::SetMaster { gain, mute } => RtMsg::SetMaster { gain, mute },
            ControlMsg::TrackCreate { track, kind } => self.prepare_track(track, kind)?,
            ControlMsg::TrackRemove { track } => {
                self.existing_track(track)?;
                self.shadow.tracks[track.0 as usize] = None;
                self.placements.remove(&track.0);
                RtMsg::TrackRemove(track)
            }
            ControlMsg::TrackSetMix {
                track,
                gain,
                pan,
                mute,
            } => {
                self.existing_track(track)?;
                RtMsg::TrackSetMix {
                    track,
                    gain,
                    pan,
                    mute,
                }
            }
            ControlMsg::TrackSetSolo { track, solo } => {
                self.existing_track(track)?;
                RtMsg::TrackSetSolo { track, solo }
            }
            ControlMsg::TrackArm { track, armed } => {
                self.existing_track(track)?;
                RtMsg::TrackArm { track, armed }
            }
            ControlMsg::TrackSetMonitor { track, monitor } => {
                self.existing_track(track)?;
                RtMsg::TrackSetMonitor { track, monitor }
            }
            ControlMsg::SelectTrack { track } => {
                if let Some(t) = track {
                    self.existing_track(t)?;
                }
                RtMsg::SelectTrack { track }
            }
            ControlMsg::DeviceInsert {
                track,
                device,
                kind,
                state,
            } => self.prepare_device(track, device, kind, &state)?,
            ControlMsg::DeviceRemove { track, device } => {
                self.existing_device(track, device)?;
                self.set_device(track, device, false);
                RtMsg::DeviceRemove { track, device }
            }
            ControlMsg::DeviceSetEnabled {
                track,
                device,
                enabled,
            } => {
                self.existing_device(track, device)?;
                RtMsg::DeviceSetEnabled {
                    track,
                    device,
                    enabled,
                }
            }
            ControlMsg::ParamSet {
                track,
                device,
                param,
                value,
            } => {
                self.existing_device(track, device)?;
                RtMsg::ParamSet {
                    track,
                    device,
                    param,
                    value,
                }
            }
            ControlMsg::ClipNotesUpload {
                clip,
                length,
                looped,
                first,
                total,
                notes,
            } => {
                // Everything allocated before the chunk is staged: a refusal leaves the
                // staging as it was.
                let (mut value, growth) = self.prepare_clip(
                    clip,
                    Clip {
                        length,
                        looped,
                        content: ClipContent::Notes(Table::from_vec(Vec::new())),
                    },
                )?;
                let staged = stage(
                    &mut self.notes,
                    clip.0,
                    first,
                    total,
                    notes,
                    &mut self.rejected,
                )?;
                let Some(notes) = staged else {
                    return Ok(None);
                };
                value.content = ClipContent::Notes(Table::from_vec(notes));
                self.finish_clip(clip, value, growth)
            }
            ControlMsg::ClipRemove { clip } => {
                self.notes.remove(&clip.0);
                RtMsg::ClipRemove(clip)
            }
            ControlMsg::AudioSourceDeclare {
                source,
                sample_rate,
                channels,
                frames,
            } => {
                let value = try_box(Source {
                    sample_rate,
                    channels,
                    frames,
                })?;
                let grown = grow_for(
                    self.shadow.sources,
                    source.0 as usize,
                    limits::MAX_AUDIO_SOURCES,
                )?;
                if let Some(g) = &grown {
                    self.shadow.sources = g.capacity();
                }
                RtMsg::SourcePut(Put {
                    slot: source.0,
                    value,
                    grown,
                })
            }
            ControlMsg::AudioSourceRelease { source } => RtMsg::SourceRemove(source),
            ControlMsg::ClipAudioSet {
                clip,
                source,
                start,
                length,
            } => {
                let (value, growth) = self.prepare_clip(
                    clip,
                    Clip {
                        length: Tick::ZERO,
                        looped: false,
                        content: ClipContent::Audio {
                            source,
                            start,
                            length,
                        },
                    },
                )?;
                self.notes.remove(&clip.0);
                self.finish_clip(clip, value, growth)
            }
            ControlMsg::TrackPlacementsUpload {
                track,
                first,
                total,
                placements,
            } => {
                self.existing_track(track)?;
                let staged = stage(
                    &mut self.placements,
                    track.0,
                    first,
                    total,
                    placements,
                    &mut self.rejected,
                )?;
                let Some(placements) = staged else {
                    return Ok(None);
                };
                RtMsg::PlacementsSet {
                    track,
                    placements: Table::from_vec(placements),
                }
            }
            ControlMsg::AutomationLaneUpload {
                lane,
                track,
                target,
                first,
                total,
                points,
            } => {
                self.existing_track(track)?;
                // Allocate the box and the grown table before staging the last chunk, so
                // that a refusal leaves the staging as it was.
                let growth = self.shadow.lanes.growth(lane.0, limits::MAX_LANES)?;
                self.shadow.lanes.reserve(&growth)?;
                let mut value = try_box(Lane {
                    track,
                    target,
                    points: Table::from_vec(Vec::new()),
                })?;
                let staged = stage(
                    &mut self.points,
                    lane.0,
                    first,
                    total,
                    points,
                    &mut self.rejected,
                )?;
                let Some(points) = staged else {
                    return Ok(None);
                };
                value.points = Table::from_vec(points);
                self.shadow.lanes.commit(lane.0, &growth);
                RtMsg::LanePut(PagedPut {
                    slot: lane.0,
                    value,
                    growth,
                })
            }
            ControlMsg::AutomationLaneRemove { lane } => {
                self.points.remove(&lane.0);
                RtMsg::LaneRemove(lane)
            }
            ControlMsg::SceneSetSlot { track, scene, clip } => {
                let capacity = self.existing_track(track)?.scenes_capacity;
                let grown = grow_for(capacity, scene.0 as usize, limits::MAX_SCENES)?;
                if let (Some(g), Some(Some(t))) =
                    (&grown, self.shadow.tracks.get_mut(track.0 as usize))
                {
                    t.scenes_capacity = g.capacity();
                }
                RtMsg::SceneSet {
                    track,
                    scene,
                    clip,
                    grown,
                }
            }
            ControlMsg::LaunchClip { track, scene } => {
                self.existing_track(track)?;
                RtMsg::Launch(LaunchMsg::Clip { track, scene })
            }
            ControlMsg::LaunchScene { scene } => RtMsg::Launch(LaunchMsg::Scene { scene }),
            ControlMsg::StopTrack { track } => {
                self.existing_track(track)?;
                RtMsg::Launch(LaunchMsg::StopTrack { track })
            }
            ControlMsg::StopAll => RtMsg::Launch(LaunchMsg::StopAll),
            ControlMsg::BackToArrangement => RtMsg::Launch(LaunchMsg::BackToArrangement),
            ControlMsg::Play { from } => RtMsg::Transport(TransportMsg::Play { from }),
            ControlMsg::Stop => RtMsg::Transport(TransportMsg::Stop),
            ControlMsg::Seek { tick } => RtMsg::Transport(TransportMsg::Seek { tick }),
            ControlMsg::SetLoop { region } => RtMsg::Transport(TransportMsg::SetLoop { region }),
            ControlMsg::SetRecording { recording } => {
                RtMsg::Transport(TransportMsg::SetRecording { recording })
            }
            ControlMsg::NoteOn { .. }
            | ControlMsg::NoteOff { .. }
            | ControlMsg::AllNotesOff { .. } => {
                return Ok(None);
            }
        };
        Ok(Some(rt))
    }

    fn existing_track(&self, track: TrackSlot) -> Result<&TrackShadow, MsgError> {
        self.shadow.track(track).ok_or(MsgError::NoSuchTarget)
    }

    fn existing_device(&self, track: TrackSlot, device: DeviceSlot) -> Result<(), MsgError> {
        if self.shadow.has_device(track, device) {
            Ok(())
        } else {
            Err(MsgError::NoSuchTarget)
        }
    }

    fn set_device(&mut self, track: TrackSlot, device: DeviceSlot, present: bool) {
        if let Some(Some(t)) = self.shadow.tracks.get_mut(track.0 as usize)
            && let Some(flag) = t.devices.get_mut(device.0 as usize)
        {
            *flag = present;
        }
    }

    fn prepare_track(&mut self, track: TrackSlot, kind: TrackKind) -> Result<RtMsg, MsgError> {
        let slot = track.0 as usize;
        let value = try_box(Track {
            kind,
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            armed: false,
            monitor: false,
            devices: Table::try_filled(initial::DEVICES_PER_TRACK)?,
            scenes: Table::try_filled(initial::SCENES_PER_TRACK)?,
            placements: Table::from_vec(Vec::new()),
            launch: None,
        })?;
        let shadow = TrackShadow::try_new()?;
        let grown = grow_for(self.shadow.tracks.len(), slot, limits::MAX_TRACKS)?;
        if let Some(g) = &grown {
            let more = g.capacity() - self.shadow.tracks.len();
            self.shadow
                .tracks
                .try_reserve_exact(more)
                .map_err(|_| oom::<Option<TrackShadow>>(more))?;
            self.shadow.tracks.resize_with(g.capacity(), || None);
        }
        // Commit: nothing below allocates or fails.
        self.shadow.tracks[slot] = Some(shadow);
        // A new track starts with an empty arrangement: a staging for an old one is void.
        self.placements.remove(&track.0);
        Ok(RtMsg::TrackPut(Put {
            slot: track.0,
            value,
            grown,
        }))
    }

    fn prepare_device(
        &mut self,
        track: TrackSlot,
        device: DeviceSlot,
        kind: DeviceKind,
        state: &[u8],
    ) -> Result<RtMsg, MsgError> {
        let capacity = self.existing_track(track)?.devices.len();
        let factory = (self.factories)(kind).ok_or(MsgError::UnknownDevice(kind))?;
        let mut built = factory(&self.setup)?;
        for (id, value) in state_pairs(state) {
            built.set_param(id, value);
        }
        built.reset();
        let value = try_box(DeviceEntry {
            device: built,
            enabled: true,
        })?;
        let slot = device.0 as usize;
        let grown = grow_for(capacity, slot, limits::MAX_DEVICES_PER_TRACK)?;
        let mut flags = None;
        if let Some(g) = &grown {
            flags = Some(try_vec(g.capacity(), false)?);
        }
        // Commit.
        if let Some(Some(t)) = self.shadow.tracks.get_mut(track.0 as usize) {
            if let Some(mut flags) = flags {
                flags[..t.devices.len()].copy_from_slice(&t.devices);
                t.devices = flags;
            }
            if let Some(flag) = t.devices.get_mut(slot) {
                *flag = true;
            }
        }
        Ok(RtMsg::DevicePut {
            track,
            put: Put {
                slot: device.0.into(),
                value,
                grown,
            },
        })
    }

    /// Allocates what a clip put needs (its box, the growth of the clip table, the room
    /// of the shadow's flags), changing nothing.
    #[allow(clippy::type_complexity)]
    fn prepare_clip(
        &mut self,
        clip: ClipSlot,
        value: Clip,
    ) -> Result<(Box<Clip>, Option<Box<PagedGrowth<Box<Clip>>>>), MsgError> {
        let value = try_box(value)?;
        let growth = self.shadow.clips.growth(clip.0, limits::MAX_CLIPS)?;
        self.shadow.clips.reserve(&growth)?;
        Ok((value, growth))
    }

    /// Commits a clip put prepared by [`Preparer::prepare_clip`]: no allocation.
    fn finish_clip(
        &mut self,
        clip: ClipSlot,
        value: Box<Clip>,
        growth: Option<Box<PagedGrowth<Box<Clip>>>>,
    ) -> RtMsg {
        self.shadow.clips.commit(clip.0, &growth);
        RtMsg::ClipPut(PagedPut {
            slot: clip.0,
            value,
            growth,
        })
    }
}

/// The live event of a live note, `None` for any other message.
fn live_event(msg: &ControlMsg) -> Option<LiveEvent> {
    match *msg {
        ControlMsg::NoteOn {
            track,
            pitch,
            velocity,
        } => Some(LiveEvent::NoteOn {
            track,
            pitch,
            velocity,
        }),
        ControlMsg::NoteOff { track, pitch } => Some(LiveEvent::NoteOff { track, pitch }),
        ControlMsg::AllNotesOff { track } => Some(LiveEvent::AllNotesOff { track }),
        _ => None,
    }
}

/// Stages one chunk of an upload into `map[slot]` (module doc of `crate::msg`): appended
/// when `first` is the staged length and `total` the staged total; otherwise the staging
/// restarts from this chunk when it is a first chunk, or is dropped, and the chunk counts
/// as rejected. Returns the whole list once `first + len == total`. A refused allocation
/// leaves `map` as it was.
fn stage<T>(
    map: &mut HashMap<u32, Stage<T>>,
    slot: u32,
    first: u32,
    total: u32,
    chunk: Vec<T>,
    rejected: &mut u32,
) -> Result<Option<Vec<T>>, MsgError> {
    let continues = map
        .get(&slot)
        .is_some_and(|s| s.items.len() as u64 == u64::from(first) && s.total == total);
    if !continues {
        let had_staging = map.contains_key(&slot);
        if first != 0 {
            // Out of order, duplicated, a changed total, or no staging to continue.
            map.remove(&slot);
            *rejected = rejected.saturating_add(1);
            return Ok(None);
        }
        let mut items = Vec::new();
        items
            .try_reserve_exact(total as usize)
            .map_err(|_| oom::<T>(total as usize))?;
        map.try_reserve(1).map_err(|_| oom::<Stage<T>>(1))?;
        if had_staging {
            *rejected = rejected.saturating_add(1);
        }
        map.insert(slot, Stage { total, items });
    }
    let Some(stage) = map.get_mut(&slot) else {
        return Ok(None);
    };
    // `check_limits` proved `first + len <= total`, and the staging holds `first` items of
    // a reservation of `total`: this extends within the reserved capacity.
    if stage.items.len() + chunk.len() > stage.total as usize {
        map.remove(&slot);
        *rejected = rejected.saturating_add(1);
        return Ok(None);
    }
    stage.items.extend(chunk);
    if stage.items.len() == stage.total as usize {
        Ok(map.remove(&slot).map(|s| s.items))
    } else {
        Ok(None)
    }
}

// ---------------------------------------------------------------- the audio side

/// The audio side of the channel (module doc): owns the [`State`] and applies the rt
/// messages at the start of each block. Nothing here allocates, frees, locks or panics.
pub struct Engine {
    state: Box<State>,
    rt: Consumer<RtMsg>,
    garbage: Producer<Garbage>,
    live: Consumer<LiveEvent>,
    overflow: Arc<LiveOverflow>,
    overflow_seen: u32,
    live_block: Table<LiveEvent>,
    release_live: bool,
    applied: u64,
    skipped: u32,
    leaked: u32,
}

impl core::fmt::Debug for Engine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Engine")
            .field("applied", &self.applied)
            .field("skipped", &self.skipped)
            .finish_non_exhaustive()
    }
}

/// Puts `put` in `table` (growing it first when it carries a larger table) and returns
/// what is left over: the old table, the replaced value, or the value itself when its slot
/// is beyond the table (`true`: a contract violation, counted as skipped).
fn put_slot<T>(table: &mut Slots<T>, put: Put<T>) -> (Spare<T>, bool) {
    let Put { slot, value, grown } = put;
    let mut spare = Spare::NONE;
    let mut violation = false;
    if let Some(mut next) = grown {
        if next.capacity() > table.capacity() {
            next.take_slots_from(table);
            spare.table = Some(core::mem::replace(table, next));
        } else {
            // A "larger" table that is not: moving into it would lose live entries.
            spare.table = Some(next);
            violation = true;
        }
    }
    match table.as_mut_slice().get_mut(slot as usize) {
        Some(entry) => spare.item = entry.replace(value),
        None => {
            spare.item = Some(value);
            violation = true;
        }
    }
    (spare, violation)
}

/// Takes the value out of slot `slot` (`None` when empty); `Err` when the slot is beyond
/// the table (a contract violation, counted as skipped).
fn take_slot<T>(table: &mut Slots<T>, slot: u32) -> Result<Option<T>, ()> {
    match table.as_mut_slice().get_mut(slot as usize) {
        Some(entry) => Ok(entry.take()),
        None => Err(()),
    }
}

impl Engine {
    /// The state the audio side renders from.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The state, for the render (MO-3) that mutates its devices.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// Rt messages applied so far.
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// Rt messages that named a slot beyond its table or a missing track: contract
    /// violations, skipped (their content went to the garbage).
    pub fn skipped(&self) -> u32 {
        self.skipped
    }

    /// Garbage items that found the garbage ring full (never, by the invariant of
    /// `crate::msg`) and were leaked rather than freed on the audio thread.
    pub fn leaked(&self) -> u32 {
        self.leaked
    }

    /// The live events taken at the start of this block, in their order.
    pub fn live_events(&self) -> &[LiveEvent] {
        self.live_block.as_slice()
    }

    /// Whether every live note must be released in this block, after [`Engine::live_events`]:
    /// live events were lost since the last block (the live queue overflowed).
    pub fn release_live(&self) -> bool {
        self.release_live
    }

    /// The start of a block: applies up to [`limits::MAX_APPLY_PER_BLOCK`] rt messages
    /// (the rest wait for the next block), takes the live events, then reads the overflow
    /// counter (module doc of `crate::msg`, live input).
    pub fn begin_block(&mut self) {
        for _ in 0..limits::MAX_APPLY_PER_BLOCK {
            let Ok(msg) = self.rt.pop() else { break };
            self.apply(msg);
        }
        self.live_block.clear();
        while !self.live_block.is_full() {
            let Ok(event) = self.live.pop() else { break };
            // Room was checked just above: the push cannot fail.
            let _ = self.live_block.push(event);
        }
        let total = self.overflow.total();
        self.release_live = total != self.overflow_seen;
        self.overflow_seen = total;
    }

    /// `Engine::apply` of the spec: applies one rt message on the audio path. No
    /// allocation, no free (what it replaces goes to the garbage ring), no panic (a slot
    /// that does not exist is skipped and counted).
    pub fn apply(&mut self, msg: RtMsg) {
        self.applied += 1;
        let state = &mut *self.state;
        let mut skipped = false;
        let garbage = match msg {
            RtMsg::Reset(next) => Some(Garbage::State(core::mem::replace(&mut self.state, next))),
            RtMsg::SetTransport {
                tempo,
                signature,
                launch_quantize,
            } => {
                state.transport = TransportSettings {
                    tempo,
                    signature,
                    launch_quantize,
                };
                None
            }
            RtMsg::SetMetronome {
                enabled,
                volume,
                count_in_bars,
            } => {
                state.metronome = Metronome {
                    enabled,
                    volume,
                    count_in_bars,
                };
                None
            }
            RtMsg::SetMaster { gain, mute } => {
                state.master = Master { gain, mute };
                None
            }
            RtMsg::TrackPut(put) => {
                let (spare, out) = put_slot(&mut state.tracks, put);
                skipped = out;
                Some(Garbage::Tracks(spare))
            }
            RtMsg::TrackRemove(track) => match take_slot(&mut state.tracks, track.0) {
                Ok(old) => Some(Garbage::Tracks(Spare {
                    table: None,
                    item: old,
                })),
                Err(()) => {
                    skipped = true;
                    None
                }
            },
            RtMsg::TrackSetMix {
                track,
                gain,
                pan,
                mute,
            } => {
                match state.track_mut(track) {
                    Some(t) => {
                        t.gain = gain;
                        t.pan = pan;
                        t.mute = mute;
                    }
                    None => skipped = true,
                }
                None
            }
            RtMsg::TrackSetSolo { track, solo } => {
                match state.track_mut(track) {
                    Some(t) => t.solo = solo,
                    None => skipped = true,
                }
                None
            }
            RtMsg::TrackArm { track, armed } => {
                match state.track_mut(track) {
                    Some(t) => t.armed = armed,
                    None => skipped = true,
                }
                None
            }
            RtMsg::TrackSetMonitor { track, monitor } => {
                match state.track_mut(track) {
                    Some(t) => t.monitor = monitor,
                    None => skipped = true,
                }
                None
            }
            RtMsg::SelectTrack { track } => {
                state.selected = track;
                None
            }
            RtMsg::DevicePut { track, put } => match state.track_mut(track) {
                Some(t) => {
                    let (spare, out) = put_slot(&mut t.devices, put);
                    skipped = out;
                    Some(Garbage::Devices(spare))
                }
                None => {
                    skipped = true;
                    Some(Garbage::Devices(Spare {
                        table: put.grown,
                        item: Some(put.value),
                    }))
                }
            },
            RtMsg::DeviceRemove { track, device } => {
                match state
                    .track_mut(track)
                    .map(|t| take_slot(&mut t.devices, device.0.into()))
                {
                    Some(Ok(old)) => Some(Garbage::Devices(Spare {
                        table: None,
                        item: old,
                    })),
                    _ => {
                        skipped = true;
                        None
                    }
                }
            }
            RtMsg::DeviceSetEnabled {
                track,
                device,
                enabled,
            } => {
                match device_mut(state, track, device) {
                    Some(entry) => entry.enabled = enabled,
                    None => skipped = true,
                }
                None
            }
            RtMsg::ParamSet {
                track,
                device,
                param,
                value,
            } => {
                match device_mut(state, track, device) {
                    Some(entry) => entry.device.set_param(param, value),
                    None => skipped = true,
                }
                None
            }
            RtMsg::ClipPut(put) => {
                let (spare, out) = state.clips.put(put);
                skipped = out;
                Some(Garbage::Clips(spare))
            }
            RtMsg::ClipRemove(clip) => match state.clips.take(clip.0) {
                Ok(old) => Some(Garbage::Clips(PagedSpare {
                    growth: None,
                    item: old,
                })),
                Err(()) => {
                    skipped = true;
                    None
                }
            },
            RtMsg::SourcePut(put) => {
                let (spare, out) = put_slot(&mut state.sources, put);
                skipped = out;
                Some(Garbage::Sources(spare))
            }
            RtMsg::SourceRemove(source) => match take_slot(&mut state.sources, source.0) {
                Ok(old) => Some(Garbage::Sources(Spare {
                    table: None,
                    item: old,
                })),
                Err(()) => {
                    skipped = true;
                    None
                }
            },
            RtMsg::PlacementsSet { track, placements } => match state.track_mut(track) {
                Some(t) => Some(Garbage::Placements(core::mem::replace(
                    &mut t.placements,
                    placements,
                ))),
                None => {
                    skipped = true;
                    Some(Garbage::Placements(placements))
                }
            },
            RtMsg::LanePut(put) => {
                let (spare, out) = state.lanes.put(put);
                skipped = out;
                Some(Garbage::Lanes(spare))
            }
            RtMsg::LaneRemove(lane) => match state.lanes.take(lane.0) {
                Ok(old) => Some(Garbage::Lanes(PagedSpare {
                    growth: None,
                    item: old,
                })),
                Err(()) => {
                    skipped = true;
                    None
                }
            },
            RtMsg::SceneSet {
                track,
                scene,
                clip,
                grown,
            } => match state.track_mut(track) {
                Some(t) => {
                    let mut old = None;
                    if let Some(mut next) = grown {
                        if next.capacity() > t.scenes.capacity() {
                            next.take_slots_from(&mut t.scenes);
                            old = Some(core::mem::replace(&mut t.scenes, next));
                        } else {
                            old = Some(next);
                            skipped = true;
                        }
                    }
                    match t.scenes.as_mut_slice().get_mut(scene.0 as usize) {
                        Some(entry) => *entry = clip,
                        None => skipped = true,
                    }
                    old.map(Garbage::Scenes)
                }
                None => {
                    skipped = true;
                    grown.map(Garbage::Scenes)
                }
            },
            RtMsg::Transport(order) => {
                let p = &mut state.playback;
                match order {
                    TransportMsg::Play { from } => {
                        p.playing = true;
                        p.cue = Some(from);
                    }
                    TransportMsg::Stop => p.playing = false,
                    TransportMsg::Seek { tick } => p.cue = Some(tick),
                    TransportMsg::SetLoop { region } => p.loop_region = region,
                    TransportMsg::SetRecording { recording } => p.recording = recording,
                }
                None
            }
            RtMsg::Launch(order) => {
                match order {
                    LaunchMsg::Clip { track, scene } => match state.track_mut(track) {
                        Some(t) => t.launch = Some(LaunchRequest::Clip(scene)),
                        None => skipped = true,
                    },
                    LaunchMsg::StopTrack { track } => match state.track_mut(track) {
                        Some(t) => t.launch = Some(LaunchRequest::Stop),
                        None => skipped = true,
                    },
                    LaunchMsg::Scene { scene } => state.grid.scene = Some(scene),
                    LaunchMsg::StopAll => state.grid.stop_all = true,
                    LaunchMsg::BackToArrangement => state.grid.back_to_arrangement = true,
                }
                None
            }
        };
        if skipped {
            self.skipped = self.skipped.saturating_add(1);
        }
        if let Some(garbage) = garbage
            && !garbage_is_empty(&garbage)
            && let Err(PushError::Full(lost)) = self.garbage.push(garbage)
        {
            // Never happens (the invariant of `crate::msg`); if it did, leak rather than
            // free on the audio thread.
            core::mem::forget(lost);
            self.leaked = self.leaked.saturating_add(1);
        }
    }
}

fn device_mut(state: &mut State, track: TrackSlot, device: DeviceSlot) -> Option<&mut DeviceEntry> {
    state
        .track_mut(track)?
        .devices
        .as_mut_slice()
        .get_mut(device.0 as usize)?
        .as_deref_mut()
}

fn garbage_is_empty(garbage: &Garbage) -> bool {
    match garbage {
        Garbage::Tracks(s) => s.is_empty(),
        Garbage::Devices(s) => s.is_empty(),
        Garbage::Clips(s) => s.is_empty(),
        Garbage::Lanes(s) => s.is_empty(),
        Garbage::Sources(s) => s.is_empty(),
        Garbage::State(_) | Garbage::Placements(_) | Garbage::Scenes(_) => false,
    }
}

#[cfg(feature = "testing")]
pub mod cycle;
