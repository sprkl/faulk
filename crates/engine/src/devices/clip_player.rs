//! The audio clip player (`faulk.clip_player`, AU-4, `docs/specs/02-decoupage-mvp.md` AU-4):
//! the instrument stage of an audio track. It plays the audio clips the track's players
//! choose (the launch grid's [`crate::clip_player::ClipPlayer`] and the arrangement's frame
//! placements), reading their sources through AU-3's [`Voice`]s
//! ([`crate::audio_source`]), resampled to the engine's rate, with short fades.
//!
//! # A stage, not a registry device
//!
//! The project keeps a device `faulk.clip_player` in the instrument slot of every audio
//! track, and the engine never receives it: its slot is a hole in the chain. The player is
//! held by the track ([`crate::engine::Track::audio`]) and the graph runs it in place of an
//! instrument for an audio track ([`crate::graph`]); the effects after the hole process its
//! output. It is not a [`crate::device::Device`]: a device receives notes and parameters,
//! and this one needs the engine's source table, the track's stream ports and the disk
//! request queue, which the device contract does not give. It has no parameter (the clip's
//! gain is the clip's).
//!
//! # What plays
//!
//! The decisions are the existing players': the grid's `ClipPlayer` starts an audio clip at
//! its launch boundary and hands the track over to and from the arrangement, recording each
//! change of a read with its frame ([`crate::clip_player::Handover`]). While the arrangement
//! holds the track, the audio placement at a tick is the last frame placement started at or
//! before it, until its effective end: the least of `start + Tempo::frames_to_ticks(length,
//! source rate)` (the rule `clip.import_audio` places with) and the next placement's start
//! (the MIDI rule of [`crate::arrangement`]). A grid clip played once ends with its last
//! frame, not at a tick (audio is not warped): the player tells the grid's player
//! ([`AudioPlayer::take_ended`], `ClipPlayer::audio_ended`).
//!
//! # Time
//!
//! What plays from tick `A` (a launch boundary, a placement's start) plays its first frame at
//! the sample `floor(sample_of(A))`, the note rule of TR-1, and at block frame `f` the frame
//! `offset + (f − floor(sample_of(A))) × source rate / engine rate` of its clip, wrapped
//! through the clip's loop (`ls + (p − le) mod (le − ls)`); in the arrangement a clip without
//! a loop repeats whole (#473's rule). A looped grid clip before its anchor (a transport loop
//! that wrapped back past it) keeps its phase, as a MIDI clip does; a clip played once is
//! silent there. Once started, a stream runs on its own: the position is computed again only
//! at a jump of the transport (a segment that does not start where the last one ended) or a
//! change of what plays. A tempo change does not move a playing clip; a placement's end
//! follows the tempo (the model's rule).
//!
//! # Streams, resampling and fades
//!
//! A track has two [`Stream`]s, one per stream port: the one that plays and one that fades
//! out. A change of what plays starts a new stream on the free port with a fade in while the
//! old one fades out: a crossfade. A change while the old one still fades waits for it (at
//! most one fade) and then starts from where its timeline is then.
//!
//! Each stream resamples its source to the engine's rate: a 4-point cubic Hermite
//! interpolator over a window `x[-1], x0, x1, x2` (the voice runs two frames ahead of `x0`)
//! with a fixed-point phase (32-bit fraction; the step is `(source rate << 32) / engine
//! rate`, integer, so both builds agree to the bit). At equal rates the phase stays 0 and
//! each output sample is the source sample itself. The rate is read every block, so a new
//! engine rate applies at once. Mono plays on both sides, stereo on its two, a source of more
//! channels plays its first two. AU-3 already turns non-finite and subnormal source samples
//! into 0; a subnormal output is flushed to 0.
//!
//! Fades last [`FADE_SECONDS`]:
//!
//! - a fade in at every start (a launch, a placement, a jump) and a fade out from the sample
//!   of every stop or change;
//! - a **gate** on the frames the voice delivers: a frame it could not deliver (its ring not
//!   there yet after a jump into a spot that is not resident, an underrun) holds the last
//!   delivered value while the gate closes, and the gate opens again from the first frame
//!   that comes, so neither the start of a late stream nor an underrun is a step;
//! - at the clip's end, a fade down to 0 over its last frames;
//! - at each loop wrap, the step between the frame before the loop end and the loop start is
//!   carried as an offset that decays to 0 over the fade (a declick: no gap in the level);
//! - a change of the clip's gain ramps over the same length.
//!
//! Nothing here allocates, locks, blocks or panics: the scratch is the graph's
//! ([`Scratch`]), every count saturates, and a missing clip, source or port silences the
//! stream.

use crate::audio_source::{DiskStats, MAX_CHANNELS, Path, Port, PortId, RequestTx, Voice};
use crate::clip_player::{Handover, Owner};
use crate::engine::{AudioClip, Clip, ClipContent, Paged, Source};
use crate::msg::{
    ClipSlot, MsgError, Placement, PlacementSlot, Slots, SourceSlot, Span, TrackSlot,
};
use crate::time::{SampleRate, Tempo, Tick};
use crate::transport::Segment;

/// Length of every fade of the player, in seconds (96 frames at 48 kHz).
pub const FADE_SECONDS: f64 = 0.002;
/// Source frames pulled from a voice at once.
pub const CHUNK: usize = 256;
/// The largest ratio of a source's rate to the engine's: beyond, the source plays slower
/// (it bounds the frames a block pulls).
pub const MAX_RATIO: u64 = 64;

/// Frames of a fade at `rate`, at least 1.
pub fn fade_frames(rate: u32) -> u32 {
    ((f64::from(rate) * FADE_SECONDS).round() as u32).max(1)
}

/// The phase step of a source at `source` Hz in an engine at `engine` Hz: source frames per
/// output frame, with a 32-bit fraction, in `[2^-32, MAX_RATIO]`.
pub fn step_of(source: u32, engine: u32) -> u64 {
    ((u64::from(source.max(1)) << 32) / u64::from(engine.max(1))).clamp(1, MAX_RATIO << 32)
}

/// The scratch of the players, shared by every track (they render one after another).
pub struct Scratch {
    /// A chunk of a voice's frames, every channel interleaved.
    raw: Box<[f32]>,
    /// The same chunk on two channels, and whether the voice delivered each frame.
    stereo: Box<[[f32; 2]]>,
    real: Box<[bool]>,
}

impl Scratch {
    /// Off the audio path, fallibly.
    pub fn try_new() -> Result<Scratch, MsgError> {
        fn boxed<T: Clone>(n: usize, v: T) -> Result<Box<[T]>, MsgError> {
            let mut b = Vec::new();
            b.try_reserve_exact(n).map_err(|_| MsgError::OutOfMemory {
                bytes: n.saturating_mul(size_of::<T>()),
            })?;
            b.resize(n, v);
            Ok(b.into_boxed_slice())
        }
        Ok(Scratch {
            raw: boxed(CHUNK * MAX_CHANNELS, 0.0)?,
            stereo: boxed(CHUNK, [0.0; 2])?,
            real: boxed(CHUNK, false)?,
        })
    }
}

impl core::fmt::Debug for Scratch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Scratch").finish_non_exhaustive()
    }
}

/// What the players read besides their track: the engine's sources, the voices' request
/// queue, the disk's counters, the engine's rate (whole hertz) and the scratch.
pub struct Io<'a> {
    pub sources: &'a Slots<Box<Source>>,
    pub tx: &'a mut RequestTx,
    pub stats: &'a DiskStats,
    pub rate: u32,
    pub scratch: &'a mut Scratch,
}

/// The track's side: its slot and instance (the voices' port ids), its ports, its placements
/// (sorted, [`crate::arrangement::prepare_table`]), the clips (`None` when the grid's player
/// did not run: nothing new plays, and a stream keeps its gain) and the tempo.
pub struct TrackIo<'a> {
    pub slot: TrackSlot,
    pub instance: u32,
    pub ports: &'a mut [Port],
    pub placements: &'a [Placement],
    pub clips: Option<&'a Paged<Box<Clip>>>,
    pub tempo: Tempo,
}

impl TrackIo<'_> {
    fn clip(&self, slot: ClipSlot) -> Option<AudioClip> {
        match &self.clips?.get(slot.0)?.content {
            ClipContent::Audio(a) => Some(**a),
            ClipContent::Notes(_) => None,
        }
    }
}

/// What a stream plays: a grid clip from its anchor, or a placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ident {
    Grid {
        clip: ClipSlot,
        anchor: Tick,
    },
    Place {
        placement: PlacementSlot,
        clip: ClipSlot,
        start: Tick,
    },
}

impl Ident {
    fn clip(self) -> ClipSlot {
        match self {
            Ident::Grid { clip, .. } | Ident::Place { clip, .. } => clip,
        }
    }
}

/// What should play at a point, and the tick where that may change.
#[derive(Clone, Copy, Debug)]
struct Want {
    ident: Option<Ident>,
    until: Option<Tick>,
}

const NOTHING: Want = Want {
    ident: None,
    until: None,
};

/// One stream: a voice on a port, its resampler and its fades.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stream {
    voice: Voice,
    ident: Option<Ident>,
    /// The clip as it was when the stream started (an edit of its range restarts it).
    clip: Option<AudioClip>,
    source: SourceSlot,
    path: Path,
    step: u64,
    frac: u32,
    /// `x[-1], x0, x1, x2` of the left and right outputs.
    win: [[f32; 4]; 2],
    /// Whether the voice delivered `x[-1], x0, x1, x2`.
    real: [bool; 4],
    /// The source frame of `x0` along the path.
    x0: u64,
    /// The source frame of the last frame pulled (`x2`); `None` before the first.
    pulled: Option<u64>,
    /// The last frame pulled (after its declick), and the last one the voice delivered.
    last: [f32; 2],
    held: [f32; 2],
    /// The loop wrap's offset and the frames it still decays over.
    declick: [f32; 2],
    declick_left: u32,
    gain: f32,
    fade: f32,
    gate: f32,
    out: bool,
    live: bool,
}

/// The audio clip player of a track (module doc).
#[derive(Debug, Default)]
pub struct AudioPlayer {
    streams: [Stream; 2],
    /// The stream that plays (the other one fades out or is idle).
    current: usize,
    /// Where the last rendered segment ended: the next one continues it when it starts there.
    last_to: Option<Tick>,
    /// The owner of the sub-block being rendered.
    owner: Owner,
    /// A change that waits for a free port (module doc).
    pending: Option<Ident>,
    /// A grid clip played once that reached its last frame, for the grid's player.
    ended: Option<(ClipSlot, Tick)>,
}

impl AudioPlayer {
    /// Whether a stream sounds (playing or fading out) or one waits to start.
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.streams.iter().any(|s| s.live)
    }

    /// The source frame the playing stream is at, if one plays (tests).
    pub fn position(&self) -> Option<u64> {
        let s = &self.streams[self.current];
        (s.live && !s.out).then_some(s.x0)
    }

    /// The grid clip played once whose last frame was played since the last call, with its
    /// anchor.
    pub fn take_ended(&mut self) -> Option<(ClipSlot, Tick)> {
        self.ended.take()
    }

    /// Stops every stream at once (`AllSoundOff`: the output is already silent). What plays
    /// starts again at the next block, with a fade in.
    pub fn reset(&mut self) {
        for s in &mut self.streams {
            s.voice.stop();
            s.live = false;
        }
        self.pending = None;
        self.last_to = None;
    }

    /// The transport is stopped for `[start, end)`: every stream fades out.
    pub fn render_stopped(
        &mut self,
        io: &mut Io<'_>,
        track: &mut TrackIo<'_>,
        start: usize,
        end: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        self.last_to = None;
        self.pending = None;
        for s in &mut self.streams {
            s.out |= s.live;
        }
        self.run(io, track, start, end, left, right);
    }

    /// A retiring track (`Reset`, `TrackRemove`): its streams go on, with no change, under the
    /// mixer's fade out.
    pub fn render_tail(
        &mut self,
        io: &mut Io<'_>,
        track: &mut TrackIo<'_>,
        start: usize,
        end: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        self.pending = None;
        self.run(io, track, start, end, left, right);
    }

    /// Renders `[start, end)` of the block, a sub-block of `segment`, adding into `left` and
    /// `right` (module doc). `entry` and `handovers` are what the grid's player said of its
    /// read of this sub-block: the owner at its start and its changes, at frames of the block.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        io: &mut Io<'_>,
        track: &mut TrackIo<'_>,
        segment: &Segment,
        start: usize,
        end: usize,
        entry: Owner,
        handovers: &[Handover],
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let seg_end = segment.offset + segment.frames;
        let end = end.min(seg_end);
        if start >= end {
            return;
        }
        let mut jumped = start == segment.offset && self.last_to != Some(segment.from);
        self.owner = entry;
        let mut f = start;
        let mut h = 0usize;
        // Each step moves `f` forward by at least one frame.
        while f < end {
            while let Some(ho) = handovers.get(h)
                && ho.frame as usize <= f
            {
                self.owner = ho.owner;
                h += 1;
            }
            let tick = tick_at(segment, f);
            let want = self.want(io, track, tick);
            self.switch(
                io,
                track,
                segment,
                f,
                want.ident,
                core::mem::take(&mut jumped),
            );
            let mut next = end;
            if let Some(ho) = handovers.get(h) {
                next = next.min((ho.frame as usize).max(f + 1));
            }
            if let Some(u) = want.until {
                next = next.min(frame_of(segment, u).max(f + 1));
            }
            if self.pending.is_some() {
                // The oldest stream ends within a fade: the pending change starts then.
                let tail = self.streams.iter().map(|s| s.fade).fold(1.0f32, f32::min);
                let left_frames = (tail * fade_frames(io.rate) as f32).ceil() as usize;
                next = next.min(f + left_frames.max(1));
            }
            self.run(io, track, f, next, left, right);
            f = next;
            if let Some(ident) = self.pending
                && f < end
                && self.free_port().is_some()
            {
                self.pending = None;
                self.begin(io, track, segment, f, ident);
            }
        }
        if end == seg_end {
            self.last_to = Some(segment.to);
        }
    }

    /// What should play at `tick` for the current owner.
    fn want(&self, io: &Io<'_>, track: &TrackIo<'_>, tick: Tick) -> Want {
        if self.owner.taken {
            let Some(p) = self.owner.playing else {
                return NOTHING;
            };
            let Some(clip) = track.clip(p.clip) else {
                return NOTHING;
            };
            if tick < p.anchor && clip.looping.is_none() {
                // Played once: silent until its anchor.
                return Want {
                    ident: None,
                    until: Some(p.anchor),
                };
            }
            return Want {
                ident: Some(Ident::Grid {
                    clip: p.clip,
                    anchor: p.anchor,
                }),
                until: None,
            };
        }
        if track.clips.is_none() {
            return NOTHING;
        }
        match locate(track, io.sources, tick) {
            Located::In { placement, end } => Want {
                ident: Some(Ident::Place {
                    placement: placement.placement,
                    clip: placement.clip,
                    start: placement.start,
                }),
                until: Some(end),
            },
            Located::Gap { until } => Want { ident: None, until },
        }
    }

    /// Makes `ident` what plays from block frame `f` (module doc): nothing when it already
    /// plays and the transport did not jump; else the stream that plays fades out and a new
    /// one starts on the other port (or waits for it).
    fn switch(
        &mut self,
        io: &mut Io<'_>,
        track: &mut TrackIo<'_>,
        segment: &Segment,
        f: usize,
        ident: Option<Ident>,
        jumped: bool,
    ) {
        let cur = &self.streams[self.current];
        let playing = cur.live && !cur.out;
        let same = playing
            && cur.ident == ident
            && ident.is_some_and(|i| same_range(cur.clip.as_ref(), track.clip(i.clip()).as_ref()));
        if same && !jumped {
            return;
        }
        if !playing && ident.is_none() {
            self.pending = None;
            return;
        }
        if self.pending.is_some() && self.pending == ident && !jumped {
            return;
        }
        self.streams[self.current].out |= playing;
        self.pending = None;
        let Some(ident) = ident else {
            return;
        };
        if self.free_port().is_none() {
            // Both ports sound: it starts when the oldest has faded out.
            self.pending = Some(ident);
            return;
        }
        self.begin(io, track, segment, f, ident);
    }

    /// A port no stream sounds on: the other one first.
    fn free_port(&self) -> Option<usize> {
        let other = 1 - self.current;
        if !self.streams[other].live {
            Some(other)
        } else if !self.streams[self.current].live {
            Some(self.current)
        } else {
            None
        }
    }

    /// Starts `ident` on the free port at frame `f`.
    fn begin(
        &mut self,
        io: &mut Io<'_>,
        track: &mut TrackIo<'_>,
        segment: &Segment,
        f: usize,
        ident: Ident,
    ) {
        let Some(port) = self.free_port() else {
            self.pending = Some(ident);
            return;
        };
        if self.streams[self.current].live {
            self.streams[self.current].out = true;
        }
        if start_stream(&mut self.streams[port], port, io, track, segment, f, ident) {
            self.current = port;
        } else if let Ident::Grid { clip, anchor } = ident {
            // Nothing of it can play (past its end, its source gone): the grid lets it go,
            // rather than report a clip that plays silence for ever.
            self.ended = Some((clip, anchor));
        }
    }

    /// Renders the live streams over `[a, b)`.
    fn run(
        &mut self,
        io: &mut Io<'_>,
        track: &mut TrackIo<'_>,
        a: usize,
        b: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        for (i, s) in self.streams.iter_mut().enumerate() {
            if s.live
                && s.render(i, io, track, a, b, left, right)
                && let Some(Ident::Grid { clip, anchor }) = s.ident
            {
                self.ended = Some((clip, anchor));
            }
        }
    }
}

/// Whether two states of a clip play the same frames (only the gain differs).
fn same_range(a: Option<&AudioClip>, b: Option<&AudioClip>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.source == b.source
                && a.start == b.start
                && a.length == b.length
                && a.looping == b.looping
        }
        _ => false,
    }
}

/// The sample of the block where `tick` plays: the floor of its fractional sample, which may
/// lie outside the block.
fn sample_floor(segment: &Segment, tick: Tick) -> i64 {
    // `as` saturates and sends NaN to 0.
    segment.sample_of(tick).floor() as i64
}

/// The block frame of `tick`, within the segment.
fn frame_of(segment: &Segment, tick: Tick) -> usize {
    let last = (segment.offset + segment.frames) as i64;
    sample_floor(segment, tick).clamp(segment.offset as i64, last) as usize
}

/// The last tick that plays at or before block frame `f`: the largest `t` whose sample
/// (its floor) is at most `f`.
fn tick_at(segment: &Segment, f: usize) -> Tick {
    let guess = segment.position_at_frame(f as f64 + 1.0).floor();
    // `as` saturates and sends NaN to 0.
    let mut t = Tick(guess as i64);
    // The float guess is off by a tick or two at most.
    for _ in 0..4 {
        if sample_floor(segment, t) > f as i64 {
            t = Tick(t.0.saturating_sub(1));
        } else if sample_floor(segment, Tick(t.0.saturating_add(1))) <= f as i64 {
            t = Tick(t.0.saturating_add(1));
        } else {
            break;
        }
    }
    t
}

/// What the arrangement plays at a tick.
enum Located {
    In { placement: Placement, end: Tick },
    Gap { until: Option<Tick> },
}

fn source_of(sources: &Slots<Box<Source>>, slot: SourceSlot) -> Option<&Source> {
    sources.as_slice().get(slot.0 as usize)?.as_deref()
}

/// The effective end of the frame placement `i` (module doc); its start when it is not one,
/// or when its clip or its source is unknown.
fn audio_end(track: &TrackIo<'_>, sources: &Slots<Box<Source>>, i: usize) -> Tick {
    let table = track.placements;
    let Some(p) = table.get(i) else {
        return Tick(i64::MIN);
    };
    let Span::Frames { length, .. } = p.span else {
        return p.start;
    };
    let Some(rate) = track
        .clip(p.clip)
        .and_then(|c| source_of(sources, c.source))
        .and_then(|s| SampleRate::new(s.sample_rate))
    else {
        return p.start;
    };
    let own = track.tempo.audio_end_tick(p.start, length, rate);
    match table.get(i + 1) {
        Some(next) => own.min(next.start),
        None => own,
    }
}

/// The frame placement that plays at `t`, by a binary search (a table out of order gives
/// some answer, never a panic; a gap is at least one tick, so a walk always moves on).
fn locate(track: &TrackIo<'_>, sources: &Slots<Box<Source>>, t: Tick) -> Located {
    let table = track.placements;
    let started = table.partition_point(|p| p.start <= t);
    if let Some(i) = started.checked_sub(1)
        && let Some(&placement) = table.get(i)
        && matches!(placement.span, Span::Frames { .. })
    {
        let end = audio_end(track, sources, i);
        if t < end {
            return Located::In { placement, end };
        }
    }
    let until = table
        .get(started)
        .map(|p| p.start.max(Tick(t.0.saturating_add(1))));
    Located::Gap { until }
}

/// The clip-relative position of `ident` at block frame `f` (32-bit fraction), wrapped
/// through its loop, and that loop (clip-relative); `None` when nothing of the clip plays
/// there (past the end of a clip played once, before the anchor of one).
fn position(
    segment: &Segment,
    f: usize,
    ident: Ident,
    clip: &AudioClip,
    placements: &[Placement],
    step: u64,
) -> Option<(u128, Option<(u64, u64)>)> {
    let length = clip.length.0;
    let own = clip.looping.map(|(s, e)| (s.0, e.0));
    let (origin, offset, looping) = match ident {
        Ident::Grid { anchor, .. } => (anchor, 0u64, own),
        Ident::Place { placement, .. } => {
            let p = placements.iter().find(|p| p.placement == placement)?;
            let Span::Frames { offset, .. } = p.span else {
                return None;
            };
            // A clip without a loop repeats whole in the arrangement (#473).
            (
                p.start,
                offset.0,
                own.or((length > 0).then_some((0, length))),
            )
        }
    };
    let looping = looping.filter(|&(s, e)| s < e && e <= length);
    let elapsed = f as i64 - sample_floor(segment, origin);
    let mut pos = (i128::from(offset) << 32) + i128::from(elapsed) * i128::from(step);
    if pos < 0 {
        // Before the anchor: a looped clip keeps its phase, one played once is silent.
        let (s, e) = looping?;
        let span = i128::from(e - s) << 32;
        pos = (i128::from(s) << 32) + (pos - (i128::from(e) << 32)).rem_euclid(span);
    }
    let mut pos = pos as u128;
    if let Some((s, e)) = looping {
        let (s, e) = (u128::from(s) << 32, u128::from(e) << 32);
        if pos >= e {
            pos = s + (pos - e) % (e - s);
        }
    }
    if pos >= u128::from(length) << 32 {
        return None;
    }
    Some((pos, looping))
}

/// Starts `ident` on stream `s` (port `port`) at block frame `f`. `false` when nothing of it
/// can play there (its clip, its source or its port is gone, or it is past its end).
fn start_stream(
    s: &mut Stream,
    port: usize,
    io: &mut Io<'_>,
    track: &mut TrackIo<'_>,
    segment: &Segment,
    f: usize,
    ident: Ident,
) -> bool {
    let Some(clip) = track.clip(ident.clip()) else {
        return false;
    };
    let Some(src) = source_of(io.sources, clip.source) else {
        return false;
    };
    let step = step_of(src.sample_rate, io.rate);
    let Some((pos, looping)) = position(segment, f, ident, &clip, track.placements, step) else {
        return false;
    };
    let Some(p) = track.ports.get_mut(port) else {
        return false;
    };
    let base = clip.start.0;
    let path = Path::new(
        base.saturating_add((pos >> 32) as u64),
        base.saturating_add(clip.length.0),
        looping.map(|(a, b)| (base.saturating_add(a), base.saturating_add(b))),
        src.frames.0,
    );
    let mut voice = s.voice;
    voice.start(
        PortId {
            track: track.slot,
            instance: track.instance,
            port: port as u8,
        },
        p,
        clip.source,
        src.sample_rate,
        u16::from(src.channels),
        &src.segments,
        path,
        io.tx,
    );
    *s = Stream {
        voice,
        ident: Some(ident),
        clip: Some(clip),
        source: clip.source,
        path,
        step,
        frac: pos as u32,
        x0: path.start,
        gain: if clip.gain.is_finite() {
            clip.gain.max(0.0)
        } else {
            0.0
        },
        live: true,
        ..Stream::default()
    };
    // The window: `x0`, `x1`, `x2` from the source; `x[-1]` is `x0` (the fade in covers it).
    s.pull(port, io, track, 3);
    let (stereo, real) = (&io.scratch.stereo, &io.scratch.real);
    for c in 0..2 {
        s.win[c] = [stereo[0][c], stereo[0][c], stereo[1][c], stereo[2][c]];
    }
    s.real = [real[0], real[0], real[1], real[2]];
    true
}

impl Stream {
    /// Pulls `n` (at most [`CHUNK`]) frames of the voice into the scratch's `stereo` and
    /// `real`, mapped to two channels, gated and declicked (module doc). Returns `n`.
    fn pull(&mut self, port: usize, io: &mut Io<'_>, track: &mut TrackIo<'_>, n: usize) -> usize {
        let n = n.min(CHUNK);
        let ch = usize::from(self.voice.channels().max(1)).min(MAX_CHANNELS);
        let Scratch { raw, stereo, real } = &mut *io.scratch;
        let raw = &mut raw[..n * ch];
        let got = match track.ports.get_mut(port) {
            Some(p) => {
                let segments =
                    source_of(io.sources, self.source).map_or(&[][..], |s| &s.segments[..]);
                self.voice.read(p, segments, io.tx, io.stats, raw, n)
            }
            None => {
                raw.fill(0.0);
                0
            }
        };
        let edge = fade_frames(source_of(io.sources, self.source).map_or(1, |s| s.sample_rate));
        for k in 0..n {
            let frame = &raw[k * ch..k * ch + ch];
            let delivered = k < got;
            let at = match self.pulled {
                None => self.path.start,
                Some(p) => self.path.advance(p, 1),
            };
            // A loop wrap between the last frame pulled and this one.
            let wrapped = self.pulled.is_some_and(|p| at < p);
            self.pulled = Some(at);
            let mut v = if delivered {
                let l = frame.first().copied().unwrap_or(0.0);
                let r = if ch > 1 { frame[1] } else { l };
                [l, r]
            } else {
                self.held
            };
            if delivered {
                self.held = v;
            }
            if wrapped {
                // The step at the wrap is carried, then decays (module doc).
                for ((d, last), x) in self.declick.iter_mut().zip(self.last).zip(v) {
                    *d = last - x;
                }
                self.declick_left = edge;
            }
            if self.declick_left > 0 {
                let w = self.declick_left as f32 / edge as f32;
                for (x, d) in v.iter_mut().zip(self.declick) {
                    *x += d * w;
                }
                self.declick_left -= 1;
            }
            self.last = v;
            stereo[k] = v;
            real[k] = delivered;
        }
        n
    }

    /// Renders `[a, b)` of the block into `left` and `right` (added). Returns `true` when the
    /// stream played the last frame of its path (not when it faded out).
    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        port: usize,
        io: &mut Io<'_>,
        track: &mut TrackIo<'_>,
        a: usize,
        b: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) -> bool {
        let b = b.min(left.len()).min(right.len());
        if a >= b {
            return false;
        }
        let Some(rate) = source_of(io.sources, self.source).map(|s| s.sample_rate) else {
            self.stop();
            return false;
        };
        // The engine's rate may have changed: the step follows it.
        self.step = step_of(rate, io.rate);
        let fade_step = 1.0 / fade_frames(io.rate) as f32;
        let edge = u64::from(fade_frames(rate));
        // The gain follows the clip (an edit of its gain ramps); a clip gone fades out.
        let target = match (track.clips, self.ident) {
            (None, _) => self.gain,
            (Some(_), Some(i)) => match track.clip(i.clip()) {
                Some(c) if c.gain.is_finite() => c.gain.max(0.0),
                Some(_) => 0.0,
                None => {
                    self.out = true;
                    self.gain
                }
            },
            (Some(_), None) => {
                self.out = true;
                self.gain
            }
        };
        let mut need = (u64::from(self.frac) + (b - a) as u64 * self.step) >> 32;
        let mut have = 0usize;
        let mut used = 0usize;
        for i in a..b {
            if self.path.ended(self.x0) {
                break;
            }
            let t = self.frac as f32 * (1.0 / 4_294_967_296.0);
            if self.out {
                self.fade = (self.fade - fade_step).max(0.0);
            } else if self.fade < 1.0 {
                self.fade = (self.fade + fade_step).min(1.0);
            }
            if self.real[1] {
                if self.gate < 1.0 {
                    self.gate = (self.gate + fade_step).min(1.0);
                }
            } else if self.gate > 0.0 {
                self.gate = (self.gate - fade_step).max(0.0);
            }
            if self.gain != target {
                self.gain = approach(self.gain, target, fade_step);
            }
            let end_run = self.path.end.saturating_sub(self.x0);
            let edge_gain = if end_run < edge && self.path.looping.is_none_or(|(_, e)| self.x0 >= e)
            {
                end_run as f32 / edge as f32
            } else {
                1.0
            };
            let env = self.fade * self.gate * edge_gain * self.gain;
            if env != 0.0 {
                left[i] += flush(hermite(&self.win[0], t) * env);
                right[i] += flush(hermite(&self.win[1], t) * env);
            }
            let next = u64::from(self.frac) + self.step;
            self.frac = next as u32;
            for _ in 0..(next >> 32) {
                if used == have {
                    let k = (need as usize).clamp(1, CHUNK);
                    have = self.pull(port, io, track, k);
                    need = need.saturating_sub(have as u64);
                    used = 0;
                }
                let v = io.scratch.stereo[used];
                let r = io.scratch.real[used];
                used += 1;
                for (c, w) in self.win.iter_mut().enumerate() {
                    *w = [w[1], w[2], w[3], v[c]];
                }
                self.real = [self.real[1], self.real[2], self.real[3], r];
                self.x0 = self.path.advance(self.x0, 1);
            }
            if self.out && self.fade <= 0.0 {
                break;
            }
        }
        if self.out && self.fade <= 0.0 {
            self.stop();
            return false;
        }
        if self.path.ended(self.x0) {
            let natural = !self.out;
            self.stop();
            return natural;
        }
        false
    }

    fn stop(&mut self) {
        self.voice.stop();
        self.live = false;
    }
}

/// A subnormal sample becomes 0 (they slow some processors down).
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < f32::MIN_POSITIVE { 0.0 } else { x }
}

/// One step of a linear ramp from `g` toward `target`.
fn approach(g: f32, target: f32, step: f32) -> f32 {
    if g < target {
        (g + step).min(target)
    } else {
        (g - step).max(target)
    }
}

/// The 4-point cubic Hermite interpolation of `w = [x-1, x0, x1, x2]` at `t` in `[0, 1)`:
/// `x0` itself at `t = 0`.
#[inline]
fn hermite(w: &[f32; 4], t: f32) -> f32 {
    if t == 0.0 {
        return w[1];
    }
    let c1 = 0.5 * (w[2] - w[0]);
    let c2 = w[0] - 2.5 * w[1] + 2.0 * w[2] - 0.5 * w[3];
    let c3 = 0.5 * (w[3] - w[0]) + 1.5 * (w[1] - w[2]);
    ((c3 * t + c2) * t + c1) * t + w[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_returns_x0_at_zero_and_follows_a_line() {
        let w = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(hermite(&w, 0.0), 2.0);
        assert!((hermite(&w, 0.5) - 2.5).abs() < 1e-6);
        assert!((hermite(&w, 0.25) - 2.25).abs() < 1e-6);
    }

    #[test]
    fn the_step_is_exact_at_equal_rates_and_bounded() {
        assert_eq!(step_of(48_000, 48_000), 1 << 32);
        assert_eq!(step_of(44_100, 48_000), (44_100u64 << 32) / 48_000);
        assert_eq!(step_of(u32::MAX, 1), MAX_RATIO << 32);
        assert_eq!(step_of(0, 0), 1 << 32);
        assert_eq!(step_of(1, u32::MAX), 1);
    }

    #[test]
    fn fades_last_two_milliseconds() {
        assert_eq!(fade_frames(48_000), 96);
        assert_eq!(fade_frames(44_100), 88);
        assert_eq!(fade_frames(0), 1);
    }

    #[test]
    fn a_subnormal_output_is_flushed() {
        assert_eq!(flush(f32::MIN_POSITIVE / 2.0), 0.0);
        assert_eq!(flush(0.5), 0.5);
    }
}
