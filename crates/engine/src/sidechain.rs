//! The detection inputs of the graph (#952, a sidechain): which tracks other tracks' devices
//! listen to, the order that renders them first, and the buffers that hold what they gave.
//!
//! A device whose descriptor declares [`crate::device::DeviceDescriptor::sidechain`] may
//! listen to another track for its detection (`ControlMsg::DeviceSidechain`, its
//! [`DeviceEntry::key`]). The graph ([`crate::graph`] > Sidechain) then:
//!
//! - **plans**, at the start of a block after any change of the tracks or of a chain
//!   ([`Keys::changed`]): pass 1 marks every track a device of another existing track names
//!   (a **source**), the devices still fading out of a chain (#233) included, so that a
//!   Compressor removed while it pumps keeps hearing its source over its fade; pass 2 walks, depth first and in slot order, from each track along each
//!   of its devices' sources, and appends a source to the plan when its walk ends (post-order:
//!   a source comes after the sources it listens to itself). An edge into a track still being
//!   walked (a cycle, which the project refuses) is left out. The first [`MAX_KEY_SOURCES`]
//!   sources get a buffer, in that order; the others get none (the project refuses more).
//!   The walk visits each track once and each device once: O(tracks + devices), no
//!   allocation, its stack and marks prepared off the audio path;
//! - **renders** the plan's sources first, in its order, then every other track in slot
//!   order; a track renders once per block whatever the plan says (`TrackGraph::rendered_at`),
//!   so a plan can only change what a detection hears, never which tracks sound;
//! - **taps** each source with a buffer right after its chain, before its strip (fader, pan,
//!   mute, solo): [`Keys::fill`] copies the block; a source that rendered nothing is marked
//!   silent;
//! - **reads**, for a device with a source, the frames of the sub-block it renders from that
//!   source's buffer when it was filled in this block ([`Keys::key`]), else silence: a source
//!   with no track or no buffer, one that rendered after the device (a cycle), and the frames
//!   of a catch-up (#761), which belong to an earlier block.
//!
//! Limits: the detection is not compensated for latency. A source whose chain delays its sound
//! by `P_s` frames, heard by a device after devices that delay by `P_c`, is detected `P_s −
//! P_c` frames late (early when negative): a Ceiling at its defaults on the source (1.5 ms of
//! look-ahead) makes it 72 frames late at 48 kHz (`tests/it/sidechain.rs`); with no device
//! of latency on either track both are 0. While a slot of the source switches its timing
//! (#761), the tap takes the block as the chain left it: the frames a pause holds back are the
//! ones before the switched slot, and the frames a catch-up adds go to the mix only.

use crate::device::Key;
use crate::engine::{DeviceEntry, Track};
use crate::msg::{MsgError, limits};

/// The most tracks other tracks listen to (#952): the buffers [`Keys`] holds. The project
/// refuses a sidechain to a further one (`project::limits::MAX_SIDECHAIN_SOURCES`).
pub const MAX_KEY_SOURCES: usize = 32;

/// A slot of [`Keys::buf_of`] without a buffer.
const NO_BUF: u8 = u8::MAX;

/// The marks of the plan's walk.
const SOURCE: u8 = 1;
const ON_STACK: u8 = 2;
const DONE: u8 = 4;

/// The graph's detection inputs (module doc), allocated once with the graph.
pub(crate) struct Keys {
    /// The buffers, `frames` per source, one after the other.
    left: Box<[f32]>,
    right: Box<[f32]>,
    frames: usize,
    /// Silence, for a key with nothing to give.
    zeros: Box<[f32]>,
    /// Per buffer: the block it was filled in, and whether it holds silence.
    stamp: [u64; MAX_KEY_SOURCES],
    silent: [bool; MAX_KEY_SOURCES],
    /// Per track slot: its buffer, or [`NO_BUF`].
    buf_of: Box<[u8]>,
    /// Per track slot: the walk's marks.
    mark: Box<[u8]>,
    /// The walk's stack: a track and the next of its devices to look at.
    stack: Box<[(u32, u32)]>,
    /// The sources with a buffer, in render order.
    plan: [u32; MAX_KEY_SOURCES],
    plan_len: usize,
    /// A change of the tracks or of a chain since the last plan.
    dirty: bool,
}

impl core::fmt::Debug for Keys {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Keys")
            .field("plan", &&self.plan[..self.plan_len])
            .finish_non_exhaustive()
    }
}

fn buffer<T: Copy>(len: usize, value: T) -> Result<Box<[T]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| MsgError::OutOfMemory {
            bytes: len.saturating_mul(size_of::<T>()),
        })?;
    v.resize(len, value);
    Ok(v.into_boxed_slice())
}

impl Keys {
    /// The buffers for blocks of up to `max_frames`, off the audio path.
    pub(crate) fn try_new(max_frames: usize) -> Result<Self, MsgError> {
        let tracks = limits::MAX_TRACKS as usize;
        Ok(Keys {
            left: buffer(MAX_KEY_SOURCES * max_frames, 0.0)?,
            right: buffer(MAX_KEY_SOURCES * max_frames, 0.0)?,
            frames: max_frames,
            zeros: buffer(max_frames, 0.0)?,
            stamp: [0; MAX_KEY_SOURCES],
            silent: [true; MAX_KEY_SOURCES],
            buf_of: buffer(tracks, NO_BUF)?,
            mark: buffer(tracks, 0)?,
            stack: buffer(tracks, (0, 0))?,
            plan: [0; MAX_KEY_SOURCES],
            plan_len: 0,
            dirty: true,
        })
    }

    /// Bytes [`Keys::try_new`] allocates for blocks of up to `max_frames`.
    pub(crate) const fn bytes_for(max_frames: usize) -> u64 {
        let tracks = limits::MAX_TRACKS as u64;
        (2 * MAX_KEY_SOURCES as u64 + 1) * max_frames as u64 * 4
            + tracks * (2 + size_of::<(u32, u32)>() as u64)
    }

    /// The tracks or a chain changed: plan again before the next block.
    pub(crate) fn changed(&mut self) {
        self.dirty = true;
    }

    /// Plans again if something changed (module doc).
    pub(crate) fn plan_if_changed(&mut self, tracks: &[Option<Box<Track>>]) {
        if self.dirty {
            self.plan(tracks);
        }
    }

    /// The sources with a buffer, in render order.
    pub(crate) fn plan_order(&self) -> ([u32; MAX_KEY_SOURCES], usize) {
        (self.plan, self.plan_len)
    }

    /// The plan (module doc): no allocation, O(tracks + devices).
    fn plan(&mut self, tracks: &[Option<Box<Track>>]) {
        self.dirty = false;
        for &slot in &self.plan[..self.plan_len] {
            if let Some(b) = self.buf_of.get_mut(slot as usize) {
                *b = NO_BUF;
            }
        }
        self.plan_len = 0;
        let n = tracks.len().min(self.mark.len());
        let mark = &mut self.mark[..n];
        mark.fill(0);
        // The source a device of track `t` listens to, when it is another existing track.
        let source = |t: usize, e: &DeviceEntry| -> Option<usize> {
            let s = e.key()?.0 as usize;
            (s < n && s != t && tracks[s].is_some()).then_some(s)
        };
        // A track's devices: its chain, then the ones fading out of it.
        let devices = |t: usize| {
            tracks[t].as_deref().map_or((&[][..], &[][..]), |tr| {
                (tr.devices.as_slice(), tr.graph.leaving().entries())
            })
        };
        let device = |t: usize, i: usize| {
            let (chain, outgoing) = devices(t);
            match chain.get(i) {
                Some(e) => e.as_deref(),
                None => outgoing.get(i - chain.len()).and_then(|e| e.as_deref()),
            }
        };
        let count = |t: usize| {
            let (chain, outgoing) = devices(t);
            chain.len() + outgoing.len()
        };
        // Pass 1: the sources.
        for t in 0..n {
            for i in 0..count(t) {
                if let Some(s) = device(t, i).and_then(|e| source(t, e)) {
                    mark[s] |= SOURCE;
                }
            }
        }
        // Pass 2: the walk, in post-order.
        for root in 0..n {
            if tracks[root].is_none() || mark[root] & DONE != 0 {
                continue;
            }
            let mut depth = 1;
            self.stack[0] = (root as u32, 0);
            mark[root] |= ON_STACK;
            while depth > 0 {
                let (t, from) = self.stack[depth - 1];
                let t = t as usize;
                let len = count(t);
                let mut next = None;
                let mut i = from as usize;
                while i < len {
                    let s = device(t, i).and_then(|e| source(t, e));
                    i += 1;
                    if let Some(s) = s
                        && mark[s] & (ON_STACK | DONE) == 0
                    {
                        next = Some(s);
                        break;
                    }
                }
                match next {
                    // Each track goes on the stack once (`ON_STACK`, then `DONE`): `depth`
                    // never exceeds `n`.
                    Some(s) if depth < self.stack.len() => {
                        self.stack[depth - 1].1 = i as u32;
                        self.stack[depth] = (s as u32, 0);
                        mark[s] |= ON_STACK;
                        depth += 1;
                    }
                    _ => {
                        depth -= 1;
                        mark[t] = (mark[t] & !ON_STACK) | DONE;
                        if mark[t] & SOURCE != 0 && self.plan_len < MAX_KEY_SOURCES {
                            self.buf_of[t] = self.plan_len as u8;
                            self.plan[self.plan_len] = t as u32;
                            self.plan_len += 1;
                        }
                    }
                }
            }
        }
    }

    /// What track `slot` gave in the block `stamp`: its chain's output (`left`, `right`, the
    /// block's frames), or `None` when it rendered nothing (silence). Nothing for a track
    /// without a buffer.
    pub(crate) fn fill(&mut self, slot: usize, stamp: u64, out: Option<(&[f32], &[f32])>) {
        let Some(&b) = self.buf_of.get(slot) else {
            return;
        };
        let b = usize::from(b);
        if b >= MAX_KEY_SOURCES {
            return;
        }
        self.stamp[b] = stamp;
        self.silent[b] = true;
        if let Some((l, r)) = out {
            let len = l.len().min(r.len()).min(self.frames);
            let at = b * self.frames;
            self.left[at..at + len].copy_from_slice(&l[..len]);
            self.right[at..at + len].copy_from_slice(&r[..len]);
            self.silent[b] = false;
        }
    }

    /// The key of the frames `at..at + frames` of the block `stamp` from track `source`
    /// (module doc): its buffer when it was filled in that block, else silence.
    pub(crate) fn key(&self, source: u32, stamp: u64, at: usize, frames: usize) -> Key<'_> {
        let frames = frames.min(self.frames);
        let zeros = &self.zeros[..frames];
        let silence = Key {
            left: zeros,
            right: zeros,
        };
        let Some(&b) = self.buf_of.get(source as usize) else {
            return silence;
        };
        let b = usize::from(b);
        if b >= MAX_KEY_SOURCES
            || self.stamp[b] != stamp
            || self.silent[b]
            || at + frames > self.frames
        {
            return silence;
        }
        let from = b * self.frames + at;
        Key {
            left: &self.left[from..from + frames],
            right: &self.right[from..from + frames],
        }
    }

    /// Silence of `frames` frames.
    pub(crate) fn silence(&self, frames: usize) -> Key<'_> {
        let zeros = &self.zeros[..frames.min(self.frames)];
        Key {
            left: zeros,
            right: zeros,
        }
    }
}

/// What [`crate::graph`]'s `run_chain` gives a device with a source: the graph's keys, the
/// block, and where the sub-block it runs starts in the block (`None`: frames of an earlier
/// block, a catch-up, #761, or the master's chain: silence).
#[derive(Clone, Copy)]
pub(crate) struct KeyIn<'a> {
    pub(crate) keys: &'a Keys,
    pub(crate) stamp: u64,
    pub(crate) at: Option<usize>,
}

impl<'a> KeyIn<'a> {
    /// The same, `skip` frames further into the sub-block (a pause, #761).
    pub(crate) fn skipped(self, skip: usize) -> Self {
        KeyIn {
            at: self.at.map(|a| a + skip),
            ..self
        }
    }

    /// The key of `entry` over `frames` frames, `None` when it listens to its own track.
    pub(crate) fn of(&self, entry: &DeviceEntry, frames: usize) -> Option<Key<'a>> {
        let source = entry.key()?;
        Some(match self.at {
            Some(at) => self.keys.key(source.0, self.stamp, at, frames),
            None => self.keys.silence(frames),
        })
    }
}
