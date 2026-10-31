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
//! the tracks is the graph's ([`crate::graph`], MO-3: [`Engine::render`]); routing events
//! with their offsets ([`crate::events`], MO-4) is the graph's too; the transport clock
//! TR-1's; launching LA's. [`State`] holds what they will read, nothing more.
//!
//! A track that leaves its slot (removed, replaced, or dropped by a `Reset`) is not garbage
//! at `apply`: it fades out in the graph, then comes back through the retire ring, which
//! [`Preparer::collect`] drains with the garbage ring.
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

use crate::arrangement;
use crate::automation::TrackLanes;
use crate::clip_player::{ClipNotes, ClipPlayer};
use crate::device::{Device, DeviceRole, DeviceSetup, EventBuffer, Factory};
use crate::graph::{BlockClock, Graph, RETIRE_RING_CAPACITY, TRACK_INPUT_EVENTS, TrackGraph};
use crate::launcher;
use crate::limiter::Limiter;
use crate::metronome::{Clicker, MAX_COUNT_IN_BARS};
use crate::msg::{
    ActiveClip, AutomationPoint, AutomationTarget, ClipSlot, ControlMsg, DeviceKind, DeviceSlot,
    Feedback, Garbage, HEADER_BYTES, Header, LaneSlot, LaunchMsg, LiveEvent, LiveOverflow,
    MsgError, Note, PAGE_SLOTS, Page, PagedGrowth, PagedPut, PagedSpare, Placement, Put, RtMsg,
    SceneSlot, Slots, SourceSlot, Spare, Table, TrackKind, TrackMeter, TrackSlot, TransportMsg,
    TransportState, limits, state_pairs, try_box,
};
use crate::time::{Frame, LoopRegion, Quantize, SampleRate, Tempo, Tick, TimeSignature};
use crate::transport::{Segment, Transport};

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

/// The resolver of the built-in devices: the registry of [`crate::devices`] (DV-1).
pub fn builtin_factory(kind: DeviceKind) -> Option<Factory> {
    crate::devices::factory(kind)
}

// ---------------------------------------------------------------- the engine state

/// A device of a track's chain, as `prepare` built it.
pub struct DeviceEntry {
    pub device: Box<dyn Device>,
    pub enabled: bool,
    /// A MIDI effect's output, sized by `prepare` from its declared bounds
    /// ([`crate::graph::TRACK_INPUT_EVENTS`]); `None` for the other roles.
    pub events_out: Option<EventBuffer>,
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
    /// The last launch asked for this track, not yet taken by its player.
    pub launch: Option<LaunchRequest>,
    /// `BackToArrangement` asked for this track, not yet taken by its player (GR-1).
    pub back: bool,
    /// The graph's levels of the track, and its link while it retires.
    pub graph: TrackGraph,
    /// The clip player (CL-2): the launched clip and its read state.
    pub player: ClipPlayer,
}

/// What a clip plays.
#[derive(Debug)]
pub enum ClipContent {
    /// A MIDI clip's notes, in their delivery order ([`crate::clip_player`]).
    Notes(ClipNotes),
    Audio {
        source: SourceSlot,
        start: Frame,
        length: Frame,
    },
}

/// A clip: its content, its length and its loop (ticks, MIDI clips).
#[derive(Debug)]
pub struct Clip {
    pub length: Tick,
    /// The loop `[start, end)` within `[0, length]`; `None`: played once (CL-2).
    pub looping: Option<LoopRegion>,
    /// The track whose clip player plays it, written by the audio side (CL-2): a clip
    /// plays on one track at a time.
    pub on_track: Option<TrackSlot>,
    pub content: ClipContent,
}

/// An automation lane: what it drives and its points (a content snapshot), sorted by tick,
/// one per tick, finite (the Preparer's work); a gain lane's values are in dB; and the audio
/// side's state of its reading ([`crate::automation`], AM-2).
#[derive(Debug)]
pub struct Lane {
    pub track: TrackSlot,
    pub target: AutomationTarget,
    pub points: Table<AutomationPoint>,
    pub run: crate::automation::LaneRun,
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
    pub pan: f32,
    pub mute: bool,
}

impl Default for Master {
    fn default() -> Self {
        Master {
            gain: 1.0,
            pan: 0.0,
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

    /// The value in `slot`, to change, if any.
    pub fn get_mut(&mut self, slot: u32) -> Option<&mut T> {
        let slot = slot as usize;
        self.pages
            .as_mut_slice()
            .get_mut(slot / PAGE_SLOTS)?
            .as_mut()?
            .as_mut_slice()
            .get_mut(slot % PAGE_SLOTS)?
            .as_mut()
    }

    /// Calls `f` with the slot and the value of every value held, in slot order, visiting
    /// every slot of the allocated pages. No allocation.
    pub fn for_each_mut(&mut self, mut f: impl FnMut(u32, &mut T)) {
        for (p, page) in self.pages.as_mut_slice().iter_mut().enumerate() {
            let Some(page) = page.as_mut() else {
                continue;
            };
            for (i, value) in page.as_mut_slice().iter_mut().enumerate() {
                if let Some(value) = value.as_mut() {
                    // A directory addresses fewer than `u32::MAX` slots (`limits`).
                    f((p * PAGE_SLOTS + i) as u32, value);
                }
            }
        }
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

/// Makes a lane's points what the audio side reads (AM-2, [`crate::automation`]): finite
/// values only, sorted by tick, one point per tick (the compiler sends them so; otherwise
/// which of equal ticks stays is the unstable sort's choice, the same in every build). A gain
/// lane keeps its dB: the audio side reads them along the fader's travel, which clamps
/// them to its range ([`crate::fader`]). In place, no allocation, off the audio path.
fn sanitize_points(points: &mut Vec<AutomationPoint>) {
    points.retain(|p| p.value.is_finite());
    if !points.is_sorted_by(|a, b| a.tick < b.tick) {
        points.sort_unstable_by_key(|p| p.tick);
        points.dedup_by_key(|p| p.tick);
    }
}

/// Everything the audio side holds about the project. Built by `prepare` (the initial one
/// by [`channel`], a fresh one for each `Reset`), mutated only by `apply`.
#[derive(Debug)]
pub struct State {
    pub transport: TransportSettings,
    pub metronome: Metronome,
    pub master: Master,
    /// The master's timed gain, pan and mute ([`crate::mixer`], MX-1): set off the audio
    /// path with [`State::set_master_schedule`]; a `Reset` brings the new state's, none.
    pub(crate) master_schedule: Option<Box<crate::events::Schedule>>,
    /// Counts the replacements of `master_schedule`: the graph's master strip goes back to
    /// the faders when it changes.
    pub(crate) master_schedule_gen: u32,
    pub playback: Playback,
    pub selected: Option<TrackSlot>,
    pub tracks: Slots<Box<Track>>,
    /// The master's chain (#364): effects only, run in slot order over the sum of the tracks,
    /// before the master's fader ([`crate::graph`]). Addressed by [`TrackSlot::MASTER`].
    pub master_devices: Slots<Box<DeviceEntry>>,
    pub clips: Paged<Box<Clip>>,
    pub lanes: Paged<Box<Lane>>,
    pub sources: Slots<Box<Source>>,
}

impl State {
    /// Puts `schedule` on the master (its timed gain, pan and mute, [`crate::mixer`]) and
    /// returns the one it replaces, to be freed off the audio path. No message fills it:
    /// the master's automation is #378's; the tests set it.
    pub fn set_master_schedule(
        &mut self,
        schedule: Option<Box<crate::events::Schedule>>,
    ) -> Option<Box<crate::events::Schedule>> {
        self.master_schedule_gen = self.master_schedule_gen.wrapping_add(1);
        core::mem::replace(&mut self.master_schedule, schedule)
    }

    /// The master's schedule, if any.
    pub fn master_schedule(&self) -> Option<&crate::events::Schedule> {
        self.master_schedule.as_deref()
    }

    /// An empty state with the initial capacities, through fallible allocations.
    pub fn try_new() -> Result<Self, MsgError> {
        Ok(State {
            transport: TransportSettings::default(),
            metronome: Metronome::default(),
            master: Master::default(),
            master_schedule: None,
            master_schedule_gen: 0,
            playback: Playback::default(),
            selected: None,
            tracks: Table::try_filled(initial::TRACKS)?,
            master_devices: Table::try_filled(initial::DEVICES_PER_TRACK)?,
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

    /// The device row of track `track`, or of the master's chain for [`TrackSlot::MASTER`].
    pub fn chain(&self, track: TrackSlot) -> Option<&Slots<Box<DeviceEntry>>> {
        if track.is_master() {
            Some(&self.master_devices)
        } else {
            Some(&self.track(track)?.devices)
        }
    }

    /// [`State::chain`], to mutate it.
    pub fn chain_mut(&mut self, track: TrackSlot) -> Option<&mut Slots<Box<DeviceEntry>>> {
        if track.is_master() {
            Some(&mut self.master_devices)
        } else {
            Some(&mut self.track_mut(track)?.devices)
        }
    }

    /// The device in `device` of track `track` (or of the master), if it exists.
    pub fn device(&self, track: TrackSlot, device: DeviceSlot) -> Option<&DeviceEntry> {
        self.chain(track)?
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
    let (retired_tx, retired_rx) = RingBuffer::new(RETIRE_RING_CAPACITY);
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
        retired: retired_rx,
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
        stopped_at: None,
    };
    let graph = Graph::try_new(&setup, retired_tx)?;
    // TR-1 cuts a block of `frames` in at most `frames + 2` segments (at most `frames + 1`
    // wraps, then the rest of the block).
    let segments = Table::try_with_capacity(graph.max_frames().saturating_add(2))?;
    let engine = Engine {
        state: try_box(State::try_new()?)?,
        graph,
        segments,
        transport: Transport::new(rate_of(&setup)),
        clicker: Clicker::new(rate_of(&setup)),
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
        cut: None,
        limiter: Limiter::new(rate_of(&setup)),
    };
    Ok((preparer, engine))
}

// ---------------------------------------------------------------- the prepare side

/// What the prepare side knows of a track it created.
#[derive(Debug)]
struct TrackShadow {
    /// One entry per slot of the track's device table: `None` when empty, otherwise the
    /// capacity of the device's MIDI output (0 for a device that writes no notes), from
    /// which a MIDI effect inserted after it is sized.
    devices: Vec<Option<usize>>,
    scenes_capacity: usize,
}

impl TrackShadow {
    fn try_new() -> Result<Self, MsgError> {
        Ok(TrackShadow {
            devices: try_vec(initial::DEVICES_PER_TRACK, None)?,
            scenes_capacity: initial::SCENES_PER_TRACK,
        })
    }
}

/// The runtime tables as the audio side will hold them once everything sent is applied.
#[derive(Debug)]
struct Shadow {
    /// One entry per slot of the track table.
    tracks: Vec<Option<TrackShadow>>,
    /// The master's chain (#364): its device row only (`scenes_capacity` is not used).
    master: TrackShadow,
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

    /// Whether the page of `slot` was sent (a value may be there).
    fn has_page(&self, slot: u32) -> bool {
        self.pages
            .get(slot as usize / PAGE_SLOTS)
            .copied()
            .unwrap_or(false)
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
            master: TrackShadow::try_new()?,
            clips: PagedShadow::try_new(initial::CLIP_PAGES)?,
            lanes: PagedShadow::try_new(initial::LANE_PAGES)?,
            sources: initial::SOURCES,
        })
    }

    fn track(&self, slot: TrackSlot) -> Option<&TrackShadow> {
        self.tracks.get(slot.0 as usize)?.as_ref()
    }

    /// The shadow of the device row of a track, or of the master's chain.
    fn chain(&self, track: TrackSlot) -> Option<&TrackShadow> {
        if track.is_master() {
            Some(&self.master)
        } else {
            self.track(track)
        }
    }

    fn chain_mut(&mut self, track: TrackSlot) -> Option<&mut TrackShadow> {
        if track.is_master() {
            Some(&mut self.master)
        } else {
            self.tracks.get_mut(track.0 as usize)?.as_mut()
        }
    }

    fn has_device(&self, track: TrackSlot, device: DeviceSlot) -> bool {
        self.chain(track)
            .and_then(|t| t.devices.get(device.0 as usize))
            .is_some_and(Option::is_some)
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
    retired: Consumer<Box<Track>>,
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
    stopped_at: Option<usize>,
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
    /// stream, a refused allocation or a stalled engine stops it and is returned. The
    /// messages before the one that stopped it were handled; [`Preparer::stopped_at`] gives
    /// the offset of that message, from which the stream can be sent again once the cause
    /// is gone (a refused message left nothing behind), or the project resyncs with a
    /// `Reset`.
    pub fn submit(&mut self, stream: &[u8]) -> Result<(), MsgError> {
        let mut at = 0usize;
        self.stopped_at = None;
        while at < stream.len() {
            let bytes = &stream[at..];
            match Header::decode(bytes, self.epoch) {
                Ok(header) => {
                    let end = HEADER_BYTES + header.len as usize;
                    let outcome = ControlMsg::decode(&header, &bytes[HEADER_BYTES..end])
                        .and_then(|msg| self.handle(msg));
                    if let Err(error) = outcome {
                        if error.stops_the_stream() {
                            self.stopped_at = Some(at);
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
                Err(error) => {
                    self.stopped_at = Some(at);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// The byte offset, in the stream of the last [`Preparer::submit`], of the message that
    /// stopped it; `None` when the whole stream was handled.
    pub fn stopped_at(&self) -> Option<usize> {
        self.stopped_at
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

    /// Drops what the audio side sent back (the garbage, and the tracks that finished
    /// fading out); returns how many items.
    pub fn collect(&mut self) -> usize {
        let mut n = 0;
        while let Ok(garbage) = self.garbage.pop() {
            drop(garbage);
            n += 1;
        }
        while let Ok(track) = self.retired.pop() {
            drop(track);
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
            ControlMsg::SetMaster { gain, pan, mute } => RtMsg::SetMaster { gain, pan, mute },
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
                self.remove_device(track, device);
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
                looping,
                first,
                total,
                notes,
            } => {
                if !completes(&self.notes, clip.0, first, total, notes.len()) {
                    stage(
                        &mut self.notes,
                        clip.0,
                        first,
                        total,
                        notes,
                        &mut self.rejected,
                    )?;
                    return Ok(None);
                }
                // The last chunk: everything allocated before it is staged, so that a
                // refusal leaves the staging as it was.
                // The table is allocated now and filled once the notes are staged.
                let table = ClipNotes::try_with_capacity(total as usize)?;
                let (mut value, growth) = self.prepare_clip(
                    clip,
                    Clip {
                        length,
                        looping,
                        on_track: None,
                        content: ClipContent::Notes(ClipNotes::default()),
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
                let mut table = table;
                table.fill(&notes);
                value.content = ClipContent::Notes(table);
                self.finish_clip(clip, value, growth)
            }
            ControlMsg::ClipRemove { clip } => {
                self.notes.remove(&clip.0);
                if !self.shadow.clips.has_page(clip.0) {
                    // Never put (an upload rejected or not finished): nothing to remove.
                    return Ok(None);
                }
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
            ControlMsg::AudioSourceRelease { source } => {
                if source.0 as usize >= self.shadow.sources {
                    return Ok(None);
                }
                RtMsg::SourceRemove(source)
            }
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
                        looping: None,
                        on_track: None,
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
                let Some(mut placements) = staged else {
                    return Ok(None);
                };
                // Sorted and sane for the arrangement player; in place, no allocation.
                arrangement::prepare_table(&mut placements);
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
                // The master's row (#378) is not a track of the table.
                if !track.is_master() {
                    self.existing_track(track)?;
                }
                if !completes(&self.points, lane.0, first, total, points.len()) {
                    stage(
                        &mut self.points,
                        lane.0,
                        first,
                        total,
                        points,
                        &mut self.rejected,
                    )?;
                    return Ok(None);
                }
                // The last chunk: allocate the box and the grown table before staging it,
                // so that a refusal leaves the staging as it was.
                let growth = self.shadow.lanes.growth(lane.0, limits::MAX_LANES)?;
                self.shadow.lanes.reserve(&growth)?;
                let mut value = try_box(Lane {
                    track,
                    target,
                    points: Table::from_vec(Vec::new()),
                    run: crate::automation::LaneRun::default(),
                })?;
                let staged = stage(
                    &mut self.points,
                    lane.0,
                    first,
                    total,
                    points,
                    &mut self.rejected,
                )?;
                let Some(mut points) = staged else {
                    return Ok(None);
                };
                sanitize_points(&mut points);
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
                if !self.shadow.lanes.has_page(lane.0) {
                    return Ok(None);
                }
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
            ControlMsg::BackTrackToArrangement { track } => {
                self.existing_track(track)?;
                RtMsg::Launch(LaunchMsg::BackTrackToArrangement { track })
            }
            ControlMsg::Play { from } => RtMsg::Transport(TransportMsg::Play { from }),
            ControlMsg::Stop => RtMsg::Transport(TransportMsg::Stop),
            ControlMsg::AllSoundOff => RtMsg::AllSoundOff,
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

    fn remove_device(&mut self, track: TrackSlot, device: DeviceSlot) {
        if let Some(t) = self.shadow.chain_mut(track)
            && let Some(flag) = t.devices.get_mut(device.0 as usize)
        {
            *flag = None;
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
            back: false,
            graph: TrackGraph::new(),
            player: ClipPlayer::default(),
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
        let shadow = self.shadow.chain(track).ok_or(MsgError::NoSuchTarget)?;
        let capacity = shadow.devices.len();
        // A MIDI effect receives at most what the track gives a sub-block, or what the
        // largest MIDI effect before it writes (MO-4: chained outputs never fill). One
        // inserted later before it does not resize it: CT-5's safety net covers that.
        let input_events = shadow
            .devices
            .iter()
            .take(device.0 as usize)
            .flatten()
            .copied()
            .fold(TRACK_INPUT_EVENTS, usize::max);
        let factory = (self.factories)(kind).ok_or(MsgError::UnknownDevice(kind))?;
        let mut built = factory(&self.setup)?;
        for (id, value) in state_pairs(state) {
            built.set_param(id, value);
        }
        built.reset();
        let descriptor = built.descriptor();
        // The master's chain holds effects only (#364): nothing feeds an instrument or a
        // MIDI effect there, and the graph runs the chain without notes.
        if track.is_master() && descriptor.role != DeviceRole::Effect {
            return Err(MsgError::NoSuchTarget);
        }
        let events_out = match (descriptor.role, descriptor.midi_output) {
            (DeviceRole::MidiEffect, Some(bounds)) => Some(EventBuffer::try_with_capacity(
                bounds.capacity(input_events),
            )?),
            _ => None,
        };
        let midi_out = events_out.as_ref().map_or(0, EventBuffer::capacity);
        let value = try_box(DeviceEntry {
            device: built,
            enabled: true,
            events_out,
        })?;
        let slot = device.0 as usize;
        let grown = grow_for(capacity, slot, limits::MAX_DEVICES_PER_TRACK)?;
        let mut flags = None;
        if let Some(g) = &grown {
            flags = Some(try_vec(g.capacity(), None)?);
        }
        // Commit.
        if let Some(t) = self.shadow.chain_mut(track) {
            if let Some(mut flags) = flags {
                flags[..t.devices.len()].copy_from_slice(&t.devices);
                t.devices = flags;
            }
            if let Some(flag) = t.devices.get_mut(slot) {
                *flag = Some(midi_out);
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

/// The track whose player plays the clip in `slot`, when its `on_track` says so and the
/// track agrees (a slot reused by another track is never named).
fn playing_track(state: &State, slot: ClipSlot) -> Option<TrackSlot> {
    let t = state.clips.get(slot.0)?.on_track?;
    let track = state.tracks.as_slice().get(t.0 as usize)?.as_deref()?;
    (track.player.holding_clip() == Some(slot)).then_some(t)
}

/// A track that leaves its slot gives back the clip its player was playing (CL-2), so that
/// another track may launch it. No allocation.
fn give_back_clip(clips: &mut Paged<Box<Clip>>, slot: TrackSlot, track: &Track) {
    if let Some(c) = track.player.holding_clip()
        && let Some(clip) = clips.get_mut(c.0)
        && clip.on_track == Some(slot)
    {
        clip.on_track = None;
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

/// Whether this chunk completes its upload, as [`stage`] would decide it.
fn completes<T>(
    map: &HashMap<u32, Stage<T>>,
    slot: u32,
    first: u32,
    total: u32,
    len: usize,
) -> bool {
    let staged = match map.get(&slot) {
        Some(s) if s.items.len() as u64 == u64::from(first) && s.total == total => s.items.len(),
        _ if first == 0 => 0,
        _ => return false,
    };
    (staged as u64).saturating_add(len as u64) == u64::from(total)
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
    graph: Graph,
    /// The segments of the block being rendered, taken from the clock once.
    segments: Table<Segment>,
    transport: Transport,
    clicker: Clicker,
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
    /// The cut an `AllSoundOff` started, if one is running (see [`Cut`]).
    cut: Option<Cut>,
    /// The output protection (#402): the last stage of every rendered slice.
    limiter: Limiter,
}

/// An `AllSoundOff` in progress: the output fades out linearly over one graph fade, then
/// every device is reset and the output resumes. `done` counts the frames faded so far.
#[derive(Clone, Copy, Debug)]
struct Cut {
    done: u32,
}

/// The rate an engine is built for: the setup's, rounded, or 48 kHz when it is not usable.
fn rate_of(setup: &DeviceSetup) -> SampleRate {
    let hz = setup.sample_rate.round();
    if hz.is_finite() && (1.0..=u32::MAX as f32).contains(&hz) {
        SampleRate::new(hz as u32).unwrap_or(SampleRate::HZ_48000)
    } else {
        SampleRate::HZ_48000
    }
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

    /// The graph that renders the state (MO-3).
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Renders the graph ([`crate::graph`]) into `left` and `right` (their common length),
    /// with the live events [`Engine::begin_block`] took: the first render of at least one
    /// frame delivers them, later ones until the next `begin_block` render without them (a
    /// note is never played twice). No allocation, no free, no lock, no panic.
    ///
    /// The render is cut in blocks of at most the setup's `max_block_frames`; each is one
    /// block of the musical clock (TR-1): the orders `begin_block` applied take effect at
    /// the start of the first, the clock cuts the block in segments at its loop wraps, the
    /// graph renders the tracks over them (the timed events of MO-4, [`crate::events`]),
    /// then the metronome (TR-2) is added to the output, after the master, and last the output
    /// protection ([`crate::limiter`], #402), after the output's meter.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let total = left.len().min(right.len());
        let mut start = 0;
        while start < total {
            let frames = (total - start).min(self.graph.max_frames().max(1));
            let end = start + frames;
            let first = start == 0;
            self.start_clock(frames);
            self.segments.clear();
            while let Some(segment) = self.transport.next_segment() {
                // Room for every segment of the block (`channel`): never refused.
                if self.segments.push(segment).is_err() {
                    break;
                }
            }
            let clock = BlockClock {
                running: self.transport.is_running(),
                segments: self.segments.as_slice(),
                position: self.transport.position(),
                frame: self.transport.total_samples(),
                cues: self.transport.cues(),
            };
            let live: &[LiveEvent] = if first {
                self.live_block.as_slice()
            } else {
                &[]
            };
            self.graph.block(
                &mut self.state,
                &clock,
                live,
                first && self.release_live,
                &mut left[start..end],
                &mut right[start..end],
            );
            self.clicker.render(
                &self.state.metronome,
                self.state.transport.signature,
                self.segments.as_slice(),
                &mut left[start..end],
                &mut right[start..end],
            );
            let cut_done = self.apply_cut(&mut left[start..end], &mut right[start..end]);
            // The output's meter: what leaves the engine, the metronome included.
            self.graph
                .meter_output(&left[start..end], &right[start..end]);
            // The output protection (#402), after the meter: the meter shows the level
            // before it, the speakers and the export get the protected signal.
            self.limiter
                .process(&mut left[start..end], &mut right[start..end]);
            self.transport.end_block();
            if cut_done {
                self.reset_devices();
            }
            start = end;
        }
        if total > 0 {
            self.live_block.clear();
            self.release_live = false;
        }
    }

    /// Applies the running cut (if any) to one rendered segment: a linear fade to zero over
    /// [`Graph::fade`] frames, then silence for the rest of the segment. Returns whether
    /// the fade ended in this segment, so that the devices are reset before the next one.
    /// No allocation.
    fn apply_cut(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        let Some(mut cut) = self.cut else {
            return false;
        };
        let total = self.graph.fade().max(1);
        let mut ended = false;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            if cut.done >= total {
                ended = true;
                *l = 0.0;
                *r = 0.0;
            } else {
                let gain = 1.0 - cut.done as f32 / total as f32;
                *l *= gain;
                *r *= gain;
                cut.done += 1;
            }
        }
        // A fade that ends on the last frame of the segment is over too: the next frame
        // would be silent.
        if cut.done >= total {
            ended = true;
        }
        self.cut = if ended { None } else { Some(cut) };
        ended
    }

    /// Resets every device of every track: voices die, tails (delay lines) are hidden.
    /// Constant time per device ([`Device::reset`]), no allocation.
    fn reset_devices(&mut self) {
        // The silence a cut leaves does not carry the gain of the overs before it.
        self.limiter.reset();
        for track in self.state.tracks.as_mut_slice().iter_mut().flatten() {
            for entry in track.devices.as_mut_slice().iter_mut().flatten() {
                entry.device.reset();
            }
            track.graph.release_notes();
        }
        for entry in self
            .state
            .master_devices
            .as_mut_slice()
            .iter_mut()
            .flatten()
        {
            entry.device.reset();
        }
    }

    /// The current gain of the output protection ([`crate::limiter`], #402): `1.0` at rest,
    /// less than `1.0` while limiting. For tests only: the limiter has no observable effect
    /// beyond the output samples it shapes.
    pub fn limiter_gain(&self) -> f32 {
        self.limiter.gain()
    }

    /// The musical clock (TR-1), as of the start of the next block.
    pub fn clock(&self) -> &Transport {
        &self.transport
    }

    /// Writes the transport part of a report: its state and the playhead (the start of
    /// the next block). No allocation.
    pub fn report_transport(&self, feedback: &mut Feedback) {
        feedback.transport = self.transport.state();
        feedback.playhead_tick = self.transport.playhead_tick();
        feedback.playhead_frame = self.transport.playhead_frame();
    }

    /// The grid cells that are not idle, for the report ([`crate::launcher::active_clips`]):
    /// at most two per track. No allocation.
    pub fn active_clips(&self, push: impl FnMut(ActiveClip)) {
        let running = self.transport.state() != TransportState::Stopped;
        launcher::active_clips(&self.state, running, push);
    }

    /// Voices sounding in the tracks' devices after the last rendered block (MO-5).
    pub fn voices(&self) -> u32 {
        self.graph.voices()
    }

    /// Takes the meters of the period of `frames` frames (MO-5): one per track slot, in
    /// `out` (zeros for an empty slot; a track beyond `out` is not reported), and returns
    /// the number of slots written (up to the last track) and the output's meter after the
    /// master. Starts a new period. No allocation.
    pub fn take_meters(&mut self, frames: u32, out: &mut [TrackMeter]) -> (usize, TrackMeter) {
        self.graph.take_meters(&mut self.state, frames, out)
    }

    /// Changes the sample rate of the engine's own processing (clock, metronome, graph
    /// fades) and keeps the playhead. The devices were built for the old rate by
    /// `prepare` (and the preparer keeps its setup): a host that changes the device rate
    /// builds a new channel; this keeps the clock, the metronome and the fades coherent
    /// with a rate the host switched to while it keeps the same engine.
    pub fn set_sample_rate(&mut self, rate: SampleRate) {
        self.transport.set_sample_rate(rate);
        self.clicker.set_sample_rate(rate);
        self.limiter.set_sample_rate(rate);
        self.graph.set_sample_rate(rate.hz() as f32);
    }

    /// Starts the clock block of one render: a count-in first, then the clock reads the
    /// orders.
    fn start_clock(&mut self, frames: usize) {
        let state = &mut *self.state;
        let m = state.metronome;
        let p = &mut state.playback;
        // A recording started from a stopped clock with the metronome on begins
        // `count_in_bars` early: the playhead runs through negative ticks, clicking, up
        // to the cue.
        if p.playing
            && p.recording
            && m.enabled
            && m.count_in_bars > 0
            && !self.transport.is_running()
            && let Some(cue) = p.cue
            // A cue at or past the end of the loop plays on past it (TR-1): a pre-roll
            // that starts inside the loop would wrap and never reach the cue.
            && p.loop_region.is_none_or(|r| cue.0 < r.end().0)
        {
            let bars = i64::from(m.count_in_bars.min(MAX_COUNT_IN_BARS));
            let ticks = state
                .transport
                .signature
                .ticks_per_bar()
                .0
                .saturating_mul(bars);
            p.cue = Some(cue.saturating_sub(Tick(ticks)));
        }
        self.transport.begin_block(state, frames);
    }

    /// A whole block of the audio side: [`Engine::begin_block`], then [`Engine::render`].
    /// A block of zero frames is no block: nothing is applied nor taken from the live
    /// queue, so no note-off is lost.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if left.is_empty() || right.is_empty() {
            return;
        }
        self.begin_block();
        self.render(left, right);
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
            RtMsg::Reset(next) => {
                let mut old = core::mem::replace(&mut self.state, next);
                // The old project's tracks fade out (at most `MAX_TRACKS` boxes moved).
                for entry in old.tracks.as_mut_slice() {
                    if let Some(track) = entry.take() {
                        self.graph.retire(track);
                    }
                }
                // The master's strip takes the new state's faders back.
                self.graph.reset_master();
                // The limiter is not reset on a project swap: the retiring tracks keep
                // fading through the same limiter, so a project opened while the previous
                // one was hot does not produce a gain jump. The release (80 ms) decays the
                // deficit naturally; the new project's overs are caught on the first sample.
                Some(Garbage::State(old))
            }
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
            RtMsg::SetMaster { gain, pan, mute } => {
                // A gesture on the master's fader or pan holds the lane that drives it (#378),
                // as one on a track's does; the mute holds neither.
                let moved_gain = state.master.gain.to_bits() != gain.to_bits();
                let moved_pan = state.master.pan.to_bits() != pan.to_bits();
                state.master = Master { gain, pan, mute };
                hold_lanes(state, &self.graph, TrackSlot::MASTER, |target| {
                    (moved_gain && target == AutomationTarget::MixerGain)
                        || (moved_pan && target == AutomationTarget::MixerPan)
                });
                None
            }
            RtMsg::TrackPut(put) => {
                let slot = put.slot as usize;
                let (mut spare, out) = put_slot(&mut state.tracks, put);
                skipped = out;
                // Inside the table the put happened and the spare item is the track it
                // replaced, which was playing: it fades out. Beyond, it is the new track,
                // never heard: garbage.
                if slot < state.tracks.capacity()
                    && let Some(old) = spare.item.take()
                {
                    give_back_clip(&mut state.clips, TrackSlot(slot as u32), &old);
                    self.graph.retire(old);
                }
                Some(Garbage::Tracks(spare))
            }
            RtMsg::TrackRemove(track) => match take_slot(&mut state.tracks, track.0) {
                Ok(old) => {
                    // A removed track is no longer the live input's target; a track created
                    // later in the slot starts unselected.
                    if state.selected == Some(track) {
                        state.selected = None;
                    }
                    // It fades out, then comes back through the retire ring.
                    if let Some(old) = old {
                        give_back_clip(&mut state.clips, track, &old);
                        self.graph.retire(old);
                    }
                    None
                }
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
                        // A gesture on the fader or the pan holds the lane that drives it
                        // (AM-2); one on the mute holds neither.
                        let moved_gain = t.gain.to_bits() != gain.to_bits();
                        let moved_pan = t.pan.to_bits() != pan.to_bits();
                        t.gain = gain;
                        t.pan = pan;
                        t.mute = mute;
                        hold_lanes(state, &self.graph, track, |target| {
                            (moved_gain && target == AutomationTarget::MixerGain)
                                || (moved_pan && target == AutomationTarget::MixerPan)
                        });
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
            RtMsg::DevicePut { track, put } => match state.chain_mut(track) {
                Some(chain) => {
                    let device = DeviceSlot(put.slot as u16);
                    let midi = takes_notes(&put.value);
                    let (spare, out) = put_slot(chain, put);
                    skipped = out;
                    if (midi || spare.item.as_deref().is_some_and(takes_notes))
                        && let Some(t) = state.track_mut(track)
                    {
                        t.graph.release_notes();
                    }
                    resend_lanes(state, &self.graph, track, device);
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
                match state.chain_mut(track).map(|chain| {
                    let old = take_slot(chain, device.0.into());
                    let release = matches!(&old, Ok(Some(entry)) if takes_notes(entry));
                    (old, release)
                }) {
                    Some((Ok(old), release)) => {
                        if release && let Some(t) = state.track_mut(track) {
                            t.graph.release_notes();
                        }
                        Some(Garbage::Devices(Spare {
                            table: None,
                            item: old,
                        }))
                    }
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
                let mut release = false;
                match device_mut(state, track, device) {
                    Some(entry) => {
                        let changed = entry.enabled != enabled;
                        release = changed && takes_notes(entry);
                        if changed && !takes_notes(entry) {
                            // An audio effect that is switched on or off starts clean: a
                            // tail (an echo) never comes back from before the switch.
                            entry.device.reset();
                        }
                        entry.enabled = enabled;
                    }
                    None => skipped = true,
                }
                resend_lanes(state, &self.graph, track, device);
                if release && let Some(t) = state.track_mut(track) {
                    t.graph.release_notes();
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
                    Some(entry) => {
                        entry.device.set_param(param, value);
                        // A gesture holds the lane that drives this parameter (AM-2).
                        let target = AutomationTarget::DeviceParam { device, param };
                        hold_lanes(state, &self.graph, track, |t| t == target);
                    }
                    None => skipped = true,
                }
                None
            }
            RtMsg::ClipPut(mut put) => {
                let slot = ClipSlot(put.slot);
                // A clip replaced while it plays: the player goes on in the new table.
                let player = playing_track(state, slot);
                put.value.on_track = player;
                let (spare, out) = state.clips.put(put);
                skipped = out;
                if !out
                    && let Some(t) = player.and_then(|t| state.track_mut(t))
                    && t.player.clip_changed(slot, false)
                {
                    t.graph.release_notes();
                }
                Some(Garbage::Clips(spare))
            }
            RtMsg::ClipRemove(clip) => match state.clips.take(clip.0) {
                Ok(old) => {
                    if let Some(c) = &old
                        && let Some(t) = c.on_track.and_then(|t| state.track_mut(t))
                        && t.player.clip_changed(clip, true)
                    {
                        t.graph.release_notes();
                    }
                    Some(Garbage::Clips(PagedSpare {
                        growth: None,
                        item: old,
                    }))
                }
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
            RtMsg::LanePut(mut put) => {
                // A new upload of the same lane goes on where the old one was (its
                // gesture); a strip that loses a lane goes back to its fader.
                let lost = match state.lanes.get(put.slot) {
                    Some(old) if old.track == put.value.track && old.target == put.value.target => {
                        put.value.run.inherit(&old.run);
                        None
                    }
                    Some(old) => Some((old.track, old.target)),
                    None => None,
                };
                let (spare, out) = state.lanes.put(put);
                skipped = out;
                if !out && let Some((track, target)) = lost {
                    forget_lane(state, &mut self.graph, track, target);
                }
                Some(Garbage::Lanes(spare))
            }
            RtMsg::LaneRemove(lane) => match state.lanes.take(lane.0) {
                Ok(old) => {
                    if let Some(old) = &old {
                        forget_lane(state, &mut self.graph, old.track, old.target);
                    }
                    Some(Garbage::Lanes(PagedSpare {
                        growth: None,
                        item: old,
                    }))
                }
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
                        Some(entry) => {
                            *entry = clip;
                            if let Some(clip) = clip {
                                // A clip that plays is reported in the cell it sits in now.
                                t.player.moved(clip, scene);
                            }
                        }
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
            RtMsg::AllSoundOff => {
                // A cut already running is not restarted (a double order fades once).
                if self.cut.is_none() {
                    self.cut = Some(Cut { done: 0 });
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
                    LaunchMsg::Scene { scene } => launcher::launch_scene(state, scene),
                    LaunchMsg::StopAll => launcher::stop_all(state),
                    LaunchMsg::BackToArrangement => launcher::back_to_arrangement(state),
                    LaunchMsg::BackTrackToArrangement { track } => match state.track_mut(track) {
                        Some(t) => launcher::back_track(t),
                        None => skipped = true,
                    },
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

/// The lanes of `track`, through the list the last block linked (AM-2,
/// [`crate::automation`]).
fn lanes_of<'a>(state: &'a mut State, graph: &Graph, track: TrackSlot) -> Option<TrackLanes<'a>> {
    let head = if track.is_master() {
        graph.master_lanes.head
    } else {
        state.track(track).map(|t| t.graph.lanes)?
    };
    Some(TrackLanes {
        table: &mut state.lanes,
        head,
    })
}

/// A manual gesture on `track` holds its lanes whose target `matches`.
fn hold_lanes(
    state: &mut State,
    graph: &Graph,
    track: TrackSlot,
    matches: impl Fn(AutomationTarget) -> bool,
) {
    if let Some(mut lanes) = lanes_of(state, graph, track) {
        lanes.hold(track, matches);
    }
}

/// The device in slot `device` of `track` changed (replaced, switched on or off): its lanes
/// send their value again.
fn resend_lanes(state: &mut State, graph: &Graph, track: TrackSlot, device: DeviceSlot) {
    if let Some(mut lanes) = lanes_of(state, graph, track) {
        lanes.resend(track, device);
    }
}

/// A lane of `track` on `target` is gone: a mixer lane gives the strip's field back to the
/// fader (AM-2); a device keeps the lane's last value.
fn forget_lane(state: &mut State, graph: &mut Graph, track: TrackSlot, target: AutomationTarget) {
    if track.is_master() {
        match target {
            AutomationTarget::MixerGain => graph.forget_master_gain(),
            AutomationTarget::MixerPan => graph.forget_master_pan(),
            AutomationTarget::DeviceParam { .. } => {}
        }
        return;
    }
    let Some(t) = state.track_mut(track) else {
        return;
    };
    match target {
        AutomationTarget::MixerGain => t.graph.forget_gain(),
        AutomationTarget::MixerPan => t.graph.forget_pan(),
        AutomationTarget::DeviceParam { .. } => {}
    }
}

/// Whether a chain entry receives notes (a MIDI effect or an instrument): changing one
/// (insert, remove, enable, disable) switches the instrument's note stream, or skips the
/// note-offs a bypassed one would have needed, so the graph releases the track's notes at
/// its next block (MO-3). An audio effect receives no notes: changing it releases nothing.
fn takes_notes(entry: &DeviceEntry) -> bool {
    entry.device.descriptor().role != DeviceRole::Effect
}

fn device_mut(state: &mut State, track: TrackSlot, device: DeviceSlot) -> Option<&mut DeviceEntry> {
    state
        .chain_mut(track)?
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
