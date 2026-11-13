//! Automation playback (AM-2, `docs/specs/02-decoupage-mvp.md` > AM-2): the engine reads
//! each lane of a track while the clock runs and sends its values, dated to the sample, to
//! the device or to the track's strip.
//!
//! # A lane
//!
//! A lane ([`crate::engine::Lane`]) is a table of points sorted by tick, at most one per
//! tick, with finite values (the Preparer makes them so; a gain lane keeps its dB, which the
//! fader's law clamps to its range). The shape of a point is the transition
//! **after** it: `Linear` goes in a straight line to the next point's value, `Step` holds
//! its value until the next point. Before its first point a lane holds the first value,
//! after its last point the last value.
//!
//! # Curved transitions (#881)
//!
//! `EaseIn`, `EaseOut` and `Smooth` follow the same line as `Linear` (in the parameter's unit,
//! along the fader's travel for a volume lane), warped in time: the progress `t` of the
//! transition, from 0 at the point to 1 at the next, becomes [`warp`]`(curve, t)`: `t²` for
//! `EaseIn` (slow, then fast), `t (2 − t)` for `EaseOut` (fast, then slow), `t² (3 − 2t)` for
//! `Smooth` (slow at both ends). The formulas of the production sheet (the music-producer
//! skill's `ease-in`, `ease-out`, `smooth`), plain products, so the native and the web builds
//! compute the same bits. Everything below that speaks of a linear transition holds for a
//! curved one: it moves (a slope), it is read at the control instants, a volume lane sends it
//! as chords of its curve.
//!
//! A point is **reached** at frame `c` of a clock segment when its sample is `c` or
//! earlier: `floor(sample_of(tick)) <= c`, the floor rule of every timed event (TR-1,
//! MO-4). The value of a lane at frame `c` is the value of the transition after the last
//! point reached, at the playhead `max(position_at(c), tick of that point)`: a point's
//! value falls exactly on its sample and a linear transition is linear in frames.
//!
//! # A volume lane (#389)
//!
//! A straight line drawn on a volume lane is a line along the **travel of the fader**
//! ([`crate::fader`], the law of the mixer's fader and of the lane's axis in the timeline), not
//! in amplitude and not in dB: a fade from 0 dB to -96 dB over eight bars goes down like a
//! fader pulled at constant speed. A linear transition goes in a straight line in position
//! from the position of the point's dB to the position of the next one's. The lane's lowest
//! value, -96 dB, is silence: an amplitude of exactly 0. A pan lane and a device parameter
//! stay linear in their own unit.
//!
//! The strip draws straight lines between the values it receives (below), so the curve is
//! sent as **chords**: the value of a volume lane at a frame is the straight line, in
//! amplitude, between the curve's amplitudes at the two anchors around the frame, the last
//! control frame (or the point just reached) and the next control frame (or the next point).
//! The anchors are the same whatever the block size, so the render does not depend on it
//! either; a chord is at most [`CONTROL_FRAMES`] long, and a point and a control frame are
//! on the curve exactly.
//!
//! # Pieces and control instants
//!
//! The graph renders a track whose lanes play in **pieces** of a clock segment: a piece
//! ends at the next point of any of its lanes, so a point always starts a piece, and, while
//! a lane moves along a slope, at the next **control frame** (every [`CONTROL_FRAMES`]-th
//! frame since the engine started, whatever the block size). A **control instant** of a
//! lane is a control frame, the frame of one of its points, or the first frame after a jump
//! or after a gesture lets go of it.
//!
//! - **The strip** (a gain or pan lane): every sub-block of the track (a piece, or less
//!   when its events fill the sink) receives the lane's value at its first frame and, on a
//!   slope, its value at its last frame as a ramp end ([`crate::mixer::GAIN_RAMP`],
//!   [`crate::mixer::PAN_RAMP`]): the strip draws the straight line between the two, so a
//!   gain lane is followed on every sample, whatever the block size. A pan lane is exact at
//!   both ends of each sub-block and straight in level between (the pan law is a cosine);
//!   so is a gain that ramps while the pan ramps too.
//! - **A device** receives a lane's value at a control instant only, and only when it
//!   differs from the last one it received (at a jump or a takeover, always): it smooths
//!   each one as it declares (AM-3), so a held value is reached exactly, and the instants
//!   do not depend on the block size, so neither does the render.
//!
//! Two lanes of one parameter (a merged project) both send; at equal offsets the one in
//! the lower engine slot is sent last and wins.
//!
//! # Manual gestures
//!
//! While the clock runs a lane owns its parameter. A manual gesture on an automated
//! parameter (a `ParamSet` for its device and parameter, a `TrackSetMix` whose gain or
//! pan changed) **holds** the matching lanes of the track: they send nothing, and the
//! gesture applies, until the playhead reaches the lane's next point, where the lane takes
//! over at that point's sample (after the last point: until the next jump). A jump (play,
//! seek, loop wrap) gives every parameter back to its lanes at once.
//!
//! A gesture that leaves the value as the project holds it is no `TrackSetMix` that moves
//! (the engine could not tell it from a mute): the page says so by `HoldMix`, which holds the
//! lanes of the fields it names, takes the fader value it carries and gives the strip's field
//! back to the fader (#417). A `ParamSet` holds its lanes whatever its value.
//!
//! # Stopped (#388)
//!
//! What sounds is what the page shows: when the clock stops, and at every seek while it is
//! stopped, each lane sends its value at the playhead ([`TrackLanes::push_frozen`], the lane
//! read through a one-frame [`Segment::frozen`]; before the first point, the first value).
//! Then it sends nothing: a gesture made while stopped holds until the next play or seek,
//! which give the parameter back to its lanes. A lane that is uploaded again (its points
//! edited), created, or whose device was built again sends its value at the playhead too.
//! A new upload of a lane
//! (its points edited) keeps its hold; a lane uploaded in the same batch of messages as the
//! gesture is not held (the list is the last block's). A device built again in its slot,
//! or switched on or off, receives its lanes' values again.
//!
//! # The master's lanes (#378)
//!
//! The master plays lanes like a track: its gain and pan, and the parameters of the effects
//! of its chain, addressed by [`TrackSlot::MASTER`]. A master lane is read by the same code
//! ([`TrackLanes`]), through its own list head ([`MasterLanes`]), in the graph's master pass
//! ([`crate::graph`]): the master's chain runs once per sub-block, as a track's, and the
//! master's strip applies the lanes' gain and pan sub-block by sub-block. A manual gesture
//! on the master (`SetMaster` that moves the gain or the pan, a `ParamSet` on its chain)
//! holds the matching master lanes like it holds a track's. A master with no lane renders as
//! it did before (one pass of the chain over the block).
//!
//! # Lanes of a track
//!
//! The lanes live in the engine's lane table, by slot. At the start of every block the
//! graph links each lane to its track ([`link`]: an intrusive list through
//! [`crate::graph::TrackGraph`] and the lanes; it visits every track slot and every slot of
//! the lane pages in use, no allocation), so no message can leave a stale list. A lane
//! whose track does not exist is not linked. A track plays up to [`LANES_PER_TRACK`] lanes,
//! the project's own limit (`project::limits::MAX_LANES_PER_TRACK` is this constant).
//!
//! Nothing here allocates, locks or panics.

use crate::device::Event;
use crate::engine::{Lane, Paged, Track};
use crate::events::Sink;

use crate::mixer::{GAIN, GAIN_RAMP, MIXER_SLOT, PAN, PAN_RAMP};
use crate::msg::{AutomationPoint, AutomationTarget, Curve, DeviceSlot, ParamId, Slots, TrackSlot};
use crate::transport::Segment;

/// Frames between two control frames (module doc): the rate at which a device receives a
/// moving value, and the longest straight run of a pan ramp's levels.
pub const CONTROL_FRAMES: u64 = 64;

/// Most lanes a track plays: the project's limit.
pub const LANES_PER_TRACK: usize = 10_000;

/// Room a track's sink keeps for the lanes' values: a lane sends at most two values in a
/// sub-block (its value and a ramp end), and its first again when an empty sub-block at
/// the end of a segment kept the sink.
pub const LANE_ROOM: usize = 3 * LANES_PER_TRACK;

/// End of a track's list of lanes.
pub const NO_LANE: u32 = u32::MAX;

/// Who owns a lane's parameter (module doc).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Manual {
    /// The lane does.
    #[default]
    Free,
    /// A gesture arrived; the lane has not been read since.
    Held,
    /// A gesture holds the parameter until the lane has reached more than this many
    /// points.
    Until(usize),
}

/// The audio side's state of a lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneRun {
    /// The next lane of the same track (rebuilt every block by [`link`]).
    pub(crate) next: u32,
    /// Points reached at the last reading (a hint: [`reached`] checks it).
    index: usize,
    manual: Manual,
    /// The bits of the last value sent to a device; `None`: the next instant sends.
    sent: Option<u32>,
}

impl Default for LaneRun {
    fn default() -> Self {
        LaneRun {
            next: NO_LANE,
            index: 0,
            manual: Manual::Free,
            sent: None,
        }
    }
}

impl LaneRun {
    /// Who owns the parameter.
    pub fn manual(&self) -> Manual {
        self.manual
    }

    /// Carries the gesture of the lane a new upload replaces, and its place in the track's
    /// list until the next block links it again (the points changed: the value is sent
    /// again).
    pub(crate) fn inherit(&mut self, old: &LaneRun) {
        self.manual = old.manual;
        self.index = old.index;
        self.next = old.next;
    }
}

// ---------------------------------------------------------------- evaluation

/// Whether the point at `tick` is reached at frame `c` of the block (module doc).
fn is_reached(segment: &Segment, tick: crate::time::Tick, c: usize) -> bool {
    segment.sample_of(tick) < (c as f64) + 1.0
}

/// The number of points reached at frame `c`, from the hint `index`: O(1) while the
/// playhead moves on by at most one point, a binary search otherwise (a jump, or several
/// points on one frame).
pub fn reached(points: &[AutomationPoint], index: usize, segment: &Segment, c: usize) -> usize {
    let n = points.len();
    let i = index.min(n);
    let before = i == 0 || is_reached(segment, points[i - 1].tick, c);
    if before {
        if i == n || !is_reached(segment, points[i].tick, c) {
            return i;
        }
        if i + 1 == n || !is_reached(segment, points[i + 1].tick, c) {
            return i + 1;
        }
    }
    points.partition_point(|p| is_reached(segment, p.tick, c))
}

/// The value of a lane at frame `c` with `r` points reached (module doc); `None` for a
/// lane with no point.
pub fn value_at(points: &[AutomationPoint], r: usize, segment: &Segment, c: usize) -> Option<f32> {
    let first = points.first()?;
    if r == 0 {
        return Some(first.value);
    }
    let prev = points.get(r - 1)?;
    let Some(next) = points.get(r) else {
        return Some(prev.value);
    };
    if prev.curve == Curve::Step || next.tick <= prev.tick {
        return Some(prev.value);
    }
    let (a, b) = (prev.tick.0 as f64, next.tick.0 as f64);
    let x = segment.position_at(c).max(a);
    let t = warp(prev.curve, ((x - a) / (b - a)).clamp(0.0, 1.0));
    let (va, vb) = (f64::from(prev.value), f64::from(next.value));
    Some((va + (vb - va) * t) as f32)
}

/// The progress of a transition of shape `curve` at linear progress `t` in `[0, 1]` (module
/// doc, "Curved transitions"): 0 at 0, 1 at 1, monotonic; `t` itself for `Linear` (and for
/// `Step`, which never reaches here).
pub fn warp(curve: Curve, t: f64) -> f64 {
    match curve {
        Curve::Step | Curve::Linear => t,
        Curve::EaseIn => t * t,
        Curve::EaseOut => t * (2.0 - t),
        Curve::Smooth => t * t * (3.0 - 2.0 * t),
    }
}

/// What the engine sends for a lane at frame `c` (module doc): the value of the lane in its
/// unit, except for a volume lane, which goes to the strip as an amplitude along the fader's
/// travel ([`gain_value`]).
fn lane_value(
    target: AutomationTarget,
    points: &[AutomationPoint],
    r: usize,
    segment: &Segment,
    base: u64,
    c: usize,
) -> Option<f32> {
    if target == AutomationTarget::MixerGain {
        gain_value(points, r, segment, base, c)
    } else {
        value_at(points, r, segment, c)
    }
}

/// The amplitude of a volume lane at frame `c` with `r` points reached (module doc, "A volume
/// lane"). A held value is the amplitude of its dB. On a linear transition the fader's
/// position goes in a straight line from the value of the point to the value of the next
/// one, and the amplitude is drawn as a chord of that curve between the two **anchors**
/// around `c`: the last control frame or the frame of the last point reached, and the next
/// control frame or the frame of the next point. The anchors do not depend on how the clock
/// was cut into blocks, so neither does the line the strip draws between the values it
/// receives; at a point and at a control frame the value is exact.
fn gain_value(
    points: &[AutomationPoint],
    r: usize,
    segment: &Segment,
    base: u64,
    c: usize,
) -> Option<f32> {
    let first = points.first()?;
    if r == 0 {
        return Some(held_gain(first.value));
    }
    let prev = points.get(r - 1)?;
    let Some(next) = points.get(r) else {
        return Some(held_gain(prev.value));
    };
    if prev.curve == Curve::Step || next.tick <= prev.tick || prev.value == next.value {
        return Some(held_gain(prev.value));
    }
    let control = c as i64 - (base.wrapping_add(c as u64) % CONTROL_FRAMES) as i64;
    let point = segment.sample_of(prev.tick).floor();
    let a = (control as f64).max(point);
    let landing = segment.sample_of(next.tick).floor();
    let b = ((control + CONTROL_FRAMES as i64) as f64).min(landing);
    let (pa, pb) = (position(prev.value), position(next.value));
    let ga = f64::from(crate::fader::gain_at(along(prev, next, segment, a, pa, pb)));
    let gb = if b >= landing {
        f64::from(crate::fader::gain_at(pb))
    } else {
        f64::from(crate::fader::gain_at(along(prev, next, segment, b, pa, pb)))
    };
    if b <= a {
        return Some(ga as f32);
    }
    let x = ((c as f64 - a) / (b - a)).clamp(0.0, 1.0);
    Some((ga + (gb - ga) * x) as f32)
}

/// The amplitude of a held gain in dB (`MIN_DB` is silence).
fn held_gain(db: f32) -> f32 {
    crate::fader::gain_at(position(db))
}

/// The fader position of a gain in dB.
fn position(db: f32) -> f64 {
    crate::fader::position_of_db(f64::from(db))
}

/// The fader position at fractional frame `frame` of a transition from `prev` to `next` (the
/// lane's straight line in travel, from `pa` to `pb`, warped by the curve of `prev`).
fn along(
    prev: &AutomationPoint,
    next: &AutomationPoint,
    segment: &Segment,
    frame: f64,
    pa: f64,
    pb: f64,
) -> f64 {
    let (ta, tb) = (prev.tick.0 as f64, next.tick.0 as f64);
    let x = segment.position_at_frame(frame).max(ta);
    let t = warp(prev.curve, ((x - ta) / (tb - ta)).clamp(0.0, 1.0));
    pa + (pb - pa) * t
}

/// Whether the transition after the last of `r` points reached moves.
fn on_slope(points: &[AutomationPoint], r: usize) -> bool {
    match (r.checked_sub(1).and_then(|i| points.get(i)), points.get(r)) {
        (Some(prev), Some(next)) => prev.curve != Curve::Step && prev.value != next.value,
        _ => false,
    }
}

/// The frame of a point (the floor of its sample; `as` saturates and sends NaN to 0).
fn frame_of(segment: &Segment, point: &AutomationPoint) -> usize {
    segment.sample_of(point.tick).floor() as usize
}

/// The frame of the next point after the `r` reached, if any (at least `c + 1`).
fn next_boundary(
    points: &[AutomationPoint],
    r: usize,
    segment: &Segment,
    c: usize,
) -> Option<usize> {
    points.get(r).map(|p| frame_of(segment, p).max(c + 1))
}

/// Whether frame `c` of a block that starts at absolute frame `base` is a control frame.
fn is_control_frame(base: u64, c: usize) -> bool {
    base.wrapping_add(c as u64).is_multiple_of(CONTROL_FRAMES)
}

/// Where a lane's value goes in a track's sink: the device and the parameter id.
fn address(target: AutomationTarget, ramp: bool) -> (DeviceSlot, ParamId) {
    match target {
        AutomationTarget::DeviceParam { device, param } => (device, param),
        AutomationTarget::MixerGain => (MIXER_SLOT, if ramp { GAIN_RAMP } else { GAIN }),
        AutomationTarget::MixerPan => (MIXER_SLOT, if ramp { PAN_RAMP } else { PAN }),
    }
}

// ---------------------------------------------------------------- the lanes of a track

/// Links every lane to its track for this block (module doc), no allocation. The list
/// runs from the highest slot to the lowest.
pub(crate) fn link(
    tracks: &mut Slots<Box<Track>>,
    lanes: &mut Paged<Box<Lane>>,
    master: &mut MasterLanes,
) {
    for track in tracks.as_mut_slice().iter_mut().flatten() {
        track.graph.lanes = NO_LANE;
        track.graph.lane_count = 0;
    }
    master.head = NO_LANE;
    master.count = 0;
    lanes.for_each_mut(|slot, lane| {
        lane.run.next = NO_LANE;
        if lane.track.is_master() {
            if (master.count as usize) < LANES_PER_TRACK {
                lane.run.next = master.head;
                master.head = slot;
                master.count += 1;
            }
            return;
        }
        let Some(track) = tracks
            .as_mut_slice()
            .get_mut(lane.track.0 as usize)
            .and_then(Option::as_deref_mut)
        else {
            return;
        };
        if (track.graph.lane_count as usize) < LANES_PER_TRACK {
            lane.run.next = track.graph.lanes;
            track.graph.lanes = slot;
            track.graph.lane_count += 1;
        }
    });
}

/// What the graph keeps of the master's lanes (#378), as [`crate::graph::TrackGraph`] does
/// of a track's: the first of its lanes, linked at every block, and how many are linked; the
/// tick the next clock segment starts at when the playhead moves on without a jump; the
/// playhead and cue count the lanes were last applied at while the clock is stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MasterLanes {
    pub(crate) head: u32,
    pub(crate) count: u32,
    pub(crate) expect: Option<crate::time::Tick>,
    pub(crate) stopped_at: Option<(u64, u64)>,
}

impl MasterLanes {
    pub(crate) const fn new() -> Self {
        MasterLanes {
            head: NO_LANE,
            count: 0,
            expect: None,
            stopped_at: None,
        }
    }
}

/// The lanes of one track, through its list (module doc).
pub(crate) struct TrackLanes<'a> {
    pub(crate) table: &'a mut Paged<Box<Lane>>,
    pub(crate) head: u32,
}

impl TrackLanes<'_> {
    /// Calls `f` on every lane of the list (at most [`LANES_PER_TRACK`]).
    fn each(&mut self, mut f: impl FnMut(&mut Lane)) {
        let mut at = self.head;
        for _ in 0..LANES_PER_TRACK {
            if at == NO_LANE {
                return;
            }
            let Some(lane) = self.table.get_mut(at) else {
                return;
            };
            f(lane);
            at = lane.run.next;
        }
    }

    /// Whether the track has a lane.
    pub(crate) fn is_empty(&self) -> bool {
        self.head == NO_LANE
    }

    /// The playhead jumped: every parameter goes back to its lanes, which send their value
    /// at once.
    pub(crate) fn jump(&mut self) {
        self.each(|lane| {
            lane.run.manual = Manual::Free;
            lane.run.sent = None;
        });
    }

    /// The clock is stopped at `position`: every lane whose parameter is not held by a
    /// gesture and has not applied its value yet sends it at offset 0 (a device's value, a
    /// strip's gain or pan, without a ramp end). `moved` is a stop or a seek: it gives every
    /// parameter back to its lanes first, like a jump. Between two such events a lane sends
    /// nothing, so a gesture holds until the next play or seek.
    pub(crate) fn push_frozen(&mut self, position: f64, moved: bool, sink: &mut Sink) {
        let segment = Segment::frozen(position);
        self.each(|lane| {
            if moved {
                lane.run.manual = Manual::Free;
                lane.run.sent = None;
            }
            if lane.run.manual != Manual::Free || lane.run.sent.is_some() {
                return;
            }
            let points = lane.points.as_slice();
            let r = reached(points, lane.run.index, &segment, 0);
            lane.run.index = r;
            let Some(value) = lane_value(lane.target, points, r, &segment, 0, 0) else {
                return;
            };
            let (device, id) = address(lane.target, false);
            let pushed = sink.push_lane_param(
                device,
                Event::ParamValue {
                    offset: 0,
                    id,
                    value,
                },
            );
            if pushed {
                lane.run.sent = Some(value.to_bits());
            }
        });
    }

    /// Holds the lanes of `track` whose target `matches` (a manual gesture). The list is
    /// the last block's: a lane uploaded since is not in it, a slot reused by another
    /// track's lane is skipped.
    pub(crate) fn hold(&mut self, track: TrackSlot, matches: impl Fn(AutomationTarget) -> bool) {
        self.each(|lane| {
            if lane.track == track && matches(lane.target) {
                lane.run.manual = Manual::Held;
            }
        });
    }

    /// The device in slot `device` of `track` was replaced or switched on or off: its lanes
    /// send their value again at the next sub-block.
    pub(crate) fn resend(&mut self, track: TrackSlot, device: DeviceSlot) {
        self.each(|lane| {
            if lane.track == track
                && matches!(lane.target, AutomationTarget::DeviceParam { device: d, .. } if d == device)
            {
                lane.run.sent = None;
            }
        });
    }

    /// The end of a piece that starts at frame `start` of `segment` and may last up to
    /// `end`, in a block that starts at absolute frame `base`: the next point of a lane,
    /// and the next control frame while a lane is on a slope. At least `start + 1`, at
    /// most `end` (when `end > start`).
    pub(crate) fn piece_end(
        &mut self,
        segment: &Segment,
        base: u64,
        start: usize,
        end: usize,
    ) -> usize {
        let mut cut = end;
        let to_control = CONTROL_FRAMES - base.wrapping_add(start as u64) % CONTROL_FRAMES;
        let control = start.saturating_add(to_control as usize);
        self.each(|lane| {
            let points = lane.points.as_slice();
            let r = reached(points, lane.run.index, segment, start);
            lane.run.index = r;
            if let Some(b) = next_boundary(points, r, segment, start) {
                cut = cut.min(b);
            }
            if on_slope(points, r) {
                cut = cut.min(control);
            }
        });
        cut.clamp(start + 1, end.max(start + 1))
    }

    /// Pushes, at offset 0 of the sub-block that starts at frame `start` (of a block at
    /// absolute frame `base`), the value of every lane that plays (module doc), and settles
    /// the gestures.
    pub(crate) fn push_start(
        &mut self,
        segment: &Segment,
        base: u64,
        start: usize,
        sink: &mut Sink,
    ) {
        let control = is_control_frame(base, start);
        self.each(|lane| {
            let points = lane.points.as_slice();
            let r = reached(points, lane.run.index, segment, start);
            lane.run.index = r;
            lane.run.manual = match lane.run.manual {
                Manual::Held => Manual::Until(r),
                Manual::Until(a) if r > a => {
                    // The lane takes over at this point: its value goes out now.
                    lane.run.sent = None;
                    Manual::Free
                }
                m => m,
            };
            if lane.run.manual != Manual::Free {
                return;
            }
            let Some(value) = lane_value(lane.target, points, r, segment, base, start) else {
                return;
            };
            if let AutomationTarget::DeviceParam { .. } = lane.target {
                // The last point reached lands on this frame (a point of an earlier block has
                // a negative sample here).
                let at_point = r > 0
                    && points
                        .get(r - 1)
                        .is_some_and(|p| segment.sample_of(p.tick).floor() == start as f64);
                let instant = lane.run.sent.is_none() || control || at_point;
                if !instant || lane.run.sent == Some(value.to_bits()) {
                    return;
                }
            }
            let (device, id) = address(lane.target, false);
            let pushed = sink.push_lane_param(
                device,
                Event::ParamValue {
                    offset: 0,
                    id,
                    value,
                },
            );
            if pushed {
                lane.run.sent = Some(value.to_bits());
            }
        });
    }

    /// Pushes, at the last frame of the sub-block `[start, end)`, the value there of every
    /// mixer lane that plays on a slope, as a ramp end of the strip.
    pub(crate) fn push_end(
        &mut self,
        segment: &Segment,
        base: u64,
        start: usize,
        end: usize,
        sink: &mut Sink,
    ) {
        if end <= start + 1 {
            return;
        }
        let last = end - 1;
        self.each(|lane| {
            if lane.run.manual != Manual::Free
                || matches!(lane.target, AutomationTarget::DeviceParam { .. })
            {
                return;
            }
            let points = lane.points.as_slice();
            // No point is reached inside a piece: the transition is the one at `start`.
            let r = lane.run.index;
            if !on_slope(points, r) {
                return;
            }
            let Some(value) = lane_value(lane.target, points, r, segment, base, last) else {
                return;
            };
            let (device, id) = address(lane.target, true);
            sink.push_lane_param(
                device,
                Event::ParamValue {
                    offset: (last - start) as u32,
                    id,
                    value,
                },
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Tick;

    fn point(tick: i64, value: f32, curve: Curve) -> AutomationPoint {
        AutomationPoint {
            tick: Tick(tick),
            value,
            curve,
        }
    }

    /// A segment of 1 000 frames from tick 0, 25 frames per tick (120 BPM at 48 kHz).
    fn segment() -> Segment {
        let mut s = Segment::default();
        s.frames = 1000;
        s.to = Tick(40);
        s.with_mapping(0.0, 0.0, 25.0)
    }

    #[test]
    fn a_linear_transition_is_linear_in_frames_and_a_point_lands_on_its_frame() {
        let s = segment();
        let p = [point(4, 0.0, Curve::Linear), point(8, 1.0, Curve::Step)];
        // Before the first point: its value.
        assert_eq!(reached(&p, 0, &s, 99), 0);
        assert_eq!(value_at(&p, 0, &s, 99), Some(0.0));
        // Tick 4 is frame 100.
        assert_eq!(reached(&p, 0, &s, 100), 1);
        assert_eq!(value_at(&p, 1, &s, 100), Some(0.0));
        assert_eq!(value_at(&p, 1, &s, 150), Some(0.5));
        assert_eq!(value_at(&p, 1, &s, 199), Some(0.99));
        assert_eq!(reached(&p, 1, &s, 200), 2);
        assert_eq!(value_at(&p, 2, &s, 200), Some(1.0));
        assert_eq!(value_at(&p, 2, &s, 999), Some(1.0));
    }

    #[test]
    fn a_step_holds_until_the_next_point() {
        let s = segment();
        let p = [point(0, 0.25, Curve::Step), point(10, 0.75, Curve::Linear)];
        assert_eq!(value_at(&p, reached(&p, 0, &s, 249), &s, 249), Some(0.25));
        assert_eq!(value_at(&p, reached(&p, 0, &s, 250), &s, 250), Some(0.75));
    }

    #[test]
    fn the_hint_is_checked_after_a_jump_and_several_points_on_one_frame() {
        let s = segment();
        let p: Vec<_> = (0..20).map(|i| point(i, i as f32, Curve::Step)).collect();
        // Back from the end, forward from the start, then a wrong hint.
        assert_eq!(reached(&p, 20, &s, 30), 2);
        assert_eq!(reached(&p, 0, &s, 400), 17);
        assert_eq!(reached(&p, 7, &s, 0), 1);
        // Less than one sample per tick: ticks 0 to 9 on frames 0 to 2.
        let dense = Segment::default().with_mapping(0.0, 0.0, 0.3);
        assert_eq!(reached(&p, 0, &dense, 0), 4);
        assert_eq!(reached(&p, 1, &dense, 2), 10);
        assert_eq!(value_at(&p, 10, &dense, 2), Some(9.0));
    }

    #[test]
    fn a_curved_transition_warps_the_straight_line_and_keeps_its_ends() {
        let s = segment();
        // Tick 4 is frame 100, tick 8 frame 200: frame 150 is the middle, 125 a quarter.
        for (curve, quarter, middle) in [
            (Curve::Linear, 0.25, 0.5),
            (Curve::EaseIn, 0.0625, 0.25),
            (Curve::EaseOut, 0.4375, 0.75),
            (Curve::Smooth, 0.15625, 0.5),
        ] {
            let p = [point(4, 0.0, curve), point(8, 1.0, Curve::Step)];
            assert_eq!(value_at(&p, 1, &s, 100), Some(0.0), "{curve:?}");
            assert_eq!(value_at(&p, 1, &s, 125), Some(quarter), "{curve:?}");
            assert_eq!(value_at(&p, 1, &s, 150), Some(middle), "{curve:?}");
            assert_eq!(value_at(&p, 2, &s, 200), Some(1.0), "{curve:?}");
            assert!(on_slope(&p, 1), "{curve:?}");
        }
        for curve in [Curve::Linear, Curve::EaseIn, Curve::EaseOut, Curve::Smooth] {
            assert_eq!(warp(curve, 0.0), 0.0);
            assert_eq!(warp(curve, 1.0), 1.0);
            let mut last = 0.0;
            for k in 1..=100 {
                let w = warp(curve, f64::from(k) / 100.0);
                assert!(w >= last && w <= 1.0, "{curve:?} at {k}");
                last = w;
            }
        }
        assert!(!on_slope(
            &[point(4, 0.0, Curve::Step), point(8, 1.0, Curve::Step)],
            1
        ));
        assert!(!on_slope(
            &[point(4, 1.0, Curve::Smooth), point(8, 1.0, Curve::Step)],
            1
        ));
    }

    #[test]
    fn a_lane_with_no_point_has_no_value() {
        assert_eq!(value_at(&[], 0, &segment(), 0), None);
        assert!(!on_slope(&[], 0));
    }
}
