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
//!    carries no time), its schedule is read ([`crate::events`], MO-4), and its chain runs
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
//! 3. the master's gain over the sum.
//!
//! One stereo scratch serves every track, since they render one after another: the
//! buffers of the graph are allocated once, by [`crate::engine::channel`], off the audio
//! path.
//!
//! # Mix
//!
//! A track's level on each channel is `audible × gain × balance(pan)`: `audible` is false
//! when the track is muted, or when another track is soloed and this one is not (solo is
//! cumulative: every soloed track plays); the balance keeps unity at the centre and lowers
//! the opposite side, `left = min(1, 1 − pan)`, `right = min(1, 1 + pan)` (pan clamped to
//! [−1, 1]; a negative or non-finite gain is silence). The final pan law, the mixer's
//! automation and its meters are MX-1's and MO-5's: this law is the graph's until then.
//!
//! Every level moves by a linear ramp of [`FADE_SECONDS`] (the smoothing rule of CT-5: a
//! new target starts a new ramp from the current value), the master's too, so that a gain,
//! a pan, a mute or a solo never jumps.
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

use rtrb::{Producer, PushError};

use crate::device::{
    AudioIo, BlockCtx, DeviceRole, DeviceSetup, Event, EventBuffer, Transport, VoiceId,
};
use crate::engine::{DeviceEntry, State, Track};
use crate::events::{NOTE_ROOM, PARAM_CHUNK, Read, Schedule, Sink};
use crate::msg::{DeviceSlot, LiveEvent, MsgError, TrackMeter, TrackSlot};
use crate::transport::Segment;

/// Duration of a track's fade in or out, and of every change of a level, in seconds.
pub const FADE_SECONDS: f64 = 0.005;

/// Slots of the retire ring: retired tracks waiting for the prepare side.
pub const RETIRE_RING_CAPACITY: usize = 4096;

/// Most note events a track's chain receives in a sub-block ([`crate::events`]): the live
/// block and its releases, the release of a jump, and the schedule's room. A MIDI effect's
/// output is sized from it (`MidiOutput::capacity`), whatever the size of the tables the
/// schedule reads: a sub-block is cut when its room is full (MO-4). Chained MIDI effects
/// that multiply their notes beyond it meet CT-5's overflow safety net.
pub const TRACK_INPUT_EVENTS: usize = NOTE_ROOM;

/// Samples below this magnitude leave the graph as 0 (no denormal reaches the output).
const DENORMAL_FLOOR: f32 = 1e-20;

/// Frames a fade lasts at `sample_rate` (at least 1).
pub fn fade_frames(sample_rate: f32) -> u32 {
    let frames = (FADE_SECONDS * f64::from(sample_rate)).round();
    if frames.is_finite() && frames >= 1.0 {
        frames.min(f64::from(u32::MAX)) as u32
    } else {
        1
    }
}

// ---------------------------------------------------------------- ramps

/// A linear ramp under CT-5's smoothing rule: `n` frames after a target is set, the value
/// is the target; in between, `start + (target − start) · i / n`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    start: f32,
    target: f32,
    frames: u32,
    done: u32,
}

impl Ramp {
    /// A still ramp at `value`.
    pub const fn new(value: f32) -> Self {
        Ramp {
            start: value,
            target: value,
            frames: 0,
            done: 0,
        }
    }

    /// The value at the next frame.
    pub fn current(&self) -> f32 {
        self.at(0)
    }

    /// The value `k` frames after the next one.
    fn at(&self, k: u32) -> f32 {
        let i = self.done.saturating_add(k);
        if i >= self.frames {
            self.target
        } else {
            self.start + (self.target - self.start) * (i as f32 / self.frames as f32)
        }
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Whether the ramp has reached its target.
    pub fn is_still(&self) -> bool {
        self.done >= self.frames
    }

    /// Aims at `target` over `frames` frames from the current value; the same target
    /// again changes nothing.
    pub fn set(&mut self, target: f32, frames: u32) {
        if target == self.target {
            return;
        }
        self.start = self.current();
        self.target = target;
        self.frames = frames;
        self.done = 0;
    }

    fn advance(&mut self, frames: u32) {
        self.done = self.done.saturating_add(frames).min(self.frames);
    }
}

/// A meter accumulated between two reports (MO-5): the peak and the sum of squares of each
/// channel. No allocation: four floats.
#[derive(Clone, Copy, Debug, Default)]
pub struct MeterAcc {
    peak: [f32; 2],
    squares: [f32; 2],
}

impl MeterAcc {
    /// Adds one sample of channel `i` (already scaled by the fader). A non-finite sample
    /// stays out of both the peak and the sum of squares.
    #[inline]
    fn push(&mut self, i: usize, sample: f32) {
        let a = sample.abs();
        // Below 1e-18 (-360 dB) a sample is silence for a meter, and squaring it would be a
        // denormal on every sample of a decaying tail.
        if a.is_finite() && a > 1e-18 {
            self.peak[i] = self.peak[i].max(a);
            self.squares[i] += a * a;
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
        };
        *self = MeterAcc::default();
        m
    }
}

/// What the graph keeps in a track: its two levels and, while it retires, the next track of
/// the retiring list.
#[derive(Debug)]
pub struct TrackGraph {
    left: Ramp,
    right: Ramp,
    /// The chain's note stream changed (a MIDI effect or an instrument inserted, removed,
    /// enabled or disabled): the next rendered block releases every note of the track
    /// first.
    release: bool,
    /// The tick-dated events the graph plays on the track ([`crate::events`], MO-4). No
    /// message fills it yet: the clip player (CL-2), the arrangement (AR-2) and the
    /// automation (AM-2) compile their tables into it; tests set it off the audio path
    /// ([`TrackGraph::set_schedule`]).
    schedule: Option<Box<Schedule>>,
    /// The track's meter since the last report, after its fader (MO-5).
    meter: MeterAcc,
    next: Option<Box<Track>>,
}

impl TrackGraph {
    /// A new track's: both levels at 0, so that it fades in.
    pub const fn new() -> Self {
        TrackGraph {
            left: Ramp::new(0.0),
            right: Ramp::new(0.0),
            release: false,
            schedule: None,
            meter: MeterAcc {
                peak: [0.0; 2],
                squares: [0.0; 2],
            },
            next: None,
        }
    }

    /// Puts `schedule` on the track and returns the one it replaces (to be freed off the
    /// audio path). When the old one holds notes, the next rendered block releases every
    /// note of the track first, as for a changed chain: nothing would end them.
    pub fn set_schedule(&mut self, schedule: Option<Box<Schedule>>) -> Option<Box<Schedule>> {
        let old = core::mem::replace(&mut self.schedule, schedule);
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
    pub(crate) fn release_notes(&mut self) {
        self.release = true;
    }

    /// Whether a release waits for the next rendered block.
    pub fn releasing(&self) -> bool {
        self.release
    }

    /// The levels (left, right) at the next frame.
    pub fn levels(&self) -> (f32, f32) {
        (self.left.current(), self.right.current())
    }

    /// Both levels at 0 and still: the track is silent.
    fn is_silent(&self) -> bool {
        self.left.is_still()
            && self.right.is_still()
            && self.left.target == 0.0
            && self.right.target == 0.0
    }
}

impl Default for TrackGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// The levels of a track: `audible × gain × balance(pan)` (module doc).
pub fn mix_levels(gain: f32, pan: f32, audible: bool) -> (f32, f32) {
    if !audible || !gain.is_finite() || gain <= 0.0 {
        return (0.0, 0.0);
    }
    let pan = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    (gain * (1.0 - pan).min(1.0), gain * (1.0 + pan).min(1.0))
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
}

/// The clock as the graph reads it for one block: the segments TR-1 cut it in (none when
/// stopped), and the position of its first frame.
#[derive(Clone, Copy, Debug)]
pub struct BlockClock<'a> {
    pub running: bool,
    pub segments: &'a [Segment],
    /// The playhead at the block's first frame, in fractional ticks.
    pub position: f64,
}

/// The render of the graph (module doc), owned by the audio side.
pub struct Graph {
    sample_rate: f32,
    fade: u32,
    max_frames: usize,
    scratch: Scratch,
    master: Ramp,
    /// The output's meter since the last report, after the master and the metronome (MO-5).
    master_meter: MeterAcc,
    /// Voices sounding in the tracks' devices after the last block (MO-5).
    voices: u32,
    retiring: Option<Box<Track>>,
    retiring_len: usize,
    retired: Producer<Box<Track>>,
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
        retired: Producer<Box<Track>>,
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
                sink: Sink::try_new()?,
                merged: EventBuffer::try_with_capacity(NOTE_ROOM + PARAM_CHUNK + 1)?,
                unused: EventBuffer::try_with_capacity(1)?,
            },
            master: Ramp::new(1.0),
            master_meter: MeterAcc::default(),
            voices: 0,
            retiring: None,
            retiring_len: 0,
            retired,
        })
    }

    /// The largest block the graph renders at once (the setup's).
    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// Frames of a fade.
    pub fn fade(&self) -> u32 {
        self.fade
    }

    /// Tracks fading out or waiting for room in the retire ring.
    pub fn retiring(&self) -> usize {
        self.retiring_len
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
        for (slot, entry) in state.tracks.as_mut_slice().iter_mut().enumerate() {
            let Some(track) = entry.as_deref_mut() else {
                continue;
            };
            let audible = !track.mute && (!any_solo || track.solo);
            let (l, r) = mix_levels(track.gain, track.pan, audible);
            track.graph.left.set(l, fade);
            track.graph.right.set(r, fade);
            let input = Input {
                slot: TrackSlot(slot as u32),
                live,
                release,
            };
            render_track(
                track,
                Some(input),
                clock,
                &ctx,
                &mut self.scratch,
                left,
                right,
            );
            voices = voices.saturating_add(track_voices(track));
        }
        self.voices = voices;

        // The retiring tracks: rendered toward silence, then handed to the retire ring.
        let mut rest = self.retiring.take();
        let mut kept: Option<Box<Track>> = None;
        while let Some(mut track) = rest {
            rest = track.graph.next.take();
            track.graph.left.set(0.0, fade);
            track.graph.right.set(0.0, fade);
            let done = track.graph.is_silent()
                || !render_track(
                    &mut track,
                    None,
                    clock,
                    &ctx,
                    &mut self.scratch,
                    left,
                    right,
                )
                || track.graph.is_silent();
            if done {
                match self.retired.push(track) {
                    Ok(()) => {
                        self.retiring_len -= 1;
                        continue;
                    }
                    Err(PushError::Full(back)) => track = back,
                }
            }
            track.graph.next = kept;
            kept = Some(track);
        }
        self.retiring = kept;

        // The master.
        let target = if state.master.mute {
            0.0
        } else if state.master.gain.is_finite() {
            state.master.gain.max(0.0)
        } else {
            0.0
        };
        self.master.set(target, fade);
        let master = self.master;
        if master.is_still() {
            let g = master.target;
            for s in left.iter_mut().chain(right.iter_mut()) {
                *s = flush(*s * g);
            }
        } else {
            for (k, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                let g = master.at(k as u32);
                *l = flush(*l * g);
                *r = flush(*r * g);
            }
        }
        self.master.advance(frames as u32);
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

/// A sample as it leaves the graph: a denormal, a NaN or an infinity (a device breaking
/// its contract, a sum that overflowed) is 0.
fn flush(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        x
    }
}

/// The live input of a track in its slot.
struct Input<'a> {
    slot: TrackSlot,
    live: &'a [LiveEvent],
    release: bool,
}

/// Whether the track has a device to render.
fn has_enabled_device(track: &Track) -> bool {
    track.devices.as_slice().iter().flatten().any(|d| d.enabled)
}

/// Renders one track and sums it into `out_left` and `out_right` with its levels; `false`
/// when the track is empty (nothing rendered, its levels untouched). The chain runs once
/// per sub-block ([`crate::events`]): per segment of `clock`, cut again when the sink is
/// full; a retiring track (`input` is `None`) reads no events.
fn render_track(
    track: &mut Track,
    input: Option<Input<'_>>,
    clock: &BlockClock<'_>,
    ctx: &BlockCtx,
    scratch: &mut Scratch,
    out_left: &mut [f32],
    out_right: &mut [f32],
) -> bool {
    if !has_enabled_device(track) {
        return false;
    }
    let frames = ctx.frames as usize;
    let Scratch {
        left,
        right,
        sink,
        merged,
        unused,
    } = scratch;
    let (left, right) = (&mut left[..frames], &mut right[..frames]);
    left.fill(0.0);
    right.fill(0.0);
    sink.clear();
    if core::mem::take(&mut track.graph.release) {
        // Before the live notes: they reach the chain as it is now and play.
        sink.push_note(Event::AllNotesOff { offset: 0 });
    }
    let reads = input.is_some();
    if let Some(input) = input {
        route_live(&input, sink);
    }
    let mut schedule = if reads {
        track.graph.schedule.as_deref_mut()
    } else {
        None
    };
    let devices = track.devices.as_mut_slice();
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
            sink,
            merged,
            unused,
            &sub,
            &mut left[from..to],
            &mut right[from..to],
        );
    };
    if !clock.running || clock.segments.is_empty() {
        if let Some(s) = schedule.as_deref_mut() {
            s.stop(sink);
        }
        chain(sink, 0, frames, clock.position);
    } else {
        let mut start = 0;
        let mut i = 0;
        let mut position = clock.position;
        while i < clock.segments.len() && start < frames {
            let segment = clock.segments[i];
            position = segment.position_at(start);
            let read = match schedule.as_deref_mut() {
                Some(s) => s.read(&segment, start, sink),
                None => Read::Done,
            };
            let end = match read {
                Read::Done => {
                    i += 1;
                    segment.offset + segment.frames
                }
                Read::Full(at) => start + at.max(1),
            }
            .min(frames);
            // A segment of no frame (an `f64` edge of the clock) leaves its events to the
            // next sub-block.
            if end > start {
                chain(sink, start, end, position);
                sink.clear();
                start = end;
            }
        }
        if start < frames {
            // Unreachable: the segments of a running block cover it.
            chain(sink, start, frames, position);
        }
    }

    // The mix.
    let (gl, gr) = (&mut track.graph.left, &mut track.graph.right);
    // The track's meter: the signal after its fader, as it enters the sum.
    let meter = &mut track.graph.meter;
    if gl.is_still() && gr.is_still() {
        let (a, b) = (gl.target, gr.target);
        for s in left.iter() {
            meter.push(0, s * a);
        }
        for s in right.iter() {
            meter.push(1, s * b);
        }
    } else {
        for k in 0..frames {
            meter.push(0, left[k] * gl.at(k as u32));
            meter.push(1, right[k] * gr.at(k as u32));
        }
    }
    if gl.is_still() && gr.is_still() {
        let (a, b) = (gl.target, gr.target);
        if a != 0.0 {
            for (o, s) in out_left.iter_mut().zip(left.iter()) {
                *o += s * a;
            }
        }
        if b != 0.0 {
            for (o, s) in out_right.iter_mut().zip(right.iter()) {
                *o += s * b;
            }
        }
    } else {
        for k in 0..frames {
            out_left[k] += left[k] * gl.at(k as u32);
            out_right[k] += right[k] * gr.at(k as u32);
        }
    }
    gl.advance(frames as u32);
    gr.advance(frames as u32);
    true
}

/// The live notes of the track at offset 0, then a release when live events were lost.
/// The sink has room for the whole live block ([`crate::events::LIVE_EVENTS`]).
fn route_live(input: &Input<'_>, sink: &mut Sink) {
    for event in input.live {
        let event = match *event {
            LiveEvent::NoteOn {
                track,
                pitch,
                velocity,
            } if track == input.slot => live_voice(pitch).map(|voice| Event::NoteOn {
                offset: 0,
                voice,
                key: pitch,
                velocity: f32::from(velocity.min(127)) / 127.0,
                tune_cents: 0.0,
            }),
            LiveEvent::NoteOff { track, pitch } if track == input.slot => {
                live_voice(pitch).map(|voice| Event::NoteOff {
                    offset: 0,
                    voice,
                    key: pitch,
                })
            }
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
/// takes notes ([`Sink::merge_for`]).
fn run_chain(
    devices: &mut [Option<Box<DeviceEntry>>],
    sink: &Sink,
    merged: &mut EventBuffer,
    unused: &mut EventBuffer,
    ctx: &BlockCtx,
    left: &mut [f32],
    right: &mut [f32],
) {
    let mut stream = Stream::Input;
    let mut instrument = false;
    for i in 0..devices.len() {
        let (before, rest) = devices.split_at_mut(i);
        let Some(Some(entry)) = rest.first_mut() else {
            continue;
        };
        if !entry.enabled {
            continue;
        }
        let role = entry.device.descriptor().role;
        match role {
            DeviceRole::MidiEffect if entry.events_out.is_none() => continue,
            DeviceRole::Instrument if instrument => continue,
            _ => {}
        }
        let notes: &[Event] = match (role, stream) {
            (DeviceRole::Effect, _) => &[],
            (_, Stream::Input) => sink.notes(),
            (_, Stream::Device(j)) => before
                .get(j)
                .and_then(|e| e.as_deref())
                .and_then(|e| e.events_out.as_ref())
                .map_or(&[], EventBuffer::events),
        };
        // Slots are below `MAX_DEVICES_PER_TRACK`: they fit in a `u16`.
        let slot = DeviceSlot(i.min(usize::from(u16::MAX)) as u16);
        let values = sink.params_for(slot);
        let events: &[Event] = if values == 0 {
            notes
        } else if notes.len() + values < merged.capacity() {
            sink.merge_for(slot, notes, merged);
            merged.events()
        } else {
            // More than the merge holds (after a chained MIDI effect with a large output):
            // the values apply at the sub-block's first frame, and no note is lost.
            for (d, e) in sink.params() {
                if *d == slot
                    && let Event::ParamValue { id, value, .. } = *e
                {
                    entry.device.set_param(id, value);
                }
            }
            notes
        };
        let DeviceEntry {
            device, events_out, ..
        } = &mut **entry;
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
            DeviceRole::Instrument => {
                instrument = true;
                unused.clear();
                let mut io = AudioIo {
                    left,
                    right,
                    events_out: unused,
                };
                device.process(ctx, events, &mut io);
            }
            DeviceRole::Effect => {
                unused.clear();
                let mut io = AudioIo {
                    left,
                    right,
                    events_out: unused,
                };
                device.process(ctx, events, &mut io);
            }
        }
    }
}
