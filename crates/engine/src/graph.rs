//! The track graph (MO-3, `docs/specs/02-decoupage-mvp.md` § 2.3 and § 2.5): renders the
//! tracks of the engine [`State`], each through its chain of devices, and sums them into
//! the master.
//!
//! # A block
//!
//! [`crate::engine::Engine::render`] renders one block after `begin_block` applied the rt
//! messages, with the segments the clock cut it in ([`BlockClock`]):
//!
//! 1. every track, in slot order: a track with no enabled device is **empty** and costs a
//!    slot visit, nothing else (no buffer cleared, no mixing). Otherwise the track's scratch
//!    is cleared, its live notes are routed to it (offset 0 of the block: a live event
//!    carries no time), its schedule, or its launched clip ([`crate::clip_player`], CL-2), is
//!    read ([`crate::events`], MO-4), and its chain runs
//!    once per **sub-block** (a segment of the clock, cut again when the events of the
//!    track fill its sink: a sub-block never crosses a loop wrap), in slot order: a **MIDI effect** turns the current note
//!    stream into its own output, which becomes the stream (only its notes: CT-5); the
//!    **first** enabled **instrument** receives the stream and writes the sound (a second
//!    one is ignored: the project allows one); an **audio effect** transforms the sound in
//!    place; a disabled device is bypassed. The track is then summed into the output with
//!    its mix. The slot order is the chain order: the project keeps CT-5's (MIDI effects,
//!    instrument, effects); a device placed elsewhere runs where it is (an effect before
//!    the instrument transforms silence, a MIDI effect after it feeds nothing).
//!
//!    The voice ids a MIDI effect writes are its own (CT-5): when a MIDI effect or an
//!    instrument is inserted, removed, enabled or disabled, the instrument would never
//!    receive the note-offs of the notes it holds under the old stream (or while it was
//!    bypassed), so the next rendered block of the track starts with an `AllNotesOff`
//!    before its live notes. The ids need no other remapping: the output of a MIDI effect
//!    replaces the stream (two streams are never merged), and the live input and a
//!    schedule use separate ranges ([`crate::events::SCHEDULE_FIRST_VOICE`]);
//! 2. every **retiring** track (below), with a target of silence;
//! 3. the master's chain of effects over the sum (#364): `run_chain` over the whole block, as
//!    a track's chain but with no note stream; then the master's strip. The master's meter
//!    and the export read after the strip, so after the chain.
//!
//! One stereo scratch serves every track, since they render one after another: the
//! buffers of the graph are allocated once, by [`crate::engine::channel`], off the audio
//! path.
//!
//! # Mix
//!
//! The strips (a track's, the master's), their pan law and their timed parameters are the
//! mixer's ([`crate::mixer`], MX-1): a track's level on each channel is
//! `audible × gain × balance(pan)`, `audible` being false when the track is muted or when
//! another track is soloed and this one is not (solo is cumulative: every soloed track
//! plays). The track is mixed into the sum sub-block by sub-block, right after its chain,
//! with the timed gain, pan and mute its schedule addresses to the strip
//! ([`crate::mixer::MIXER_SLOT`]); the master's strip applies over the sum with those of
//! the master's schedule ([`State::set_master_schedule`]).
//!
//! Every change a message makes moves by a linear ramp of [`FADE_SECONDS`] (the smoothing
//! rule of CT-5: a new target starts a new ramp from the current value), the master's too,
//! so that a gain, a pan, a mute or a solo never jumps; a timed gain or pan is exact.
//!
//! # Fades in and out
//!
//! A new track's levels start at 0: it **fades in** over [`FADE_SECONDS`] from its first
//! rendered block. A track that leaves its slot (`TrackRemove`, a `TrackPut` on an
//! occupied slot, every track of the old state on a `Reset`) is not sent to the garbage by
//! `apply`: it joins the **retiring** list, an intrusive list linked through
//! [`TrackGraph`] (moving a `Box`: no allocation, no bound), and **fades out**: it keeps
//! rendering, with a target of silence, until both its levels are 0 and still. It is then
//! unlinked and pushed on the **retire ring**, which the prepare side drains
//! (`Preparer::collect`) and frees. When that ring is full, the track stays in the list,
//! silent and no longer rendered, and is pushed again at the next block: nothing is lost,
//! nothing is freed on the audio thread.
//!
//! The retire ring is not the garbage ring: the garbage ring's bound (one item per rt
//! message, CT-3) holds what `apply` returns, and a fade that ends in a render is not an rt
//! message.
//!
//! # Device fades
//!
//! A device switched on or off, removed or replaced while its chain is heard fades over
//! [`FADE_SECONDS`] too (#233), so that a chain never jumps. Each instrument and audio effect
//! has a `fade` in `[0, 1]` ([`DeviceEntry::fade`]), moving linearly toward 1 while it is
//! enabled and toward 0 otherwise. A slot where it moves runs as a blend of the signal
//! entering the slot (`dry`) and the device's output: `dry + fade × (wet − dry)`. A settled
//! device costs nothing more: at 1 it runs in place, at 0 it is skipped.
//!
//! - A device fading out (switched off) receives no event: an instrument's voices and an
//!   effect's tail ring down under the fade; at 0 it is reset, so nothing of it comes back
//!   when it is switched on again. Switched on again before 0, it fades back up from where it
//!   is, tail included.
//! - A device that leaves its slot (`DeviceRemove`, a `DevicePut` on an occupied slot) joins
//!   the chain's **outgoing** list ([`Outgoing`], at most [`OUTGOING_ROOM`]) and
//!   keeps running at that slot, with no event, over the same `dry` signal, its difference
//!   added with its falling fade: a replacement is a crossfade. At 0 it goes to the retire
//!   ring (a full ring keeps it, silent, for the next block), as a retiring track does.
//! - A MIDI effect has no sound of its own and does not fade: its change releases the
//!   instrument's notes, which the instruments end with their own release.
//! - A change on a chain nobody hears (its strip's levels both at 0: a track not yet faded
//!   in, a muted track) applies at once.
//!
//! Every device of a slot runs once per sub-block, on the same input.

use rtrb::{Producer, PushError};

use crate::automation::{LANE_ROOM, MasterLanes, NO_LANE, TrackLanes};
use crate::clip_player::Env;
use crate::device::{
    AudioIo, BlockCtx, DeviceRole, DeviceSetup, Event, EventBuffer, Transport, VoiceId,
};
use crate::engine::{Clip, DeviceEntry, Lane, Paged, State, Track};
use crate::events::{NOTE_ROOM, PARAM_CHUNK, Read, Schedule, Sink};
pub use crate::mixer::mix_levels;
use crate::mixer::{Mixer, timed};
use crate::msg::{DeviceSlot, LiveEvent, MsgError, TrackMeter, TrackSlot};
use crate::time::{Quantize, Tick, TimeSignature};
use crate::transport::Segment;

/// Duration of a track's fade in or out, and of every change of a level, in seconds.
pub const FADE_SECONDS: f64 = 0.005;

/// Slots of the retire ring: retired tracks and devices waiting for the prepare side.
pub const RETIRE_RING_CAPACITY: usize = 4096;

/// Outgoing devices one chain holds at most (#233): a device leaving a chain that already
/// has this many fading out goes to the garbage at once (a hard cut, as before #233). One
/// edit can rebuild a whole chain (a device inserted or moved near its head rebuilds every
/// device after it, `commands::engine_msgs`), so the room covers a chain of 32 devices
/// changed at once (beyond, the last ones cut). It bounds the extra device runs of a chain
/// to 32 per block for one fade, and its cost in the worst case of the engine's tables
/// (`tests/it/msg.rs`: 256 bytes a track, 2 MiB for 4 096 tracks counted twice) fits the budget.
pub const OUTGOING_ROOM: usize = 32;

/// What the audio side hands back through the retire ring once its fade is over: a track
/// that left its slot, or a device that left its chain (#233).
#[derive(Debug)]
pub enum Retired {
    Track(Box<Track>),
    Device(Box<DeviceEntry>),
}

/// A chain's outgoing devices (#233, module doc > Device fades): removed or replaced while
/// the chain was heard, each fading out at the slot it held. A fixed array of at most
/// [`OUTGOING_ROOM`] boxes, packed at its start (moving a `Box`: no allocation). An array in
/// the chain rather than a link in every [`DeviceEntry`]: the worst case of the engine's
/// tables counts every device slot of every track (`tests/it/msg.rs`), where a link would cost
/// more than the room does.
#[derive(Debug)]
pub struct Outgoing {
    devices: [Option<Box<DeviceEntry>>; OUTGOING_ROOM],
    len: usize,
}

impl Default for Outgoing {
    fn default() -> Self {
        Self::new()
    }
}

impl Outgoing {
    pub const fn new() -> Self {
        Outgoing {
            devices: [const { None }; OUTGOING_ROOM],
            len: 0,
        }
    }

    /// Devices fading out or waiting for room in the retire ring.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The devices, packed.
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut DeviceEntry> {
        self.devices[..self.len]
            .iter_mut()
            .flatten()
            .map(|e| &mut **e)
    }

    /// Takes `entry` (disabled, its `slot` set); gives it back when the room is full.
    pub(crate) fn push(&mut self, entry: Box<DeviceEntry>) -> Result<(), Box<DeviceEntry>> {
        match self.devices.get_mut(self.len) {
            Some(free) => {
                *free = Some(entry);
                self.len += 1;
                Ok(())
            }
            None => Err(entry),
        }
    }

    /// Whether a device still fading out sits at `slot`.
    fn sounds_at(&self, slot: usize) -> bool {
        self.devices[..self.len]
            .iter()
            .flatten()
            .any(|e| usize::from(e.slot) == slot && e.fade > 0.0)
    }

    /// Ends every fade at once (the panic cut, once the output is silent): the devices
    /// leave at the next sweep.
    pub(crate) fn end(&mut self) {
        for e in self.iter_mut() {
            e.fade = 0.0;
        }
    }

    /// Hands every device whose fade is over to the retire ring; one that finds the ring
    /// full stays, silent, and is pushed again at the next sweep. A device whose slot is
    /// beyond the chain's `slots` (a chain the list outlived) is never run again: its fade
    /// ends here.
    fn sweep(&mut self, retired: &mut Producer<Retired>, slots: usize) {
        let mut kept = 0;
        for i in 0..self.len {
            let Some(mut entry) = self.devices[i].take() else {
                continue;
            };
            if usize::from(entry.slot) >= slots {
                entry.fade = 0.0;
            }
            if entry.fade <= 0.0 {
                match retired.push(Retired::Device(entry)) {
                    Ok(()) => continue,
                    Err(PushError::Full(Retired::Device(back))) => entry = back,
                    Err(PushError::Full(other)) => {
                        // Unreachable: the ring gives back what it was given. Leaked rather
                        // than freed on the audio thread.
                        core::mem::forget(other);
                        continue;
                    }
                }
            }
            self.devices[kept] = Some(entry);
            kept += 1;
        }
        self.len = kept;
    }
}

/// Most note events a track's chain receives in a sub-block ([`crate::events`]): the live
/// block and its releases, the release of a jump, and the schedule's room. A MIDI effect's
/// output is sized from it (`MidiOutput::capacity`), whatever the size of the tables the
/// schedule reads: a sub-block is cut when its room is full (MO-4). Chained MIDI effects
/// that multiply their notes beyond it meet CT-5's overflow safety net.
pub const TRACK_INPUT_EVENTS: usize = NOTE_ROOM;

/// Sounding notes of an edited clip that a track ends by their own note-off at the next
/// block ([`TrackGraph::end_note`], CL-3): more than this fall back on a release of every
/// note of the track.
pub const ORPHAN_ROOM: usize = 64;

/// Frames a fade lasts at `sample_rate` (at least 1).
pub fn fade_frames(sample_rate: f32) -> u32 {
    let frames = (FADE_SECONDS * f64::from(sample_rate)).round();
    if frames.is_finite() && frames >= 1.0 {
        frames.min(f64::from(u32::MAX)) as u32
    } else {
        1
    }
}

/// The largest gain reduction a meter reports, in decibels (a bound for the page's scale).
const MAX_REDUCTION_DB: f32 = 60.0;

/// A meter accumulated between two reports (MO-5): the peak and the sum of squares of each
/// channel. No allocation: four floats.
#[derive(Clone, Copy, Debug, Default)]
pub struct MeterAcc {
    peak: [f32; 2],
    squares: [f32; 2],
    /// The largest gain reduction a device of the strip reported at the end of a block of the
    /// period, in decibels (#469).
    reduction: f32,
}

impl MeterAcc {
    /// Adds one sample of channel `i` (already scaled by the fader). A non-finite sample
    /// stays out of both the peak and the sum of squares.
    #[inline]
    pub(crate) fn push(&mut self, i: usize, sample: f32) {
        let a = sample.abs();
        // Below 1e-18 (-360 dB) a sample is silence for a meter, and squaring it would be a
        // denormal on every sample of a decaying tail.
        if a.is_finite() && a > 1e-18 {
            self.peak[i] = self.peak[i].max(a);
            self.squares[i] += a * a;
        }
    }

    /// Keeps the largest gain reduction of the period; a negative or non-finite one counts as none.
    #[inline]
    pub(crate) fn note_reduction(&mut self, db: f32) {
        if db.is_finite() && db > self.reduction {
            self.reduction = db.min(MAX_REDUCTION_DB);
        }
    }

    /// Adds a block of the two channels.
    fn add(&mut self, left: &[f32], right: &[f32]) {
        for s in left {
            self.push(0, *s);
        }
        for s in right {
            self.push(1, *s);
        }
    }

    /// The meter over `frames` frames, then a fresh period.
    pub(crate) fn take(&mut self, frames: u32) -> TrackMeter {
        let n = frames.max(1) as f32;
        let m = TrackMeter {
            peak_l: self.peak[0],
            peak_r: self.peak[1],
            rms_l: (self.squares[0] / n).sqrt(),
            rms_r: (self.squares[1] / n).sqrt(),
            gain_reduction_db: self.reduction,
        };
        *self = MeterAcc::default();
        m
    }
}

/// What the graph keeps in a track: its two levels and, while it retires, the next track of
/// the retiring list.
#[derive(Debug)]
pub struct TrackGraph {
    /// The track's strip: gain, pan, mute and solo as two ramps ([`crate::mixer`], MX-1).
    mixer: Mixer,
    /// The chain's note stream changed (a MIDI effect or an instrument inserted, removed,
    /// enabled or disabled): the next rendered block releases every note of the track
    /// first.
    release: bool,
    /// Notes of an edited clip that no table can end any more, as `voice | key << 24`: the
    /// next rendered block ends each with its own note-off, the live notes untouched.
    orphans: [u32; ORPHAN_ROOM],
    orphan_len: usize,
    /// The tick-dated events the graph plays on the track ([`crate::events`], MO-4). No
    /// message fills it (the automation is read from its lanes, [`crate::automation`]);
    /// tests set it off the audio path ([`TrackGraph::set_schedule`]). It is silent while the track's clip player runs: it
    /// plays the track's arrangement, a launched clip or has a launch to take
    /// ([`crate::clip_player`], CL-2, [`crate::arrangement`], AR-2).
    schedule: Option<Box<Schedule>>,
    /// The track's meter since the last report, after its fader (MO-5).
    meter: MeterAcc,
    /// The first of the track's automation lanes, linked at every block
    /// ([`crate::automation::link`]), and how many are linked.
    pub(crate) lanes: u32,
    pub(crate) lane_count: u32,
    /// The tick the next clock segment starts at when the playhead moves on without a jump
    /// (`None` after a stopped block): a jump gives the parameters back to the lanes.
    lane_expect: Option<Tick>,
    /// The bits of the playhead and the cue count the lanes were last applied at while the
    /// clock is stopped (`None` while it runs): a stop or a seek changes it and gives the
    /// parameters back to the lanes.
    stopped_at: Option<(u64, u64)>,
    /// The devices that left the track's chain while it was heard, fading out (#233).
    outgoing: Outgoing,
    next: Option<Box<Track>>,
}

impl TrackGraph {
    /// A new track's: both levels at 0, so that it fades in.
    pub const fn new() -> Self {
        TrackGraph {
            mixer: Mixer::new(),
            release: false,
            orphans: [0; ORPHAN_ROOM],
            orphan_len: 0,
            schedule: None,
            meter: MeterAcc {
                peak: [0.0; 2],
                squares: [0.0; 2],
                reduction: 0.0,
            },
            lanes: NO_LANE,
            lane_count: 0,
            lane_expect: None,
            stopped_at: None,
            outgoing: Outgoing::new(),
            next: None,
        }
    }

    /// The devices that left the track's chain and still fade out (#233).
    pub fn outgoing_len(&self) -> usize {
        self.outgoing.len()
    }

    /// The track's outgoing list.
    pub(crate) fn outgoing(&mut self) -> &mut Outgoing {
        &mut self.outgoing
    }

    /// A gain lane is gone: the strip takes the fader's gain back.
    pub(crate) fn forget_gain(&mut self) {
        self.mixer.forget_gain();
    }

    /// A pan lane is gone: the strip takes the fader's pan back.
    pub(crate) fn forget_pan(&mut self) {
        self.mixer.forget_pan();
    }

    /// Puts `schedule` on the track and returns the one it replaces (to be freed off the
    /// audio path). When the old one holds notes, the next rendered block releases every
    /// note of the track first, as for a changed chain: nothing would end them.
    pub fn set_schedule(&mut self, schedule: Option<Box<Schedule>>) -> Option<Box<Schedule>> {
        let old = core::mem::replace(&mut self.schedule, schedule);
        // The lane that drove the strip may be gone: the faders take it back.
        self.mixer.forget();
        if old.as_deref().is_some_and(|s| s.held() > 0) {
            self.release = true;
        }
        old
    }

    /// The track's schedule, if any.
    pub fn schedule(&self) -> Option<&Schedule> {
        self.schedule.as_deref()
    }

    /// Releases every note of the track at the next rendered block: the notes a MIDI
    /// effect held under its own voice ids would never get their note-off otherwise.
    ///
    /// It cuts the notes played live too, so it is for the cases that must: a chain that
    /// changed (the instrument that sounded is gone or replaced), the panic cut
    /// (`reset_devices`), a schedule dropped with notes held, and the fallbacks of an edited
    /// clip that have more notes to end than [`ORPHAN_ROOM`] (#443; a limit the memory
    /// budget accepts). An edited clip's notes are otherwise ended one by one with
    /// [`TrackGraph::end_note`], which leaves the live notes alone.
    pub(crate) fn release_notes(&mut self) {
        self.release = true;
    }

    /// Ends the note `(voice, key)` of an edited clip by its own note-off at the next
    /// rendered block (CL-3). With no room left the track releases every note instead.
    pub(crate) fn end_note(&mut self, voice: VoiceId, key: u8) {
        match self.orphans.get_mut(self.orphan_len) {
            Some(slot) => {
                *slot = voice.get() | u32::from(key.min(127)) << 24;
                self.orphan_len += 1;
            }
            None => self.release = true,
        }
    }

    /// Notes that [`TrackGraph::end_note`] can still take before the next rendered block.
    pub(crate) fn orphan_room(&self) -> usize {
        ORPHAN_ROOM - self.orphan_len.min(ORPHAN_ROOM)
    }

    /// Whether a release waits for the next rendered block.
    pub fn releasing(&self) -> bool {
        self.release
    }

    /// The levels (left, right) at the next frame.
    pub fn levels(&self) -> (f32, f32) {
        self.mixer.levels()
    }

    /// Both levels at 0 and still: the track is silent.
    fn is_silent(&self) -> bool {
        self.mixer.is_silent()
    }
}

impl Default for TrackGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- the graph

/// The buffers a track renders in, shared by every track.
struct Scratch {
    left: Box<[f32]>,
    right: Box<[f32]>,
    /// The events of the track's sub-block being rendered.
    sink: Sink,
    /// The events of one device: its parameter values merged with the note stream. It
    /// holds a sub-block's notes and values; for a device that has values and follows a
    /// chained MIDI effect whose output is larger, the values apply through
    /// `Device::set_param` at the sub-block's first frame instead (no note is lost).
    merged: EventBuffer,
    /// The `events_out` given to the devices that write no events.
    unused: EventBuffer,
    /// The buffers of a device fade (#233).
    blend: Blend,
}

/// The buffers a slot of a chain uses while one of its devices fades (#233, module doc >
/// Device fades): the signal entering the slot, and an outgoing device's output.
struct Blend {
    dry_left: Box<[f32]>,
    dry_right: Box<[f32]>,
    old_left: Box<[f32]>,
    old_right: Box<[f32]>,
}

/// The clock as the graph reads it for one block: the segments TR-1 cut it in (none when
/// stopped), and the position of its first frame.
#[derive(Clone, Copy, Debug)]
pub struct BlockClock<'a> {
    pub running: bool,
    pub segments: &'a [Segment],
    /// The playhead at the block's first frame, in fractional ticks.
    pub position: f64,
    /// The block's first frame since the engine started (the automation's control frames,
    /// [`crate::automation`]).
    pub frame: u64,
    /// The cues the clock has taken ([`crate::transport::Transport::cues`]): a seek while stopped, even to the
    /// tick the playhead stands on, changes it.
    pub cues: u64,
}

/// The render of the graph (module doc), owned by the audio side.
pub struct Graph {
    sample_rate: f32,
    fade: u32,
    max_frames: usize,
    scratch: Scratch,
    /// The master's strip, and the timed parameters it plays ([`crate::mixer`], MX-1).
    master: Mixer,
    /// The `master_schedule_gen` of the state the master strip last read.
    master_gen: u32,
    /// The master's automation lanes, linked at every block (#378).
    pub(crate) master_lanes: MasterLanes,
    /// The output's meter since the last report, after the master and the metronome (MO-5).
    master_meter: MeterAcc,
    /// Voices sounding in the tracks' devices after the last block (MO-5).
    voices: u32,
    retiring: Option<Box<Track>>,
    retiring_len: usize,
    retired: Producer<Retired>,
    /// The devices that left the master's chain while it was heard, fading out (#233).
    master_outgoing: Outgoing,
}

impl core::fmt::Debug for Graph {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Graph")
            .field("sample_rate", &self.sample_rate)
            .field("max_frames", &self.max_frames)
            .field("retiring", &self.retiring_len)
            .finish_non_exhaustive()
    }
}

fn buffer(frames: usize) -> Result<Box<[f32]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(frames)
        .map_err(|_| MsgError::OutOfMemory {
            bytes: frames.saturating_mul(size_of::<f32>()),
        })?;
    v.resize(frames, 0.0);
    Ok(v.into_boxed_slice())
}

/// The voice id of a live note: its key, plus one (0 is not a voice id).
fn live_voice(pitch: u8) -> Option<VoiceId> {
    VoiceId::new(u32::from(pitch) + 1)
}

impl Graph {
    /// The graph's buffers for blocks up to `setup.max_block_frames`, off the audio path;
    /// `retired` is the retire ring's producer.
    pub(crate) fn try_new(
        setup: &DeviceSetup,
        retired: Producer<Retired>,
    ) -> Result<Self, MsgError> {
        let max_frames = setup.max_block_frames.max(1);
        let sample_rate = if setup.sample_rate.is_finite() && setup.sample_rate > 0.0 {
            setup.sample_rate
        } else {
            48_000.0
        };
        Ok(Graph {
            sample_rate,
            fade: fade_frames(sample_rate),
            max_frames,
            scratch: Scratch {
                left: buffer(max_frames)?,
                right: buffer(max_frames)?,
                sink: Sink::try_with_lane_room(NOTE_ROOM, PARAM_CHUNK, LANE_ROOM)?,
                merged: EventBuffer::try_with_capacity(NOTE_ROOM + PARAM_CHUNK + 1)?,
                unused: EventBuffer::try_with_capacity(1)?,
                blend: Blend {
                    dry_left: buffer(max_frames)?,
                    dry_right: buffer(max_frames)?,
                    old_left: buffer(max_frames)?,
                    old_right: buffer(max_frames)?,
                },
            },
            master: Mixer::unity(),
            master_gen: 0,
            master_lanes: MasterLanes::new(),
            master_meter: MeterAcc::default(),
            voices: 0,
            retiring: None,
            retiring_len: 0,
            retired,
            master_outgoing: Outgoing::new(),
        })
    }

    /// The largest block the graph renders at once (the setup's).
    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// A `Reset` gave the master its default state: the strip takes it back.
    pub(crate) fn reset_master(&mut self) {
        self.master.forget();
    }

    /// A master gain lane is gone: the strip takes the fader's gain back (#378).
    pub(crate) fn forget_master_gain(&mut self) {
        self.master.forget_gain();
    }

    /// A master pan lane is gone: the strip takes the fader's pan back.
    pub(crate) fn forget_master_pan(&mut self) {
        self.master.forget_pan();
    }

    /// The master's levels (left, right) at the next frame.
    pub fn master_levels(&self) -> (f32, f32) {
        self.master.levels()
    }

    /// Frames of a fade.
    pub fn fade(&self) -> u32 {
        self.fade
    }

    /// Tracks fading out or waiting for room in the retire ring.
    pub fn retiring(&self) -> usize {
        self.retiring_len
    }

    /// Devices that left the master's chain and still fade out (#233).
    pub fn master_outgoing_len(&self) -> usize {
        self.master_outgoing.len()
    }

    /// The master's outgoing list.
    pub(crate) fn master_outgoing(&mut self) -> &mut Outgoing {
        &mut self.master_outgoing
    }

    /// Changes the sample rate the fades and the devices' contexts use.
    pub(crate) fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate > 0.0 {
            self.sample_rate = sample_rate;
            self.fade = fade_frames(sample_rate);
        }
    }

    /// Takes a track that left its slot: it fades out from its current levels. No
    /// allocation: the box is linked at the head of the list.
    pub(crate) fn retire(&mut self, mut track: Box<Track>) {
        track.graph.next = self.retiring.take();
        self.retiring = Some(track);
        self.retiring_len += 1;
    }

    /// Renders one block of `left.len().min(right.len())` frames, at most
    /// [`Graph::max_frames`] (module doc), cut by `clock`'s segments; `live` and `release`
    /// (the live events of the block and whether the live notes must be released) are
    /// delivered at its first frame.
    pub(crate) fn block(
        &mut self,
        state: &mut State,
        clock: &BlockClock<'_>,
        live: &[LiveEvent],
        release: bool,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let frames = left.len().min(right.len()).min(self.max_frames);
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        left.fill(0.0);
        right.fill(0.0);
        let ctx = BlockCtx {
            sample_rate: self.sample_rate,
            frames: frames as u32,
            transport: Transport {
                playing: clock.running,
                tempo: state.transport.tempo,
                signature: state.transport.signature,
                position_ticks: clock.position,
            },
        };
        let fade = self.fade;
        let mut voices = 0u32;
        let any_solo = state.tracks.as_slice().iter().flatten().any(|t| t.solo);
        let (quantize, signature) = (state.transport.launch_quantize, state.transport.signature);
        crate::automation::link(&mut state.tracks, &mut state.lanes, &mut self.master_lanes);
        let clips = &mut state.clips;
        let lanes = &mut state.lanes;
        for (slot, entry) in state.tracks.as_mut_slice().iter_mut().enumerate() {
            let Some(track) = entry.as_deref_mut() else {
                continue;
            };
            let audible = !any_solo || track.solo;
            track
                .graph
                .mixer
                .sync(track.gain, track.pan, track.mute, audible, fade);
            let input = Input {
                slot: TrackSlot(slot as u32),
                live,
                release,
                clips: Some(ClipsIn {
                    clips: &mut *clips,
                    quantize,
                    signature,
                }),
                lanes: Some(&mut *lanes),
            };
            render_track(
                track,
                Some(input),
                clock,
                &ctx,
                &mut self.scratch,
                fade,
                left,
                right,
            );
            // The devices whose fade-out ended leave for the prepare side (#233).
            let slots = track.devices.as_slice().len();
            track.graph.outgoing.sweep(&mut self.retired, slots);
            voices = voices.saturating_add(track_voices(track));
            // After the whole block: the reduction the strip's compressors ended it with.
            track
                .graph
                .meter
                .note_reduction(chain_reduction(track.devices.as_slice()));
        }
        self.voices = voices;

        // The retiring tracks: rendered toward silence, then handed to the retire ring.
        let mut rest = self.retiring.take();
        let mut kept: Option<Box<Track>> = None;
        while let Some(mut track) = rest {
            rest = track.graph.next.take();
            track.graph.mixer.fade_out(fade);
            let done = track.graph.is_silent()
                || !render_track(
                    &mut track,
                    None,
                    clock,
                    &ctx,
                    &mut self.scratch,
                    fade,
                    left,
                    right,
                )
                || track.graph.is_silent();
            if done {
                match self.retired.push(Retired::Track(track)) {
                    Ok(()) => {
                        self.retiring_len -= 1;
                        continue;
                    }
                    Err(PushError::Full(Retired::Track(back))) => track = back,
                    Err(PushError::Full(other)) => {
                        // Unreachable: the ring gives back what it was given. Leaked rather
                        // than freed on the audio thread.
                        core::mem::forget(other);
                        self.retiring_len -= 1;
                        continue;
                    }
                }
            }
            track.graph.next = kept;
            kept = Some(track);
        }
        self.retiring = kept;

        self.master_pass(state, clock, &ctx, left, right);
        self.master_outgoing
            .sweep(&mut self.retired, state.master_devices.as_slice().len());
        // The master pass has several paths (with or without lanes): the reduction is read once, after any of them.
        self.master_meter
            .note_reduction(chain_reduction(state.master_devices.as_slice()));
    }

    /// The master's strip over the sum (module doc), with the timed parameters of the
    /// master's schedule, read like a track's.
    fn master_pass(
        &mut self,
        state: &mut State,
        clock: &BlockClock<'_>,
        ctx: &BlockCtx,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let fade = self.fade;
        if state.master_schedule_gen != self.master_gen {
            // The lane that drove the master may be gone: the faders take it back.
            self.master_gen = state.master_schedule_gen;
            self.master.forget();
        }
        let frames = left.len().min(right.len());
        self.master.sync(
            state.master.gain,
            state.master.pan,
            state.master.mute,
            true,
            fade,
        );
        // With lanes (#378) the chain and the strip run sub-block by sub-block, as a track's.
        if self.master_lanes.head != NO_LANE {
            self.master_lane_pass(state, clock, ctx, left, right);
            return;
        }
        let Graph {
            master,
            scratch,
            master_outgoing,
            ..
        } = self;
        // The master's chain (#364) runs over the whole sum, before the fader: effects only,
        // no note, so the sink is empty. An empty or bypassed chain leaves the sum as it is.
        scratch.sink.clear();
        run_chain(
            state.master_devices.as_mut_slice(),
            master_outgoing,
            &scratch.sink,
            &mut scratch.merged,
            &mut scratch.unused,
            &mut scratch.blend,
            fade,
            ctx,
            left,
            right,
        );
        let sink = &mut scratch.sink;
        let Some(schedule) = state.master_schedule.as_deref_mut() else {
            master.apply_in_place(left, right, core::iter::empty(), fade);
            return;
        };
        if !clock.running || clock.segments.is_empty() {
            schedule.stop(sink);
            master.apply_in_place(left, right, timed(sink.params()), fade);
            return;
        }
        let mut start = 0;
        let mut i = 0;
        while i < clock.segments.len() && start < frames {
            let segment = clock.segments[i];
            let end = match schedule.read(&segment, start, sink) {
                Read::Done => {
                    i += 1;
                    segment.offset + segment.frames
                }
                Read::Full(at) => start + at.max(1),
            }
            .min(frames);
            if end > start {
                master.apply_in_place(
                    &mut left[start..end],
                    &mut right[start..end],
                    timed(sink.params()),
                    fade,
                );
                sink.clear();
                start = end;
            }
        }
        if start < frames {
            // Unreachable: the segments of a running block cover it.
            master.apply_in_place(
                &mut left[start..frames],
                &mut right[start..frames],
                core::iter::empty(),
                fade,
            );
        }
    }

    /// The master's chain and strip over the sum when the master has automation lanes (#378):
    /// the same sub-block loop as a track's ([`render_track`]), without a clip player. A piece
    /// ends at the next point of a lane, and at the next control frame while a lane moves; the
    /// chain runs over the piece with the devices' values of the sink, and the strip applies
    /// the gain and pan lanes' values of the sink (and the master schedule's, if any).
    fn master_lane_pass(
        &mut self,
        state: &mut State,
        clock: &BlockClock<'_>,
        ctx: &BlockCtx,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let fade = self.fade;
        let frames = left.len().min(right.len());
        let Graph {
            master,
            scratch,
            master_lanes,
            master_outgoing,
            ..
        } = self;
        let State {
            master_devices,
            master_schedule,
            lanes: lane_table,
            ..
        } = state;
        let Scratch {
            sink,
            merged,
            unused,
            blend,
            ..
        } = scratch;
        sink.clear();
        let mut lanes = Some(TrackLanes {
            table: lane_table,
            head: master_lanes.head,
        })
        .filter(|l| !l.is_empty());
        let mut schedule = master_schedule.as_deref_mut();
        let devices = master_devices.as_mut_slice();
        let mut chain = |sink: &Sink, from: usize, to: usize, position: f64| {
            let sub = BlockCtx {
                frames: (to - from) as u32,
                transport: Transport {
                    position_ticks: position,
                    ..ctx.transport
                },
                ..*ctx
            };
            run_chain(
                devices,
                master_outgoing,
                sink,
                merged,
                unused,
                blend,
                fade,
                &sub,
                &mut left[from..to],
                &mut right[from..to],
            );
            master.apply_in_place(
                &mut left[from..to],
                &mut right[from..to],
                timed(sink.params()),
                fade,
            );
        };
        if !clock.running || clock.segments.is_empty() {
            // Stopped: the lanes apply their value at the playhead once, at the stop and at
            // every seek; a gesture holds until the next play or seek (AM-2, #388).
            master_lanes.expect = None;
            if clock.running {
                master_lanes.stopped_at = None;
            } else {
                let at = (clock.position.to_bits(), clock.cues);
                let moved = master_lanes.stopped_at != Some(at);
                master_lanes.stopped_at = Some(at);
                if let Some(l) = lanes.as_mut() {
                    l.push_frozen(clock.position, moved, sink);
                }
            }
            if let Some(s) = schedule.as_deref_mut() {
                s.stop(sink);
            }
            chain(sink, 0, frames, clock.position);
            return;
        }
        master_lanes.stopped_at = None;
        let mut start = 0;
        let mut i = 0;
        let mut position = clock.position;
        let mut entered = usize::MAX;
        let mut piece: Option<Segment> = None;
        let mut piece_from = Tick::ZERO;
        let mut pushed = false;
        while i < clock.segments.len() && start < frames {
            let segment = clock.segments[i];
            let seg_end = segment.offset + segment.frames;
            if entered != i {
                entered = i;
                piece = None;
                piece_from = segment.from;
                pushed = false;
                if let Some(l) = lanes.as_mut()
                    && master_lanes.expect != Some(segment.from)
                {
                    l.jump();
                }
                master_lanes.expect = Some(segment.to);
            }
            position = segment.position_at(start);
            let current = match piece {
                Some(p) => p,
                None => {
                    let cut = match lanes.as_mut() {
                        Some(l) if seg_end > start => {
                            l.piece_end(&segment, clock.frame, start, seg_end)
                        }
                        _ => seg_end,
                    };
                    let p = if cut >= seg_end && piece_from == segment.from {
                        segment
                    } else {
                        segment.piece(start, cut, piece_from)
                    };
                    piece = Some(p);
                    p
                }
            };
            if !pushed
                && seg_end > start
                && let Some(l) = lanes.as_mut()
            {
                l.push_start(&segment, clock.frame, start, sink);
                pushed = true;
            }
            let read = match schedule.as_deref_mut() {
                Some(s) => s.read(&current, start, sink),
                None => Read::Done,
            };
            let piece_end = current.offset + current.frames;
            let end = match read {
                Read::Done => {
                    piece = None;
                    piece_from = current.to;
                    if piece_end >= seg_end {
                        i += 1;
                    }
                    piece_end
                }
                Read::Full(at) => start + at.max(1),
            }
            .min(frames);
            if end > start {
                if let Some(l) = lanes.as_mut() {
                    l.push_end(&segment, clock.frame, start, end, sink);
                }
                chain(sink, start, end, position);
                sink.clear();
                pushed = false;
                start = end;
            }
        }
        if start < frames {
            // Unreachable: the segments of a running block cover it.
            chain(sink, start, frames, position);
        }
    }

    /// Adds a rendered slice of the output (after the master and the metronome) to the
    /// output's meter.
    pub(crate) fn meter_output(&mut self, left: &[f32], right: &[f32]) {
        self.master_meter.add(left, right);
    }

    /// Voices sounding in the tracks' devices after the last block.
    pub fn voices(&self) -> u32 {
        self.voices
    }

    /// Writes the meter of every track slot below `out.len()` into `out` (zeros for an
    /// empty slot), starts a new period for every meter, and returns the number of slots
    /// written (up to the last occupied one) and the output's meter. No allocation.
    pub(crate) fn take_meters(
        &mut self,
        state: &mut State,
        frames: u32,
        out: &mut [TrackMeter],
    ) -> (usize, TrackMeter) {
        let mut used = 0;
        for (slot, (entry, o)) in state
            .tracks
            .as_mut_slice()
            .iter_mut()
            .zip(out.iter_mut())
            .enumerate()
        {
            match entry.as_deref_mut() {
                Some(track) => {
                    *o = track.graph.meter.take(frames);
                    used = slot + 1;
                }
                None => *o = TrackMeter::default(),
            }
        }
        (used, self.master_meter.take(frames))
    }
}

/// The largest gain reduction of the enabled devices of a chain, in decibels (0 for none; a
/// disabled device is out of the chain and counts for nothing).
fn chain_reduction(devices: &[Option<Box<DeviceEntry>>]) -> f32 {
    devices
        .iter()
        .flatten()
        .filter(|d| d.enabled)
        .fold(0.0f32, |m, d| m.max(d.device.gain_reduction_db()))
}

/// Voices sounding in the enabled devices of `track`.
fn track_voices(track: &Track) -> u32 {
    track
        .devices
        .as_slice()
        .iter()
        .flatten()
        .filter(|d| d.enabled)
        .fold(0u32, |n, d| n.saturating_add(d.device.voices()))
}

impl Drop for Graph {
    /// Unlinks the retiring list one track at a time: dropping its head would recurse once
    /// per track.
    fn drop(&mut self) {
        let mut rest = self.retiring.take();
        while let Some(mut track) = rest {
            rest = track.graph.next.take();
            drop(track);
        }
    }
}

/// The live input of a track in its slot, and the clips its player reads.
struct Input<'a> {
    slot: TrackSlot,
    live: &'a [LiveEvent],
    release: bool,
    clips: Option<ClipsIn<'a>>,
    /// The engine's automation lanes (AM-2): the track plays those of its list.
    lanes: Option<&'a mut Paged<Box<Lane>>>,
}

/// What a track's clip player reads besides the track (CL-2).
struct ClipsIn<'a> {
    clips: &'a mut Paged<Box<Clip>>,
    quantize: Quantize,
    signature: TimeSignature,
}

/// Whether the track has a device to render: an enabled one, or one that still fades out
/// (in its slot or outgoing, #233).
fn chain_sounds(track: &Track) -> bool {
    !track.graph.outgoing.is_empty()
        || track
            .devices
            .as_slice()
            .iter()
            .flatten()
            .any(|d| d.enabled || d.fade > 0.0)
}

/// Renders one track and sums it into `out_left` and `out_right` with its levels; `false`
/// when the track is empty (nothing rendered, its levels untouched). The chain runs once
/// per sub-block ([`crate::events`]): per segment of `clock`, cut again when the sink is
/// full; a retiring track (`input` is `None`) reads no events.
#[allow(clippy::too_many_arguments)]
fn render_track(
    track: &mut Track,
    input: Option<Input<'_>>,
    clock: &BlockClock<'_>,
    ctx: &BlockCtx,
    scratch: &mut Scratch,
    fade: u32,
    out_left: &mut [f32],
    out_right: &mut [f32],
) -> bool {
    // A track without a device renders nothing, but its clip player still runs when a clip
    // plays or a launch waits (GR-1): the grid state must not wait for an instrument.
    let player_runs = input.as_ref().is_some_and(|i| i.clips.is_some())
        && (track.player.engaged() || track.launch.is_some() || track.back);
    if !chain_sounds(track) && !player_runs {
        // Not rendered: the next stopped block it renders applies its lanes again.
        track.graph.stopped_at = None;
        return false;
    }
    let frames = ctx.frames as usize;
    let Scratch {
        left,
        right,
        sink,
        merged,
        unused,
        blend,
    } = scratch;
    let (left, right) = (&mut left[..frames], &mut right[..frames]);
    left.fill(0.0);
    right.fill(0.0);
    sink.clear();
    let mut ended = core::mem::take(&mut track.graph.orphan_len);
    if core::mem::take(&mut track.graph.release) {
        // Before the live notes: they reach the chain as it is now and play. It ends the
        // notes of an edited clip too.
        sink.push_note(Event::AllNotesOff { offset: 0 });
        ended = 0;
    }
    // The notes of an edited clip that nothing else ends, before the live notes (the sink
    // keeps their room).
    for &packed in track.graph.orphans.iter().take(ended) {
        if let Some(voice) = VoiceId::new(packed & 0x00FF_FFFF) {
            sink.push_note(Event::NoteOff {
                offset: 0,
                voice,
                key: (packed >> 24) as u8,
            });
        }
    }
    let reads = input.is_some();
    let mut clips = None;
    let mut lane_table = None;
    let mut slot = TrackSlot(0);
    if let Some(input) = input {
        route_live(&input, sink);
        slot = input.slot;
        clips = input.clips;
        lane_table = input.lanes;
    }
    let Track {
        devices,
        scenes,
        placements,
        launch,
        back,
        graph,
        player,
        ..
    } = track;
    let TrackGraph {
        mixer,
        meter,
        schedule: track_schedule,
        lanes: lane_head,
        lane_expect,
        stopped_at,
        outgoing,
        ..
    } = graph;
    // The track's automation lanes (AM-2, [`crate::automation`]).
    let mut lanes = lane_table
        .map(|table| TrackLanes {
            table,
            head: *lane_head,
        })
        .filter(|l| !l.is_empty());
    // The player plays the track's arrangement, or its launched clip once the grid holds
    // it ([`crate::arrangement`], [`crate::clip_player`]): the schedule is silent
    // meanwhile.
    let mut env = clips.map(|c| Env {
        track: slot,
        clips: c.clips,
        scenes,
        placements: placements.as_slice(),
        launch,
        back,
        quantize: c.quantize,
        signature: c.signature,
    });
    let engaged = env.is_some()
        && (player.engaged()
            || player.arrangement_busy()
            || launch_pending(&env)
            || env.as_ref().is_some_and(|e| !e.placements.is_empty()));
    let mut schedule = if reads {
        track_schedule.as_deref_mut()
    } else {
        None
    };
    if engaged && let Some(s) = schedule.take() {
        s.stop(sink);
    }
    let mut player = if engaged {
        env.as_mut().map(|e| (&mut *player, e))
    } else {
        None
    };
    let devices = devices.as_mut_slice();
    let mut chain = |sink: &Sink, from: usize, to: usize, position: f64| {
        let sub = BlockCtx {
            frames: (to - from) as u32,
            transport: Transport {
                position_ticks: position,
                ..ctx.transport
            },
            ..*ctx
        };
        run_chain(
            devices,
            outgoing,
            sink,
            merged,
            unused,
            blend,
            fade,
            &sub,
            &mut left[from..to],
            &mut right[from..to],
        );
        // The mix, with the strip's timed parameters, then the track's meter: after its
        // fader, as the signal enters the sum.
        mixer.mix_into(
            &left[from..to],
            &right[from..to],
            &mut out_left[from..to],
            &mut out_right[from..to],
            meter,
            timed(sink.params()),
            fade,
        );
    };
    if !clock.running || clock.segments.is_empty() {
        // Stopped, the lanes apply their value at the playhead once, when the clock stops
        // and at every seek (and after an upload or a rebuilt device); a gesture holds
        // until the next play or seek (AM-2).
        *lane_expect = None;
        if clock.running {
            // A running block without a segment (an `f64` edge of the clock) is not a stop.
            *stopped_at = None;
        } else {
            let at = (clock.position.to_bits(), clock.cues);
            let moved = *stopped_at != Some(at);
            *stopped_at = Some(at);
            if let Some(l) = lanes.as_mut() {
                l.push_frozen(clock.position, moved, sink);
            }
        }
        if let Some(s) = schedule.as_deref_mut() {
            s.stop(sink);
        }
        if let Some((p, env)) = player.as_mut() {
            p.stop(env, sink);
        }
        chain(sink, 0, frames, clock.position);
    } else {
        *stopped_at = None;
        let mut start = 0;
        let mut i = 0;
        let mut position = clock.position;
        // The segment whose pieces are being read, the piece being read (its end and the
        // tick it starts at), and whether the lanes' values of the sub-block that starts at
        // `start` are in the sink ([`crate::automation`]).
        let mut entered = usize::MAX;
        let mut piece: Option<Segment> = None;
        let mut piece_from = Tick::ZERO;
        let mut pushed = false;
        while i < clock.segments.len() && start < frames {
            let segment = clock.segments[i];
            let seg_end = segment.offset + segment.frames;
            if entered != i {
                entered = i;
                piece = None;
                piece_from = segment.from;
                // Values pushed under the last segment's ticks (an empty sub-block kept the
                // sink) are pushed again under this one's: the later ones win.
                pushed = false;
                if let Some(l) = lanes.as_mut()
                    && *lane_expect != Some(segment.from)
                {
                    l.jump();
                }
                *lane_expect = Some(segment.to);
            }
            position = segment.position_at(start);
            // A piece ends at the next point of a lane, or a control period on while a lane
            // moves (AM-2); without lanes it is the whole segment.
            let current = match piece {
                Some(p) => p,
                None => {
                    let cut = match lanes.as_mut() {
                        Some(l) if seg_end > start => {
                            l.piece_end(&segment, clock.frame, start, seg_end)
                        }
                        _ => seg_end,
                    };
                    let p = if cut >= seg_end && piece_from == segment.from {
                        segment
                    } else {
                        segment.piece(start, cut, piece_from)
                    };
                    piece = Some(p);
                    p
                }
            };
            if !pushed
                && seg_end > start
                && let Some(l) = lanes.as_mut()
            {
                l.push_start(&segment, clock.frame, start, sink);
                pushed = true;
            }
            let read = match (schedule.as_deref_mut(), player.as_mut()) {
                (_, Some((p, env))) => p.read(env, &current, start, sink),
                (Some(s), None) => s.read(&current, start, sink),
                (None, None) => Read::Done,
            };
            let piece_end = current.offset + current.frames;
            let end = match read {
                Read::Done => {
                    piece = None;
                    piece_from = current.to;
                    if piece_end >= seg_end {
                        i += 1;
                    }
                    piece_end
                }
                Read::Full(at) => start + at.max(1),
            }
            .min(frames);
            // A segment of no frame (an `f64` edge of the clock) leaves its events to the
            // next sub-block.
            if end > start {
                if let Some(l) = lanes.as_mut() {
                    l.push_end(&segment, clock.frame, start, end, sink);
                }
                chain(sink, start, end, position);
                sink.clear();
                pushed = false;
                start = end;
            }
        }
        if start < frames {
            // Unreachable: the segments of a running block cover it.
            chain(sink, start, frames, position);
        }
    }

    true
}

/// Whether the track has a launch order or a `BackToArrangement` its player has not taken yet.
fn launch_pending(env: &Option<Env<'_>>) -> bool {
    env.as_ref().is_some_and(|e| e.launch.is_some() || *e.back)
}

/// The live notes of the track at offset 0, then a release when live events were lost.
/// The sink has room for the whole live block ([`crate::events::LIVE_EVENTS`]).
fn route_live(input: &Input<'_>, sink: &mut Sink) {
    for event in input.live {
        let event = match *event {
            // A live note sounds at the start of the block that takes it, stamped or not
            // (#499): by then its stamp is in the past.
            LiveEvent::NoteOn {
                track,
                pitch,
                velocity,
                ..
            } if track == input.slot => live_voice(pitch).map(|voice| Event::NoteOn {
                offset: 0,
                voice,
                key: pitch,
                velocity: f32::from(velocity.min(127)) / 127.0,
                tune_cents: 0.0,
            }),
            LiveEvent::NoteOff { track, pitch, .. } if track == input.slot => live_voice(pitch)
                .map(|voice| Event::NoteOff {
                    offset: 0,
                    voice,
                    key: pitch,
                }),
            LiveEvent::AllNotesOff { track } if track == input.slot => {
                Some(Event::AllNotesOff { offset: 0 })
            }
            _ => None,
        };
        if let Some(event) = event {
            sink.push_note(event);
        }
    }
    if input.release {
        sink.push_note(Event::AllNotesOff { offset: 0 });
    }
}

/// Where the note stream of a chain comes from at a point of it.
#[derive(Clone, Copy)]
enum Stream {
    /// The track's input.
    Input,
    /// The output of the MIDI effect in this slot.
    Device(usize),
}

/// Runs a chain in slot order (module doc) over `left` and `right`, one sub-block: each
/// device receives its own parameter values of `sink`, merged with the note stream when it
/// takes notes ([`Sink::merge_for`]). A slot where a device fades, or where an outgoing
/// device fades out, blends its devices' outputs with the signal entering it (module doc >
/// Device fades), each device run once.
#[allow(clippy::too_many_arguments)]
fn run_chain(
    devices: &mut [Option<Box<DeviceEntry>>],
    outgoing: &mut Outgoing,
    sink: &Sink,
    merged: &mut EventBuffer,
    unused: &mut EventBuffer,
    blend: &mut Blend,
    fade: u32,
    ctx: &BlockCtx,
    left: &mut [f32],
    right: &mut [f32],
) {
    let frames = left.len().min(right.len());
    let (left, right) = (&mut left[..frames], &mut right[..frames]);
    let step = 1.0 / fade.max(1) as f32;
    let mut stream = Stream::Input;
    let mut instrument = false;
    for i in 0..devices.len() {
        let leaving = !outgoing.is_empty() && outgoing.sounds_at(i);
        let (before, rest) = devices.split_at_mut(i);
        let entry = rest
            .first_mut()
            .and_then(|e| e.as_deref_mut())
            .filter(|e| e.enabled || e.fade > 0.0);
        if entry.is_none() && !leaving {
            continue;
        }
        // Whether the device in the slot runs, and with which notes (`None`: no event, it
        // fades out).
        let mut run: Option<Option<Stream>> = None;
        if let Some(entry) = entry.as_deref() {
            let role = entry.device.descriptor().role;
            run = match role {
                DeviceRole::MidiEffect if entry.events_out.is_none() => None,
                // A second instrument is ignored (the project allows one).
                DeviceRole::Instrument if instrument => None,
                _ if !entry.enabled => Some(None),
                _ => Some(Some(stream)),
            };
        }
        let fading = entry
            .as_deref()
            .is_some_and(|e| run.is_some() && e.fade != e.target());
        if fading || leaving {
            blend.dry_left[..frames].copy_from_slice(left);
            blend.dry_right[..frames].copy_from_slice(right);
        }
        if let Some(entry) = entry {
            let Some(notes_from) = run else {
                // Not run: its fade has nothing to fade.
                entry.fade = entry.target();
                if leaving {
                    fade_out_at(outgoing, i, blend, unused, step, ctx, left, right);
                }
                continue;
            };
            let role = entry.device.descriptor().role;
            // Slots are below `MAX_DEVICES_PER_TRACK`: they fit in a `u16`.
            let slot = DeviceSlot(i.min(usize::from(u16::MAX)) as u16);
            let events: &[Event] = match notes_from {
                // Fading out: no event, its voices and tail ring down.
                None => &[],
                Some(from) => {
                    if role == DeviceRole::Instrument {
                        instrument = true;
                    }
                    let notes: &[Event] = match (role, from) {
                        (DeviceRole::Effect, _) => &[],
                        (_, Stream::Input) => sink.notes(),
                        (_, Stream::Device(j)) => before
                            .get(j)
                            .and_then(|e| e.as_deref())
                            .and_then(|e| e.events_out.as_ref())
                            .map_or(&[], EventBuffer::events),
                    };
                    let values = sink.params_for(slot);
                    if values == 0 {
                        notes
                    } else if notes.len() + values < merged.capacity() {
                        sink.merge_for(slot, notes, merged);
                        merged.events()
                    } else {
                        // More than the merge holds (after a chained MIDI effect with a large
                        // output): the values apply at the sub-block's first frame, and no
                        // note is lost.
                        for (d, e) in sink.params() {
                            if *d == slot
                                && let Event::ParamValue { id, value, .. } = *e
                            {
                                entry.device.set_param(id, value);
                            }
                        }
                        notes
                    }
                }
            };
            let DeviceEntry {
                device, events_out, ..
            } = &mut *entry;
            match role {
                DeviceRole::MidiEffect => {
                    let Some(out) = events_out.as_mut() else {
                        continue;
                    };
                    out.clear();
                    let mut io = AudioIo {
                        left,
                        right,
                        events_out: out,
                    };
                    device.process(ctx, events, &mut io);
                    out.finish();
                    out.retain_notes();
                    stream = Stream::Device(i);
                }
                DeviceRole::Instrument | DeviceRole::Effect => {
                    unused.clear();
                    let mut io = AudioIo {
                        left,
                        right,
                        events_out: unused,
                    };
                    device.process(ctx, events, &mut io);
                }
            }
            if fading {
                let target = entry.target();
                let mut g = entry.fade;
                for (n, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                    g = approach(g, target, step);
                    let (dl, dr) = (blend.dry_left[n], blend.dry_right[n]);
                    *l = dl + g * (*l - dl);
                    *r = dr + g * (*r - dr);
                }
                entry.fade = g;
                if g <= 0.0 && !entry.enabled {
                    // Silent now: nothing of it comes back when it is switched on again.
                    entry.device.reset();
                }
            }
        }
        if leaving {
            fade_out_at(outgoing, i, blend, unused, step, ctx, left, right);
        }
    }
}

/// One step of a linear fade from `g` toward `target`.
#[inline]
fn approach(g: f32, target: f32, step: f32) -> f32 {
    if g < target {
        (g + step).min(target)
    } else {
        (g - step).max(target)
    }
}

/// Adds to `left` and `right` what the outgoing devices of slot `slot` still give (#233):
/// each runs, with no event, over the signal entering the slot (`blend`'s dry buffers), and
/// its difference from that signal is added with its falling fade.
#[allow(clippy::too_many_arguments)]
fn fade_out_at(
    outgoing: &mut Outgoing,
    slot: usize,
    blend: &mut Blend,
    unused: &mut EventBuffer,
    step: f32,
    ctx: &BlockCtx,
    left: &mut [f32],
    right: &mut [f32],
) {
    let frames = left.len().min(right.len());
    let Blend {
        dry_left,
        dry_right,
        old_left,
        old_right,
    } = blend;
    let (dry_left, dry_right) = (&dry_left[..frames], &dry_right[..frames]);
    let (old_left, old_right) = (&mut old_left[..frames], &mut old_right[..frames]);
    for entry in outgoing.iter_mut() {
        if usize::from(entry.slot) == slot && entry.fade > 0.0 {
            old_left.copy_from_slice(dry_left);
            old_right.copy_from_slice(dry_right);
            unused.clear();
            let mut io = AudioIo {
                left: &mut *old_left,
                right: &mut *old_right,
                events_out: &mut *unused,
            };
            entry.device.process(ctx, &[], &mut io);
            let mut g = entry.fade;
            for n in 0..frames {
                g = approach(g, 0.0, step);
                left[n] += g * (old_left[n] - dry_left[n]);
                right[n] += g * (old_right[n] - dry_right[n]);
            }
            entry.fade = g;
        }
    }
}
