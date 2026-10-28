//! `device.*`: a track's chain of devices: insert, remove, move, bypass and set a
//! parameter (epic 4, DV). PR-2 migrated `synth.set_param` of v1 as `device.set_param`;
//! DV-8 adds the others (`track.set_instrument` is in the `track` domain).
//!
//! The contract of the device commands, written once here and repeated in their
//! descriptions for agents:
//!
//! - A device is addressed by its `DeviceId`, a track by its `TrackId`. A track's chain is
//!   its instrument, always first, then its effects in order. `index` is a position among
//!   the **effects**, 0 first (right after the instrument); an index past the end means
//!   last. The instrument is never counted, moved or removed by these commands:
//!   `track.set_instrument` replaces it.
//! - The master has a chain too (#364): effects only, run on the sum of the tracks before
//!   the master's fader. `device.insert` and `device.restore` name the chain they work on:
//!   `track` for a track's, `master: true` for the master's, exactly one of the two. The
//!   other commands find a device by its identifier, which is unique in the project. The
//!   master is not a track: it has no instrument, no clip, no solo, and cannot be removed.
//! - The order of a chain is a fractional key in the document: `device.insert` and
//!   `device.move` turn the index into a key, and their inverses put the exact key back.
//! - `device.remove` takes the device with its settings; its inverse, `device.restore`,
//!   brings it back under the same identifier, order key, settings and bypass. Removal is
//!   undoable, hence not flagged destructive (the contract reserves that flag for edits
//!   without an inverse).
//! - Removing a device deletes the automation lanes that target it (AM-1); `device.restore`
//!   brings them back with it, under their identifiers and with their points. The master's
//!   devices have no lanes.
//! - A command that changes nothing returns no inverse and no event.
//!
//! Also here: the parameters of the built-in synth ([`SynthParam`]) with their engine ids
//! and defaults exported to the web app.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use engine::device::{DeviceDescriptor, DeviceRole as EngineRole};
use engine::devices::{Builtin, REGISTRY};
use engine::{Param, Synth};
use project::limits::MAX_DEVICES_PER_TRACK;
use project::order::by_order;
use project::{
    AutomationLane, AutomationTarget, CLIP_PLAYER, Device, DeviceId, DeviceRole, DeviceSource,
    LaneId, OrderKey, Track, TrackId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// For `skip_serializing_if`: a `master: false` is left out of the command.
fn is_false(value: &bool) -> bool {
    !*value
}

domain! {
    domain = "device", schema = ("DeviceCommand", "DeviceOutput");

    /// Insert an effect in a track's chain, or in the master's. Name the chain with
    /// `track` (a track's identifier) or with `master: true` (the master: the effects
    /// there process the sum of all the tracks before the master's volume), exactly one
    /// of the two. `kind` is the type id of a built-in effect (an instrument is refused:
    /// `track.set_instrument` replaces a track's instrument, and the master has none).
    /// `index` is the position among the chain's effects, 0 first (right after the
    /// instrument on a track); an index past the end means last. The new device starts at
    /// its default settings, not bypassed. Refused with `invalid` when both or neither of
    /// `track` and `master` are given, when the effects around that position share an
    /// order key (a merge made them equal), and with `limit` at the safety limit of
    /// devices per chain. Undo: `device.remove`.
    "device.insert" (kind = "edit", undo = "inverse", destructive = false)
    Insert {
        /// The track whose chain gets the effect; absent for the master.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<TrackId>,
        /// `true` to insert in the master's chain instead of a track's.
        #[serde(default, skip_serializing_if = "is_false")]
        master: bool,
        kind: String,
        index: u32,
    } => {
        /// The device created.
        device: DeviceId,
    };

    /// Remove an effect from its track or from the master, with its settings. The
    /// automation lanes that target the effect are deleted with it. Refused for an
    /// instrument (the oscillator, the clip player: replace it with `track.set_instrument`).
    /// Undo: `device.restore` brings it back with its identifier, position, settings, bypass
    /// and automation lanes.
    "device.remove" (kind = "edit", undo = "inverse", destructive = false)
    Remove { device: DeviceId } => {
        /// The track the device was in; `null` when it was in the master's chain.
        track: Option<TrackId>,
    };

    /// Bring a removed effect back with its identifier, as `device.remove` took it out:
    /// the inverse of a remove. `track` or `master: true` names the chain, as for
    /// `device.insert`. `snapshot` is the whole device: its order key, bypass, source,
    /// settings and state. Refused when both or neither of `track` and `master` are given,
    /// when the identifier is already in use, at the safety limit of devices, or when the
    /// snapshot is not an effect (a built-in device must be one of the registry, with only
    /// parameters it declares; a module device is taken as it is) or does not fit the
    /// chain. `lanes` are the automation lanes that targeted the device, as `device.remove`
    /// took them out (absent: none); each must target this device and a parameter it has,
    /// under an identifier not in use. Undo: `device.remove`.
    "device.restore" (kind = "edit", undo = "inverse", destructive = false)
    Restore {
        /// The track whose chain gets the effect back; absent for the master.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<TrackId>,
        /// `true` to restore into the master's chain instead of a track's.
        #[serde(default, skip_serializing_if = "is_false")]
        master: bool,
        device: DeviceId,
        snapshot: Device,
        /// The automation lanes of the device, by identifier.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        lanes: BTreeMap<LaneId, AutomationLane>,
    } => {};

    /// Move an effect to `index` among the effects of its chain (a track's or the master's),
    /// 0 first (right after the instrument on a track); an index past the end means last. The instrument cannot be moved.
    /// Refused with `invalid` when the effects around that position share an order key.
    /// Undo: `device.restore_order` puts the exact previous order key back.
    "device.move" (kind = "edit", undo = "inverse", destructive = false)
    Move { device: DeviceId, index: u32 } => {
        /// The effect's index before the command.
        previous_index: u32,
    };

    /// Put an effect back under an exact order key: the inverse of `device.move`, which
    /// takes an index. Refused when the key would put the effect before the instrument.
    /// Undo: set the key before this command back.
    "device.restore_order" (kind = "edit", undo = "inverse", destructive = false)
    RestoreOrder { device: DeviceId, order: OrderKey } => {};

    /// Bypass a device (`bypass: true`: the chain skips it, its settings are kept) or put
    /// it back (`false`). An instrument that is bypassed is silent. Refused for the clip
    /// player of an audio track. Undo: set the previous state back.
    "device.set_bypass" (kind = "edit", undo = "inverse", destructive = false)
    SetBypass { device: DeviceId, bypass: bool } => {
        /// Whether the device was bypassed before the command.
        previous: bool,
    };

    /// Set a parameter of a device, by the parameter's id, in the unit the device
    /// declares. The parameter must be one the device declares and the value must be
    /// finite; it is clamped to the parameter's range and rounded to its step, and the
    /// value stored (and returned) is the result. The built-in synth's parameters:
    /// `waveform` (0 sine, 1 saw, 2 square, 3 triangle), `attack`, `decay` and
    /// `release` (seconds, 0.002 to 30), `sustain` (0 to 1), `gain` (0 to 1, linear).
    /// Every device's parameters (key, unit, range, default, and a `description` of what
    /// each does) are in the device descriptors of the schema bundle. Undo: set the parameter back to its previous value.
    "device.set_param" (kind = "edit", undo = "inverse", destructive = false)
    SetParam {
        device: DeviceId,
        param: String,
        value: f64,
    } => {
        /// The value before the command.
        previous: f64,
        /// The value now: the one asked for, clamped and rounded.
        value: f64,
    };
}

type Out = Outcome<Command, Output>;

/// The devices the commands can build: the engine's registry. With the `testing` feature
/// (the crate's own tests and the integration tests of the workspace, never a shipped
/// build) the engine's test effect comes too, so that effects can be exercised before the
/// registry holds one (DV-3 to DV-5).
#[cfg(not(feature = "testing"))]
pub fn registry() -> &'static [Builtin] {
    REGISTRY
}

/// See the build without `testing`.
#[cfg(feature = "testing")]
pub fn registry() -> &'static [Builtin] {
    static ALL: std::sync::OnceLock<Vec<Builtin>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let test_effect = Builtin {
            descriptor: &engine::device::testing::GAIN,
            factory: engine::device::testing::TestGain::build,
        };
        REGISTRY.iter().copied().chain([test_effect]).collect()
    })
}

/// Applies a command of this domain.
pub fn apply(command: &Command, state: &mut State, ctx: &mut Context) -> Result<Out, CommandError> {
    apply_in(registry(), command, state, ctx)
}

/// [`apply`] against a given registry of devices.
pub(crate) fn apply_in(
    registry: &[Builtin],
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Out, CommandError> {
    match command {
        Command::Insert {
            track,
            master,
            kind,
            index,
        } => insert(
            registry,
            state,
            ctx,
            &chain_target(track, *master)?,
            kind,
            *index,
        ),
        Command::Remove { device } => remove(state, device),
        Command::Restore {
            track,
            master,
            device,
            snapshot,
            lanes,
        } => restore(
            registry,
            state,
            &chain_target(track, *master)?,
            device,
            snapshot,
            lanes,
        ),
        Command::Move { device, index } => move_device(state, device, *index),
        Command::RestoreOrder { device, order } => restore_order(state, device, order),
        Command::SetBypass { device, bypass } => set_bypass(state, device, *bypass),
        Command::SetParam {
            device,
            param,
            value,
        } => set_param(registry, state, device, param, *value),
    }
}

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

fn unchanged(output: Output) -> Out {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

/// The chain a device lives in: a track's, or the master's (#364).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    Track(TrackId),
    Master,
}

impl Place {
    /// The track, `None` for the master: the form of the command outputs and events.
    fn track(&self) -> Option<TrackId> {
        match self {
            Self::Track(t) => Some(t.clone()),
            Self::Master => None,
        }
    }

    /// The command fields `(track, master)` that name this chain.
    fn fields(&self) -> (Option<TrackId>, bool) {
        (self.track(), matches!(self, Self::Master))
    }
}

/// The chain `track` / `master` of a command names: exactly one of the two.
fn chain_target(track: &Option<TrackId>, master: bool) -> Result<Place, CommandError> {
    match (track, master) {
        (Some(t), false) => Ok(Place::Track(t.clone())),
        (None, true) => Ok(Place::Master),
        (Some(_), true) => Err(invalid(
            "master",
            "name the track or the master, not both: give `track`, or `master: true`",
        )),
        (None, false) => Err(invalid(
            "track",
            "name the chain: a `track`, or `master: true` for the master's chain",
        )),
    }
}

fn chain_changed(place: &Place) -> Vec<ProjectEvent> {
    vec![ProjectEvent::DeviceChainChanged {
        track: place.track(),
    }]
}

type Devices = BTreeMap<DeviceId, Device>;

/// The chain at `place`.
fn chain_mut<'a>(state: &'a mut State, place: &Place) -> Result<&'a mut Devices, CommandError> {
    match place {
        Place::Master => Ok(&mut state.project.master_devices),
        Place::Track(t) => Ok(&mut track_mut(state, t)?.devices),
    }
}

fn limit() -> CommandError {
    CommandError::Limit {
        entity: "devices".into(),
        max: MAX_DEVICES_PER_TRACK as u64,
    }
}

/// The chain that holds `device`: the master's, or a track's.
fn place_of(state: &State, device: &DeviceId) -> Result<Place, CommandError> {
    if state.project.master_devices.contains_key(device) {
        return Ok(Place::Master);
    }
    state
        .project
        .tracks
        .iter()
        .find(|(_, t)| t.devices.contains_key(device))
        .map(|(id, _)| Place::Track(id.clone()))
        .ok_or_else(|| not_found("device", device))
}

fn track_mut<'a>(state: &'a mut State, track: &TrackId) -> Result<&'a mut Track, CommandError> {
    state
        .project
        .tracks
        .get_mut(track)
        .ok_or_else(|| not_found("track", track))
}

fn is_clip_player(device: &Device) -> bool {
    matches!(&device.source, DeviceSource::Builtin { id, .. } if id == CLIP_PLAYER)
}

/// The descriptor of a built-in device of `registry`; `None` for the clip player and for
/// a module.
pub(crate) fn descriptor_of<'r>(
    registry: &'r [Builtin],
    device: &Device,
) -> Option<&'r DeviceDescriptor> {
    let DeviceSource::Builtin { id, .. } = &device.source else {
        return None;
    };
    registry
        .iter()
        .find(|b| b.descriptor.kind == id)
        .map(|b| b.descriptor)
}

/// A new device of a built-in kind, at its default settings, not bypassed.
pub(crate) fn built(builtin: &Builtin, role: DeviceRole, order: OrderKey) -> Device {
    let descriptor = builtin.descriptor;
    Device {
        order,
        role,
        enabled: true,
        source: DeviceSource::Builtin {
            id: descriptor.kind.into(),
            version: descriptor.version,
        },
        params: descriptor
            .params
            .iter()
            .map(|p| (p.key.to_owned(), p.default))
            .collect(),
        state: None,
    }
}

/// A device is one Faulk has: a built-in id of `registry` (or the clip player), with only
/// parameters its descriptor declares. A restore from outside must not bring in what the
/// registry cannot build.
pub(crate) fn check_device(
    registry: &[Builtin],
    id: &DeviceId,
    device: &Device,
) -> Result<(), CommandError> {
    let DeviceSource::Builtin { id: kind, .. } = &device.source else {
        return Ok(());
    };
    if kind == CLIP_PLAYER {
        if !device.params.is_empty() {
            return Err(invalid(
                "snapshot",
                format!("device {id}: the clip player has no parameters"),
            ));
        }
        return Ok(());
    }
    let builtin = registry
        .iter()
        .find(|b| b.descriptor.kind == kind)
        .ok_or_else(|| invalid("snapshot", format!("device {id}: unknown device {kind}")))?;
    if let Some(param) = device.params.keys().find(|k| {
        !builtin
            .descriptor
            .params
            .iter()
            .any(|p| p.key == k.as_str())
    }) {
        return Err(invalid(
            "snapshot",
            format!("device {id}: {kind} has no parameter {param}"),
        ));
    }
    Ok(())
}

/// The effects of a chain in order, without `except`.
fn effects<'a>(devices: &'a Devices, except: Option<&DeviceId>) -> Vec<(&'a DeviceId, &'a Device)> {
    by_order(devices, |d| &d.order)
        .into_iter()
        .filter(|(id, d)| d.role == DeviceRole::Effect && Some(*id) != except)
        .collect()
}

/// The order key that puts an effect at `index` among the track's effects (without
/// `except`, the device being moved). What the chain holds before its effects (the
/// instrument) bounds the key below.
fn key_at(
    devices: &Devices,
    except: Option<&DeviceId>,
    index: usize,
) -> Result<OrderKey, CommandError> {
    let chain: Vec<&OrderKey> = by_order(devices, |d| &d.order)
        .into_iter()
        .filter(|(id, _)| Some(*id) != except)
        .map(|(_, d)| &d.order)
        .collect();
    let fx = effects(devices, except).len();
    let position = (chain.len() - fx) + index.min(fx);
    let key = OrderKey::for_index(&chain, position).map_err(|e| invalid("index", e.to_string()))?;
    // Neighbours that share a key leave none between them: refuse rather than report a
    // position that does not happen.
    if chain.iter().filter(|k| ***k < key).count() != position {
        return Err(invalid(
            "index",
            "the devices around that position share an order key, so none fits between them",
        ));
    }
    Ok(key)
}

fn insert(
    registry: &[Builtin],
    state: &mut State,
    ctx: &mut Context,
    place: &Place,
    kind: &str,
    index: u32,
) -> Result<Out, CommandError> {
    let builtin = registry
        .iter()
        .find(|b| b.descriptor.kind == kind)
        .ok_or_else(|| not_found("device kind", kind))?;
    match builtin.descriptor.role {
        EngineRole::Effect => {}
        EngineRole::Instrument if *place == Place::Master => {
            return Err(invalid(
                "kind",
                format!("{kind} is an instrument: the master holds effects only"),
            ));
        }
        EngineRole::Instrument => {
            return Err(invalid(
                "kind",
                format!("{kind} is an instrument: track.set_instrument replaces the instrument"),
            ));
        }
        EngineRole::MidiEffect => {
            return Err(invalid(
                "kind",
                format!("{kind} is a MIDI effect, which a chain does not hold yet"),
            ));
        }
    }
    let devices = chain_mut(state, place)?;
    if devices.len() >= MAX_DEVICES_PER_TRACK {
        return Err(limit());
    }
    let key = key_at(devices, None, usize::try_from(index).unwrap_or(usize::MAX))?;
    let id = DeviceId::generate(&mut ctx.ids);
    devices.insert(id.clone(), built(builtin, DeviceRole::Effect, key));
    Ok(Outcome {
        output: Output::Insert { device: id.clone() },
        undo: Undo::Inverse(Box::new(Command::Remove { device: id })),
        events: chain_changed(place),
        control: vec![],
    })
}

fn remove(state: &mut State, device_id: &DeviceId) -> Result<Out, CommandError> {
    let place = place_of(state, device_id)?;
    let device = chain_mut(state, &place)?
        .get(device_id)
        .ok_or_else(|| not_found("device", device_id))?;
    if device.role == DeviceRole::Instrument {
        return Err(invalid(
            "device",
            "an instrument is not removed: track.set_instrument replaces it",
        ));
    }
    let snapshot = chain_mut(state, &place)?
        .remove(device_id)
        .ok_or_else(|| not_found("device", device_id))?;
    // The lanes that target the device go with it (AM-1), and come back with it.
    let mut lanes = BTreeMap::new();
    let mut events = Vec::new();
    if let Place::Track(track_id) = &place {
        let track = track_mut(state, track_id)?;
        let ids: Vec<LaneId> = track
            .automation
            .iter()
            .filter(|(_, lane)| matches!(&lane.target, AutomationTarget::Device { device: d, .. } if d == device_id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(lane) = track.automation.remove(&id) {
                events.push(ProjectEvent::AutomationLaneDeleted {
                    track: track_id.clone(),
                    lane: id.clone(),
                });
                lanes.insert(id, lane);
            }
        }
    }
    let (track, master) = place.fields();
    events.extend(chain_changed(&place));
    Ok(Outcome {
        output: Output::Remove {
            track: track.clone(),
        },
        undo: Undo::Inverse(Box::new(Command::Restore {
            track,
            master,
            device: device_id.clone(),
            snapshot,
            lanes,
        })),
        events,
        control: vec![],
    })
}

fn restore(
    registry: &[Builtin],
    state: &mut State,
    place: &Place,
    device_id: &DeviceId,
    snapshot: &Device,
    lanes: &BTreeMap<LaneId, AutomationLane>,
) -> Result<Out, CommandError> {
    if snapshot.role != DeviceRole::Effect || is_clip_player(snapshot) {
        return Err(invalid("snapshot", "only an effect is restored"));
    }
    if state.project.contains_id(&device_id.0) {
        return Err(invalid(
            "device",
            format!("the identifier {device_id} is already in use"),
        ));
    }
    check_device(registry, device_id, snapshot)?;
    check_lanes(state, place, device_id, snapshot, lanes)?;
    let devices = chain_mut(state, place)?;
    if devices.len() >= MAX_DEVICES_PER_TRACK {
        return Err(limit());
    }
    devices.insert(device_id.clone(), snapshot.clone());
    if let Place::Track(track_id) = place {
        track_mut(state, track_id)?
            .automation
            .extend(lanes.iter().map(|(id, lane)| (id.clone(), lane.clone())));
    }
    // The snapshot is data from outside: the whole project must stay valid with it (the
    // chain keeps its instrument first, parameters are finite, limits hold). A refusal
    // takes the device and its lanes back out, so the state is untouched.
    if let Err(violations) = state.project.validate() {
        if let Ok(devices) = chain_mut(state, place) {
            devices.remove(device_id);
        }
        if let Place::Track(track_id) = place
            && let Ok(track) = track_mut(state, track_id)
        {
            for id in lanes.keys() {
                track.automation.remove(id);
            }
        }
        let first = violations
            .first()
            .map_or_else(String::new, ToString::to_string);
        return Err(invalid("snapshot", first));
    }
    // The lanes first: the engine uploads them with the chain, which follows.
    let mut events: Vec<ProjectEvent> = Vec::new();
    if let Place::Track(track_id) = place {
        events.extend(
            lanes
                .keys()
                .map(|lane| ProjectEvent::AutomationLaneCreated {
                    track: track_id.clone(),
                    lane: lane.clone(),
                }),
        );
    }
    events.extend(chain_changed(place));
    Ok(Outcome {
        output: Output::Restore {},
        undo: Undo::Inverse(Box::new(Command::Remove {
            device: device_id.clone(),
        })),
        events,
        control: vec![],
    })
}

/// The lanes a restored device brings back: only in a track (the master has none), each
/// targeting this device and a parameter it has, under a new identifier, within the safety limits of lanes.
pub(crate) fn check_lanes(
    state: &State,
    place: &Place,
    device_id: &DeviceId,
    snapshot: &Device,
    lanes: &BTreeMap<LaneId, AutomationLane>,
) -> Result<(), CommandError> {
    if lanes.is_empty() {
        return Ok(());
    }
    let Place::Track(track_id) = place else {
        return Err(invalid(
            "lanes",
            "the master's devices have no automation lanes",
        ));
    };
    let track = state
        .project
        .tracks
        .get(track_id)
        .ok_or_else(|| not_found("track", track_id))?;
    for (id, lane) in lanes {
        if state.project.contains_id(&id.0) {
            return Err(invalid(
                "lanes",
                format!("the identifier {id} is already in use"),
            ));
        }
        let AutomationTarget::Device { device, param } = &lane.target else {
            return Err(invalid(
                "lanes",
                format!("lane {id} does not target a device"),
            ));
        };
        if device != device_id || !snapshot.params.contains_key(param) {
            return Err(invalid(
                "lanes",
                format!("lane {id} does not target a parameter of device {device_id}"),
            ));
        }
        crate::automation::check_restored_lane(snapshot, lane)?;
    }
    if track.automation.len().saturating_add(lanes.len()) > project::limits::MAX_LANES_PER_TRACK {
        return Err(CommandError::Limit {
            entity: "lanes".into(),
            max: project::limits::MAX_LANES_PER_TRACK as u64,
        });
    }
    if state.project.counts().lanes.saturating_add(lanes.len()) > project::limits::MAX_LANES {
        return Err(CommandError::Limit {
            entity: "lanes".into(),
            max: project::limits::MAX_LANES as u64,
        });
    }
    Ok(())
}

fn move_device(state: &mut State, device_id: &DeviceId, index: u32) -> Result<Out, CommandError> {
    let place = place_of(state, device_id)?;
    let devices = chain_mut(state, &place)?;
    let role = devices
        .get(device_id)
        .map(|d| d.role)
        .ok_or_else(|| not_found("device", device_id))?;
    if role != DeviceRole::Effect {
        return Err(invalid("device", "an instrument stays first in its chain"));
    }
    let previous_index = effects(devices, None)
        .iter()
        .position(|(id, _)| *id == device_id)
        .unwrap_or(0);
    let output = Output::Move {
        previous_index: u32::try_from(previous_index).unwrap_or(u32::MAX),
    };
    let others = effects(devices, Some(device_id)).len();
    let index = usize::try_from(index).unwrap_or(usize::MAX).min(others);
    if index == previous_index {
        return Ok(unchanged(output));
    }
    let key = key_at(devices, Some(device_id), index)?;
    let device = devices
        .get_mut(device_id)
        .ok_or_else(|| not_found("device", device_id))?;
    let before = std::mem::replace(&mut device.order, key);
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::RestoreOrder {
            device: device_id.clone(),
            order: before,
        })),
        events: chain_changed(&place),
        control: vec![],
    })
}

fn restore_order(
    state: &mut State,
    device_id: &DeviceId,
    order: &OrderKey,
) -> Result<Out, CommandError> {
    let place = place_of(state, device_id)?;
    let devices = chain_mut(state, &place)?;
    let device = devices
        .get_mut(device_id)
        .ok_or_else(|| not_found("device", device_id))?;
    if device.role != DeviceRole::Effect {
        return Err(invalid("device", "an instrument stays first in its chain"));
    }
    let output = Output::RestoreOrder {};
    if device.order == *order {
        return Ok(unchanged(output));
    }
    let before = std::mem::replace(&mut device.order, order.clone());
    // The effect must not land before the instrument (the master's chain has none: any
    // position holds).
    let instrument_first = by_order(&*devices, |d| &d.order)
        .iter()
        .position(|(_, d)| d.role == DeviceRole::Instrument)
        .is_none_or(|i| i == 0);
    if !instrument_first {
        if let Some(device) = devices.get_mut(device_id) {
            device.order = before;
        }
        return Err(invalid(
            "order",
            "that key would put the effect before the instrument",
        ));
    }
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::RestoreOrder {
            device: device_id.clone(),
            order: before,
        })),
        events: chain_changed(&place),
        control: vec![],
    })
}

fn set_bypass(state: &mut State, device_id: &DeviceId, bypass: bool) -> Result<Out, CommandError> {
    let device = state
        .project
        .device_mut(device_id)
        .ok_or_else(|| not_found("device", device_id))?;
    if is_clip_player(device) {
        return Err(invalid("device", "the clip player cannot be bypassed"));
    }
    let previous = !device.enabled;
    let output = Output::SetBypass { previous };
    if previous == bypass {
        return Ok(unchanged(output));
    }
    device.enabled = !bypass;
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::SetBypass {
            device: device_id.clone(),
            bypass: previous,
        })),
        events: vec![ProjectEvent::DeviceBypassChanged {
            device: device_id.clone(),
            bypass,
        }],
        control: vec![],
    })
}

fn set_param(
    registry: &[Builtin],
    state: &mut State,
    device_id: &DeviceId,
    param: &str,
    value: f64,
) -> Result<Out, CommandError> {
    if !value.is_finite() {
        return Err(invalid(
            "value",
            format!("a parameter value is finite, not {value}"),
        ));
    }
    let device = state
        .project
        .device_mut(device_id)
        .ok_or_else(|| not_found("device", device_id))?;
    // A built-in device declares its parameters: the id is checked against the descriptor
    // and the value is clamped to the range and rounded to the step. A device without a
    // descriptor (a module, until MO brings its own) keeps what the project holds.
    let value = match descriptor_of(registry, device) {
        Some(descriptor) => descriptor
            .params
            .iter()
            .find(|p| p.key == param)
            .ok_or_else(|| not_found("param", param))?
            .sanitize_f64(value)
            .unwrap_or(value),
        None => value,
    };
    let slot = device
        .params
        .get_mut(param)
        .ok_or_else(|| not_found("param", param))?;
    let previous = *slot;
    let output = Output::SetParam { previous, value };
    if previous == value {
        // Nothing changed: nothing to undo, nothing to tell the interface.
        return Ok(unchanged(output));
    }
    *slot = value;
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::SetParam {
            device: device_id.clone(),
            param: param.to_owned(),
            value: previous,
        })),
        events: vec![ProjectEvent::DeviceParamChanged {
            device: device_id.clone(),
            param: param.to_owned(),
            value,
        }],
        control: vec![],
    })
}

/// The built-in synth's type id.
pub const SYNTH_KIND: &str = "faulk.osc_synth";

/// A parameter of the built-in synth, with its unit and range. Its serde name is the
/// parameter id that `device.set_param` takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SynthParam {
    /// Oscillator waveform: 0 sine, 1 saw, 2 square, 3 triangle.
    Waveform,
    /// Attack time in seconds, 0.002 to 30.
    Attack,
    /// Decay time in seconds, 0.002 to 30.
    Decay,
    /// Sustain level, 0 to 1.
    Sustain,
    /// Release time in seconds, 0.002 to 30.
    Release,
    /// Master gain, 0 to 1 (linear).
    Gain,
}

impl SynthParam {
    /// Every parameter, in a stable order.
    pub const ALL: [SynthParam; 6] = [
        Self::Waveform,
        Self::Attack,
        Self::Decay,
        Self::Sustain,
        Self::Release,
        Self::Gain,
    ];

    pub fn to_engine(self) -> Param {
        match self {
            Self::Waveform => Param::Waveform,
            Self::Attack => Param::Attack,
            Self::Decay => Param::Decay,
            Self::Sustain => Param::Sustain,
            Self::Release => Param::Release,
            Self::Gain => Param::Gain,
        }
    }

    /// The parameter id, as `device.set_param` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Waveform => "waveform",
            Self::Attack => "attack",
            Self::Decay => "decay",
            Self::Sustain => "sustain",
            Self::Release => "release",
            Self::Gain => "gain",
        }
    }

    /// The parameter with this id, if the synth has one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// Stable numeric id of each synth parameter across the WebAssembly boundary,
/// exported so the web app never hard-codes them.
pub fn param_ids() -> Vec<(&'static str, u32)> {
    SynthParam::ALL
        .iter()
        .map(|p| (p.name(), p.to_engine().id()))
        .collect()
}

/// Default value of each synth parameter, read from a fresh engine, exported so the
/// web app starts from the engine's values.
pub fn param_defaults() -> Vec<(&'static str, f32)> {
    let synth = Synth::new(48_000.0);
    SynthParam::ALL
        .iter()
        .map(|p| (p.name(), synth.param(p.to_engine())))
        .collect()
}
