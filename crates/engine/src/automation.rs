//! Automation playback (AM-2, `docs/specs/02-decoupage-mvp.md` > AM-2): the engine reads
//! each lane of a track while the clock runs and sends its values, dated to the sample, to
//! the device or to the track's strip.
//!
//! # A lane
//!
//! A lane ([`crate::engine::Lane`]) is a table of points sorted by tick, at most one per
//! tick, with finite values (the Preparer makes them so, and turns a gain lane's dB into a
//! linear gain: a gain lane moves in amplitude). The shape of a point is the transition
//! **after** it: `Linear` goes in a straight line to the next point's value, `Step` holds
//! its value until the next point. Before its first point a lane holds the first value,
//! after its last point the last value.
//!
//! A point is **reached** at frame `c` of a clock segment when its sample is `c` or
//! earlier: `floor(sample_of(tick)) <= c`, the floor rule of every timed event (TR-1,
//! MO-4). The value of a lane at frame `c` is the value of the transition after the last
//! point reached, at the playhead `max(position_at(c), tick of that point)`: a point's
//! value falls exactly on its sample and a linear transition is linear in frames.
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
//! seek, loop wrap) gives every parameter back to its lanes at once. Stopped, the lanes
//! send nothing: the last value, a gesture's or a lane's, stays. A new upload of a lane
//! (its points edited) keeps its hold; a lane uploaded in the same batch of messages as the
//! gesture is not held (the list is the last block's). A device built again in its slot,
//! or switched on or off, receives its lanes' values again.
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
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    let (va, vb) = (f64::from(prev.value), f64::from(next.value));
    Some((va + (vb - va) * t) as f32)
}

/// Whether the transition after the last of `r` points reached moves.
fn on_slope(points: &[AutomationPoint], r: usize) -> bool {
    match (r.checked_sub(1).and_then(|i| points.get(i)), points.get(r)) {
        (Some(prev), Some(next)) => prev.curve == Curve::Linear && prev.value != next.value,
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
pub(crate) fn link(tracks: &mut Slots<Box<Track>>, lanes: &mut Paged<Box<Lane>>) {
    for track in tracks.as_mut_slice().iter_mut().flatten() {
        track.graph.lanes = NO_LANE;
        track.graph.lane_count = 0;
    }
    lanes.for_each_mut(|slot, lane| {
        lane.run.next = NO_LANE;
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
            let Some(value) = value_at(points, r, segment, start) else {
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
            let Some(value) = value_at(points, r, segment, last) else {
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
    fn a_lane_with_no_point_has_no_value() {
        assert_eq!(value_at(&[], 0, &segment(), 0), None);
        assert!(!on_slope(&[], 0));
    }
}
