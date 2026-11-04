//! Audio sources streamed from disk (AU-3, `docs/architecture/03-stockage-local.md` § 3.3,
//! § 5.4 and § 5.11): the engine's side of the disk Worker. Pure, no I/O: the host reads the
//! files and hands the bytes to the [`Disk`] planner, which converts them and writes them
//! where the audio side reads them.
//!
//! # The pieces
//!
//! - A **source** ([`crate::engine::Source`]) is a media an audio clip uses, declared by
//!   `AudioSourceDeclare` with the SHA-256 of its blob. It holds up to [`MAX_SEGMENTS`]
//!   **resident segments** ([`Segment`]): PCM kept in memory, the whole source when it is
//!   short (≤ [`SHORT_SOURCE_MS`] and ≤ [`SHORT_SOURCE_BYTES`] decoded), else the first
//!   [`PRELOAD_MS`] of each clip and each short loop ([`SHORT_LOOP_MS`]). A segment is filled
//!   whole by the prepare side, then sent (`RtMsg::SegmentPut`): the audio side only ever
//!   sees complete segments, and a replaced one comes back through the garbage ring.
//! - Each track has [`PORTS_PER_TRACK`] **ports** ([`Port`]); a port has a **ring**
//!   ([`Ring`], single producer: the prepare side; single consumer: the audio thread)
//!   attached only while a voice streams through it, sized for its source (3 s of its
//!   frames, all its channels) and counted in the budget. Rings are attached and detached by
//!   `RtMsg::StreamAttach`; a detached ring comes back through the garbage ring.
//! - A **voice** ([`Voice`], audio side) plays a [`Path`] of a source: from a frame, to an
//!   end, wrapping in a loop. It reads the segments that cover its position (an instant
//!   start, short sources and short loops entirely from memory) and hands over to its port's
//!   ring at the first frame they do not cover. It asks for that ring through the **request
//!   queue** (`rtrb`, audio → prepare), never by touching the planner.
//! - The [`Disk`] planner (prepare side) reads the requests, probes each source's file (the
//!   blob read directly by ranges when its [`layout::PcmLayout`] matches the media, else the
//!   PCM cache `.faulk/cache/pcm/<hash>.pcm`), keeps each requested ring 2.5 s ahead
//!   ([`LOOKAHEAD_MS`]), loads the segments, and holds the **budget** of PCM in memory
//!   (rings and segments) with LRU eviction. Its [`Job`]s are byte ranges of files; the host
//!   reads them (`web/src/lib/audio/stream/`) and calls [`Disk::done`] or [`Disk::failed`].
//!
//! # The ring protocol
//!
//! A port's **generation** (`Port::generation`, audio side) counts the voice's requests. A request
//! carries it. The planner, when it takes a request, empties the ring (`write = read`: the
//! voice does not read meanwhile, see below), stores the generation in the ring's `ack`
//! (Release), and appends the request's path from `ring_from`. The voice reads the ring only
//! while `ack` equals its generation, so the data of an older request is never played, and a
//! read job started under an older generation is dropped when it completes. Data words are
//! `AtomicU32` (Relaxed); `write` (producer, Release) and `read` (consumer, Release) count
//! frames since the ring was made and never wrap in practice (u64).
//!
//! # Underruns
//!
//! A voice on its ring that gets fewer frames than its block asks plays silence for the rest
//! and keeps its position (the late frames are dropped when they arrive: `debt`), so time
//! never drifts. That block counts one underrun (`DiskStats::underruns`, reported as
//! `Feedback.disk_underruns`) once the ring has delivered since the request, or once the
//! voice waited longer than [`START_GRACE_MS`] for its first frame (a stalled disk is
//! counted; the normal latency of a jump into a spot that is not resident is not).
//!
//! # What is not here
//!
//! Playing audio clips (AU-4's clip player uses [`Voice`]; until then the web host drives one
//! test voice, `Host::probe_*`), resampling and channel mapping (AU-4: a voice gives every
//! channel of its source), the native helper's disk (AU-7: an engine without an executor
//! keeps every source offline).

pub mod layout;
#[cfg(feature = "testing")]
pub mod scenario;

use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
use std::collections::HashMap;
use std::sync::Arc;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::msg::{MsgError, RtMsg, SourceSlot, TrackSlot, limits, try_box};
use layout::PcmLayout;

/// Ports per track.
pub const PORTS_PER_TRACK: usize = 2;
/// Resident segments per source at most.
pub const MAX_SEGMENTS: usize = 16;
/// Most channels a source may have (the model's limit).
pub const MAX_CHANNELS: usize = 64;
/// Capacity of a ring, in milliseconds of its source.
pub const RING_MS: u64 = 3_000;
/// How far ahead the planner keeps a ring, in milliseconds of its source.
pub const LOOKAHEAD_MS: u64 = 2_500;
/// A ring is read again once this much of it was played (so its reads stay large).
pub const REFILL_MS: u64 = 500;
/// The start of each clip kept in memory.
pub const PRELOAD_MS: u64 = 1_500;
/// A source at most this long (and [`SHORT_SOURCE_BYTES`] decoded) is kept whole.
pub const SHORT_SOURCE_MS: u64 = 30_000;
/// A source at most this large decoded (and [`SHORT_SOURCE_MS`] long) is kept whole.
pub const SHORT_SOURCE_BYTES: u64 = 10_000_000;
/// A clip loop at most this long is kept whole in memory.
pub const SHORT_LOOP_MS: u64 = 10_000;
/// A source read in the last this many milliseconds is never evicted.
pub const IN_USE_MS: u64 = 5_000;
/// A ring no request touched for this long is detached (its memory freed).
pub const DETACH_MS: u64 = 10_000;
/// How often an unreadable source is probed again (the page may be regenerating its cache,
/// or the blob may come back).
pub const REPROBE_MS: u64 = 2_000;
/// How long a voice may wait for the first frame of its ring before its silence counts as
/// underruns.
pub const START_GRACE_MS: u64 = 250;
/// A voice playing from memory tells the planner it still uses its source this often.
pub const KEEPALIVE_MS: u64 = 1_000;
/// Largest read job.
pub const JOB_BYTES: usize = 256 * 1024;
/// Read jobs in flight at most.
pub const MAX_JOBS_IN_FLIGHT: usize = 8;
/// The default budget of PCM in memory (§ 5.4: "proposition : 300 Mo, réglable").
pub const DEFAULT_BUDGET_BYTES: u64 = 300_000_000;
/// Capacity of the request queue (audio → prepare).
pub const REQUEST_QUEUE_CAPACITY: usize = 4_096;
/// The planner sends no message while the prepare side's queue holds more than this: it
/// never fills the queue that the project's messages need (`MsgError::EngineBusy`).
pub const MAX_PENDING_FOR_DISK: usize = 64;
/// Bytes of the PCM cache header (`audio_import::cache`).
pub const CACHE_HEADER_BYTES: u64 = 32;
/// The first bytes of a PCM cache.
pub const CACHE_MAGIC: &[u8; 8] = b"FKPCM001";
/// Bytes of the head of a blob read to find its layout (`layout::HEAD_BYTES`).
pub const PROBE_BYTES: u32 = layout::HEAD_BYTES as u32;

// ---------------------------------------------------------------- shared handle

struct SharedInner<T> {
    refs: AtomicU32,
    value: T,
}

/// A reference-counted handle allocated fallibly ([`Shared::try_new`]; `Arc` has no stable
/// fallible constructor). The audio side holds one and never drops it: what it holds goes
/// back through the garbage ring, or with its track through the retire ring.
pub struct Shared<T> {
    ptr: NonNull<SharedInner<T>>,
}

// SAFETY: a `Shared<T>` gives only `&T` (as `Arc`), so it is as thread-safe as `Arc<T>`.
unsafe impl<T: Send + Sync> Send for Shared<T> {}
// SAFETY: same as `Send`.
unsafe impl<T: Send + Sync> Sync for Shared<T> {}

impl<T> Shared<T> {
    /// A new handle, the only one.
    pub fn try_new(value: T) -> Result<Self, MsgError> {
        let inner = try_box(SharedInner {
            refs: AtomicU32::new(1),
            value,
        })?;
        Ok(Shared {
            ptr: NonNull::from(Box::leak(inner)),
        })
    }

    /// Another handle to the same value (no allocation).
    pub fn share(&self) -> Self {
        self.inner().refs.fetch_add(1, Ordering::Relaxed);
        Shared { ptr: self.ptr }
    }

    /// Whether both handles name the same value.
    pub fn same(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }

    fn inner(&self) -> &SharedInner<T> {
        // SAFETY: `ptr` came from `Box::leak` and is freed only when the last handle drops
        // (`Drop`), so it is valid while `self` exists; only shared references are made.
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> core::ops::Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner().value
    }
}

impl<T> Drop for Shared<T> {
    fn drop(&mut self) {
        if self.inner().refs.fetch_sub(1, Ordering::Release) == 1 {
            fence(Ordering::Acquire);
            // SAFETY: this was the last handle (the count reached 0), the pointer came from
            // `Box::leak` in `try_new`, and no other handle can reach it any more.
            drop(unsafe { Box::from_raw(self.ptr.as_ptr()) });
        }
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

// ---------------------------------------------------------------- path

/// What a voice plays of a source: from `start`, to `end` (exclusive), wrapping from the
/// loop's end to its start while the position is before the loop's end. Absolute source
/// frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Path {
    pub start: u64,
    pub end: u64,
    pub looping: Option<(u64, u64)>,
}

impl Path {
    /// A path within a source of `frames` frames: `end` clamped to it, a loop kept only
    /// when `start' < end' <= end`.
    pub fn new(start: u64, end: u64, looping: Option<(u64, u64)>, frames: u64) -> Path {
        let end = end.min(frames);
        let looping = looping.filter(|&(s, e)| s < e && e <= end);
        Path {
            start: start.min(end),
            end,
            looping,
        }
    }

    /// Frames playable from `pos` before a wrap or the end.
    pub fn run(&self, pos: u64) -> u64 {
        match self.looping {
            Some((_, e)) if pos < e => e - pos,
            _ => self.end.saturating_sub(pos),
        }
    }

    /// The position `k` frames after `pos` along the path.
    pub fn advance(&self, pos: u64, k: u64) -> u64 {
        match self.looping {
            Some((s, e)) if pos < e => {
                let p = pos.saturating_add(k);
                if p < e { p } else { s + (p - e) % (e - s) }
            }
            _ => pos.saturating_add(k).min(self.end),
        }
    }

    /// Whether the path has nothing left at `pos`.
    pub fn ended(&self, pos: u64) -> bool {
        self.run(pos) == 0
    }
}

// ---------------------------------------------------------------- shared structures

/// What the planner and the audio side report (`Feedback`), and the planner's clock.
#[derive(Debug, Default)]
pub struct DiskStats {
    /// Blocks in which a voice had fewer frames than it needed (cumulative; audio side).
    pub underruns: AtomicU32,
    /// Declared sources the engine cannot read (blob missing, cache not ready, no disk).
    pub offline: AtomicU32,
    /// Bytes of PCM in memory: rings and segments, loaded or loading.
    pub pcm_bytes: AtomicU64,
    /// Bit 0: something did not fit the budget (rings alone over it, or a ring refused).
    pub flags: AtomicU32,
}

/// [`DiskStats::flags`]: over budget.
pub const FLAG_OVER_BUDGET: u32 = 1;

/// A stream ring: interleaved frames of one source, all its channels.
#[derive(Debug)]
pub struct Ring {
    pub channels: u16,
    /// Capacity in frames.
    pub frames: u64,
    /// The generation of the request whose data the ring holds; 0: none.
    pub ack: AtomicU32,
    pub write: AtomicU64,
    pub read: AtomicU64,
    data: Box<[AtomicU32]>,
}

impl Ring {
    /// A ring of `frames` frames of `channels` channels, empty.
    pub fn try_new(channels: u16, frames: u64) -> Result<Ring, MsgError> {
        let n = (frames as usize).saturating_mul(channels as usize);
        let mut v: Vec<AtomicU32> = Vec::new();
        v.try_reserve_exact(n).map_err(|_| MsgError::OutOfMemory {
            bytes: n.saturating_mul(4),
        })?;
        v.resize_with(n, || AtomicU32::new(0));
        Ok(Ring {
            channels,
            frames,
            ack: AtomicU32::new(0),
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            data: v.into_boxed_slice(),
        })
    }

    /// Bytes of its data.
    pub fn bytes(&self) -> u64 {
        self.data.len() as u64 * 4
    }

    fn sample(&self, frame: u64, channel: usize) -> f32 {
        let at = (frame % self.frames.max(1)) as usize * self.channels as usize + channel;
        self.data
            .get(at)
            .map_or(0.0, |w| f32::from_bits(w.load(Ordering::Relaxed)))
    }

    fn set(&self, frame: u64, channel: usize, value: f32) {
        let at = (frame % self.frames.max(1)) as usize * self.channels as usize + channel;
        if let Some(w) = self.data.get(at) {
            w.store(value.to_bits(), Ordering::Relaxed);
        }
    }
}

/// A port of a track: the generation of its voice's requests and the ring the planner
/// attached. Audio side; `ring` changes only through `RtMsg::StreamAttach`.
#[derive(Debug, Default)]
pub struct Port {
    pub generation: u32,
    pub ring: Option<Shared<Ring>>,
}

/// A resident range of a source: frames `[first, first + frames)`, interleaved, every channel.
#[derive(Debug)]
pub struct Segment {
    pub first: u64,
    pub frames: u64,
    pub channels: u16,
    pub data: Box<[f32]>,
}

impl Segment {
    fn covers(&self, pos: u64) -> bool {
        pos >= self.first && pos < self.first.saturating_add(self.frames)
    }
}

/// Where a request comes from and what it asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskRequest {
    pub track: TrackSlot,
    /// The track's instance (`Track::instance`): a request of a track that was replaced is
    /// ignored.
    pub instance: u32,
    pub port: u8,
    pub generation: u32,
    pub source: SourceSlot,
    pub path: Path,
    /// Where the ring must start (the first frame the segments do not cover); `None`: the
    /// voice plays from memory and needs no ring.
    pub ring_from: Option<u64>,
    /// Only says the source is still in use.
    pub keepalive: bool,
}

/// The request queue: the audio side's half.
pub type RequestTx = Producer<DiskRequest>;

/// A request queue.
pub fn request_queue() -> (Producer<DiskRequest>, Consumer<DiskRequest>) {
    RingBuffer::new(REQUEST_QUEUE_CAPACITY)
}

// ---------------------------------------------------------------- voice

/// Identifies a port for a voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortId {
    pub track: TrackSlot,
    pub instance: u32,
    pub port: u8,
}

/// The audio side of one stream: plays a path of a source from its segments and its port's
/// ring. Never allocates, never panics, never blocks.
#[derive(Clone, Copy, Debug, Default)]
pub struct Voice {
    id: Option<PortId>,
    source: SourceSlot,
    sample_rate: u32,
    channels: u16,
    path: Path,
    pos: u64,
    /// Generation of its request; 0: none.
    generation: u32,
    on_ring: bool,
    /// The ring's request starts here.
    ring_from: Option<u64>,
    /// The request is not queued yet (the queue was full).
    unsent: bool,
    /// The ring delivered since the request.
    primed: bool,
    /// Frames the voice played as silence that the ring still owes (dropped when they come).
    debt: u64,
    /// Frames waited for the first frame of the ring.
    waited: u64,
    /// Frames since the last request or keepalive.
    since_request: u64,
}

impl Voice {
    /// Whether it plays.
    pub fn active(&self) -> bool {
        self.id.is_some() && !self.path.ended(self.pos)
    }

    /// Its position in the source.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// The source it plays.
    pub fn source(&self) -> SourceSlot {
        self.source
    }

    /// Channels it gives per frame.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Starts playing `path` of `source` (declared with `sample_rate` and `channels`) through
    /// `port`: computes where the resident segments of `segments` stop covering the path and
    /// asks for a ring from there (or for none, when they cover all of it).
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        id: PortId,
        port: &mut Port,
        source: SourceSlot,
        sample_rate: u32,
        channels: u16,
        segments: &[Option<Box<Segment>>],
        path: Path,
        tx: &mut RequestTx,
    ) {
        port.generation = port.generation.wrapping_add(1).max(1);
        *self = Voice {
            id: Some(id),
            source,
            sample_rate,
            channels: channels.max(1),
            path,
            pos: path.start,
            generation: port.generation,
            on_ring: false,
            ring_from: handoff(&path, segments),
            unsent: true,
            primed: false,
            debt: 0,
            waited: 0,
            since_request: 0,
        };
        self.send(tx, false);
    }

    /// Stops it (nothing is asked of the planner: its ring detaches when idle).
    pub fn stop(&mut self) {
        self.id = None;
    }

    fn send(&mut self, tx: &mut RequestTx, keepalive: bool) {
        let Some(id) = self.id else { return };
        let request = DiskRequest {
            track: id.track,
            instance: id.instance,
            port: id.port,
            generation: self.generation,
            source: self.source,
            path: self.path,
            ring_from: self.ring_from,
            keepalive,
        };
        if tx.push(request).is_ok() {
            if !keepalive {
                self.unsent = false;
            }
            self.since_request = 0;
        }
    }

    /// Writes `frames` frames (interleaved, [`Voice::channels`] each) into `out` and returns
    /// how many came from the source; the rest is silence (past the end, waiting for the
    /// ring, or an underrun, counted in `stats`). `segments` are its source's, `port` the
    /// one it started on.
    pub fn read(
        &mut self,
        port: &mut Port,
        segments: &[Option<Box<Segment>>],
        tx: &mut RequestTx,
        stats: &DiskStats,
        out: &mut [f32],
        frames: usize,
    ) -> usize {
        let ch = self.channels as usize;
        let frames = frames.min(out.len() / ch.max(1));
        let out = &mut out[..frames * ch];
        out.fill(0.0);
        if self.id.is_none() {
            return 0;
        }
        if self.unsent {
            self.send(tx, false);
        }
        self.since_request = self.since_request.saturating_add(frames as u64);
        // On the segments or on the ring: the planner keeps what a playing voice uses, and
        // gives a ring back to a voice that lost it (detached, or of the wrong shape).
        if self.since_request >= (self.sample_rate as u64).saturating_mul(KEEPALIVE_MS) / 1000 {
            self.send(tx, true);
        }
        let mut done = 0usize;
        let mut delivered = 0usize;
        while done < frames && !self.path.ended(self.pos) {
            let want = (frames - done) as u64;
            if !self.on_ring {
                if let Some(seg) = segments.iter().flatten().find(|s| s.covers(self.pos)) {
                    let n = want
                        .min(seg.first + seg.frames - self.pos)
                        .min(self.path.run(self.pos));
                    copy_segment(seg, self.pos, n as usize, ch, &mut out[done * ch..]);
                    self.pos = self.path.advance(self.pos, n);
                    done += n as usize;
                    delivered += n as usize;
                    continue;
                }
                // Off the segments: the ring takes over, from where it was asked to start.
                // A segment evicted before the voice reached it moves that point: ask again.
                self.on_ring = true;
                if self.ring_from != Some(self.pos) {
                    self.ring_from = Some(self.pos);
                    self.renew(port, tx);
                }
            }
            let got = self.read_ring(port, want, ch, &mut out[done * ch..]);
            done += got as usize;
            delivered += got as usize;
            if got < want && !self.path.ended(self.pos) {
                // Silence for the rest of the block; time goes on.
                let missing = want - got;
                self.pos = self.path.advance(self.pos, missing);
                self.debt = self.debt.saturating_add(missing);
                if !self.primed {
                    self.waited = self.waited.saturating_add(missing);
                }
                let grace = (self.sample_rate as u64).saturating_mul(START_GRACE_MS) / 1000;
                // Not while the port has no ring: its source is offline (counted as such) or
                // the planner has not attached one yet.
                if port.ring.is_some() && (self.primed || self.waited > grace) {
                    stats.underruns.fetch_add(1, Ordering::Relaxed);
                }
                break;
            }
        }
        delivered
    }

    /// A new request on the same port (a new generation).
    fn renew(&mut self, port: &mut Port, tx: &mut RequestTx) {
        port.generation = port.generation.wrapping_add(1).max(1);
        self.generation = port.generation;
        self.unsent = true;
        self.primed = false;
        self.debt = 0;
        self.waited = 0;
        self.send(tx, false);
    }

    /// Reads up to `want` frames of the ring of its generation.
    fn read_ring(&mut self, port: &Port, want: u64, ch: usize, out: &mut [f32]) -> u64 {
        let Some(ring) = port.ring.as_deref() else {
            return 0;
        };
        if ring.ack.load(Ordering::Acquire) != self.generation || ring.channels as usize != ch {
            return 0;
        }
        let w = ring.write.load(Ordering::Acquire);
        let mut r = ring.read.load(Ordering::Relaxed);
        let mut avail = w.saturating_sub(r);
        // Frames that came after the voice had to go on without them.
        let drop = self.debt.min(avail);
        r += drop;
        avail -= drop;
        self.debt -= drop;
        // The ring holds the path itself (wraps included), never past its end.
        let n = want.min(avail);
        for f in 0..n {
            for c in 0..ch {
                out[f as usize * ch + c] = ring.sample(r + f, c);
            }
        }
        ring.read.store(r + n, Ordering::Release);
        if n > 0 {
            self.primed = true;
        }
        self.pos = self.path.advance(self.pos, n);
        n
    }
}

/// `jobs` as the table a wasm host reads ([`Disk::encode_jobs`]): as many as `out` holds;
/// returns how many were written.
pub fn encode_jobs(jobs: &[Job], out: &mut [u8]) -> usize {
    let mut n = 0;
    for (job, chunk) in jobs
        .iter()
        .zip(out.as_chunks_mut::<JOB_WIRE_BYTES>().0.iter_mut())
    {
        chunk.fill(0);
        chunk[0..4].copy_from_slice(&job.id.to_le_bytes());
        chunk[4] = job.file as u8;
        chunk[8..16].copy_from_slice(&job.offset.to_le_bytes());
        chunk[16..20].copy_from_slice(&job.len.to_le_bytes());
        chunk[24..56].copy_from_slice(&job.media);
        n += 1;
    }
    n
}

/// Copies `n` frames of `seg` from source frame `pos` (it covers them) into `out`.
fn copy_segment(seg: &Segment, pos: u64, n: usize, ch: usize, out: &mut [f32]) {
    let sc = seg.channels as usize;
    let at = (pos - seg.first) as usize;
    for f in 0..n {
        for c in 0..ch {
            let v = if c < sc {
                seg.data.get((at + f) * sc + c).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            if let Some(o) = out.get_mut(f * ch + c) {
                *o = v;
            }
        }
    }
}

/// Where the segments stop covering `path` (the ring's start), or `None` when they cover all
/// of it (a short source, a short loop with its start).
pub fn handoff(path: &Path, segments: &[Option<Box<Segment>>]) -> Option<u64> {
    let mut pos = path.start;
    // A loop is walked at most once around: each step leaves one segment or wraps once.
    for _ in 0..(2 * MAX_SEGMENTS + 2) {
        if path.ended(pos) {
            return None;
        }
        let seg = segments.iter().flatten().find(|s| s.covers(pos))?;
        let run = path.run(pos);
        let covered = (seg.first + seg.frames - pos).min(run);
        if covered < run {
            return Some(pos + covered);
        }
        // Covered up to a wrap or the end.
        let next = path.advance(pos, covered);
        match path.looping {
            Some((s, e)) if pos < e => {
                if next == s && segments.iter().flatten().any(|g| covers_range(g, s, e)) {
                    return None;
                }
                pos = next;
            }
            _ => return None,
        }
    }
    Some(pos)
}

fn covers_range(seg: &Segment, from: u64, to: u64) -> bool {
    from >= seg.first && to <= seg.first.saturating_add(seg.frames)
}

// ---------------------------------------------------------------- the planner

/// Which file of a media a job reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FileKind {
    /// `blobs/<hash>`.
    Blob = 0,
    /// `.faulk/cache/pcm/<hash>.pcm`.
    Cache = 1,
}

/// A byte range of a file to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: u32,
    pub file: FileKind,
    pub media: [u8; 32],
    pub offset: u64,
    pub len: u32,
}

/// Size of a [`Job`] in the table a wasm host reads (`Disk::encode_jobs`).
pub const JOB_WIRE_BYTES: usize = 56;

/// Why a read failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The file is not there.
    NotFound = 1,
    /// The file is there but could not be read.
    Unreadable = 2,
    /// The file changed since the source was probed (its cache was made again).
    Changed = 3,
}

impl Failure {
    /// From a host's code; anything unknown is `Unreadable`.
    pub fn from_code(code: u32) -> Failure {
        match code {
            1 => Failure::NotFound,
            3 => Failure::Changed,
            _ => Failure::Unreadable,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Status {
    /// Not probed yet, or probe pending: not readable.
    Unprobed,
    Probing,
    Direct(PcmLayout),
    Cached,
    /// Not readable: probed again at `retry`.
    Offline {
        retry: u64,
    },
}

impl Status {
    fn ready(&self) -> bool {
        matches!(self, Status::Direct(_) | Status::Cached)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SegKind {
    Whole,
    Preload,
}

#[derive(Debug, Default)]
enum SegSlot {
    #[default]
    Empty,
    Loading {
        first: u64,
        frames: u64,
        kind: SegKind,
        data: Box<[f32]>,
        filled: u64,
        job: Option<u32>,
    },
    Loaded {
        first: u64,
        frames: u64,
        kind: SegKind,
        bytes: u64,
    },
}

impl SegSlot {
    fn range(&self) -> Option<(u64, u64)> {
        match self {
            SegSlot::Empty => None,
            SegSlot::Loading { first, frames, .. } | SegSlot::Loaded { first, frames, .. } => {
                Some((*first, *frames))
            }
        }
    }

    fn bytes(&self) -> u64 {
        match self {
            SegSlot::Empty => 0,
            SegSlot::Loading { data, .. } => data.len() as u64 * 4,
            SegSlot::Loaded { bytes, .. } => *bytes,
        }
    }
}

#[derive(Debug)]
struct SourceState {
    /// Bumped by every declare, release and change: a job of an older one is dropped.
    generation: u32,
    media: [u8; 32],
    rate: u32,
    channels: u16,
    frames: u64,
    status: Status,
    last_used: u64,
    /// A voice asked for it (a start or a keepalive) since it was declared.
    played: bool,
    segments: [SegSlot; MAX_SEGMENTS],
}

impl SourceState {
    fn frame_bytes(&self) -> u64 {
        match self.status {
            Status::Direct(l) => l.frame_bytes() as u64,
            _ => u64::from(self.channels) * 4,
        }
    }

    fn file(&self) -> FileKind {
        if matches!(self.status, Status::Direct(_)) {
            FileKind::Blob
        } else {
            FileKind::Cache
        }
    }

    fn offset_of(&self, frame: u64) -> u64 {
        match self.status {
            Status::Direct(l) => l.data_offset + frame * l.frame_bytes() as u64,
            _ => CACHE_HEADER_BYTES + frame * u64::from(self.channels) * 4,
        }
    }

    fn ms_to_frames(&self, ms: u64) -> u64 {
        u64::from(self.rate).saturating_mul(ms) / 1000
    }

    fn decoded_bytes(&self, frames: u64) -> u64 {
        frames.saturating_mul(u64::from(self.channels)) * 4
    }

    fn is_short(&self) -> bool {
        self.frames <= self.ms_to_frames(SHORT_SOURCE_MS)
            && self.decoded_bytes(self.frames) <= SHORT_SOURCE_BYTES
    }
}

#[derive(Clone, Copy, Debug)]
struct ClipInfo {
    source: SourceSlot,
    start: u64,
    length: u64,
    looping: Option<(u64, u64)>,
}

#[derive(Debug)]
struct PortState {
    instance: u32,
    generation: u32,
    source: SourceSlot,
    source_gen: u32,
    path: Path,
    cursor: u64,
    ring: Option<Shared<Ring>>,
    /// The ring was asked for and not given (budget, memory).
    starved: bool,
    /// Its ring was detached for want of use: given back only by a request or a keepalive.
    parked: bool,
    job: Option<u32>,
    touched: u64,
    last_read: u64,
}

#[derive(Clone, Copy, Debug)]
enum JobFor {
    ProbeBlob {
        source: u32,
        generation: u32,
    },
    ProbeCache {
        source: u32,
        generation: u32,
    },
    Ring {
        track: u32,
        port: u8,
        generation: u32,
        ring_id: usize,
        frames: u64,
        source: u32,
        source_gen: u32,
    },
    Segment {
        source: u32,
        generation: u32,
        index: usize,
        frames: u64,
    },
}

/// The planner (module doc). Lives on the prepare side ([`crate::engine::Preparer`]).
pub struct Disk {
    stats: Arc<DiskStats>,
    requests: Consumer<DiskRequest>,
    budget: u64,
    now: u64,
    sources: Vec<Option<SourceState>>,
    /// The last generation given to each source slot, kept across releases.
    generations: Vec<u32>,
    clips: HashMap<u32, ClipInfo>,
    /// Track slot → instance, for the audio tracks of the current state.
    tracks: HashMap<u32, u32>,
    ports: HashMap<(u32, u8), PortState>,
    /// The jobs the last poll returned.
    jobs: Vec<Job>,
    /// Jobs made since, returned by the next poll.
    queued: Vec<Job>,
    in_flight: HashMap<u32, JobFor>,
    next_job: u32,
    next_instance: u32,
    /// Messages for the audio side, sent by the preparer when its queue allows.
    outbox: std::collections::VecDeque<RtMsg>,
    ring_bytes: u64,
    over_budget: bool,
}

impl core::fmt::Debug for Disk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Disk")
            .field("budget", &self.budget)
            .field("ring_bytes", &self.ring_bytes)
            .field("in_flight", &self.in_flight.len())
            .finish_non_exhaustive()
    }
}

impl Disk {
    /// A planner reading `requests`, reporting into `stats`.
    pub fn new(stats: Arc<DiskStats>, requests: Consumer<DiskRequest>) -> Disk {
        Disk {
            stats,
            requests,
            budget: DEFAULT_BUDGET_BYTES,
            now: 0,
            sources: Vec::new(),
            generations: Vec::new(),
            clips: HashMap::new(),
            tracks: HashMap::new(),
            ports: HashMap::new(),
            jobs: Vec::new(),
            queued: Vec::new(),
            in_flight: HashMap::new(),
            next_job: 1,
            next_instance: 1,
            outbox: std::collections::VecDeque::new(),
            ring_bytes: 0,
            over_budget: false,
        }
    }

    /// The stats it reports into.
    pub fn stats(&self) -> &Arc<DiskStats> {
        &self.stats
    }

    /// Sets the budget of PCM in memory, at most [`limits::PCM_BUDGET_BYTES`].
    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes.min(limits::PCM_BUDGET_BYTES);
    }

    /// The budget.
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Bytes of PCM it holds (rings and segments).
    pub fn pcm_bytes(&self) -> u64 {
        self.ring_bytes + self.segment_bytes()
    }

    fn segment_bytes(&self) -> u64 {
        self.sources
            .iter()
            .flatten()
            .map(|s| s.segments.iter().map(SegSlot::bytes).sum::<u64>())
            .sum()
    }

    /// Read jobs in flight.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    // ------------------------------------------------ what the control stream says

    /// A `Reset`: the new state has no track, source nor clip.
    pub fn reset(&mut self) {
        self.sources.clear();
        self.clips.clear();
        self.tracks.clear();
        self.ports.clear();
        self.in_flight.clear();
        self.outbox.clear();
        self.ring_bytes = 0;
        self.publish();
    }

    /// A track created in `slot` (any kind: every track has ports): its instance.
    pub fn track_created(&mut self, slot: TrackSlot) -> u32 {
        self.track_removed(slot);
        let instance = self.next_instance;
        self.next_instance = self.next_instance.wrapping_add(1).max(1);
        if self.tracks.try_reserve(1).is_ok() {
            self.tracks.insert(slot.0, instance);
        }
        instance
    }

    /// The track in `slot` is removed (or replaced): its ports are forgotten (their rings go
    /// with the track).
    pub fn track_removed(&mut self, slot: TrackSlot) {
        self.tracks.remove(&slot.0);
        // Its rings not yet sent would land on the track that takes the slot.
        self.outbox
            .retain(|m| !matches!(m, RtMsg::StreamAttach { track, .. } if *track == slot));
        let gone: Vec<(u32, u8)> = self
            .ports
            .keys()
            .filter(|(t, _)| *t == slot.0)
            .copied()
            .collect();
        for key in gone {
            if let Some(p) = self.ports.remove(&key)
                && let Some(r) = &p.ring
            {
                self.ring_bytes = self.ring_bytes.saturating_sub(r.bytes());
            }
        }
        self.publish();
    }

    /// `AudioSourceDeclare`.
    pub fn declare(
        &mut self,
        slot: SourceSlot,
        rate: u32,
        channels: u8,
        frames: u64,
        media: [u8; 32],
    ) -> Result<(), MsgError> {
        let i = slot.0 as usize;
        if i >= self.sources.len() {
            let more = i + 1 - self.sources.len();
            self.sources
                .try_reserve(more)
                .map_err(|_| MsgError::OutOfMemory {
                    bytes: more * size_of::<Option<SourceState>>(),
                })?;
            self.sources.resize_with(i + 1, || None);
        }
        self.forget_segments_of(slot);
        // Generations survive a release: a job for the slot's previous media is never taken
        // for the next one.
        if i >= self.generations.len() {
            let more = i + 1 - self.generations.len();
            self.generations
                .try_reserve(more)
                .map_err(|_| MsgError::OutOfMemory { bytes: more * 4 })?;
            self.generations.resize(i + 1, 0);
        }
        let generation = self.generations[i].wrapping_add(1).max(1);
        self.generations[i] = generation;
        self.sources[i] = Some(SourceState {
            generation,
            media,
            rate: rate.max(1),
            channels: u16::from(channels).clamp(1, MAX_CHANNELS as u16),
            frames,
            status: Status::Unprobed,
            last_used: self.now,
            played: false,
            segments: Default::default(),
        });
        self.publish();
        Ok(())
    }

    /// `AudioSourceRelease`.
    pub fn release(&mut self, slot: SourceSlot) {
        self.forget_segments_of(slot);
        if let Some(s) = self.sources.get_mut(slot.0 as usize) {
            *s = None;
        }
        self.publish();
    }

    /// Its segments not yet sent would land on the source that takes the slot.
    fn forget_segments_of(&mut self, slot: SourceSlot) {
        self.outbox
            .retain(|m| !matches!(m, RtMsg::SegmentPut { source, .. } if *source == slot));
    }

    /// `ClipAudioSet`.
    pub fn clip_set(
        &mut self,
        clip: u32,
        source: SourceSlot,
        start: u64,
        length: u64,
        looping: Option<(u64, u64)>,
    ) {
        if self.clips.try_reserve(1).is_ok() {
            self.clips.insert(
                clip,
                ClipInfo {
                    source,
                    start,
                    length,
                    looping,
                },
            );
        }
    }

    /// `ClipRemove` (or a clip that became a MIDI clip).
    pub fn clip_removed(&mut self, clip: u32) {
        self.clips.remove(&clip);
    }

    // ------------------------------------------------ the poll

    /// The next message for the audio side, when the preparer's queue holds at most
    /// `pending` messages ([`MAX_PENDING_FOR_DISK`]).
    pub fn next_message(&mut self, pending: usize) -> Option<RtMsg> {
        if pending > MAX_PENDING_FOR_DISK {
            return None;
        }
        self.outbox.pop_front()
    }

    /// Takes the requests, plans the reads at `now_ms` (the host's clock, milliseconds) and
    /// returns the new jobs.
    pub fn poll(&mut self, now_ms: u64) -> &[Job] {
        self.now = self.now.max(now_ms);
        self.jobs.clear();
        while let Ok(request) = self.requests.pop() {
            self.take(request);
        }
        // The outbox is the planner's own backpressure: nothing new while it is not sent.
        let blocked = !self.outbox.is_empty();
        self.probe();
        self.fill_rings();
        if !blocked {
            self.load_segments();
            self.detach_idle();
        }
        self.publish();
        // The jobs made since the last poll (by completions too), then this poll's.
        self.jobs.append(&mut self.queued);
        &self.jobs
    }

    /// The jobs of the last poll.
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// The jobs of the last poll, as the table a wasm host reads: [`JOB_WIRE_BYTES`] each,
    /// `id u32, file u8, pad ×3, offset u64, len u32, pad u32, media [u8; 32]`.
    pub fn encode_jobs(&self, out: &mut [u8]) -> usize {
        encode_jobs(&self.jobs, out)
    }

    /// Whether the planner has nothing under way: no read job in flight or waiting for the
    /// next poll, and no message waiting for the audio side. The offline render (#647)
    /// renders a block only once the planner is idle after a poll.
    pub fn idle(&self) -> bool {
        self.in_flight.is_empty() && self.queued.is_empty() && self.outbox.is_empty()
    }

    /// Whether no audio source is declared (nothing to read, no voice to feed).
    pub fn no_sources(&self) -> bool {
        self.sources.iter().all(Option::is_none)
    }

    /// The media of the sources that are offline now (a probe or a read failed: the file is
    /// missing, unreadable, or its PCM cache is not made) and that a voice asked for since
    /// they were declared (a clip of theirs played), in slot order.
    pub fn offline_media(&self) -> impl Iterator<Item = [u8; 32]> + '_ {
        self.sources
            .iter()
            .flatten()
            .filter(|s| s.played && matches!(s.status, Status::Offline { .. }))
            .map(|s| s.media)
    }

    fn job(
        &mut self,
        file: FileKind,
        media: [u8; 32],
        offset: u64,
        len: u64,
        what: JobFor,
    ) -> bool {
        if self.in_flight.len() >= MAX_JOBS_IN_FLIGHT
            || self.queued.try_reserve(1).is_err()
            || self.in_flight.try_reserve(1).is_err()
        {
            return false;
        }
        let id = self.next_job;
        self.next_job = self.next_job.wrapping_add(1).max(1);
        self.queued.push(Job {
            id,
            file,
            media,
            offset,
            len: len.min(u64::from(u32::MAX)) as u32,
        });
        self.in_flight.insert(id, what);
        true
    }

    fn take(&mut self, r: DiskRequest) {
        if self.tracks.get(&r.track.0) != Some(&r.instance) || r.port as usize >= PORTS_PER_TRACK {
            return;
        }
        let now = self.now;
        let Some(source) = self
            .sources
            .get_mut(r.source.0 as usize)
            .and_then(|s| s.as_mut())
        else {
            return;
        };
        source.last_used = now;
        source.played = true;
        let (source_gen, rate, channels, ready) = (
            source.generation,
            source.rate,
            source.channels,
            source.status.ready(),
        );
        if r.keepalive {
            if let Some(p) = self.ports.get_mut(&(r.track.0, r.port))
                && p.generation == r.generation
            {
                p.touched = now;
                p.parked = false;
            }
            return;
        }
        let key = (r.track.0, r.port);
        if !self.ports.contains_key(&key) {
            if self.ports.try_reserve(1).is_err() {
                return;
            }
            self.ports.insert(
                key,
                PortState {
                    instance: r.instance,
                    generation: 0,
                    source: r.source,
                    source_gen,
                    path: r.path,
                    cursor: 0,
                    ring: None,
                    starved: false,
                    parked: false,
                    job: None,
                    touched: now,
                    last_read: 0,
                },
            );
        }
        let Some(from) = r.ring_from else {
            let Some(p) = self.ports.get_mut(&key) else {
                return;
            };
            p.instance = r.instance;
            p.generation = r.generation;
            p.path = r.path;
            p.source = r.source;
            p.source_gen = source_gen;
            p.touched = now;
            p.job = None;
            return;
        };
        let Some(p) = self.ports.get_mut(&key) else {
            return;
        };
        p.instance = r.instance;
        p.generation = r.generation;
        p.path = r.path;
        p.source = r.source;
        p.source_gen = source_gen;
        p.cursor = from;
        p.touched = now;
        p.job = None;
        p.starved = false;
        p.parked = false;
        if ready {
            self.attach_ring(key, rate, channels);
        }
        self.restart_ring(key);
    }

    /// Gives the port a ring that fits its source (`rate`, `channels`): the one it has, or a
    /// new one within the budget (`StreamAttach`); none when it does not fit (starved).
    fn attach_ring(&mut self, key: (u32, u8), rate: u32, channels: u16) {
        let want_frames = (u64::from(rate).saturating_mul(RING_MS) / 1000).max(1);
        let Some(p) = self.ports.get(&key) else {
            return;
        };
        if p.ring
            .as_deref()
            .is_some_and(|ring| ring.channels == channels && ring.frames >= want_frames)
        {
            return;
        }
        let instance = p.instance;
        let bytes = want_frames
            .saturating_mul(u64::from(channels))
            .saturating_mul(4);
        let old = p.ring.as_deref().map_or(0, Ring::bytes);
        self.make_room(bytes.saturating_sub(old));
        // A ring is never refused for the budget (§ 5.4: the buffers of the voices that play
        // are never cut; going over it is reported), only past the hard cap the wasm32 memory
        // reserves for PCM.
        let after = self.pcm_bytes().saturating_sub(old) + bytes;
        if after > self.budget {
            self.over_budget = true;
        }
        let ring = if after <= limits::PCM_BUDGET_BYTES && self.outbox.try_reserve(1).is_ok() {
            Ring::try_new(channels, want_frames)
                .ok()
                .and_then(|r| Shared::try_new(r).ok())
        } else {
            None
        };
        let Some(p) = self.ports.get_mut(&key) else {
            return;
        };
        match ring {
            Some(ring) => {
                self.ring_bytes = self.ring_bytes.saturating_sub(old) + ring.bytes();
                self.outbox.push_back(RtMsg::StreamAttach {
                    track: TrackSlot(key.0),
                    instance,
                    port: key.1,
                    ring: Some(ring.share()),
                });
                p.ring = Some(ring);
                p.starved = false;
                p.last_read = 0;
            }
            None => {
                p.starved = true;
                self.over_budget = true;
            }
        }
    }

    /// Empties the port's ring for its request's generation and acknowledges it.
    fn restart_ring(&mut self, key: (u32, u8)) {
        let Some(p) = self.ports.get_mut(&key) else {
            return;
        };
        if let Some(ring) = p.ring.as_deref()
            && ring.ack.load(Ordering::Relaxed) != p.generation
        {
            // The voice reads nothing until `ack` is its generation, so `read` is stable.
            let read = ring.read.load(Ordering::Acquire);
            ring.write.store(read, Ordering::Relaxed);
            ring.ack.store(p.generation, Ordering::Release);
            p.last_read = read;
        }
    }

    /// Evicts segments (LRU, not in use) until `bytes` more fit the budget, if it can.
    fn make_room(&mut self, bytes: u64) {
        while self.pcm_bytes() + bytes > self.budget {
            if !self.evict_one() {
                return;
            }
        }
    }

    /// Evicts the least recently used segment not in use: whole sources first, then
    /// preloads. `false` when none can go.
    fn evict_one(&mut self) -> bool {
        let now = self.now;
        let mut best: Option<(u8, u64, usize, usize)> = None;
        for (si, s) in self.sources.iter().enumerate() {
            let Some(s) = s else { continue };
            if now.saturating_sub(s.last_used) < IN_USE_MS {
                continue;
            }
            for (gi, g) in s.segments.iter().enumerate() {
                let kind = match g {
                    SegSlot::Loaded { kind, .. } | SegSlot::Loading { kind, .. } => *kind,
                    SegSlot::Empty => continue,
                };
                let rank = if kind == SegKind::Whole { 0 } else { 1 };
                let key = (rank, s.last_used, si, gi);
                if best.is_none_or(|b| key < b) {
                    best = Some(key);
                }
            }
        }
        let Some((_, _, si, gi)) = best else {
            return false;
        };
        self.drop_segment(si, gi);
        true
    }

    fn drop_segment(&mut self, si: usize, gi: usize) {
        let Some(s) = self.sources.get_mut(si).and_then(|s| s.as_mut()) else {
            return;
        };
        let was = core::mem::replace(&mut s.segments[gi], SegSlot::Empty);
        if matches!(was, SegSlot::Loaded { .. }) && self.outbox.try_reserve(1).is_ok() {
            self.outbox.push_back(RtMsg::SegmentPut {
                source: SourceSlot(si as u32),
                index: gi as u8,
                segment: None,
            });
        }
    }

    fn probe(&mut self) {
        let now = self.now;
        for i in 0..self.sources.len() {
            let Some(s) = self.sources[i].as_ref() else {
                continue;
            };
            let due = match s.status {
                Status::Unprobed => true,
                Status::Offline { retry } => now >= retry,
                _ => false,
            };
            if !due {
                continue;
            }
            let (media, generation) = (s.media, s.generation);
            if self.job(
                FileKind::Blob,
                media,
                0,
                u64::from(PROBE_BYTES),
                JobFor::ProbeBlob {
                    source: i as u32,
                    generation,
                },
            ) && let Some(s) = self.sources[i].as_mut()
            {
                s.status = Status::Probing;
            }
        }
    }

    fn fill_rings(&mut self) {
        // The emptiest rings first (by the time they hold), so that a voice that just started
        // never waits behind the top-ups of the others.
        let mut keys: Vec<(u64, (u32, u8))> = self
            .ports
            .iter()
            .map(|(k, p)| {
                let held = p.ring.as_deref().map_or(0, |r| {
                    r.write
                        .load(Ordering::Relaxed)
                        .saturating_sub(r.read.load(Ordering::Relaxed))
                });
                let rate = self
                    .sources
                    .get(p.source.0 as usize)
                    .and_then(|s| s.as_ref())
                    .map_or(1, |s| u64::from(s.rate));
                (held.saturating_mul(1000) / rate.max(1), *k)
            })
            .collect();
        keys.sort_unstable();
        for (_, key) in keys {
            // A port that waited for its source to become readable, or for room: its ring.
            let need = self.ports.get(&key).and_then(|p| {
                let s = self.sources.get(p.source.0 as usize)?.as_ref()?;
                let want_frames = (u64::from(s.rate).saturating_mul(RING_MS) / 1000).max(1);
                let fits = p
                    .ring
                    .as_deref()
                    .is_some_and(|r| r.channels == s.channels && r.frames >= want_frames);
                let wants = !fits && !p.parked && p.generation != 0 && !p.path.ended(p.cursor);
                (wants && s.generation == p.source_gen && s.status.ready())
                    .then_some((s.rate, s.channels))
            });
            if let Some((rate, channels)) = need {
                self.attach_ring(key, rate, channels);
                self.restart_ring(key);
            }
            let Some(p) = self.ports.get(&key) else {
                continue;
            };
            let Some(ring) = p.ring.as_deref() else {
                continue;
            };
            if p.job.is_some() || ring.ack.load(Ordering::Relaxed) != p.generation {
                continue;
            }
            let Some(s) = self
                .sources
                .get(p.source.0 as usize)
                .and_then(|s| s.as_ref())
            else {
                continue;
            };
            if s.generation != p.source_gen || !s.status.ready() || p.path.ended(p.cursor) {
                continue;
            }
            let read = ring.read.load(Ordering::Acquire);
            let write = ring.write.load(Ordering::Relaxed);
            let fill = write.saturating_sub(read);
            let target = s
                .ms_to_frames(LOOKAHEAD_MS)
                .min(ring.frames * 9 / 10)
                .max(1);
            // Topped up by chunks of at least REFILL_MS, not by the block just played: one read
            // per ring at a time, so small reads would starve the others.
            if fill.saturating_add(s.ms_to_frames(REFILL_MS)) > target {
                continue;
            }
            let per_job = (JOB_BYTES as u64 / s.frame_bytes().max(1)).max(1);
            let frames = (target - fill)
                .min(ring.frames - fill)
                .min(p.path.run(p.cursor))
                .min(per_job);
            if frames == 0 {
                continue;
            }
            let (file, media, offset, bytes, generation, ring_id, track, port, source, source_gen) = (
                s.file(),
                s.media,
                s.offset_of(p.cursor),
                frames * s.frame_bytes(),
                p.generation,
                ring as *const Ring as usize,
                key.0,
                key.1,
                p.source.0,
                s.generation,
            );
            if !self.job(
                file,
                media,
                offset,
                bytes,
                JobFor::Ring {
                    track,
                    port,
                    generation,
                    ring_id,
                    frames,
                    source,
                    source_gen,
                },
            ) {
                return;
            }
            let id = self.queued.last().map(|j| j.id);
            if let Some(p) = self.ports.get_mut(&key) {
                p.job = id;
            }
        }
    }

    /// The segments a source should have, by priority: whole when short, else the clip
    /// starts and the short loops.
    fn wanted(&self, si: usize) -> Vec<(u64, u64, SegKind)> {
        let Some(s) = self.sources.get(si).and_then(|s| s.as_ref()) else {
            return Vec::new();
        };
        if s.is_short() {
            return vec![(0, s.frames, SegKind::Whole)];
        }
        let preload = s.ms_to_frames(PRELOAD_MS);
        let short_loop = s.ms_to_frames(SHORT_LOOP_MS);
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        let mut clips: Vec<&ClipInfo> = self
            .clips
            .values()
            .filter(|c| c.source.0 as usize == si)
            .collect();
        clips.sort_by_key(|c| (c.start, c.length));
        for c in clips {
            let end = c.start.saturating_add(c.length).min(s.frames);
            ranges.push((c.start, c.start.saturating_add(preload).min(end)));
            if let Some((ls, le)) = c.looping {
                let (ls, le) = (c.start + ls, (c.start + le).min(end));
                if le > ls {
                    let to = if le - ls <= short_loop {
                        le
                    } else {
                        ls.saturating_add(preload).min(le)
                    };
                    ranges.push((ls, to));
                }
            }
        }
        ranges.retain(|(a, b)| b > a);
        ranges.sort_unstable();
        // Merge overlapping or touching ranges.
        let mut merged: Vec<(u64, u64, SegKind)> = Vec::new();
        for (a, b) in ranges {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b, SegKind::Preload)),
            }
        }
        merged.truncate(MAX_SEGMENTS);
        merged.into_iter().map(|(a, b, k)| (a, b - a, k)).collect()
    }

    fn load_segments(&mut self) {
        for si in 0..self.sources.len() {
            if !self.sources[si].as_ref().is_some_and(|s| s.status.ready()) {
                continue;
            }
            let wanted = self.wanted(si);
            // Drop what is no longer wanted.
            for gi in 0..MAX_SEGMENTS {
                let keep = self.sources[si]
                    .as_ref()
                    .and_then(|s| s.segments[gi].range());
                if let Some(range) = keep
                    && !wanted.iter().any(|w| (w.0, w.1) == range)
                {
                    self.drop_segment(si, gi);
                }
            }
            for (first, frames, kind) in wanted {
                let Some(s) = self.sources[si].as_ref() else {
                    break;
                };
                if s.segments
                    .iter()
                    .any(|g| g.range() == Some((first, frames)))
                {
                    continue;
                }
                let Some(gi) = s.segments.iter().position(|g| matches!(g, SegSlot::Empty)) else {
                    break;
                };
                let bytes = s.decoded_bytes(frames);
                self.make_room(bytes);
                if self.pcm_bytes() + bytes > self.budget {
                    // It streams instead; tried again at the next poll.
                    continue;
                }
                let n = (bytes / 4) as usize;
                let mut v: Vec<f32> = Vec::new();
                if v.try_reserve_exact(n).is_err() {
                    continue;
                }
                v.resize(n, 0.0);
                if let Some(s) = self.sources[si].as_mut() {
                    s.segments[gi] = SegSlot::Loading {
                        first,
                        frames,
                        kind,
                        data: v.into_boxed_slice(),
                        filled: 0,
                        job: None,
                    };
                }
            }
            // One job in flight per loading segment.
            for gi in 0..MAX_SEGMENTS {
                let Some(s) = self.sources[si].as_ref() else {
                    break;
                };
                let SegSlot::Loading {
                    first,
                    frames,
                    filled,
                    job: None,
                    ..
                } = s.segments[gi]
                else {
                    continue;
                };
                let per_job = (JOB_BYTES as u64 / s.frame_bytes().max(1)).max(1);
                let n = (frames - filled).min(per_job);
                let (file, media, offset, generation) =
                    (s.file(), s.media, s.offset_of(first + filled), s.generation);
                if !self.job(
                    file,
                    media,
                    offset,
                    n * s.frame_bytes(),
                    JobFor::Segment {
                        source: si as u32,
                        generation,
                        index: gi,
                        frames: n,
                    },
                ) {
                    return;
                }
                let id = self.queued.last().map(|j| j.id);
                if let Some(SegSlot::Loading { job, .. }) =
                    self.sources[si].as_mut().map(|s| &mut s.segments[gi])
                {
                    *job = id;
                }
            }
        }
    }

    fn detach_idle(&mut self) {
        let now = self.now;
        let keys: Vec<(u32, u8)> = self.ports.keys().copied().collect();
        for key in keys {
            let Some(p) = self.ports.get_mut(&key) else {
                continue;
            };
            if let Some(ring) = p.ring.as_deref() {
                let read = ring.read.load(Ordering::Relaxed);
                if read != p.last_read {
                    p.last_read = read;
                    p.touched = now;
                }
            }
            if p.ring.is_none() || now.saturating_sub(p.touched) < DETACH_MS {
                continue;
            }
            if self.outbox.try_reserve(1).is_err() {
                return;
            }
            if let Some(ring) = p.ring.take() {
                self.ring_bytes = self.ring_bytes.saturating_sub(ring.bytes());
            }
            p.job = None;
            // Its voice stopped (a playing voice sends a keepalive every second): its next
            // request or keepalive gets a ring again.
            p.parked = true;
            let instance = p.instance;
            self.outbox.push_back(RtMsg::StreamAttach {
                track: TrackSlot(key.0),
                instance,
                port: key.1,
                ring: None,
            });
        }
    }

    fn publish(&mut self) {
        let offline = self
            .sources
            .iter()
            .flatten()
            .filter(|s| !s.status.ready())
            .count() as u32;
        self.stats.offline.store(offline, Ordering::Relaxed);
        self.stats
            .pcm_bytes
            .store(self.pcm_bytes(), Ordering::Relaxed);
        let over = self.ring_bytes > self.budget
            || self.over_budget
            || self.ports.values().any(|p| p.starved);
        self.over_budget = false;
        self.stats
            .flags
            .store(if over { FLAG_OVER_BUDGET } else { 0 }, Ordering::Relaxed);
    }

    // ------------------------------------------------ completions

    /// Job `id` read `bytes` of a file of `file_len` bytes. Stale jobs are dropped.
    pub fn done(&mut self, id: u32, bytes: &[u8], file_len: u64) {
        let Some(what) = self.in_flight.remove(&id) else {
            return;
        };
        match what {
            JobFor::ProbeBlob { source, generation } => {
                let Some(s) = self.source_of(source, generation) else {
                    return;
                };
                match PcmLayout::parse(bytes, file_len)
                    .filter(|l| l.matches(s.rate, s.channels, s.frames))
                {
                    Some(l) => s.status = Status::Direct(l),
                    None => {
                        let media = s.media;
                        s.status = Status::Probing;
                        if !self.job(
                            FileKind::Cache,
                            media,
                            0,
                            CACHE_HEADER_BYTES,
                            JobFor::ProbeCache { source, generation },
                        ) && let Some(s) = self.source_of(source, generation)
                        {
                            s.status = Status::Unprobed;
                        }
                    }
                }
            }
            JobFor::ProbeCache { source, generation } => {
                let now = self.now;
                let Some(s) = self.source_of(source, generation) else {
                    return;
                };
                s.status = if cache_header_ok(bytes, file_len, s.rate, s.channels, s.frames) {
                    Status::Cached
                } else {
                    Status::Offline {
                        retry: now + REPROBE_MS,
                    }
                };
            }
            JobFor::Ring {
                track,
                port,
                generation,
                ring_id,
                frames,
                source,
                source_gen,
            } => {
                let Some(p) = self.ports.get_mut(&(track, port)) else {
                    return;
                };
                if p.job == Some(id) {
                    p.job = None;
                }
                let Some(ring) = p.ring.as_deref() else {
                    return;
                };
                if p.generation != generation
                    || ring as *const Ring as usize != ring_id
                    || ring.ack.load(Ordering::Relaxed) != generation
                {
                    return;
                }
                let Some(s) = self
                    .sources
                    .get(source as usize)
                    .and_then(|s| s.as_ref())
                    .filter(|s| s.generation == source_gen)
                else {
                    return;
                };
                let ch = s.channels as usize;
                let fb = s.frame_bytes() as usize;
                let n = (bytes.len() / fb.max(1)).min(frames as usize) as u64;
                let w = ring.write.load(Ordering::Relaxed);
                let free = ring
                    .frames
                    .saturating_sub(w.saturating_sub(ring.read.load(Ordering::Acquire)));
                let n = n.min(free);
                let status = s.status;
                for f in 0..n as usize {
                    for c in 0..ch {
                        ring.set(w + f as u64, c, sample_at(&status, bytes, f * fb, c));
                    }
                }
                ring.write.store(w + n, Ordering::Release);
                p.cursor = p.path.advance(p.cursor, n);
            }
            JobFor::Segment {
                source,
                generation,
                index,
                frames,
            } => {
                let now = self.now;
                let Some(s) = self.source_of(source, generation) else {
                    return;
                };
                let status = s.status;
                let ch = s.channels as usize;
                let fb = s.frame_bytes() as usize;
                let SegSlot::Loading {
                    data, filled, job, ..
                } = &mut s.segments[index]
                else {
                    return;
                };
                if *job != Some(id) {
                    return;
                }
                *job = None;
                let n = (bytes.len() / fb.max(1)).min(frames as usize);
                let at = *filled as usize * ch;
                for f in 0..n {
                    for c in 0..ch {
                        if let Some(d) = data.get_mut(at + f * ch + c) {
                            *d = sample_at(&status, bytes, f * fb, c);
                        }
                    }
                }
                *filled += n as u64;
                if n == 0 {
                    // A short read at the end of a file that is shorter than declared.
                    s.status = Status::Offline {
                        retry: now + REPROBE_MS,
                    };
                    s.segments[index] = SegSlot::Empty;
                    return;
                }
                self.finish_segment(source as usize, index);
            }
        }
        self.publish();
    }

    fn finish_segment(&mut self, si: usize, gi: usize) {
        let Some(s) = self.sources.get_mut(si).and_then(|s| s.as_mut()) else {
            return;
        };
        let complete =
            matches!(&s.segments[gi], SegSlot::Loading { frames, filled, .. } if filled >= frames);
        if !complete || self.outbox.try_reserve(1).is_err() {
            return;
        }
        let channels = s.channels;
        let SegSlot::Loading {
            first,
            frames,
            kind,
            data,
            ..
        } = core::mem::replace(&mut s.segments[gi], SegSlot::Empty)
        else {
            return;
        };
        let bytes = data.len() as u64 * 4;
        let Ok(segment) = try_box(Segment {
            first,
            frames,
            channels,
            data,
        }) else {
            return;
        };
        s.segments[gi] = SegSlot::Loaded {
            first,
            frames,
            kind,
            bytes,
        };
        self.outbox.push_back(RtMsg::SegmentPut {
            source: SourceSlot(si as u32),
            index: gi as u8,
            segment: Some(segment),
        });
    }

    /// Job `id` failed.
    pub fn failed(&mut self, id: u32, why: Failure) {
        let Some(what) = self.in_flight.remove(&id) else {
            return;
        };
        let (source, generation) = match what {
            JobFor::ProbeBlob { source, generation } => (source, generation),
            JobFor::ProbeCache { source, generation } => (source, generation),
            JobFor::Ring {
                track,
                port,
                source,
                source_gen,
                ..
            } => {
                if let Some(p) = self.ports.get_mut(&(track, port))
                    && p.job == Some(id)
                {
                    p.job = None;
                }
                (source, source_gen)
            }
            JobFor::Segment {
                source, generation, ..
            } => (source, generation),
        };
        let now = self.now;
        let Some(s) = self.source_of(source, generation) else {
            return;
        };
        if why == Failure::Changed {
            // Read again from the probe: the file may have another layout now.
            s.generation = s.generation.wrapping_add(1);
            s.status = Status::Unprobed;
        } else {
            s.status = Status::Offline {
                retry: now + REPROBE_MS,
            };
        }
        // Its segments still loading are dropped (their jobs are stale now).
        for g in s.segments.iter_mut() {
            if matches!(g, SegSlot::Loading { .. }) {
                *g = SegSlot::Empty;
            }
        }
        if why == Failure::Changed {
            for gi in 0..MAX_SEGMENTS {
                self.drop_segment(source as usize, gi);
            }
        }
        self.publish();
    }

    fn source_of(&mut self, source: u32, generation: u32) -> Option<&mut SourceState> {
        self.sources
            .get_mut(source as usize)
            .and_then(|s| s.as_mut())
            .filter(|s| s.generation == generation)
    }
}

/// Sample `c` of the frame at byte `at` of `bytes`, as the source's file stores it.
fn sample_at(status: &Status, bytes: &[u8], at: usize, c: usize) -> f32 {
    let v = match status {
        Status::Direct(l) => {
            let size = l.encoding.bytes();
            let i = at + c * size;
            bytes.get(i..i + size).map_or(0.0, |b| l.encoding.sample(b))
        }
        _ => {
            let i = at + c * 4;
            bytes
                .get(i..i + 4)
                .map_or(0.0, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }
    };
    // A file's NaN, infinity or subnormal never reaches the audio path.
    if v.is_normal() { v } else { 0.0 }
}

/// Whether a PCM cache's first bytes and length are those of this media (`audio_import::cache`:
/// magic, rate, channels, frames; the decoder's version is the page's to check when it makes
/// the caches).
pub fn cache_header_ok(head: &[u8], file_len: u64, rate: u32, channels: u16, frames: u64) -> bool {
    if head.len() < CACHE_HEADER_BYTES as usize || &head[..8] != CACHE_MAGIC {
        return false;
    }
    let r = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
    let c = u16::from_le_bytes([head[12], head[13]]);
    let f = u64::from_le_bytes(head[16..24].try_into().unwrap_or([0; 8]));
    let size = frames
        .checked_mul(u64::from(channels))
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_add(CACHE_HEADER_BYTES));
    r == rate && c == channels && f == frames && size == Some(file_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_wraps_in_its_loop_and_stops_at_its_end() {
        let p = Path::new(10, 100, Some((20, 30)), 1_000);
        assert_eq!(p.run(10), 20);
        assert_eq!(p.advance(10, 20), 20);
        assert_eq!(p.advance(25, 5), 20);
        assert_eq!(p.advance(25, 27), 22);
        assert!(!p.ended(29));
        let once = Path::new(10, 100, None, 50);
        assert_eq!(once.end, 50);
        assert_eq!(once.advance(40, 100), 50);
        assert!(once.ended(50));
        // A loop beyond the end, or empty, is no loop.
        assert_eq!(Path::new(0, 10, Some((5, 20)), 100).looping, None);
        assert_eq!(Path::new(0, 10, Some((5, 5)), 100).looping, None);
    }

    #[test]
    fn a_shared_handle_frees_its_value_with_the_last_handle() {
        let a = Shared::try_new(AtomicU32::new(7)).unwrap();
        let b = a.share();
        assert!(a.same(&b));
        drop(a);
        assert_eq!(b.load(Ordering::Relaxed), 7);
        drop(b);
    }

    #[test]
    fn the_cache_header_is_checked_field_by_field() {
        let mut h = [0u8; 32];
        h[..8].copy_from_slice(CACHE_MAGIC);
        h[8..12].copy_from_slice(&48_000u32.to_le_bytes());
        h[12..14].copy_from_slice(&2u16.to_le_bytes());
        h[16..24].copy_from_slice(&10u64.to_le_bytes());
        assert!(cache_header_ok(&h, 32 + 80, 48_000, 2, 10));
        assert!(!cache_header_ok(&h, 32 + 79, 48_000, 2, 10), "size");
        assert!(!cache_header_ok(&h, 32 + 80, 44_100, 2, 10), "rate");
        assert!(!cache_header_ok(&h[..31], 32 + 80, 48_000, 2, 10), "short");
    }
}
