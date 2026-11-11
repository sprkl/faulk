//! Delay lines of the latency compensation (#749): what keeps every track, the master and
//! the metronome on one timing whatever the latency of the devices
//! ([`crate::device::Device::latency_frames`]).
//!
//! A [`Delay`] is a stereo ring allocated off the audio path, read at a **tap** (a delay in
//! frames). A change of tap never jumps: the line crossfades linearly from the old tap to the
//! new one over a fade ([`crate::graph::FADE_SECONDS`]), and a change asked during a crossfade
//! waits for its end. A longer tap waits until the line holds that many frames since it was
//! last cleared, so that a crossfade never reads frames the line never had. A clear takes
//! constant time: a count of the frames written since hides the older content (a read beyond
//! it is 0). A line that held only silence for longer than its tap forgets what came before
//! that silence, so that a line nobody feeds for a while (a track that was not rendered)
//! never plays stale content when its tap grows.
//! At a tap of 0 with no change under way the line is a pass-through and holds nothing. On a
//! track's chain the graph moves a slot's tap with no crossfade instead ([`Delay::jump`],
//! #761), the track's [`Route`] taking the difference at the same frame; what a shorter tap
//! skips is read back ([`Delay::read_back`]) and added before the block ([`Route::add_back`]).
//!
//! A [`Latent`] wraps a device with a latency and holds its [`Bypass`]: a [`Delay`] the
//! graph feeds with the signal entering the device's slot and reads at the device's latency,
//! so that a bypassed device keeps its latency and its fades blend two signals of one timing
//! ([`crate::graph`] > Latency). The [`Bypass`] holds a second line of the same size, its
//! `strip`: the levels of the strip after the chain (gate × gain × pan, one pair per frame),
//! delayed as the slot's sound is, so that a timed gain, pan or mute lands on the sound it
//! belongs to (#758). It [`Delay::follow`]s the bypass line: whether a longer tap may start
//! depends on a line's content, and only the line of sound knows what its slot gives. Its
//! third line, `events` ([`Events`]), holds the parameter values of the effects after it in the
//! chain, dated at transport time, and hands them on by the delay its bypass line reads
//! ([`Timing::delay`]), so that an automated parameter lands on the sound it belongs to
//! (#765). The master's own [`Events`] line delays its effects' values by the tracks' latency.
//!
//! The [`Compensation`] ring delays the tracks: a track `D` frames early adds its sample of
//! block frame `i` at `i + D` ahead of the ring's read point, and the graph takes the block's
//! frames out once every track added its own. One ring serves any number of distinct delays,
//! with a fixed size. A change of delay is a [`Route`]'s move: the old and the new delay are
//! fed side by side with gains timed so that they cross in *output* time (no gap, nothing
//! heard twice).
//!
//! No allocation, no lock, no panic after the `try_new`s.

use crate::device::{AudioIo, BlockCtx, Device, DeviceDescriptor, Event};
use crate::events::Sink;
use crate::msg::{DeviceSlot, MsgError, ParamId, limits, try_box};

/// A level under which a sample counts as silence for [`Delay::busy`] (-400 dB).
const QUIET: f32 = 1e-20;

/// A zeroed buffer of `frames` samples, off the audio path (fallible).
pub(crate) fn buffer(frames: usize) -> Result<Box<[f32]>, MsgError> {
    let mut v = Vec::new();
    v.try_reserve_exact(frames)
        .map_err(|_| MsgError::OutOfMemory {
            bytes: frames.saturating_mul(size_of::<f32>()),
        })?;
    v.resize(frames, 0.0);
    Ok(v.into_boxed_slice())
}

/// A stereo delay line with a crossfaded tap (module doc).
#[derive(Debug)]
pub struct Delay {
    left: Box<[f32]>,
    right: Box<[f32]>,
    /// `left.len() - 1`: the capacity is a power of two.
    mask: usize,
    /// Where the next frame is written.
    pos: usize,
    /// Frames written since the last clear (saturating): a read further back is 0.
    written: u64,
    /// Frames since the last sample above [`QUIET`] was written (saturating).
    quiet: u64,
    /// The tap read, or crossfaded toward.
    tap: u32,
    /// The tap a crossfade comes from.
    from: u32,
    /// The crossfade's progress toward `tap`, in `[0, 1]`; 1 when no crossfade is under way.
    g: f32,
    /// How the line read its last block: what a line following it replays (#758).
    last: Timing,
}

/// How a [`Delay`] read a block (#758): its taps and its crossfade's progress at the block's
/// start, once it decided whether to start a crossfade, and whether it passed the block
/// through. A line that [`Delay::follow`]s another one reads the same frames at the same taps
/// with the same crossfade, whatever its own content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    tap: u32,
    from: u32,
    g: f32,
    pass: bool,
}

impl Timing {
    /// Whether the line passed its block through (no delay, nothing kept).
    pub fn is_pass(&self) -> bool {
        self.pass
    }

    /// A pass-through: no delay, nothing kept.
    pub const PASS: Timing = Timing {
        tap: 0,
        from: 0,
        g: 1.0,
        pass: true,
    };

    /// The delay a value takes through a line read with this timing (#765, [`Events`]): none
    /// for a pass-through, else the tap read or crossfaded toward. A parameter is not blended:
    /// it takes the new tap as soon as the crossfade starts.
    pub fn delay(&self) -> u32 {
        if self.pass { 0 } else { self.tap }
    }
}

impl Delay {
    /// A line able to delay by up to `max_tap` frames, at tap `tap` (bounded to it), empty.
    /// Off the audio path: fallible allocations.
    pub fn try_new(max_tap: u32, tap: u32) -> Result<Self, MsgError> {
        let frames = (max_tap as usize).saturating_add(1).next_power_of_two();
        let max = (frames - 1).min(u32::MAX as usize) as u32;
        Ok(Delay {
            left: buffer(frames)?,
            right: buffer(frames)?,
            mask: frames - 1,
            pos: 0,
            written: 0,
            quiet: u64::MAX,
            tap: tap.min(max),
            from: tap.min(max),
            g: 1.0,
            last: Timing::PASS,
        })
    }

    /// Bytes a line able to delay by `max_tap` frames takes (both channels).
    pub const fn bytes_for(max_tap: u32) -> u64 {
        ((max_tap as u64 + 1).next_power_of_two()) * 2 * size_of::<f32>() as u64
    }

    /// The longest tap the line holds.
    pub fn max_tap(&self) -> u32 {
        self.mask.min(u32::MAX as usize) as u32
    }

    /// The tap read, or crossfaded toward.
    pub fn tap(&self) -> u32 {
        self.tap
    }

    /// Whether a crossfade between two taps is under way.
    pub fn crossfading(&self) -> bool {
        self.g < 1.0
    }

    /// Whether the line may still give something other than silence: a sample above
    /// [`QUIET`] went in less than its tap (or the tap a crossfade comes from) ago.
    pub fn busy(&self) -> bool {
        let window = u64::from(self.tap.max(if self.crossfading() { self.from } else { 0 }));
        self.quiet <= window
    }

    /// Hides everything the line holds (constant time): it reads silence until it is fed
    /// again. The tap and a crossfade under way are kept.
    pub fn clear(&mut self) {
        self.written = 0;
        self.quiet = u64::MAX;
    }

    /// Whether the line delays nothing and moves toward nothing: at a tap of 0, no crossfade
    /// under way.
    pub fn at_rest(&self) -> bool {
        self.tap == 0 && !self.crossfading()
    }

    /// Clears the line and fills it with `frames` frames (at most what it holds) of one value,
    /// as if it had been fed that value (#758): a line of levels that was not fed while it had
    /// nothing to delay takes its history from the level it starts from, so that a longer tap
    /// can start at once. Linear in `frames`, no allocation.
    pub fn prime(&mut self, left: f32, right: f32, frames: u32) {
        let frames = (frames as usize).min(self.left.len());
        for _ in 0..frames {
            self.left[self.pos] = left;
            self.right[self.pos] = right;
            self.pos = (self.pos + 1) & self.mask;
        }
        self.written = frames as u64;
        self.quiet = if left.abs() > QUIET || right.abs() > QUIET {
            0
        } else {
            frames as u64
        };
    }

    /// Takes `other`'s content, position and tap when both hold the same frames (a device put
    /// in the place of another of the same kind and rate: the slot keeps its timing, #749);
    /// `false`, and nothing changes, otherwise. Linear in the line's size (at most
    /// [`limits::MAX_COMPENSATION_FRAMES`] frames), no allocation.
    pub fn copy_from(&mut self, other: &Delay) -> bool {
        if self.left.len() != other.left.len() {
            return false;
        }
        self.left.copy_from_slice(&other.left);
        self.right.copy_from_slice(&other.right);
        self.pos = other.pos;
        self.written = other.written;
        self.quiet = other.quiet;
        self.tap = other.tap;
        self.from = other.from;
        self.g = other.g;
        self.last = other.last;
        true
    }

    /// Clears the line and sets its tap at once, with no crossfade: for a line whose output
    /// nobody heard before (a compensation bus taken again).
    pub fn restart(&mut self, tap: u32) {
        self.clear();
        self.tap = tap.min(self.max_tap());
        self.from = self.tap;
        self.g = 1.0;
    }

    /// Sets the tap at once, with no crossfade and the content kept (#761): the slot's switch,
    /// whose other side (the track's delay) moves at the same frame. A tap beyond what the line
    /// holds reads 0 until it holds it.
    pub fn jump(&mut self, tap: u32) {
        self.tap = tap.min(self.max_tap());
        self.from = self.tap;
        self.g = 1.0;
    }

    /// Copies into `left` and `right` the frames written from `back` frames ago on, oldest
    /// first (frame `k` is the one written `back - k` frames ago, 1 being the newest): what a
    /// slot whose tap shrinks by `left.len()` gave none of (#761). 0 for what the line does not
    /// hold. No allocation.
    pub fn read_back(&self, back: u32, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        for k in 0..frames {
            let ago = u64::from(back).saturating_sub(k as u64);
            let (l, r) = if ago == 0 || ago > self.written {
                (0.0, 0.0)
            } else {
                let at = self.pos.wrapping_sub(ago as usize) & self.mask;
                (self.left[at], self.right[at])
            };
            left[k] = l;
            right[k] = r;
        }
    }

    /// The sample `delay` frames before the one just written, 0 beyond what the line holds.
    #[inline]
    fn read(line: &[f32], mask: usize, pos: usize, written: u64, delay: u32) -> f32 {
        if u64::from(delay) >= written {
            return 0.0;
        }
        line[pos.wrapping_sub(delay as usize) & mask]
    }

    /// Delays `left` and `right` in place toward the tap `target` (bounded to
    /// [`Delay::max_tap`]), a crossfade lasting `fade` frames (at least 1).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], target: u32, fade: u32) {
        let target = target.min(self.max_tap());
        if !self.busy() {
            // Only silence within reach of the tap: what is older than that silence counts as
            // stale (a line nobody fed for a while), never read by a longer tap.
            self.written = self.written.min(self.quiet);
        }
        if !self.crossfading() && target != self.tap {
            // A longer tap waits for the frames it reads: the line must hold them.
            if target < self.tap || self.written > u64::from(target) {
                self.from = self.tap;
                self.tap = target;
                self.g = 0.0;
            }
        }
        // A pass-through: nothing to keep (a later tap waits for the line to fill, which it
        // does while that tap is asked for).
        let pass = !self.crossfading() && self.tap == 0 && target == 0;
        self.last = Timing {
            tap: self.tap,
            from: self.from,
            g: self.g,
            pass,
        };
        if pass {
            self.clear();
            return;
        }
        self.run(left, right, fade);
    }

    /// Records that the line passed its last block through, for a slot the chain did not run:
    /// its strip line passes too, and nothing keeps its chain on the delayed strip's path.
    pub fn pass(&mut self) {
        self.last = Timing::PASS;
    }

    /// How the line read its last block ([`Delay::follow`]).
    pub fn timing(&self) -> Timing {
        self.last
    }

    /// Delays `left` and `right` in place exactly as the line that read its last block with
    /// `timing` did (#758): the same taps and the same crossfade over the same frames, with no
    /// decision of its own (a line of levels following a line of sound, whose content decides
    /// when a longer tap can start). A tap beyond what this line holds reads 0, as ever; a line
    /// fed every block its leader is fed, and cleared no more often, holds at least as much.
    pub fn follow(&mut self, left: &mut [f32], right: &mut [f32], timing: Timing, fade: u32) {
        let max = self.max_tap();
        self.tap = timing.tap.min(max);
        self.from = timing.from.min(max);
        self.g = if timing.g.is_finite() {
            timing.g.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.last = timing;
        if timing.pass {
            self.clear();
            return;
        }
        self.run(left, right, fade);
    }

    /// Writes a block into the line and reads it back at the taps (the crossfade advancing).
    fn run(&mut self, left: &mut [f32], right: &mut [f32], fade: u32) {
        let frames = left.len().min(right.len());
        let step = 1.0 / fade.max(1) as f32;
        let (mask, tap, from) = (self.mask, self.tap, self.from);
        for i in 0..frames {
            let (l, r) = (left[i], right[i]);
            let pos = self.pos;
            self.left[pos] = l;
            self.right[pos] = r;
            self.written = self.written.saturating_add(1);
            if l.abs() > QUIET || r.abs() > QUIET {
                self.quiet = 0;
            } else {
                self.quiet = self.quiet.saturating_add(1);
            }
            let w = self.written;
            let new_l = Self::read(&self.left, mask, pos, w, tap);
            let new_r = Self::read(&self.right, mask, pos, w, tap);
            if self.g < 1.0 {
                self.g = (self.g + step).min(1.0);
                let g = self.g;
                let old_l = Self::read(&self.left, mask, pos, w, from);
                let old_r = Self::read(&self.right, mask, pos, w, from);
                left[i] = old_l + g * (new_l - old_l);
                right[i] = old_r + g * (new_r - old_r);
            } else {
                left[i] = new_l;
                right[i] = new_r;
            }
            self.pos = (pos + 1) & mask;
        }
        if !self.crossfading() {
            self.from = self.tap;
        }
    }
}

// ---------------------------------------------------------------- event lines (#765)

/// Values a device's event line holds ([`Events`]): about 60 moving lanes on the effects after
/// a Ceiling of 5 ms at 192 kHz (15 control periods each). Beyond, the lanes send again
/// ([`Lost`]).
pub const DEVICE_EVENTS: usize = 1024;

/// Values the master's event line holds (the tracks' latency `T`, up to
/// [`limits::MAX_COMPENSATION_FRAMES`]).
pub const MASTER_EVENTS: usize = 8192;

/// How far back a full [`Events`] line looks for a value of the same parameter to give a
/// newer one to (a bounded cost per value).
const SUPERSEDE_SCAN: usize = 64;

/// Where a line's clock starts (#765): an age handed on with a value ([`Events::push_moved`])
/// is subtracted from it, and no age reaches this far (2^40 frames: 290 days at 48 kHz), so a
/// stamp never saturates, even on a line younger than the value.
const CLOCK_START: u64 = 1 << 40;

/// A parameter value waiting in an [`Events`] line.
#[derive(Clone, Copy, Debug)]
struct Pending {
    /// The line's frame the value was dated at before its delay.
    stamp: u64,
    slot: DeviceSlot,
    /// Forgotten ([`Events::forget`]): it waits for its turn and lands nowhere.
    dead: bool,
    id: ParamId,
    value: f32,
}

const NOTHING: Pending = Pending {
    stamp: 0,
    slot: DeviceSlot(0),
    dead: true,
    id: ParamId(0),
    value: 0.0,
};

/// Words of a [`Lost`] set: one bit per device slot of a chain.
const LOST_WORDS: usize = (limits::MAX_DEVICES_PER_TRACK as usize).div_ceil(64);

/// The device slots an overflowing event line dropped a value of (#765): their lanes send
/// their value again ([`crate::automation`]), so that a parameter never stays on a value older
/// than its lane's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lost {
    bits: [u64; LOST_WORDS],
}

impl Lost {
    /// Marks `slot` (a slot beyond a chain's is ignored).
    pub fn mark(&mut self, slot: DeviceSlot) {
        let s = usize::from(slot.0);
        if let Some(w) = self.bits.get_mut(s / 64) {
            *w |= 1 << (s % 64);
        }
    }

    /// Whether no slot is marked.
    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|w| *w == 0)
    }

    /// The marked slots, in order.
    pub fn slots(&self) -> impl Iterator<Item = DeviceSlot> + '_ {
        self.bits.iter().enumerate().flat_map(|(w, bits)| {
            (0..64)
                .filter(move |b| bits & (1 << b) != 0)
                .map(move |b| DeviceSlot((w * 64 + b) as u16))
        })
    }
}

/// What an [`Events`] line does with a value of its source, by its slot (#765).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    /// Not for a slot after the line: it stays in the source, read before the line.
    Stay,
    /// For an effect after the line: delayed by it.
    Delay,
    /// For another device after the line (an instrument, a MIDI effect: its values keep to
    /// its notes, which are not delayed): handed on at its own offset.
    Pass,
}

/// A line of parameter values (#765, [`crate::graph`] > Latency): the values of the effects
/// after a device with a latency (or, the master's, after the tracks' latency), dated at
/// transport time, handed on to them `delay` frames later, when the sound they belong to
/// reaches them. First in, first out, with due times that never go down; a fixed capacity
/// allocated off the audio path.
#[derive(Debug)]
pub struct Events {
    ring: Box<[Pending]>,
    /// `ring.len() - 1`: the capacity is a power of two.
    mask: usize,
    /// Where the oldest value sits.
    head: usize,
    len: usize,
    /// Frames the line ran.
    clock: u64,
    /// Values an overflow dropped since the line was made (tests).
    lost: u64,
}

impl Events {
    /// A line of `capacity` values (at least 1, rounded up to a power of two), off the audio
    /// path (fallible).
    pub fn try_new(capacity: usize) -> Result<Self, MsgError> {
        let n = capacity.max(1).next_power_of_two();
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| MsgError::OutOfMemory {
            bytes: n.saturating_mul(size_of::<Pending>()),
        })?;
        v.resize(n, NOTHING);
        Ok(Events {
            ring: v.into_boxed_slice(),
            mask: n - 1,
            head: 0,
            len: 0,
            clock: CLOCK_START,
            lost: 0,
        })
    }

    /// Bytes a line of `capacity` values takes.
    pub const fn bytes_for(capacity: usize) -> u64 {
        (if capacity == 0 { 1 } else { capacity }).next_power_of_two() as u64
            * size_of::<Pending>() as u64
    }

    /// Values waiting, forgotten ones included.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing waits.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Values an overflow dropped since the line was made (tests).
    pub fn lost(&self) -> u64 {
        self.lost
    }

    fn at(&self, i: usize) -> usize {
        (self.head + i) & self.mask
    }

    fn pop(&mut self) -> Option<Pending> {
        if self.len == 0 {
            return None;
        }
        let p = self.ring[self.head];
        self.head = (self.head + 1) & self.mask;
        self.len -= 1;
        Some(p)
    }

    fn push_back(&mut self, p: Pending) {
        let i = self.at(self.len);
        self.ring[i] = p;
        self.len += 1;
    }

    /// Gives `value` to the newest value waiting for the same parameter, when one is among
    /// the last [`SUPERSEDE_SCAN`] (`false` otherwise): no older value of it comes after that
    /// one, so the parameter still ends on its newest value.
    fn supersede(&mut self, slot: DeviceSlot, id: ParamId, value: f32) -> bool {
        for back in 0..self.len.min(SUPERSEDE_SCAN) {
            let k = self.at(self.len - 1 - back);
            let p = &mut self.ring[k];
            if !p.dead && p.slot == slot && p.id == id {
                p.value = value;
                return true;
            }
        }
        false
    }

    fn drop_value(&mut self, slot: DeviceSlot, lost: &mut Lost) {
        self.lost = self.lost.saturating_add(1);
        lost.mark(slot);
    }

    /// Carries a sub-block of `frames` frames (#765): the values of `src` whose slot `way`
    /// says [`Way::Delay`] go in, dated at the line's clock plus their offset; every value due
    /// within the sub-block after `delay` frames goes into `out` (cleared first) at the offset
    /// it is due at (0 for one overdue: a shorter delay hands on at once, in order, what it
    /// skips), with the values `way` says [`Way::Pass`] at their own offsets. `true` when
    /// `out` is the source of those slots from now on; `false`, `out` untouched, when nothing
    /// waits and nothing is delayed: `src` stays theirs. No allocation.
    ///
    /// Full, the line hands its oldest value out at once (early) to make room, or drops it
    /// when `out` is full too; values due beyond `out`'s room wait (late); a value dropped
    /// has its slot marked in `lost`.
    pub fn carry(
        &mut self,
        src: &Sink,
        way: impl Fn(DeviceSlot) -> Way,
        delay: u32,
        frames: u32,
        out: &mut Sink,
        lost: &mut Lost,
    ) -> bool {
        let start = self.clock;
        self.clock = start.saturating_add(u64::from(frames));
        if self.len == 0 && delay == 0 {
            return false;
        }
        // One pass over the source: the values to delay go in (`out` cleared at the first one,
        // before an early value may go into it); whether some pass is noted.
        let mut cleared = false;
        let mut passes = false;
        for (slot, event) in src.params() {
            let Event::ParamValue { offset, id, value } = *event else {
                continue;
            };
            match way(*slot) {
                Way::Stay => continue,
                Way::Pass => {
                    passes = true;
                    continue;
                }
                Way::Delay => {}
            }
            if !cleared {
                out.clear();
                cleared = true;
            }
            if self.len == self.ring.len() {
                // Full: the newest value of the same parameter, among the last ones, takes
                // this newer one (it lands that much earlier; nothing of the parameter is
                // lost, the line does not grow).
                if self.supersede(*slot, id, value) {
                    continue;
                }
                if let Some(old) = self.pop()
                    && !old.dead
                {
                    let early = Event::ParamValue {
                        offset: 0,
                        id: old.id,
                        value: old.value,
                    };
                    if !out.push_param(old.slot, early) {
                        self.drop_value(old.slot, lost);
                    }
                }
            }
            self.push_back(Pending {
                stamp: start.saturating_add(u64::from(offset)),
                slot: *slot,
                dead: false,
                id,
                value,
            });
        }
        if !cleared {
            if self.len == 0 {
                // Nothing waits and nothing went in: the source stays the slots' own.
                return false;
            }
            out.clear();
        }
        // The values that pass, merged by offset with the ones the line hands on (a second
        // pass over the source, only when there are some).
        let mut pass = src
            .params()
            .iter()
            .filter(|(d, e)| {
                passes && matches!(e, Event::ParamValue { .. }) && way(*d) == Way::Pass
            })
            .peekable();
        let mut put = |out: &mut Sink, slot: DeviceSlot, e: Event, lines: &mut Self| {
            if !out.push_param(slot, e) {
                lines.drop_value(slot, lost);
            }
        };
        let end = self.clock;
        while self.len > 0 {
            let p = self.ring[self.head];
            if !p.dead {
                let due = p.stamp.saturating_add(u64::from(delay));
                if due >= end || out.param_room() == 0 {
                    break;
                }
                let offset = due.saturating_sub(start).min(u64::from(frames)) as u32;
                while let Some((d, e)) = pass.next_if(|(_, e)| e.offset() < offset) {
                    put(out, *d, *e, self);
                }
                if out.param_room() == 0 {
                    // The values that pass took the room: this one waits (late).
                    break;
                }
                out.push_param(
                    p.slot,
                    Event::ParamValue {
                        offset,
                        id: p.id,
                        value: p.value,
                    },
                );
            }
            self.pop();
        }
        for (d, e) in pass {
            put(out, *d, *e, self);
        }
        true
    }

    /// Forgets the values waiting for `slot` (for its parameter `id` only, when given): a
    /// gesture that holds the parameter, a device that leaves its slot. Linear in what waits,
    /// no allocation.
    pub fn forget(&mut self, slot: DeviceSlot, id: Option<ParamId>) {
        for i in 0..self.len {
            let k = self.at(i);
            let p = &mut self.ring[k];
            if p.slot == slot && id.is_none_or(|id| p.id == id) {
                p.dead = true;
            }
        }
    }

    /// The value waiting at position `i` from the oldest, when it is not forgotten, and how
    /// long ago (in the line's frames) it was dated.
    pub fn get(&self, i: usize) -> Option<(DeviceSlot, ParamId, f32, u64)> {
        if i >= self.len {
            return None;
        }
        let p = self.ring[self.at(i)];
        (!p.dead).then(|| (p.slot, p.id, p.value, self.clock.saturating_sub(p.stamp)))
    }

    /// Puts a value handed on from a line that left the chain (#765, the line before this one
    /// for its slot) after every other one: it is newer than all of them, which went through
    /// that line before it. It is dated `age` frames ago, no earlier than the newest value, so
    /// that due times never go down. Full, it gives the value to the newest one of its
    /// parameter among the last ones, as [`Events::carry`] does; `false`, and nothing changes,
    /// when there is none.
    pub fn push_moved(&mut self, slot: DeviceSlot, id: ParamId, value: f32, age: u64) -> bool {
        if self.len == self.ring.len() {
            return self.supersede(slot, id, value);
        }
        let mut stamp = self.clock.saturating_sub(age);
        if self.len > 0 {
            stamp = stamp.max(self.ring[self.at(self.len - 1)].stamp);
        }
        self.push_back(Pending {
            stamp,
            slot,
            dead: false,
            id,
            value,
        });
        true
    }

    /// Forgets everything waiting.
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
}

// ---------------------------------------------------------------- bypass

/// The bypass line of a device with a latency (module doc), and what the graph keeps of its
/// slot's state.
#[derive(Debug)]
pub struct Bypass {
    /// The signal entering the slot, read at the device's latency.
    pub(crate) line: Delay,
    /// The levels of the strip after the chain, read at the device's part of the chain's
    /// latency (#758, [`crate::graph`] > Latency): fed by the graph, never by the device.
    pub(crate) strip: Delay,
    /// The values of the devices after it in the chain, delayed by its part of the chain's
    /// latency (#765, [`crate::graph`] > Latency): fed by the graph, never by the device.
    pub(crate) events: Events,
    /// The device was last heard settled off (its fade at 0): its slot gives the line, and an
    /// outgoing one fades the line out, not the device.
    pub(crate) off: bool,
    /// Frames the device ran since it was last off: a device fading in after a reset holds
    /// its fade at 0 until it ran its latency, so that its output is never the silence of its
    /// own empty line.
    pub(crate) warm: u32,
    /// Once outgoing: how much of its own output the device gave when it left (its fade then),
    /// the rest being its line.
    pub(crate) mix: f32,
    /// Outgoing from a track's slot where no device took its line (#761): it holds the slot's
    /// timing, first fading its own output into its line (`mix` toward 0), then until the
    /// graph switched its tap to 0, the track's delay taking the difference at the same frame.
    pub(crate) hold: bool,
    /// The slot's timing moves by its own crossfade, as on the master (#761): set when the
    /// track's delay could not take a switch (a capped compensation).
    pub(crate) glide: bool,
}

impl Bypass {
    /// The line (tests).
    pub fn line(&self) -> &Delay {
        &self.line
    }

    /// The strip's line (tests).
    pub fn strip(&self) -> &Delay {
        &self.strip
    }

    /// The event line (tests).
    pub fn events(&self) -> &Events {
        &self.events
    }

    /// Whether the strip's levels must go through the strip line in the block just run (#758):
    /// the bypass line did not pass that block through (it delays, moves, or fills toward a
    /// longer tap, which the strip line must follow frame for frame), or the strip line itself
    /// is not at rest. A device with a latency of 0 that stays at 0 needs neither.
    pub fn strip_active(&self) -> bool {
        !self.line.timing().pass || !self.strip.at_rest()
    }
}

/// A device with a latency and its bypass line (module doc): every call goes to the device;
/// [`Device::bypass`] answers with the line. A method added to [`Device`] with a default must
/// be forwarded here too, or a device with a latency answers the default.
pub struct Latent {
    inner: Box<dyn Device>,
    bypass: Bypass,
}

impl core::fmt::Debug for Latent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Latent")
            .field("kind", &self.inner.descriptor().kind)
            .field("bypass", &self.bypass)
            .finish()
    }
}

impl Latent {
    /// Wraps `device` when it declares a latency ([`Device::max_latency_frames`] above 0),
    /// with a line of that many frames (at most [`limits::MAX_COMPENSATION_FRAMES`]); gives it
    /// back as it is otherwise. Off the audio path: fallible allocations. The line starts at a
    /// tap of 0 (the slot's signal before the device was there was not delayed) and the
    /// device starts off: a device inserted on a heard chain warms up before it fades in.
    pub fn wrap(device: Box<dyn Device>) -> Result<Box<dyn Device>, MsgError> {
        let max = device
            .max_latency_frames()
            .min(limits::MAX_COMPENSATION_FRAMES);
        if max == 0 {
            return Ok(device);
        }
        let latent: Box<dyn Device> = try_box(Latent {
            inner: device,
            bypass: Bypass {
                line: Delay::try_new(max, 0)?,
                strip: Delay::try_new(max, 0)?,
                events: Events::try_new(DEVICE_EVENTS)?,
                off: true,
                warm: 0,
                mix: 0.0,
                hold: false,
                glide: false,
            },
        })?;
        Ok(latent)
    }

    /// Bytes a wrapped device of at most `max_latency` frames adds to its own (the wrapper, its
    /// bypass line, its strip line and its event line): what a device with a latency counts in
    /// its [`DeviceDescriptor::memory_bytes`].
    pub const fn bytes_for(max_latency: u32) -> u64 {
        size_of::<Latent>() as u64
            + 2 * Delay::bytes_for(max_latency)
            + Events::bytes_for(DEVICE_EVENTS)
    }
}

impl Device for Latent {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        self.inner.descriptor()
    }

    fn process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo<'_>) {
        self.inner.process(ctx, events, io);
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        self.inner.set_param(id, value);
    }

    fn reset(&mut self) {
        self.inner.reset();
    }

    fn voices(&self) -> u32 {
        self.inner.voices()
    }

    fn readout(&self) -> Option<(ParamId, f32)> {
        self.inner.readout()
    }

    fn gain_reduction_db(&self) -> f32 {
        self.inner.gain_reduction_db()
    }

    /// The device's latency, as it says (the graph bounds it to [`Device::max_latency_frames`]
    /// in one place, `graph::chain_latency`, and the line bounds a tap to its size).
    fn latency_frames(&self) -> u32 {
        self.inner.latency_frames()
    }

    fn max_latency_frames(&self) -> u32 {
        self.inner
            .max_latency_frames()
            .min(self.bypass.line.max_tap())
    }

    fn bypass(&mut self) -> Option<&mut Bypass> {
        Some(&mut self.bypass)
    }
}

// ---------------------------------------------------------------- the tracks' compensation

/// The ring that delays the tracks (module doc): frames ahead of its read point, zeroed as
/// the graph takes them out.
#[derive(Debug)]
pub struct Compensation {
    left: Box<[f32]>,
    right: Box<[f32]>,
    mask: usize,
    /// The ring's index of the block's first frame.
    pos: usize,
}

impl Compensation {
    /// A ring for delays up to [`limits::MAX_COMPENSATION_FRAMES`] in blocks of up to
    /// `max_frames`. Off the audio path: fallible allocations.
    pub fn try_new(max_frames: usize) -> Result<Self, MsgError> {
        let frames = Self::frames_for(max_frames);
        Ok(Compensation {
            left: buffer(frames)?,
            right: buffer(frames)?,
            mask: frames - 1,
            pos: 0,
        })
    }

    /// Frames of a ring for blocks of up to `max_frames`.
    pub const fn frames_for(max_frames: usize) -> usize {
        (limits::MAX_COMPENSATION_FRAMES as usize + max_frames + 1).next_power_of_two()
    }

    /// Adds `left` and `right`, frames `from..` of the block, `delay` frames later, with the
    /// gain `gain(n)` for the slice's frame `n`.
    #[inline]
    fn add(
        &mut self,
        from: usize,
        delay: u32,
        left: &[f32],
        right: &[f32],
        gain: impl Fn(usize) -> f32,
    ) {
        let delay = delay.min(limits::MAX_COMPENSATION_FRAMES) as usize;
        let start = self.pos.wrapping_add(from).wrapping_add(delay);
        for (n, (l, r)) in left.iter().zip(right).enumerate() {
            let g = gain(n);
            if g != 0.0 {
                let at = start.wrapping_add(n) & self.mask;
                self.left[at] += g * *l;
                self.right[at] += g * *r;
            }
        }
    }

    /// Adds `left` and `right`, the frames of the chain that came `back` frames before the
    /// block's first one, `delay` frames later (#761, a slot's catch-up): positions at least
    /// the block's first frame when `delay >= back`, which the switch guarantees.
    fn add_before(&mut self, back: u32, delay: u32, left: &[f32], right: &[f32]) {
        let delay = delay.min(limits::MAX_COMPENSATION_FRAMES);
        let start = self.pos.wrapping_add(delay.saturating_sub(back) as usize);
        for (n, (l, r)) in left.iter().zip(right).enumerate() {
            let at = start.wrapping_add(n) & self.mask;
            self.left[at] += *l;
            self.right[at] += *r;
        }
    }

    /// Adds the block's `left.len()` frames to `left` and `right`, zeroes them in the ring and
    /// moves to the next block.
    pub fn take(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let at = self.pos.wrapping_add(i) & self.mask;
            left[i] += self.left[at];
            right[i] += self.right[at];
            self.left[at] = 0.0;
            self.right[at] = 0.0;
        }
        self.pos = self.pos.wrapping_add(frames) & self.mask;
    }

    /// Silences everything the ring holds (the panic cut): linear in its size, no allocation.
    pub fn clear(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
    }
}

/// How a track reaches the sum through the [`Compensation`] (module doc): the delay it is
/// given, and the move under way from an older one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Route {
    /// The delay, in frames.
    delay: u32,
    /// The delay a move comes from (equal to `delay` when none is under way).
    from: u32,
    /// Frames of the move done.
    done: u32,
}

impl Route {
    /// A route at `delay`, with no move.
    pub const fn at(delay: u32) -> Self {
        Route {
            delay,
            from: delay,
            done: 0,
        }
    }

    /// The delay given, or moved toward.
    pub fn delay(&self) -> u32 {
        self.delay
    }

    /// Whether a move is under way.
    pub fn moving(&self) -> bool {
        self.from != self.delay
    }

    /// Frames a move from `from` to `to` lasts with fades of `fade` frames: the longer delay
    /// starts at once and the shorter one ends after the difference, so the two cross over
    /// `fade` frames of output.
    fn length(from: u32, to: u32, fade: u32) -> u32 {
        from.abs_diff(to).saturating_add(fade.max(1))
    }

    /// Asks for `delay` (bounded to [`limits::MAX_COMPENSATION_FRAMES`]): a move starts unless
    /// one is under way (it is asked again at the next block) or the route is there.
    pub fn ask(&mut self, delay: u32) {
        let delay = delay.min(limits::MAX_COMPENSATION_FRAMES);
        if !self.moving() && delay != self.delay {
            self.from = self.delay;
            self.delay = delay;
            self.done = 0;
        }
    }

    /// Sets the route at `delay` at once: for a track that gives nothing meanwhile, or whose
    /// chain's timing moves the other way at the same frame (#761, a slot's switch).
    pub fn snap(&mut self, delay: u32) {
        *self = Route::at(delay.min(limits::MAX_COMPENSATION_FRAMES));
    }

    /// Adds the chain's frames that came `back` frames before the block (`left.len()` of them,
    /// #761: what a slot whose timing shrank gave none of) at the delay they belong to: the
    /// one before a move under way.
    pub fn add_back(&self, ring: &mut Compensation, back: u32, left: &[f32], right: &[f32]) {
        ring.add_before(back, self.from, left, right);
    }

    /// Adds the track's mixed sub-block `left`/`right` (frames `from..` of the block) to the
    /// ring along the route, a move lasting `fade`-frame fades, and moves the move on.
    pub fn add(
        &mut self,
        ring: &mut Compensation,
        from: usize,
        left: &[f32],
        right: &[f32],
        fade: u32,
    ) {
        let frames = left.len().min(right.len());
        if !self.moving() {
            ring.add(from, self.delay, left, right, |_| 1.0);
            return;
        }
        let fade = fade.max(1);
        let (old, new) = (self.from, self.delay);
        // The longer delay's frames come out later: it starts at once, the shorter one holds
        // its gain for the difference, so their fades meet in output time.
        let hold_old = new.saturating_sub(old);
        let hold_new = old.saturating_sub(new);
        let done = self.done;
        let step = 1.0 / fade as f32;
        let ramp =
            move |t: u32, hold: u32| -> f32 { (t.saturating_sub(hold) as f32 * step).min(1.0) };
        // Frame `n` of the slice is frame `done + n + 1` of the move.
        ring.add(from, old, left, right, |n| {
            1.0 - ramp(done.saturating_add(n as u32 + 1), hold_old)
        });
        ring.add(from, new, left, right, |n| {
            ramp(done.saturating_add(n as u32 + 1), hold_new)
        });
        self.done = done.saturating_add(frames as u32);
        if self.done >= Self::length(old, new, fade) {
            self.from = self.delay;
            self.done = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize, from: f32) -> Vec<f32> {
        (0..n).map(|i| from + i as f32).collect()
    }

    #[test]
    fn a_tap_delays_by_its_frames_and_a_new_line_reads_silence_first() {
        let mut d = Delay::try_new(100, 10).unwrap();
        let (mut l, mut r) = (ramp(64, 1.0), ramp(64, 1.0));
        d.process(&mut l, &mut r, 10, 4);
        assert!(l[..10].iter().all(|&s| s == 0.0));
        assert_eq!(&l[10..], &ramp(54, 1.0)[..]);
        assert_eq!(l, r);
    }

    #[test]
    fn a_jump_reads_the_new_tap_at_once_with_no_crossfade() {
        // #761: fed 0, 1, 2…, a line at tap 3 jumps to 7: its next frame is the one written 7
        // frames before it, no blend of the two taps; a jump to 0 passes the input.
        let mut d = Delay::try_new(16, 3).unwrap();
        let (mut l, mut r) = (ramp(20, 0.0), ramp(20, 0.0));
        d.process(&mut l, &mut r, 3, 8);
        assert_eq!(l[19], 16.0);
        d.jump(7);
        assert!(!d.crossfading());
        let (mut l, mut r) = (ramp(4, 20.0), ramp(4, 20.0));
        d.process(&mut l, &mut r, 7, 8);
        assert_eq!(l, vec![13.0, 14.0, 15.0, 16.0]);
        assert_eq!(d.timing().delay(), 7);
        d.jump(0);
        let (mut l, mut r) = (ramp(4, 24.0), ramp(4, 24.0));
        d.process(&mut l, &mut r, 0, 8);
        assert_eq!(l, ramp(4, 24.0));
        // Beyond what a new line holds, a jump reads silence.
        let mut fresh = Delay::try_new(16, 0).unwrap();
        fresh.jump(5);
        let (mut l, mut r) = (ramp(8, 1.0), ramp(8, 1.0));
        fresh.process(&mut l, &mut r, 5, 8);
        assert_eq!(l, vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_line_marked_as_passing_lets_its_strip_line_rest() {
        // A slot the chain stops running: its line's last timing is a pass, so a strip line
        // that follows it passes too and the bypass needs no strip line.
        let mut d = Delay::try_new(16, 4).unwrap();
        let (mut l, mut r) = (ramp(8, 1.0), ramp(8, 1.0));
        d.process(&mut l, &mut r, 4, 8);
        assert!(!d.timing().is_pass());
        d.clear();
        d.pass();
        assert!(d.timing().is_pass());
    }

    #[test]
    fn read_back_gives_the_frames_a_shorter_tap_skips_oldest_first() {
        // #761: fed 0..20 at tap 6, the line's tap shrinks by 4: the four frames nobody heard
        // are the ones written 6, 5, 4 and 3 frames ago (14..18), and 0 beyond what it holds.
        let mut d = Delay::try_new(16, 6).unwrap();
        let (mut l, mut r) = (ramp(20, 0.0), ramp(20, 0.0));
        d.process(&mut l, &mut r, 6, 8);
        let (mut bl, mut br) = (vec![9.0; 4], vec![9.0; 4]);
        d.read_back(6, &mut bl, &mut br);
        assert_eq!(bl, vec![14.0, 15.0, 16.0, 17.0]);
        assert_eq!(bl, br);
        let mut short = Delay::try_new(16, 0).unwrap();
        let (mut l, mut r) = (ramp(2, 1.0), ramp(2, 1.0));
        short.process(&mut l, &mut r, 4, 8);
        let (mut bl, mut br) = (vec![9.0; 4], vec![9.0; 4]);
        short.read_back(4, &mut bl, &mut br);
        assert_eq!(bl, vec![0.0, 0.0, 1.0, 2.0]);
    }

    #[test]
    fn frames_added_back_land_right_before_the_ones_of_the_block() {
        // #761: a route at 6 frames that jumps to 10 adds the 4 frames of before the block,
        // then the block: the ring gives them in order, with no gap and no overlap.
        let mut ring = Compensation::try_new(8).unwrap();
        let mut route = Route::at(6);
        route.add(&mut ring, 0, &ramp(8, 0.0), &ramp(8, 0.0), 4);
        let (mut l, mut r) = (vec![0.0; 8], vec![0.0; 8]);
        ring.take(&mut l, &mut r);
        assert_eq!(l, vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        route.snap(10);
        route.add_back(&mut ring, 4, &ramp(4, 8.0), &ramp(4, 8.0));
        route.add(&mut ring, 0, &ramp(8, 12.0), &ramp(8, 12.0), 4);
        let mut out = Vec::new();
        for _ in 0..3 {
            let (mut l, mut r) = (vec![0.0; 8], vec![0.0; 8]);
            ring.take(&mut l, &mut r);
            out.extend(l);
        }
        assert_eq!(&out[..13], &ramp(13, 2.0)[..]);
    }

    #[test]
    fn the_capacity_is_a_power_of_two_above_the_longest_tap() {
        let d = Delay::try_new(100, 0).unwrap();
        assert_eq!(d.max_tap(), 127);
        assert_eq!(Delay::bytes_for(100), 128 * 8);
        assert_eq!(Delay::try_new(127, 0).unwrap().max_tap(), 127);
        assert_eq!(Delay::try_new(128, 0).unwrap().max_tap(), 255);
    }

    #[test]
    fn a_change_of_tap_crossfades_and_never_jumps() {
        // A constant signal: both taps read the same value once the line holds them, so the
        // crossfade must leave it as it is.
        let mut d = Delay::try_new(64, 8).unwrap();
        let (mut l, mut r) = (vec![1.0; 32], vec![1.0; 32]);
        d.process(&mut l, &mut r, 8, 16);
        let (mut l, mut r) = (vec![1.0; 64], vec![1.0; 64]);
        d.process(&mut l, &mut r, 20, 16);
        assert!(l.iter().all(|&s| (s - 1.0).abs() < 1e-6), "{l:?}");
        assert!(!d.crossfading());
        assert_eq!(d.tap(), 20);
    }

    #[test]
    fn a_longer_tap_waits_for_the_line_to_hold_it() {
        let mut d = Delay::try_new(64, 0).unwrap();
        let (mut l, mut r) = (vec![1.0; 8], vec![1.0; 8]);
        // At tap 0 the line was a pass-through and holds nothing: the change waits, and the
        // line fills meanwhile, still read at tap 0.
        d.process(&mut l, &mut r, 30, 4);
        assert_eq!(d.tap(), 0);
        assert_eq!(l, vec![1.0; 8]);
        for _ in 0..3 {
            let (mut l, mut r) = (vec![1.0; 8], vec![1.0; 8]);
            d.process(&mut l, &mut r, 30, 4);
            assert_eq!(l, vec![1.0; 8]);
        }
        // 32 frames held: the crossfade starts.
        let (mut l, mut r) = (vec![1.0; 8], vec![1.0; 8]);
        d.process(&mut l, &mut r, 30, 4);
        assert_eq!(d.tap(), 30);
        assert!(l.iter().all(|&s| (s - 1.0).abs() < 1e-6), "{l:?}");

        let mut d = Delay::try_new(64, 4).unwrap();
        let (mut l, mut r) = (vec![1.0; 8], vec![1.0; 8]);
        d.process(&mut l, &mut r, 4, 4);
        // 8 frames held: 30 waits, 6 does not.
        let (mut l, mut r) = (vec![1.0; 4], vec![1.0; 4]);
        d.process(&mut l, &mut r, 30, 4);
        assert_eq!(d.tap(), 4);
        d.process(&mut l, &mut r, 6, 4);
        assert_eq!(d.tap(), 6);
    }

    #[test]
    fn a_change_during_a_crossfade_waits_for_its_end() {
        let mut d = Delay::try_new(64, 2).unwrap();
        let (mut l, mut r) = (vec![1.0; 40], vec![1.0; 40]);
        d.process(&mut l, &mut r, 2, 8);
        let (mut l, mut r) = (vec![1.0; 4], vec![1.0; 4]);
        d.process(&mut l, &mut r, 10, 8);
        assert!(d.crossfading());
        d.process(&mut l, &mut r, 20, 8);
        assert_eq!(d.tap(), 10, "the second change waits");
        assert!(!d.crossfading(), "the first crossfade ended in that block");
        d.process(&mut l, &mut r, 20, 8);
        assert_eq!(d.tap(), 20);
        assert!(l.iter().all(|&s| (s - 1.0).abs() < 1e-6), "{l:?}");
    }

    #[test]
    fn a_line_that_follows_another_reads_at_its_taps_whatever_its_own_content() {
        // The leader carries sound with silences (its content makes a longer tap wait); the
        // follower carries a constant level, which alone would never wait. Block by block the
        // follower reads at the leader's taps with the leader's crossfade: fed the leader's
        // own sound it gives the leader's output, sample for sample.
        let mut leader = Delay::try_new(64, 0).unwrap();
        let mut twin = Delay::try_new(64, 0).unwrap();
        let mut level = Delay::try_new(64, 0).unwrap();
        let mut alone = Delay::try_new(64, 0).unwrap();
        let mut differ = false;
        for block in 0..40usize {
            let sound: Vec<f32> = (0..8)
                .map(|i| {
                    if block % 4 == 1 {
                        0.0
                    } else {
                        (block * 8 + i) as f32
                    }
                })
                .collect();
            let target = match block {
                0..10 => 0,
                10..25 => 30,
                _ => 12,
            };
            let (mut l, mut r) = (sound.clone(), sound.clone());
            leader.process(&mut l, &mut r, target, 6);
            let timing = leader.timing();
            let (mut tl, mut tr) = (sound.clone(), sound.clone());
            twin.follow(&mut tl, &mut tr, timing, 6);
            assert_eq!(tl, l, "block {block}: the twin gives the leader's output");
            assert_eq!(tr, r);
            let (mut ll, mut lr) = (vec![1.0; 8], vec![1.0; 8]);
            level.follow(&mut ll, &mut lr, timing, 6);
            assert_eq!(level.tap(), leader.tap(), "block {block}");
            assert_eq!(level.crossfading(), leader.crossfading(), "block {block}");
            let (mut al, mut ar) = (vec![1.0; 8], vec![1.0; 8]);
            alone.process(&mut al, &mut ar, target, 6);
            differ |= alone.tap() != leader.tap();
        }
        assert!(
            differ,
            "the control: on its own content the level line moves elsewhen"
        );
        let mut pass = Delay::try_new(64, 0).unwrap();
        let (mut l, mut r) = (vec![1.0; 8], vec![1.0; 8]);
        pass.follow(&mut l, &mut r, Timing::PASS, 6);
        assert_eq!(l, vec![1.0; 8], "a pass-through");
        assert!(!pass.busy());
    }

    #[test]
    fn a_primed_line_starts_a_longer_tap_at_once_on_its_value() {
        let mut d = Delay::try_new(64, 0).unwrap();
        d.prime(0.5, 0.25, 40);
        let (mut l, mut r) = (vec![1.0; 8], vec![1.0; 8]);
        d.process(&mut l, &mut r, 30, 4);
        assert_eq!(d.tap(), 30, "the history is there: no wait");
        // The crossfade from tap 0 (the fresh 1.0) toward the primed 0.5, then the primed value.
        assert_eq!(l[0], 1.0 + 0.25 * (0.5 - 1.0));
        assert_eq!(&l[3..], &[0.5; 5]);
        assert_eq!(&r[3..], &[0.25; 5]);
        assert!(d.busy());
        let mut silent = Delay::try_new(64, 0).unwrap();
        silent.prime(0.0, 0.0, 40);
        assert!(!silent.busy(), "a history of silence is silence");
    }

    #[test]
    fn a_bypass_with_a_latency_of_0_needs_no_strip_line() {
        let mut b = Bypass {
            line: Delay::try_new(64, 0).unwrap(),
            strip: Delay::try_new(64, 0).unwrap(),
            events: Events::try_new(4).unwrap(),
            off: false,
            warm: 0,
            mix: 0.0,
            hold: false,
            glide: false,
        };
        assert!(!b.strip_active(), "a new line passed nothing yet");
        let (mut l, mut r) = (vec![1.0; 8], vec![1.0; 8]);
        b.line.process(&mut l, &mut r, 0, 4);
        assert!(!b.strip_active(), "a pass-through");
        // Asked for a longer tap, the line fills (reading at 0): the strip line must follow.
        b.line.process(&mut l, &mut r, 20, 4);
        assert!(b.strip_active(), "filling toward 20");
        let mut lines = b;
        lines.line.restart(0);
        lines.line.process(&mut l, &mut r, 0, 4);
        lines.strip.restart(10);
        assert!(lines.strip_active(), "the strip line still delays");
        lines.strip.restart(0);
        assert!(!lines.strip_active());
    }

    #[test]
    fn a_clear_hides_the_content_in_constant_time() {
        let mut d = Delay::try_new(64, 10).unwrap();
        let (mut l, mut r) = (vec![1.0; 32], vec![1.0; 32]);
        d.process(&mut l, &mut r, 10, 4);
        d.clear();
        let (mut l, mut r) = (vec![0.5; 16], vec![0.5; 16]);
        d.process(&mut l, &mut r, 10, 4);
        assert!(l[..10].iter().all(|&s| s == 0.0), "{l:?}");
        assert!(l[10..].iter().all(|&s| s == 0.5));
    }

    #[test]
    fn a_line_left_unfed_after_its_silence_plays_no_stale_content() {
        let mut d = Delay::try_new(64, 4).unwrap();
        // Signal, then more than the tap of silence: the line is no longer busy.
        let (mut l, mut r) = (vec![1.0; 16], vec![1.0; 16]);
        d.process(&mut l, &mut r, 4, 4);
        assert!(d.busy());
        let (mut l, mut r) = (vec![0.0; 8], vec![0.0; 8]);
        d.process(&mut l, &mut r, 4, 4);
        assert!(!d.busy());
        // Not fed for a while, then a longer tap: the 1.0 still in the ring 20 frames back
        // is never read; the crossfade waits for 20 frames of the line's own silence.
        let (mut l, mut r) = (vec![0.0; 4], vec![0.0; 4]);
        d.process(&mut l, &mut r, 20, 4);
        assert_eq!(d.tap(), 4, "the longer tap waits");
        assert!(l.iter().all(|&s| s == 0.0));
        let (mut l, mut r) = (vec![0.0; 40], vec![0.0; 40]);
        d.process(&mut l, &mut r, 20, 4);
        assert_eq!(d.tap(), 4, "16 frames of silence held: still waiting");
        let (mut l, mut r) = (vec![0.0; 40], vec![0.0; 40]);
        d.process(&mut l, &mut r, 20, 4);
        assert_eq!(d.tap(), 20);
        assert!(l.iter().all(|&s| s == 0.0), "{l:?}");
    }

    /// Feeds a constant 1.0 along `route` in blocks of `block` for `blocks` blocks, asking for
    /// `to` from the first block on; returns what came out of the ring.
    fn feed(route: &mut Route, to: u32, block: usize, blocks: usize, fade: u32) -> Vec<f32> {
        let mut ring = Compensation::try_new(block).unwrap();
        let mut out = Vec::new();
        let ones = vec![1.0f32; block];
        for _ in 0..blocks {
            route.ask(to);
            route.add(&mut ring, 0, &ones, &ones, fade);
            let (mut l, mut r) = (vec![0.0; block], vec![0.0; block]);
            ring.take(&mut l, &mut r);
            assert_eq!(l, r);
            out.extend(l);
        }
        out
    }

    #[test]
    fn a_move_crosses_in_output_time_with_no_gap_and_nothing_twice() {
        // A constant signal: once both delays carry it, the output must stay at 1 through
        // the move, whatever its length against the fade. A gap would dip, a doubling go
        // above 1.
        for (from, to) in [
            (0u32, 2000u32),
            (2000, 0),
            (100, 150),
            (150, 100),
            (0, 240),
            (300, 60),
        ] {
            let mut route = Route::at(from);
            let out = feed(&mut route, to, 128, 60, 240);
            let settled = from.max(to) as usize + 1;
            for (i, s) in out.iter().enumerate().skip(settled) {
                assert!((s - 1.0).abs() < 1e-5, "{from} → {to}: frame {i} is {s}");
            }
            assert!(!route.moving(), "{from} → {to}: the move ended");
            assert_eq!(route.delay(), to);
        }
    }

    #[test]
    fn a_route_delays_by_its_frames_and_waits_for_the_end_of_a_move() {
        let mut ring = Compensation::try_new(16).unwrap();
        let mut route = Route::at(5);
        let ramp: Vec<f32> = (1..=16).map(|i| i as f32).collect();
        route.add(&mut ring, 0, &ramp, &ramp, 4);
        let (mut l, mut r) = (vec![0.0; 16], vec![0.0; 16]);
        ring.take(&mut l, &mut r);
        assert!(l[..5].iter().all(|&s| s == 0.0));
        assert_eq!(l[5], 1.0);
        assert_eq!(l[15], 11.0);
        // A move under way: a new delay waits.
        route.ask(9);
        assert!(route.moving());
        route.ask(30);
        assert_eq!(route.delay(), 9);
        // Bounded to the limit.
        let mut far = Route::at(0);
        far.snap(u32::MAX);
        assert_eq!(far.delay(), limits::MAX_COMPENSATION_FRAMES);
        // A clear silences what is ahead.
        route.add(&mut ring, 0, &ramp, &ramp, 4);
        ring.clear();
        let (mut l, mut r) = (vec![0.0; 16], vec![0.0; 16]);
        ring.take(&mut l, &mut r);
        assert!(l.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn restart_sets_the_tap_at_once_on_an_empty_line() {
        let mut d = Delay::try_new(64, 0).unwrap();
        d.restart(12);
        assert_eq!(d.tap(), 12);
        assert!(!d.crossfading());
        let (mut l, mut r) = (ramp(20, 1.0), ramp(20, 1.0));
        d.process(&mut l, &mut r, 12, 4);
        assert!(l[..12].iter().all(|&s| s == 0.0));
        assert_eq!(l[12], 1.0);
        // Beyond the capacity: bounded.
        d.restart(10_000);
        assert_eq!(d.tap(), d.max_tap());
    }

    // ------------------------------------------------------------ event lines (#765)

    fn value(offset: u32, id: u32, value: f32) -> Event {
        Event::ParamValue {
            offset,
            id: ParamId(id),
            value,
        }
    }

    /// A sink of the values `(offset, slot, id, value)`.
    fn values(list: &[(u32, u16, u32, f32)]) -> Sink {
        let mut s = Sink::try_with_capacity(0, 64).unwrap();
        for &(o, d, id, v) in list {
            assert!(s.push_param(DeviceSlot(d), value(o, id, v)));
        }
        s
    }

    /// The values of `out` as `(offset, slot, id, value)`.
    fn seen(out: &Sink) -> Vec<(u32, u16, u32, f32)> {
        out.params()
            .iter()
            .map(|(d, e)| match *e {
                Event::ParamValue { offset, id, value } => (offset, d.0, id.0, value),
                _ => panic!("{e:?}"),
            })
            .collect()
    }

    fn delay_all(_: DeviceSlot) -> Way {
        Way::Delay
    }

    #[test]
    fn a_value_lands_its_delay_later_in_order() {
        let mut line = Events::try_new(16).unwrap();
        let mut out = Sink::try_with_capacity(0, 16).unwrap();
        let mut lost = Lost::default();
        let src = values(&[(1, 2, 0, 0.5), (3, 2, 0, 0.7), (3, 3, 1, 0.1)]);
        // Delay 10 over sub-blocks of 8: due at 11, 13 and 13, the second sub-block's 3 and 5.
        assert!(line.carry(&src, delay_all, 10, 8, &mut out, &mut lost));
        assert!(seen(&out).is_empty());
        let empty = values(&[]);
        assert!(line.carry(&empty, delay_all, 10, 8, &mut out, &mut lost));
        assert_eq!(
            seen(&out),
            vec![(3, 2, 0, 0.5), (5, 2, 0, 0.7), (5, 3, 1, 0.1)]
        );
        assert!(line.is_empty());
        // Nothing waits and nothing goes in: the source stays the slots' own.
        assert!(!line.carry(&empty, delay_all, 10, 8, &mut out, &mut lost));
        assert!(lost.is_empty());
    }

    #[test]
    fn with_no_delay_and_nothing_waiting_the_source_is_kept_without_a_copy() {
        let mut line = Events::try_new(16).unwrap();
        let mut out = Sink::try_with_capacity(0, 16).unwrap();
        let mut lost = Lost::default();
        assert!(out.push_param(DeviceSlot(9), value(0, 0, 9.0)));
        let src = values(&[(1, 2, 0, 0.5)]);
        assert!(!line.carry(&src, delay_all, 0, 8, &mut out, &mut lost));
        assert_eq!(seen(&out), vec![(0, 9, 0, 9.0)], "untouched");
        // A delay with no value for the next slots: kept too.
        let stay = |_: DeviceSlot| Way::Stay;
        assert!(!line.carry(&src, stay, 10, 8, &mut out, &mut lost));
    }

    #[test]
    fn a_shorter_delay_hands_on_at_once_what_it_skips_and_a_longer_one_holds() {
        let mut line = Events::try_new(16).unwrap();
        let mut out = Sink::try_with_capacity(0, 16).unwrap();
        let mut lost = Lost::default();
        let empty = values(&[]);
        assert!(line.carry(
            &values(&[(0, 2, 0, 1.0), (6, 2, 0, 2.0)]),
            delay_all,
            20,
            8,
            &mut out,
            &mut lost
        ));
        // The delay drops to 0: both are overdue, at offset 0, in order.
        assert!(line.carry(&empty, delay_all, 0, 8, &mut out, &mut lost));
        assert_eq!(seen(&out), vec![(0, 2, 0, 1.0), (0, 2, 0, 2.0)]);
        // A value at 4 under a delay of 4 lands at 8 (the next sub-block's 0); the delay grows
        // to 12 before it does: it lands at 16.
        assert!(line.carry(
            &values(&[(4, 2, 0, 3.0)]),
            delay_all,
            4,
            8,
            &mut out,
            &mut lost
        ));
        assert!(seen(&out).is_empty());
        assert!(line.carry(&empty, delay_all, 12, 8, &mut out, &mut lost));
        assert!(seen(&out).is_empty(), "held for the longer delay");
        assert!(line.carry(&empty, delay_all, 12, 8, &mut out, &mut lost));
        assert_eq!(seen(&out), vec![(0, 2, 0, 3.0)]);
    }

    #[test]
    fn the_values_of_other_roles_pass_at_their_offsets_and_the_others_stay() {
        let mut line = Events::try_new(16).unwrap();
        let mut out = Sink::try_with_capacity(0, 16).unwrap();
        let mut lost = Lost::default();
        // Slot 1 is before the line, 2 an effect after it, 3 an instrument after it.
        let way = |d: DeviceSlot| match d.0 {
            2 => Way::Delay,
            3 => Way::Pass,
            _ => Way::Stay,
        };
        let src = values(&[
            (0, 1, 0, 1.0),
            (1, 2, 0, 2.0),
            (2, 3, 0, 3.0),
            (7, 3, 0, 4.0),
        ]);
        assert!(line.carry(&src, way, 4, 8, &mut out, &mut lost));
        // The effect's value lands at 5, between the instrument's two.
        assert_eq!(
            seen(&out),
            vec![(2, 3, 0, 3.0), (5, 2, 0, 2.0), (7, 3, 0, 4.0)]
        );
    }

    #[test]
    fn a_full_line_keeps_the_newest_value_of_a_parameter_then_hands_out_early_then_drops() {
        let mut line = Events::try_new(4).unwrap();
        let mut out = Sink::try_with_capacity(0, 2).unwrap();
        let mut lost = Lost::default();
        // Four values fill the line; a fifth of the same parameter as the newest takes its place.
        let src = values(&[
            (0, 2, 0, 1.0),
            (0, 2, 1, 1.0),
            (0, 2, 2, 1.0),
            (0, 2, 3, 1.0),
            (1, 2, 3, 5.0),
        ]);
        assert!(line.carry(&src, delay_all, 100, 8, &mut out, &mut lost));
        assert_eq!(line.len(), 4);
        assert!(seen(&out).is_empty() && lost.is_empty());
        // Three of new parameters: the two oldest go out at once (early), then `out` is full
        // and the third oldest is dropped, its slot marked.
        let src = values(&[(0, 4, 0, 1.0), (0, 5, 0, 1.0), (0, 6, 0, 1.0)]);
        assert!(line.carry(&src, delay_all, 100, 8, &mut out, &mut lost));
        assert_eq!(seen(&out), vec![(0, 2, 0, 1.0), (0, 2, 1, 1.0)]);
        assert_eq!(line.lost(), 1);
        assert_eq!(lost.slots().collect::<Vec<_>>(), vec![DeviceSlot(2)]);
        // What waits: the newest of slot 2 (5.0 for parameter 3) and the three new ones.
        let after: Vec<_> = (0..line.len()).filter_map(|i| line.get(i)).collect();
        assert_eq!(after.len(), 4);
        assert_eq!(
            (after[0].0, after[0].1, after[0].2),
            (DeviceSlot(2), ParamId(3), 5.0)
        );
    }

    #[test]
    fn a_forgotten_value_never_lands() {
        let mut line = Events::try_new(8).unwrap();
        let mut out = Sink::try_with_capacity(0, 8).unwrap();
        let mut lost = Lost::default();
        let src = values(&[(0, 2, 0, 1.0), (0, 2, 1, 2.0), (0, 3, 0, 3.0)]);
        assert!(line.carry(&src, delay_all, 12, 8, &mut out, &mut lost));
        line.forget(DeviceSlot(2), Some(ParamId(1)));
        line.forget(DeviceSlot(3), None);
        assert!(line.carry(&values(&[]), delay_all, 12, 8, &mut out, &mut lost));
        assert_eq!(seen(&out), vec![(4, 2, 0, 1.0)]);
        assert!(!line.carry(&values(&[]), delay_all, 12, 8, &mut out, &mut lost));
        assert!(line.is_empty());
    }

    #[test]
    fn a_value_handed_on_keeps_its_age_and_lands_after_the_older_ones() {
        let mut line = Events::try_new(8).unwrap();
        let mut out = Sink::try_with_capacity(0, 8).unwrap();
        let mut lost = Lost::default();
        let empty = values(&[]);
        // The line ran 96 frames: its clock is not the one the value was dated on.
        for _ in 0..12 {
            line.carry(&empty, delay_all, 72, 8, &mut out, &mut lost);
        }
        // Dated 30 frames ago, under a delay of 72: due 42 frames from now (the sixth
        // sub-block, at 2).
        assert!(line.push_moved(DeviceSlot(2), ParamId(0), 0.5, 30));
        let mut at = None;
        for n in 0..8 {
            if !line.carry(&empty, delay_all, 72, 8, &mut out, &mut lost) {
                break;
            }
            if let Some(&(o, ..)) = seen(&out).first() {
                at = Some(n * 8 + o);
                break;
            }
        }
        assert_eq!(at, Some(42), "its age is kept on the new clock");
        // After a newer value of the line (dated 10 frames ago), one dated 50 frames ago is not
        // due before it: due times never go down.
        assert!(line.carry(
            &values(&[(0, 2, 0, 1.0)]),
            delay_all,
            72,
            8,
            &mut out,
            &mut lost
        ));
        assert!(line.push_moved(DeviceSlot(2), ParamId(0), 2.0, 50));
        let mut order = Vec::new();
        for _ in 0..12 {
            if line.carry(&empty, delay_all, 72, 8, &mut out, &mut lost) {
                order.extend(seen(&out).iter().map(|v| v.3));
            }
        }
        assert_eq!(order, vec![1.0, 2.0], "the handed-on value comes last");
        // Full, it gives the value to the newest one of its parameter, or refuses.
        let mut full = Events::try_new(1).unwrap();
        assert!(full.push_moved(DeviceSlot(2), ParamId(0), 1.0, 0));
        assert!(full.push_moved(DeviceSlot(2), ParamId(0), 2.0, 0));
        assert_eq!(full.len(), 1);
        assert_eq!(full.get(0).map(|v| v.2), Some(2.0));
        assert!(!full.push_moved(DeviceSlot(3), ParamId(0), 3.0, 0));
    }

    #[test]
    fn a_timing_delays_by_its_tap_or_not_at_all_when_it_passes() {
        assert_eq!(Timing::PASS.delay(), 0);
        let mut d = Delay::try_new(64, 0).unwrap();
        let (mut l, mut r) = (vec![1.0; 80], vec![1.0; 80]);
        d.process(&mut l, &mut r, 0, 8);
        assert_eq!(d.timing().delay(), 0);
        // The pass-through kept nothing: the longer tap waits for its frames, and so do the
        // values that follow the line.
        d.process(&mut l, &mut r, 20, 8);
        assert_eq!(d.timing().delay(), 0);
        // Once the line holds them, the crossfade toward 20 starts (and ends within the
        // block): the values take 20 from its start.
        d.process(&mut l, &mut r, 20, 8);
        assert_eq!(d.tap(), 20);
        assert_eq!(d.timing().delay(), 20);
    }

    #[test]
    fn a_young_line_keeps_the_age_of_a_value_older_than_itself() {
        // A line made just now receives a value dated 30 frames ago (#799 > push_moved): under
        // a delay of 72 it is due 42 frames from now, not 72 (a stamp that saturated at the
        // line's birth would make it late).
        let mut line = Events::try_new(8).unwrap();
        let mut out = Sink::try_with_capacity(0, 8).unwrap();
        let mut lost = Lost::default();
        assert!(line.push_moved(DeviceSlot(2), ParamId(0), 0.5, 30));
        let empty = values(&[]);
        let mut at = None;
        for n in 0..12 {
            if !line.carry(&empty, delay_all, 72, 8, &mut out, &mut lost) {
                break;
            }
            if let Some(&(o, ..)) = seen(&out).first() {
                at = Some(n * 8 + o);
                break;
            }
        }
        assert_eq!(at, Some(42));
    }

    #[test]
    fn a_value_for_any_slot_lands_a_forgotten_one_never() {
        // The last slot number is a slot like another (#799: no sentinel slot).
        let mut line = Events::try_new(8).unwrap();
        let mut out = Sink::try_with_capacity(0, 8).unwrap();
        let mut lost = Lost::default();
        let src = values(&[(0, u16::MAX, 0, 1.0), (0, 3, 0, 2.0)]);
        assert!(line.carry(&src, delay_all, 4, 8, &mut out, &mut lost));
        assert_eq!(seen(&out), vec![(4, u16::MAX, 0, 1.0), (4, 3, 0, 2.0)]);
        assert!(line.carry(&src, delay_all, 12, 8, &mut out, &mut lost));
        line.forget(DeviceSlot(u16::MAX), None);
        assert!(line.carry(&values(&[]), delay_all, 12, 8, &mut out, &mut lost));
        assert_eq!(seen(&out), vec![(4, 3, 0, 2.0)]);
    }

    #[test]
    fn a_ring_of_any_capacity_wraps_in_order() {
        // A capacity rounds up to a power of two (#799: a mask, no modulo); values go round the
        // ring many times and come out in order.
        let mut line = Events::try_new(5).unwrap();
        assert_eq!(Events::bytes_for(5), Events::bytes_for(8));
        let mut out = Sink::try_with_capacity(0, 16).unwrap();
        let mut lost = Lost::default();
        let mut got = Vec::new();
        for n in 0..40u16 {
            let src = values(&[(0, n % 7, 0, f32::from(n)), (1, 9, 1, f32::from(n) + 0.5)]);
            if line.carry(&src, delay_all, 9, 4, &mut out, &mut lost) {
                got.extend(seen(&out).iter().map(|v| v.3));
            }
        }
        let want: Vec<f32> = (0..38u16)
            .flat_map(|n| [f32::from(n), f32::from(n) + 0.5])
            .take(got.len())
            .collect();
        assert!(got.len() > 60, "{}", got.len());
        assert_eq!(got, want);
        assert!(lost.is_empty());
        assert_eq!(line.lost(), 0);
    }
}
