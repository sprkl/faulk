//! The mixer of the engine (MX-1, `docs/specs/02-decoupage-mvp.md` > Mise en œuvre MX-1):
//! the strip of a track and of the master, with its pan law and its timed parameters.
//!
//! # A strip
//!
//! A strip holds a gain (linear), a pan (−1 left to +1 right) and a mute, and turns them
//! into two levels, one per channel, with
//! `level = audible × gain × balance(pan)` ([`mix_levels`]). `audible` is false for a track
//! that another soloed track silences (solo is cumulative: every soloed track plays); the
//! graph computes it. A gain that is negative, infinite or NaN is silence; a pan that is not
//! finite is the centre.
//!
//! # Pan law
//!
//! [`balance`] keeps unity at the centre and lowers the **opposite** side only, with a
//! quarter cosine: `left = cos(pan · π/2)` for `pan > 0`, `right = cos(−pan · π/2)` for
//! `pan < 0`, the near side stays at unity, and the far side is exactly 0 at the hard pan.
//! Why this one:
//!
//! - unity at the centre, so that "−6 dB requested" is −6 dB measured on a centred track and
//!   on a stereo source (a law with −3 dB or −4.5 dB at the centre would fail that);
//! - a stereo signal is never summed to one side before the pan (a "pan" that moves the
//!   whole stereo image, which attenuates one channel and boosts the other, is a different
//!   tool): the far channel goes down, the near channel does not change;
//! - the far channel is −3 dB at half pan (a straight line would be −6 dB) and the slope is
//!   zero at the centre, so the first degrees of the knob do not jump in level; the total
//!   power goes from 2 (centre) to 1 (hard pan), −3 dB.
//!
//! # Timed parameters: the same mechanism as a device's
//!
//! The gain, the pan and the mute of a track are addressed like a device's parameters: a
//! [`Event::ParamValue`] in the track's schedule ([`crate::events::ScheduledParam`]) whose
//! device is [`MIXER_SLOT`] and whose id is [`GAIN`], [`PAN`] or [`MUTE`]. The master's are
//! the same values in the master's schedule ([`crate::engine::State::set_master_schedule`]).
//! The automation (AM-2, [`crate::automation`]) sends its values to the strip the same way,
//! through the track's sink: [`GAIN`] or [`PAN`] at the first frame of each sub-block, and
//! [`GAIN_RAMP`] or [`PAN_RAMP`] at its last frame while the lane moves, so that the levels
//! draw a straight line between the two (a volume lane sends chords of its curve along the
//! fader's travel, see [`crate::automation`]).
//!
//! A timed gain or pan takes effect **at its sample**, exactly: the automation's curve is
//! the smoothing, a ramp the lane draws is followed to the sample, and the engine adds no
//! delay or second ramp to it. A timed mute (value ≥ 0.5) and every change a message makes
//! (a fader, a mute button, a solo) move by a linear ramp of [`crate::graph::FADE_SECONDS`]
//! (the smoothing rule of CT-5: a new target starts a new ramp from the current value), so
//! that nothing clicks. A step between two points of a lane with the `Step` shape is a jump
//! of the level: it is what the lane says.
//!
//! A message changes a field of the strip only when that field changed since the last
//! message (a gesture on the fader), so a lane's value is not overwritten at every block
//! nor by a gesture on another field; a gesture on a field a lane drives holds the lane
//! until its next point ([`crate::automation`]). When a lane is gone (its schedule
//! replaced, a `Reset`) the strip goes back to the faders ([`Mixer::forget`]), a lane
//! removed gives back its field only ([`Mixer::forget_gain`], [`Mixer::forget_pan`]).
//!
//! The strip keeps mute, solo and the fades in and out on a separate **gate** from the
//! gain and the pan, so that a lane is followed to the sample even while a mute or a solo
//! fades, and a new track's fade in does not smooth the lane.
//!
//! Nothing here allocates, locks or panics.

use core::f32::consts::FRAC_PI_2;

use crate::device::Event;
use crate::graph::MeterAcc;
use crate::msg::{DeviceSlot, ParamId};
use crate::param::Smoother;

/// The pseudo device slot that addresses the strip of a track (or of the master) in a
/// schedule: no chain has 65 535 devices ([`crate::msg::limits::MAX_DEVICES_PER_TRACK`]).
pub const MIXER_SLOT: DeviceSlot = DeviceSlot(u16::MAX);

/// Linear gain, 0 for silence (the project's dB values are converted by the sender).
pub const GAIN: ParamId = ParamId(0);
/// Pan, −1 (left) to +1 (right).
pub const PAN: ParamId = ParamId(1);
/// Mute: 1 (or any value ≥ 0.5) mutes, 0 does not.
pub const MUTE: ParamId = ParamId(2);
/// A linear gain reached **at** this sample, in a straight line from the strip's level at
/// the previous timed value of the sub-block (or at its start): the end of a ramp an
/// automation lane draws (AM-2, [`crate::automation`]).
pub const GAIN_RAMP: ParamId = ParamId(3);
/// A pan reached at this sample, like [`GAIN_RAMP`]: the levels move in a straight line
/// between the two pans' levels.
pub const PAN_RAMP: ParamId = ParamId(4);

/// The levels of a strip: `audible × gain × balance(pan)` (module doc).
pub fn mix_levels(gain: f32, pan: f32, audible: bool) -> (f32, f32) {
    if !audible || !gain.is_finite() || gain <= 0.0 {
        return (0.0, 0.0);
    }
    let (l, r) = balance(pan);
    (gain * l, gain * r)
}

/// The pan law (module doc): the gains of the left and of the right channel, each in 0 to 1
/// and one of them 1.
pub fn balance(pan: f32) -> (f32, f32) {
    let pan = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let far = if pan.abs() >= 1.0 {
        0.0
    } else {
        (pan.abs() * FRAC_PI_2).cos()
    };
    if pan < 0.0 { (1.0, far) } else { (far, 1.0) }
}

/// The timed parameters of a strip in a sub-block: `(offset, id, value)`, by offset.
pub type TimedParam = (u32, ParamId, f32);

/// The strip's timed parameters among the values of a sink (the ones addressed to
/// [`MIXER_SLOT`]).
pub fn timed(params: &[(DeviceSlot, Event)]) -> impl Iterator<Item = TimedParam> + '_ {
    params.iter().filter_map(|(slot, e)| match *e {
        Event::ParamValue { offset, id, value } if *slot == MIXER_SLOT => Some((offset, id, value)),
        _ => None,
    })
}

/// A strip: the values, the ramps of the levels and the gate.
///
/// The level of a channel is `gate × level`. The **gate** (0 or 1) carries what must
/// fade whatever else moves: mute, solo, the fade in of a new track, the fade out of a
/// track that leaves. The **levels** carry the gain and the pan: a message moves them by a
/// ramp, a timed value sets them at its sample. Because the two are separate, a lane's
/// gain is followed to the sample even while a mute or a solo is fading, and a fade in
/// never smooths a lane.
#[derive(Debug)]
pub struct Mixer {
    gain: f32,
    pan: f32,
    muted: bool,
    audible: bool,
    /// The message values the strip last took (`None`: not yet, or forgotten), as bits so
    /// that a NaN equals itself. Each field is compared alone: a message that changes the
    /// mute does not overwrite a gain a lane set.
    seen_gain: Option<u32>,
    seen_pan: Option<u32>,
    seen_mute: Option<bool>,
    /// The strip has taken a message at least once (a new strip jumps to its levels and
    /// fades in by the gate).
    synced: bool,
    /// Frames left of the ramp a message started on the levels by moving the gain, resp.
    /// the pan: a timed value of the **other** field that arrives meanwhile (a lane on it)
    /// glides to its level over them instead of jumping, so that the gesture keeps its
    /// ramp (AM-2). A timed value of the same field is exact, as ever: the lane wins.
    glide_gain: u32,
    glide_pan: u32,
    gate: Smoother,
    left: Smoother,
    right: Smoother,
}

impl Mixer {
    /// A strip of a new track: the gate at 0, so that it fades in at its first sync.
    pub const fn new() -> Self {
        Mixer {
            gain: 1.0,
            pan: 0.0,
            muted: false,
            audible: true,
            seen_gain: None,
            seen_pan: None,
            seen_mute: None,
            synced: false,
            glide_gain: 0,
            glide_pan: 0,
            gate: Smoother::new(0.0),
            left: Smoother::new(1.0),
            right: Smoother::new(1.0),
        }
    }

    /// The master's strip: unity, still, in the state a `SetMaster` of the defaults gives.
    pub const fn unity() -> Self {
        Mixer {
            gain: 1.0,
            pan: 0.0,
            muted: false,
            audible: true,
            seen_gain: Some(1.0f32.to_bits()),
            seen_pan: Some(0.0f32.to_bits()),
            seen_mute: Some(false),
            synced: true,
            glide_gain: 0,
            glide_pan: 0,
            gate: Smoother::new(1.0),
            left: Smoother::new(1.0),
            right: Smoother::new(1.0),
        }
    }

    /// The levels (left, right) at the next frame.
    pub fn levels(&self) -> (f32, f32) {
        let g = self.gate.current();
        (g * self.left.current(), g * self.right.current())
    }

    /// Both levels at 0 and still.
    pub fn is_silent(&self) -> bool {
        let still = |r: &Smoother| r.is_still() && r.target() == 0.0;
        still(&self.gate) || (still(&self.left) && still(&self.right))
    }

    /// Aims the gate at 0 over `fade` frames (a track that leaves).
    pub fn fade_out(&mut self, fade: u32) {
        self.gate.aim(0.0, fade);
    }

    /// Forgets the message values: the next [`Mixer::sync`] takes them again, so that a
    /// strip a lane drove goes back to the faders when the lane is gone (a schedule
    /// replaced, a `Reset`).
    pub fn forget(&mut self) {
        self.seen_gain = None;
        self.seen_pan = None;
        self.seen_mute = None;
    }

    /// Forgets the fader's gain only: a gain lane is gone, the pan's and the mute's are
    /// not (AM-2).
    pub fn forget_gain(&mut self) {
        self.seen_gain = None;
    }

    /// Forgets the fader's pan only (a pan lane is gone).
    pub fn forget_pan(&mut self) {
        self.seen_pan = None;
    }

    /// Takes the values of the track's message state and whether the track is `audible`
    /// (not silenced by a solo), at the start of a block: what changed moves by a ramp of
    /// `fade` frames (module doc).
    pub fn sync(&mut self, gain: f32, pan: f32, mute: bool, audible: bool, fade: u32) {
        let mut levels = false;
        let (mut moved_gain, mut moved_pan) = (false, false);
        let mut gate = !self.synced;
        if self.seen_gain != Some(gain.to_bits()) {
            self.seen_gain = Some(gain.to_bits());
            // Taken again after a `forget`, the same value is no gesture.
            moved_gain = self.gain.to_bits() != gain.to_bits();
            self.gain = gain;
            levels = true;
        }
        if self.seen_pan != Some(pan.to_bits()) {
            self.seen_pan = Some(pan.to_bits());
            // Taken again after a `forget`, the same value is no gesture.
            moved_pan = self.pan.to_bits() != pan.to_bits();
            self.pan = pan;
            levels = true;
        }
        if self.seen_mute != Some(mute) {
            self.seen_mute = Some(mute);
            self.muted = mute;
            gate = true;
        }
        if self.audible != audible {
            self.audible = audible;
            gate = true;
        }
        if levels {
            self.set_levels(fade, !self.synced);
            if self.synced {
                if moved_gain {
                    self.glide_gain = fade;
                }
                if moved_pan {
                    self.glide_pan = fade;
                }
            }
        }
        if gate {
            self.set_gate(fade);
        }
        self.synced = true;
    }

    fn set_levels(&mut self, fade: u32, exact: bool) {
        self.levels_to(fade, exact, 0);
    }

    /// [`Mixer::set_levels`]; an exact change glides over `glide` frames when it is not 0
    /// (a message's ramp on the other field is under way).
    fn levels_to(&mut self, fade: u32, exact: bool, glide: u32) {
        let (l, r) = mix_levels(self.gain, self.pan, true);
        if exact && glide > 0 {
            self.left.set(l, glide);
            self.right.set(r, glide);
        } else if exact {
            self.left.jump(l);
            self.right.jump(r);
        } else {
            self.left.aim(l, fade);
            self.right.aim(r, fade);
        }
    }

    fn set_gate(&mut self, fade: u32) {
        let open = self.audible && !self.muted;
        self.gate.aim(if open { 1.0 } else { 0.0 }, fade);
    }

    /// A timed parameter (module doc): a gain or a pan at once, a mute by a ramp of the
    /// gate. An unknown id is ignored.
    fn param(&mut self, id: ParamId, value: f32, fade: u32) {
        match id {
            GAIN => {
                self.gain = value;
                self.levels_to(fade, true, self.glide_pan);
            }
            PAN => {
                self.pan = value;
                self.levels_to(fade, true, self.glide_gain);
            }
            MUTE => {
                self.muted = value >= 0.5;
                self.set_gate(fade);
            }
            _ => {}
        }
    }

    /// Runs `piece(gate, left, right, from, to)` over the frames `0..frames` of a
    /// sub-block, cut at the offsets of `events`: each event takes effect at its offset
    /// (clamped to the sub-block). A ramp end ([`GAIN_RAMP`], [`PAN_RAMP`], and every other
    /// ramp end at the same offset) aims the levels at its value over the frames since the
    /// last event, so that they reach it at its offset. The ramps advance after each piece,
    /// so `piece` reads them from `at(i - from)`.
    fn drive(
        &mut self,
        frames: usize,
        events: impl Iterator<Item = TimedParam>,
        fade: u32,
        mut piece: impl FnMut(&Smoother, &Smoother, &Smoother, usize, usize),
    ) {
        let mut pos = 0;
        let mut events = events.peekable();
        while let Some((offset, id, value)) = events.next() {
            let at = (offset as usize).min(frames);
            if id == GAIN_RAMP || id == PAN_RAMP {
                self.ramp_value(id, value);
                while let Some(&(o, next, v)) = events.peek() {
                    if (next != GAIN_RAMP && next != PAN_RAMP) || (o as usize).min(frames) != at {
                        break;
                    }
                    self.ramp_value(next, v);
                    events.next();
                }
                let (l, r) = mix_levels(self.gain, self.pan, true);
                let n = at.saturating_sub(pos) as u32;
                self.left.set(l, n);
                self.right.set(r, n);
            }
            if at > pos {
                self.run(pos, at, &mut piece);
                pos = at;
            }
            if id != GAIN_RAMP && id != PAN_RAMP {
                self.param(id, value, fade);
            }
        }
        if frames > pos {
            self.run(pos, frames, &mut piece);
        }
    }

    /// Takes the value of a ramp end.
    fn ramp_value(&mut self, id: ParamId, value: f32) {
        if id == GAIN_RAMP {
            self.gain = value;
        } else {
            self.pan = value;
        }
    }

    fn run(
        &mut self,
        from: usize,
        to: usize,
        piece: &mut impl FnMut(&Smoother, &Smoother, &Smoother, usize, usize),
    ) {
        piece(&self.gate, &self.left, &self.right, from, to);
        let n = (to - from) as u32;
        self.gate.advance(n);
        self.left.advance(n);
        self.right.advance(n);
        self.glide_gain = self.glide_gain.saturating_sub(n);
        self.glide_pan = self.glide_pan.saturating_sub(n);
    }

    /// Mixes a track's sub-block: adds `left` and `right` times the levels to `out_left`
    /// and `out_right` (all of one length), and the faded signal to `meter` (the meter is
    /// after the fader). `events` are the strip's timed parameters ([`timed`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mix_into(
        &mut self,
        left: &[f32],
        right: &[f32],
        out_left: &mut [f32],
        out_right: &mut [f32],
        meter: &mut MeterAcc,
        events: impl Iterator<Item = TimedParam>,
        fade: u32,
    ) {
        let frames = left
            .len()
            .min(right.len())
            .min(out_left.len())
            .min(out_right.len());
        self.drive(frames, events, fade, |g, gl, gr, from, to| {
            let still = g.is_still() && gl.is_still() && gr.is_still();
            let (a, b) = (g.target() * gl.target(), g.target() * gr.target());
            for k in from..to {
                let (l, r) = if still {
                    (left[k] * a, right[k] * b)
                } else {
                    let i = (k - from) as u32;
                    let g = g.at(i);
                    (left[k] * (g * gl.at(i)), right[k] * (g * gr.at(i)))
                };
                meter.push(0, l);
                meter.push(1, r);
                out_left[k] += l;
                out_right[k] += r;
            }
        });
    }

    /// Applies the strip in place to `left` and `right` (the master); a sample that is not
    /// finite or is denormal leaves as 0.
    pub(crate) fn apply_in_place(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        events: impl Iterator<Item = TimedParam>,
        fade: u32,
    ) {
        let frames = left.len().min(right.len());
        self.drive(frames, events, fade, |g, gl, gr, from, to| {
            for k in from..to {
                let i = (k - from) as u32;
                let g = g.at(i);
                left[k] = flush(left[k] * (g * gl.at(i)));
                right[k] = flush(right[k] * (g * gr.at(i)));
            }
        });
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

/// Samples below this magnitude leave as 0 (no denormal reaches the output).
const DENORMAL_FLOOR: f32 = 1e-20;

/// A sample as it leaves the graph: a denormal, a NaN or an infinity is 0.
pub(crate) fn flush(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        x
    }
}
