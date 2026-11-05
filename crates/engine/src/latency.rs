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
//! At a tap of 0 with no change under way the line is a pass-through and holds nothing.
//!
//! A [`Latent`] wraps a device with a latency and holds its [`Bypass`]: a [`Delay`] the
//! graph feeds with the signal entering the device's slot and reads at the device's latency,
//! so that a bypassed device keeps its latency and its fades blend two signals of one timing
//! ([`crate::graph`] > Latency). The [`Bypass`] holds a second line of the same size, its
//! `strip`: the levels of the strip after the chain (gate × gain × pan, one pair per frame),
//! delayed as the slot's sound is, so that a timed gain, pan or mute lands on the sound it
//! belongs to (#758). It [`Delay::follow`]s the bypass line: whether a longer tap may start
//! depends on a line's content, and only the line of sound knows what its slot gives.
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
use crate::msg::{MsgError, ParamId, limits, try_box};

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
    /// A pass-through: no delay, nothing kept.
    pub const PASS: Timing = Timing {
        tap: 0,
        from: 0,
        g: 1.0,
        pass: true,
    };
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
                off: true,
                warm: 0,
                mix: 0.0,
            },
        })?;
        Ok(latent)
    }

    /// Bytes a wrapped device of at most `max_latency` frames adds to its own (the wrapper, its
    /// bypass line and its strip line): what a device with a latency counts in its
    /// [`DeviceDescriptor::memory_bytes`].
    pub const fn bytes_for(max_latency: u32) -> u64 {
        size_of::<Latent>() as u64 + 2 * Delay::bytes_for(max_latency)
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

    /// Sets the route at `delay` at once: for a track that gives nothing meanwhile.
    pub fn snap(&mut self, delay: u32) {
        *self = Route::at(delay.min(limits::MAX_COMPENSATION_FRAMES));
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
}
