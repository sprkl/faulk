//! `automation.*`: automation lanes and their points on device and mixer parameters (epic 17, AM-1).
//!
//! The contract of the automation commands, written once here and repeated in their
//! descriptions for agents:
//!
//! - A lane belongs to a track and automates one parameter of it: a parameter of one of
//!   the track's devices (`{"device": {"device": <id>, "param": <key>}}`, validated against
//!   the device's descriptor) or the track's `gain` or `pan` (`{"mixer": {"param": "gain"}}`).
//!   A track has at most one lane per parameter. The master and the devices of the master's
//!   chain are not automatable yet (a lane lives in a track).
//! - A lane holds at most one point per tick: `{tick, value, shape}`. `shape` is the
//!   transition from the point to the next one, `linear` (default) or `step`. A tick lies in
//!   `[0, 2^40]`.
//! - A value is in the parameter's unit and is rounded to its declared step: a device
//!   parameter is clamped to its range and rounded to its step (as `device.set_param`
//!   does); a gain point is in dB, -96 to +6 (no silence), rounded to 0.01 dB; a pan point is
//!   -1 to 1, rounded to 0.01. A gain or pan outside its range is refused.
//! - A batch is one history entry and is all or nothing: one point refused, nothing changes.
//!   A command that changes nothing returns no inverse and no event.
//! - Deleting a device deletes its lanes (`device.remove`) and deleting a track deletes its
//!   lanes (`track.delete`); their undo brings the lanes back with their points.
//! - A lane with more points than the density threshold of the project format is stored as a
//!   blob and read back as the same table: nothing changes for the commands.
//! - Playing the lanes is the engine's (AM-2), editing them in the timeline is the interface's
//!   (AM-4): these commands only change the project.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::device::{descriptor_of, registry};
use crate::macros::domain;
use engine::device::ParamDescriptor;
use project::limits::{MAX_LANES, MAX_LANES_PER_TRACK, MAX_POINTS, MAX_POINTS_PER_LANE, MAX_TICK};
use project::{
    AutomationLane, AutomationPoint, AutomationTarget, LaneId, MAX_GAIN_DB, MIN_GAIN_DB,
    MixerParam, Shape, Tick, Track, TrackId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The step of a gain automation point, in dB.
const GAIN_STEP_DB: f64 = 0.01;
/// The step of a pan automation point.
const PAN_STEP: f64 = 0.01;

fn linear() -> Shape {
    Shape::Linear
}

/// A point of a lane, as the commands take and return it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Point {
    /// Position in ticks, 0 to 2^40.
    pub tick: Tick,
    /// The value, in the parameter's unit; rounded to its step.
    pub value: f64,
    /// The transition from this point to the next: `linear` (default) or `step`.
    #[serde(default = "linear")]
    pub shape: Shape,
}

domain! {
    domain = "automation", schema = ("AutomationCommand", "AutomationOutput");

    /// Create an empty automation lane on a parameter of a track. `target` is
    /// `{"device": {"device": <device id>, "param": <parameter key>}}` for a parameter of
    /// one of the track's devices (the keys are in the device descriptors of the schema
    /// bundle; the device must be in this track's chain) or `{"mixer": {"param": "gain"}}`
    /// / `{"mixer": {"param": "pan"}}` for the track's volume or pan. Refused with
    /// `not_found` for an unknown track, device or parameter, with `invalid` when the track
    /// already has a lane on that parameter or when the device is in the master's chain
    /// (the master is not automatable yet), and with `limit` at the safety limit of lanes.
    /// Undo: `automation.lane.delete`.
    "automation.lane.create" (kind = "edit", undo = "inverse", destructive = false)
    LaneCreate { track: TrackId, target: AutomationTarget } => {
        /// The lane created.
        lane: LaneId,
    };

    /// Delete an automation lane with all its points. Undo: `automation.lane.restore`
    /// brings it back with its identifier and points.
    "automation.lane.delete" (kind = "edit", undo = "inverse", destructive = false)
    LaneDelete { lane: LaneId } => {
        /// The track the lane was in.
        track: TrackId,
    };

    /// Bring a deleted lane back under its identifier, as `automation.lane.delete` took it
    /// out: the inverse of a delete. `snapshot` is the whole lane: its target and points.
    /// Refused when the identifier is already in use, when the track does not exist, when
    /// the target is not valid for the track or the track already has a lane on it, at the
    /// safety limit of lanes, or when a point is not valid. Undo: `automation.lane.delete`.
    "automation.lane.restore" (kind = "edit", undo = "inverse", destructive = false)
    LaneRestore {
        track: TrackId,
        lane: LaneId,
        snapshot: AutomationLane,
    } => {};

    /// Add points to a lane, in one batch (one history entry, all or nothing). Each point
    /// is `{tick, value, shape}`: `tick` 0 to 2^40, `value` in the parameter's unit (rounded
    /// to its step; a device parameter is clamped to its range, a gain outside -96..+6 dB or
    /// a pan outside -1..1 is refused), `shape` `linear` (default) or `step`. A lane has at
    /// most one point per tick: a tick that already holds a different point, or that appears
    /// twice in the batch, is refused (use `automation.points.update` to change a point); a
    /// point equal to the one already there changes nothing. Refused with `limit` past the
    /// safety limit of points per lane or in the project. Undo: `automation.points.remove`
    /// of the points actually added.
    "automation.points.add" (kind = "edit", undo = "inverse", destructive = false)
    PointsAdd { lane: LaneId, points: Vec<Point> } => {
        /// How many points were new.
        added: u32,
    };

    /// Remove the points at these ticks from a lane, in one batch (one history entry). A
    /// tick that holds no point is ignored; the output counts the points removed. Undo:
    /// `automation.points.restore` puts the points actually removed back, exactly as they were.
    "automation.points.remove" (kind = "edit", undo = "inverse", destructive = false)
    PointsRemove { lane: LaneId, ticks: Vec<Tick> } => {
        /// How many points were in the lane.
        removed: u32,
    };

    /// Change the value or the shape of points of a lane, in one batch (one history entry,
    /// all or nothing). Each point names the tick of an existing point and gives its new
    /// `value` (the same rules as `automation.points.add`) and `shape`; the position does
    /// not change (remove and add to move a point). Refused with `not_found` when a tick
    /// holds no point and with `invalid` when a tick appears twice. A point that ends equal
    /// changes nothing. Undo: `automation.points.restore` puts the previous points back,
    /// exactly as they were.
    "automation.points.update" (kind = "edit", undo = "inverse", destructive = false)
    PointsUpdate { lane: LaneId, points: Vec<Point> } => {
        /// How many points changed.
        updated: u32,
    };

    /// Remove all the points of a lane and keep the lane. Undo: `automation.points.restore`
    /// puts the points back, exactly as they were.
    "automation.lane.clear" (kind = "edit", undo = "inverse", destructive = false)
    LaneClear { lane: LaneId } => {
        /// How many points were in the lane.
        removed: u32,
    };

    /// Put points back in a lane exactly as given, and take the points at `remove` out: the
    /// inverse of `automation.points.remove`, `automation.points.update` and
    /// `automation.lane.clear`, one history entry, all or nothing. Unlike
    /// `automation.points.add`, a value is kept as it is (not rounded to the step, not
    /// clamped), so that an undo gives back the very points that were there; a point at a
    /// tick that already holds one replaces it. A tick in range, a finite value, and a gain or
    /// pan within its range are still required, and the safety limits of points. Refused when
    /// a tick is in both lists or twice in one. Undo: `automation.points.restore` with the
    /// points it replaced and the ticks it added.
    "automation.points.restore" (kind = "edit", undo = "inverse", destructive = false)
    PointsRestore {
        lane: LaneId,
        points: Vec<Point>,
        /// The ticks whose points are taken out; absent: none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        remove: Vec<Tick>,
    } => {};
}

type Out = Outcome<Command, Output>;

fn not_found(entity: &str, id: impl ToString) -> CommandError {
    CommandError::NotFound {
        entity: entity.into(),
        id: id.to_string(),
    }
}

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

fn limit(entity: &str, max: usize) -> CommandError {
    CommandError::Limit {
        entity: entity.into(),
        max: max as u64,
    }
}

fn unchanged(output: Output) -> Out {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

/// The safety limits of lanes and points the commands enforce, as a value so that the tests
/// can use small ones.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Caps {
    pub lanes_per_track: usize,
    pub lanes: usize,
    pub points_per_lane: usize,
    pub points: usize,
}

impl Caps {
    /// The limits of the project (`project::limits`), which the engine's tables bound.
    pub(crate) const MVP: Caps = Caps {
        lanes_per_track: MAX_LANES_PER_TRACK,
        lanes: MAX_LANES,
        points_per_lane: MAX_POINTS_PER_LANE,
        points: MAX_POINTS,
    };
}

/// Applies a command of this domain.
pub fn apply(command: &Command, state: &mut State, ctx: &mut Context) -> Result<Out, CommandError> {
    apply_with(Caps::MVP, command, state, ctx)
}

/// [`apply`] under given limits.
pub(crate) fn apply_with(
    caps: Caps,
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Out, CommandError> {
    match command {
        Command::LaneCreate { track, target } => lane_create(caps, state, ctx, track, target),
        Command::LaneDelete { lane } => lane_delete(state, lane),
        Command::LaneRestore {
            track,
            lane,
            snapshot,
        } => lane_restore(caps, state, track, lane, snapshot),
        Command::PointsAdd { lane, points } => points_add(caps, state, lane, points),
        Command::PointsRemove { lane, ticks } => points_remove(state, lane, ticks),
        Command::PointsUpdate { lane, points } => points_update(caps, state, lane, points),
        Command::LaneClear { lane } => lane_clear(state, lane),
        Command::PointsRestore {
            lane,
            points,
            remove,
        } => points_restore(caps, state, lane, points, remove),
    }
}

/// How the values of a lane are bounded and rounded: what its parameter declares.
#[derive(Clone, Copy)]
enum Rule {
    /// A device parameter with its descriptor; a device without one (a module) keeps any
    /// finite value.
    Param(Option<&'static ParamDescriptor>),
    Gain,
    Pan,
}

impl Rule {
    /// The value as it is, when the parameter can hold it: finite, and for a gain or a pan
    /// within its range. What an undo gives back.
    fn verbatim(self, value: f64) -> Result<f64, CommandError> {
        if !value.is_finite() {
            return Err(invalid(
                "value",
                format!("an automation value is finite, not {value}"),
            ));
        }
        let range = match self {
            Self::Gain => Some((MIN_GAIN_DB, MAX_GAIN_DB)),
            Self::Pan => Some((-1.0, 1.0)),
            Self::Param(_) => None,
        };
        match range {
            Some((min, max)) if !(min..=max).contains(&value) => Err(invalid(
                "value",
                format!("this parameter is {min} to {max}, not {value}"),
            )),
            _ => Ok(value),
        }
    }

    /// The value a point stores for `value`: bounded and rounded to the parameter's step.
    fn value(self, value: f64) -> Result<f64, CommandError> {
        if !value.is_finite() {
            return Err(invalid(
                "value",
                format!("an automation value is finite, not {value}"),
            ));
        }
        let (min, max, step) = match self {
            Self::Param(Some(descriptor)) => {
                return Ok(descriptor.sanitize_f64(value).unwrap_or(value) + 0.0);
            }
            Self::Param(None) => return Ok(value + 0.0),
            Self::Gain => (MIN_GAIN_DB, MAX_GAIN_DB, GAIN_STEP_DB),
            Self::Pan => (-1.0, 1.0, PAN_STEP),
        };
        if !(min..=max).contains(&value) {
            return Err(invalid(
                "value",
                format!("this parameter is {min} to {max}, not {value}"),
            ));
        }
        // Whole steps divided by the step count: the double nearest to the decimal, so
        // that the file holds `0.3`, not `0.30000000000000004`. `+ 0.0` turns `-0` into `0`.
        let per_unit = (1.0 / step).round();
        Ok((value * per_unit).round() / per_unit + 0.0)
    }
}

/// The rule of a target in a track, and the check that the target exists there: the device
/// is in the track's chain and declares the parameter.
fn rule_of(state: &State, track: &Track, target: &AutomationTarget) -> Result<Rule, CommandError> {
    match target {
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        } => Ok(Rule::Gain),
        AutomationTarget::Mixer {
            param: MixerParam::Pan,
        } => Ok(Rule::Pan),
        AutomationTarget::Device { device, param } => {
            let Some(found) = track.devices.get(device) else {
                if state.project.master_devices.contains_key(device) {
                    return Err(invalid(
                        "target",
                        "the master's devices are not automatable yet: a lane lives in a track",
                    ));
                }
                return Err(not_found("device", device));
            };
            let descriptor = descriptor_of(registry(), found);
            if let Some(d) = descriptor
                && !d.params.iter().any(|p| p.key == param)
            {
                return Err(not_found("param", param));
            }
            if !found.params.contains_key(param) {
                return Err(not_found("param", param));
            }
            let param = descriptor.and_then(|d| d.params.iter().find(|p| p.key == param));
            Ok(Rule::Param(param))
        }
    }
}

/// The track that holds a lane.
fn track_of(state: &State, lane: &LaneId) -> Result<TrackId, CommandError> {
    state
        .project
        .tracks
        .iter()
        .find(|(_, t)| t.automation.contains_key(lane))
        .map(|(id, _)| id.clone())
        .ok_or_else(|| not_found("lane", lane))
}

fn track_ref<'a>(state: &'a State, track: &TrackId) -> Result<&'a Track, CommandError> {
    state
        .project
        .tracks
        .get(track)
        .ok_or_else(|| not_found("track", track))
}

/// The lane and its track, to edit.
fn lane_mut<'a>(
    state: &'a mut State,
    lane: &LaneId,
) -> Result<(TrackId, &'a mut AutomationLane), CommandError> {
    let track = track_of(state, lane)?;
    let found = state
        .project
        .tracks
        .get_mut(&track)
        .and_then(|t| t.automation.get_mut(lane))
        .ok_or_else(|| not_found("lane", lane))?;
    Ok((track, found))
}

/// Whether a lane may be added to the track under the safety limits of lanes.
fn check_lane_limits(caps: Caps, state: &State, track: &Track) -> Result<(), CommandError> {
    if track.automation.len() >= caps.lanes_per_track {
        return Err(limit("lanes", caps.lanes_per_track));
    }
    if state.project.counts().lanes >= caps.lanes {
        return Err(limit("lanes", caps.lanes));
    }
    Ok(())
}

/// Whether a new lane may be added: the limits and the one lane per parameter rule. A
/// restore applies the limits only: it gives back what was there, even a project that a merge
/// left with two lanes on one parameter.
fn check_new_lane(
    caps: Caps,
    state: &State,
    track: &Track,
    target: &AutomationTarget,
) -> Result<(), CommandError> {
    check_lane_limits(caps, state, track)?;
    if track.automation.values().any(|l| l.target == *target) {
        return Err(invalid(
            "target",
            "the track already has a lane on that parameter",
        ));
    }
    Ok(())
}

fn lane_events(
    make: fn(TrackId, LaneId) -> ProjectEvent,
    track: &TrackId,
    lane: &LaneId,
) -> Vec<ProjectEvent> {
    vec![make(track.clone(), lane.clone())]
}

fn created(track: TrackId, lane: LaneId) -> ProjectEvent {
    ProjectEvent::AutomationLaneCreated { track, lane }
}

fn deleted(track: TrackId, lane: LaneId) -> ProjectEvent {
    ProjectEvent::AutomationLaneDeleted { track, lane }
}

fn points_changed(track: TrackId, lane: LaneId) -> ProjectEvent {
    ProjectEvent::AutomationPointsChanged { track, lane }
}

fn lane_create(
    caps: Caps,
    state: &mut State,
    ctx: &mut Context,
    track_id: &TrackId,
    target: &AutomationTarget,
) -> Result<Out, CommandError> {
    let track = track_ref(state, track_id)?;
    rule_of(state, track, target)?;
    check_new_lane(caps, state, track, target)?;
    let lane = LaneId::generate(&mut ctx.ids);
    let track = state
        .project
        .tracks
        .get_mut(track_id)
        .ok_or_else(|| not_found("track", track_id))?;
    track.automation.insert(
        lane.clone(),
        AutomationLane {
            target: target.clone(),
            points: BTreeMap::new(),
        },
    );
    Ok(Outcome {
        events: lane_events(created, track_id, &lane),
        output: Output::LaneCreate { lane: lane.clone() },
        undo: Undo::Inverse(Box::new(Command::LaneDelete { lane })),
        control: vec![],
    })
}

fn lane_delete(state: &mut State, lane_id: &LaneId) -> Result<Out, CommandError> {
    let track_id = track_of(state, lane_id)?;
    let snapshot = state
        .project
        .tracks
        .get_mut(&track_id)
        .and_then(|t| t.automation.remove(lane_id))
        .ok_or_else(|| not_found("lane", lane_id))?;
    Ok(Outcome {
        events: lane_events(deleted, &track_id, lane_id),
        output: Output::LaneDelete {
            track: track_id.clone(),
        },
        undo: Undo::Inverse(Box::new(Command::LaneRestore {
            track: track_id,
            lane: lane_id.clone(),
            snapshot,
        })),
        control: vec![],
    })
}

fn lane_restore(
    caps: Caps,
    state: &mut State,
    track_id: &TrackId,
    lane_id: &LaneId,
    snapshot: &AutomationLane,
) -> Result<Out, CommandError> {
    if state.project.contains_id(&lane_id.0) {
        return Err(invalid(
            "lane",
            format!("the identifier {lane_id} is already in use"),
        ));
    }
    let track = track_ref(state, track_id)?;
    rule_of(state, track, &snapshot.target)?;
    check_lane_limits(caps, state, track)?;
    if snapshot.points.len() > caps.points_per_lane {
        return Err(limit("automation points", caps.points_per_lane));
    }
    if points_in(state).saturating_add(snapshot.points.len()) > caps.points {
        return Err(limit("automation points", caps.points));
    }
    if let Some(track) = state.project.tracks.get_mut(track_id) {
        track.automation.insert(lane_id.clone(), snapshot.clone());
    }
    // The snapshot is data from outside: the whole project must stay valid with it (ticks
    // in range, finite values, a gain or a pan in its range). A refusal takes the lane back
    // out, so the state is untouched.
    if let Err(violations) = state.project.validate() {
        if let Some(track) = state.project.tracks.get_mut(track_id) {
            track.automation.remove(lane_id);
        }
        let first = violations
            .first()
            .map_or_else(String::new, ToString::to_string);
        return Err(invalid("snapshot", first));
    }
    Ok(Outcome {
        events: lane_events(created, track_id, lane_id),
        output: Output::LaneRestore {},
        undo: Undo::Inverse(Box::new(Command::LaneDelete {
            lane: lane_id.clone(),
        })),
        control: vec![],
    })
}

/// The points of the whole project.
fn points_in(state: &State) -> usize {
    state
        .project
        .tracks
        .values()
        .flat_map(|t| t.automation.values())
        .fold(0usize, |n, l| n.saturating_add(l.points.len()))
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn check_tick(tick: Tick) -> Result<(), CommandError> {
    if (0..=MAX_TICK).contains(&tick.get()) {
        Ok(())
    } else {
        Err(invalid(
            "tick",
            format!("a tick is 0 to {MAX_TICK}, not {}", tick.get()),
        ))
    }
}

/// The points of a batch, checked and rounded, by tick; refuses two points on one tick.
fn staged(
    caps: Caps,
    rule: Rule,
    points: &[Point],
) -> Result<BTreeMap<Tick, AutomationPoint>, CommandError> {
    if points.len() > caps.points_per_lane {
        return Err(limit("automation points", caps.points_per_lane));
    }
    let mut out = BTreeMap::new();
    for p in points {
        check_tick(p.tick)?;
        let value = rule.value(p.value)?;
        let point = AutomationPoint {
            value,
            shape: p.shape,
        };
        if out.insert(p.tick, point).is_some() {
            return Err(invalid(
                "points",
                format!("tick {} appears twice in the batch", p.tick.get()),
            ));
        }
    }
    Ok(out)
}

fn as_points<'a>(points: impl IntoIterator<Item = (&'a Tick, &'a AutomationPoint)>) -> Vec<Point> {
    points
        .into_iter()
        .map(|(tick, p)| Point {
            tick: *tick,
            value: p.value,
            shape: p.shape,
        })
        .collect()
}

/// The rule of an existing lane.
fn rule_of_lane(state: &State, track_id: &TrackId, lane: &LaneId) -> Result<Rule, CommandError> {
    let track = track_ref(state, track_id)?;
    let found = track
        .automation
        .get(lane)
        .ok_or_else(|| not_found("lane", lane))?;
    rule_of(state, track, &found.target)
}

fn points_add(
    caps: Caps,
    state: &mut State,
    lane_id: &LaneId,
    points: &[Point],
) -> Result<Out, CommandError> {
    let track_id = track_of(state, lane_id)?;
    let rule = rule_of_lane(state, &track_id, lane_id)?;
    let batch = staged(caps, rule, points)?;
    let output = |added: usize| Output::PointsAdd {
        added: count(added),
    };
    let total = points_in(state);
    let (_, lane) = lane_mut(state, lane_id)?;
    let mut fresh = BTreeMap::new();
    for (tick, point) in batch {
        match lane.points.get(&tick) {
            Some(held) if *held == point => {}
            Some(_) => {
                return Err(invalid(
                    "points",
                    format!(
                        "tick {} already holds a point: automation.points.update changes it",
                        tick.get()
                    ),
                ));
            }
            None => {
                fresh.insert(tick, point);
            }
        }
    }
    if fresh.is_empty() {
        return Ok(unchanged(output(0)));
    }
    if lane.points.len().saturating_add(fresh.len()) > caps.points_per_lane {
        return Err(limit("automation points", caps.points_per_lane));
    }
    if total.saturating_add(fresh.len()) > caps.points {
        return Err(limit("automation points", caps.points));
    }
    let ticks: Vec<Tick> = fresh.keys().copied().collect();
    let added = fresh.len();
    lane.points.extend(fresh);
    Ok(Outcome {
        events: lane_events(points_changed, &track_id, lane_id),
        output: output(added),
        undo: Undo::Inverse(Box::new(Command::PointsRemove {
            lane: lane_id.clone(),
            ticks,
        })),
        control: vec![],
    })
}

fn points_remove(state: &mut State, lane_id: &LaneId, ticks: &[Tick]) -> Result<Out, CommandError> {
    let (track_id, lane) = lane_mut(state, lane_id)?;
    let mut taken = BTreeMap::new();
    for tick in ticks {
        if let Some(point) = lane.points.remove(tick) {
            taken.insert(*tick, point);
        }
    }
    let output = Output::PointsRemove {
        removed: count(taken.len()),
    };
    if taken.is_empty() {
        return Ok(unchanged(output));
    }
    Ok(Outcome {
        events: lane_events(points_changed, &track_id, lane_id),
        output,
        undo: Undo::Inverse(Box::new(Command::PointsRestore {
            lane: lane_id.clone(),
            points: as_points(&taken),
            remove: vec![],
        })),
        control: vec![],
    })
}

fn points_update(
    caps: Caps,
    state: &mut State,
    lane_id: &LaneId,
    points: &[Point],
) -> Result<Out, CommandError> {
    let track_id = track_of(state, lane_id)?;
    let rule = rule_of_lane(state, &track_id, lane_id)?;
    let batch = staged(caps, rule, points)?;
    let (_, lane) = lane_mut(state, lane_id)?;
    let mut previous = BTreeMap::new();
    let mut changes = Vec::new();
    for (tick, point) in batch {
        let held = lane
            .points
            .get(&tick)
            .ok_or_else(|| not_found("point", tick.get()))?;
        if *held != point {
            previous.insert(tick, *held);
            changes.push((tick, point));
        }
    }
    let output = Output::PointsUpdate {
        updated: count(changes.len()),
    };
    if changes.is_empty() {
        return Ok(unchanged(output));
    }
    lane.points.extend(changes);
    Ok(Outcome {
        events: lane_events(points_changed, &track_id, lane_id),
        output,
        undo: Undo::Inverse(Box::new(Command::PointsRestore {
            lane: lane_id.clone(),
            points: as_points(&previous),
            remove: vec![],
        })),
        control: vec![],
    })
}

fn lane_clear(state: &mut State, lane_id: &LaneId) -> Result<Out, CommandError> {
    let (track_id, lane) = lane_mut(state, lane_id)?;
    let taken = std::mem::take(&mut lane.points);
    let output = Output::LaneClear {
        removed: count(taken.len()),
    };
    if taken.is_empty() {
        return Ok(unchanged(output));
    }
    Ok(Outcome {
        events: lane_events(points_changed, &track_id, lane_id),
        output,
        undo: Undo::Inverse(Box::new(Command::PointsRestore {
            lane: lane_id.clone(),
            points: as_points(&taken),
            remove: vec![],
        })),
        control: vec![],
    })
}

fn points_restore(
    caps: Caps,
    state: &mut State,
    lane_id: &LaneId,
    points: &[Point],
    remove: &[Tick],
) -> Result<Out, CommandError> {
    let track_id = track_of(state, lane_id)?;
    let rule = rule_of_lane(state, &track_id, lane_id)?;
    if points.len() > caps.points_per_lane || remove.len() > caps.points_per_lane {
        return Err(limit("automation points", caps.points_per_lane));
    }
    let mut put = BTreeMap::new();
    for p in points {
        check_tick(p.tick)?;
        let point = AutomationPoint {
            value: rule.verbatim(p.value)?,
            shape: p.shape,
        };
        if put.insert(p.tick, point).is_some() {
            return Err(invalid(
                "points",
                format!("tick {} appears twice", p.tick.get()),
            ));
        }
    }
    let mut out = std::collections::BTreeSet::new();
    for tick in remove {
        if put.contains_key(tick) || !out.insert(*tick) {
            return Err(invalid(
                "remove",
                format!("tick {} appears twice or is also in `points`", tick.get()),
            ));
        }
    }
    let total = points_in(state);
    let (_, lane) = lane_mut(state, lane_id)?;
    // What the lane holds now at these ticks: the inverse puts it back, or takes the tick out.
    let mut previous = BTreeMap::new();
    let mut added = Vec::new();
    for (tick, point) in &put {
        match lane.points.get(tick) {
            Some(held) if held == point => {}
            Some(held) => {
                previous.insert(*tick, *held);
            }
            None => added.push(*tick),
        }
    }
    let gone: Vec<(Tick, AutomationPoint)> = out
        .iter()
        .filter_map(|t| lane.points.get(t).map(|p| (*t, *p)))
        .collect();
    previous.extend(gone.iter().copied());
    if previous.is_empty() && added.is_empty() {
        return Ok(unchanged(Output::PointsRestore {}));
    }
    let after = lane
        .points
        .len()
        .saturating_add(added.len())
        .saturating_sub(gone.len());
    if after > caps.points_per_lane {
        return Err(limit("automation points", caps.points_per_lane));
    }
    if total.saturating_add(added.len()).saturating_sub(gone.len()) > caps.points {
        return Err(limit("automation points", caps.points));
    }
    for (tick, _) in &gone {
        lane.points.remove(tick);
    }
    lane.points.extend(put);
    Ok(Outcome {
        events: lane_events(points_changed, &track_id, lane_id),
        output: Output::PointsRestore {},
        undo: Undo::Inverse(Box::new(Command::PointsRestore {
            lane: lane_id.clone(),
            points: as_points(&previous),
            remove: added,
        })),
        control: vec![],
    })
}

#[cfg(test)]
mod tests {
    //! The safety limits, with small ones: the real limits are millions of points.
    use super::*;
    use crate::testing::fixture;
    use project::{Track, TrackKind};

    fn caps(lanes_per_track: usize, lanes: usize, points_per_lane: usize, points: usize) -> Caps {
        Caps {
            lanes_per_track,
            lanes,
            points_per_lane,
            points,
        }
    }

    /// Applies a command with identifiers of its own: two calls never generate the same one.
    fn run(caps: Caps, state: &mut State, command: Command) -> Result<Out, CommandError> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut ctx = Context::new(
            crate::Author::User,
            project::IdGenerator::new(1_610_000_000_000 + n, 0x00DD_0000 + u128::from(n)),
        );
        apply_with(caps, &command, state, &mut ctx)
    }

    fn gain() -> AutomationTarget {
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        }
    }

    fn pan() -> AutomationTarget {
        AutomationTarget::Mixer {
            param: MixerParam::Pan,
        }
    }

    fn pts(ticks: impl IntoIterator<Item = i64>) -> Vec<Point> {
        ticks
            .into_iter()
            .map(|t| Point {
                tick: Tick(t),
                value: 0.0,
                shape: Shape::Linear,
            })
            .collect()
    }

    /// A state with a gain lane holding `n` points, and its ids.
    fn lane_with(n: i64) -> (State, TrackId, LaneId) {
        let f = fixture();
        let mut state = f.state;
        let out = run(
            Caps::MVP,
            &mut state,
            Command::LaneCreate {
                track: f.track.clone(),
                target: gain(),
            },
        )
        .unwrap();
        let Output::LaneCreate { lane } = out.output else {
            panic!()
        };
        run(
            Caps::MVP,
            &mut state,
            Command::PointsAdd {
                lane: lane.clone(),
                points: pts(0..n),
            },
        )
        .unwrap();
        (state, f.track, lane)
    }

    fn code(r: Result<Out, CommandError>) -> &'static str {
        r.map(|_| "applied").unwrap_or_else(|e| e.code())
    }

    #[test]
    fn a_lane_holds_at_most_the_points_per_lane_limit() {
        let (mut state, _, lane) = lane_with(3);
        let c = caps(10, 10, 3, 100);
        let before = state.clone();
        let add = |ticks| Command::PointsAdd {
            lane: lane.clone(),
            points: pts(ticks),
        };
        assert_eq!(code(run(c, &mut state, add(3..4))), "limit");
        assert_eq!(code(run(c, &mut state, add(3..7))), "limit");
        // A point already there is not new: the lane stays within the limit.
        assert_eq!(code(run(c, &mut state, add(0..3))), "applied");
        assert_eq!(state, before);
        // A batch longer than the limit is refused before anything is read.
        assert_eq!(code(run(c, &mut state, add(0..4))), "limit");
    }

    #[test]
    fn the_project_holds_at_most_the_points_limit() {
        let (mut state, track, lane) = lane_with(2);
        let other = run(
            Caps::MVP,
            &mut state,
            Command::LaneCreate {
                track,
                target: pan(),
            },
        )
        .unwrap();
        let Output::LaneCreate { lane: other } = other.output else {
            panic!()
        };
        let c = caps(10, 10, 100, 4);
        let add = |lane: &LaneId, ticks| Command::PointsAdd {
            lane: lane.clone(),
            points: pts(ticks),
        };
        // 2 points in the project: 2 more fit, the third does not.
        assert_eq!(code(run(c, &mut state, add(&other, 0..3))), "limit");
        assert_eq!(code(run(c, &mut state, add(&other, 0..2))), "applied");
        assert_eq!(code(run(c, &mut state, add(&lane, 10..11))), "limit");
    }

    #[test]
    fn a_track_and_the_project_hold_at_most_the_lane_limits() {
        let f = fixture();
        let mut state = f.state;
        let create = |track: &TrackId, target| Command::LaneCreate {
            track: track.clone(),
            target,
        };
        let c = caps(1, 100, 100, 100);
        run(c, &mut state, create(&f.track, gain())).unwrap();
        assert_eq!(code(run(c, &mut state, create(&f.track, pan()))), "limit");
        // Project-wide: a second track, one lane in all.
        let c = caps(10, 1, 100, 100);
        let second = TrackId::generate(&mut project::IdGenerator::new(1_600_000_000_001, 9));
        state.project.tracks.insert(
            second.clone(),
            Track::new(TrackKind::Midi, "Other", project::OrderKey::first()),
        );
        assert_eq!(code(run(c, &mut state, create(&second, gain()))), "limit");
    }

    #[test]
    fn a_restore_stays_within_the_limits_it_replaces_points_in() {
        let (mut state, _, lane) = lane_with(3);
        let c = caps(10, 10, 3, 100);
        let restore = |points, remove| Command::PointsRestore {
            lane: lane.clone(),
            points,
            remove,
        };
        // One out, one in: the count holds. Two in, one out: it does not.
        assert_eq!(
            code(run(c, &mut state, restore(pts(5..6), vec![Tick(0)]))),
            "applied"
        );
        let before = state.clone();
        assert_eq!(
            code(run(c, &mut state, restore(pts(7..9), vec![Tick(1)]))),
            "limit"
        );
        assert_eq!(state, before);
        // A lane restored past the limit is refused whole.
        let snapshot = state.project.tracks.values().next().unwrap().automation[&lane].clone();
        let (track, _) = state.project.tracks.iter().next().unwrap();
        let track = track.clone();
        let new = LaneId::generate(&mut project::IdGenerator::new(1_600_000_000_002, 5));
        let tight = caps(10, 10, 2, 100);
        assert_eq!(
            code(run(
                tight,
                &mut state,
                Command::LaneRestore {
                    track,
                    lane: new,
                    snapshot
                }
            )),
            "limit"
        );
    }
}
