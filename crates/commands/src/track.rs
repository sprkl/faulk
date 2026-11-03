//! `track.*`: create, delete, restore, rename, colour, reorder and duplicate tracks (PR-8),
//! and arm or monitor them (RE-1). Selecting a track is `view.select_track`.
//!
//! The contract of the track commands, written once here and repeated in their
//! descriptions for agents:
//!
//! - A track is addressed by its `TrackId`. The tracks are ordered; `index` is a
//!   position in that order, 0 first.
//! - **No cap below the safety limit.** Creating or duplicating a track is refused with
//!   `limit` only at `project::limits::MAX_TRACKS`; below it nothing stops a project from
//!   growing.
//! - `track.create` makes a MIDI track play one instrument of the device registry (the
//!   oscillator by default), or an audio track, which holds the clip player. The new
//!   track goes last, with the default mixer and no name unless one is given: the
//!   interface shows a default name in the user's language for an empty one.
//! - `track.delete` takes the whole track with it: its devices, clips, placements and
//!   automation. Its inverse, `track.restore`, brings all of it back under the same
//!   identifiers and order key, so deleting then undoing leaves the project as it was.
//!   It is undoable, hence not flagged destructive. The track's selection and solo are
//!   session state: deleting clears them, undoing does not bring them back.
//! - **Arming and monitoring** are session state like solo (`live`, not undoable, never in
//!   the project nor the history): `track.arm` and `track.set_monitor` write the session's
//!   `armed` and `monitor` sets, and the engine gets them as `TrackArm` and
//!   `TrackSetMonitor`. Deleting a track clears both.
//! - `track.duplicate` copies the track next to the source with new identifiers
//!   everywhere. Its clips are new, independent clips (founder's decision 11). The copy is
//!   never muted, and is neither soloed nor armed (session state is not copied).
//! - A command that changes nothing returns no inverse and no event.
//!
//! The order of tracks is a fractional key in the document. `track.move` takes the index
//! the user sees and turns it into a key; its inverse, `track.restore_order`, puts the
//! exact previous key back so that undo is exact.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, TrackPart, Undo};
use crate::macros::domain;
use crate::query::{ClipSummary, DeviceSummary, PageInfo, TrackSummary};
use engine::devices::Builtin;
use project::limits::{
    MAX_CLIPS, MAX_LANES, MAX_NAME_CHARS, MAX_NOTES, MAX_PLACEMENTS, MAX_POINTS, MAX_TRACKS,
};
use project::{
    AutomationLane, AutomationTarget, CLIP_PLAYER, Clip, ClipContent, ClipId, Device, DeviceId,
    DeviceRole, DeviceSource, LaneId, OrderKey, Placement, PlacementId, Rgb, Track, TrackId,
    TrackKind,
};
use std::collections::BTreeMap;

/// The instrument a MIDI track plays when `track.create` names none.
pub const DEFAULT_INSTRUMENT: &str = crate::device::SYNTH_KIND;

domain! {
    domain = "track", schema = ("TrackCommand", "TrackOutput");

    /// Create a track at the end of the list: `kind` is `midi` or `audio`. A MIDI track
    /// plays `instrument`, the type id of a built-in instrument (`faulk.osc_synth`, the
    /// default); an audio track plays the clip player and takes no `instrument`. `name`
    /// is optional (trimmed, at most 200 characters; empty by default) and so is `color`
    /// (`#rrggbb`; by default the first of the sixteen palette hues no track has yet, stored
    /// with the track so that it never changes when other tracks move). Refused with `limit` only at the project's safety limit of tracks.
    /// Undo: `track.delete`.
    "track.create" (kind = "edit", undo = "inverse", destructive = false)
    Create {
        kind: TrackKind,
        instrument: Option<String>,
        name: Option<String>,
        color: Option<Rgb>,
    } => {
        /// The track created.
        track: TrackId,
        /// Its instrument: the chosen one, or the clip player of an audio track.
        device: DeviceId,
    };

    /// Delete a track with its devices, clips, placements and automation. The selection,
    /// the solo, the arming and the monitoring of the track are cleared (session state,
    /// not restored by undo: an undone delete brings the track back unarmed). Undo:
    /// `track.restore` brings the whole track back with its identifiers and position.
    "track.delete" (kind = "edit", undo = "inverse", destructive = false)
    Delete { track: TrackId } => {
        /// How many devices were removed with the track.
        devices: u32,
        /// How many clips were removed with the track.
        clips: u32,
    };

    /// Bring a deleted track back with its identifier, as `track.delete` removed it: the
    /// inverse of a delete. `snapshot` is the whole track: its order key, mixer,
    /// devices, clips, placements and automation. Refused when the track's identifier or
    /// any identifier inside it is already in use, at the safety limit of tracks, or when
    /// the snapshot is not a valid track of the project. Undo: `track.delete`.
    "track.restore" (kind = "edit", undo = "inverse", destructive = false)
    Restore {
        track: TrackId,
        snapshot: Track,
    } => {};

    /// Rename a track. The name is trimmed, at most 200 characters; an empty name clears
    /// it. Undo: rename it back.
    "track.rename" (kind = "edit", undo = "inverse", destructive = false)
    Rename { track: TrackId, name: String } => {
        /// The name before the command.
        previous: String,
    };

    /// Set the colour of a track, `#rrggbb`, or `null` for none (the page then shows the hue of the
    /// track's position, which moves with the order: prefer a colour). Undo: set the previous
    /// colour back.
    "track.set_color" (kind = "edit", undo = "inverse", destructive = false)
    SetColor {
        track: TrackId,
        /// Required: `null` clears the colour, a missing field is an error.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(required)]
        color: Option<Rgb>,
    } => {
        /// The colour before the command.
        previous: Option<Rgb>,
    };

    /// Move a track to `index` in the list of tracks, 0 first; an index past the end
    /// means last. Refused with `invalid` when the tracks around that position share an
    /// order key (a merge made them equal), since none fits between them. Undo: `track.restore_order` puts the exact previous order key back.
    "track.move" (kind = "edit", undo = "inverse", destructive = false)
    Move { track: TrackId, index: u32 } => {
        /// The track's index before the command.
        previous_index: u32,
    };

    /// Put a track back under an exact order key: the inverse of `track.move`, which
    /// takes an index. Refused when the key is not a valid order key (checked on read).
    /// Undo: set the key before this command back.
    "track.restore_order" (kind = "edit", undo = "inverse", destructive = false)
    RestoreOrder { track: TrackId, order: OrderKey } => {};

    /// Replace a MIDI track's instrument by another instrument of the registry: `instrument`
    /// is the type id of a built-in instrument (`faulk.osc_synth`, `faulk.drums`). The new
    /// device starts at its default settings under a new identifier and takes the old
    /// one's place first in the chain; the effects stay. The old instrument's settings are
    /// lost unless undone. Choosing the instrument the track already plays changes
    /// nothing. The automation lanes that target the old instrument go with it (lead's
    /// decision on #380; the output counts them in `removed_lanes`, so that the interface
    /// can say so) and come back with it on undo, in the same history entry. Refused on an
    /// audio track (it plays the clip player) and for a type that is not an instrument.
    /// Undo: `track.restore_instrument` brings the old instrument back with its
    /// identifier, settings and lanes.
    "track.set_instrument" (kind = "edit", undo = "inverse", destructive = false)
    SetInstrument { track: TrackId, instrument: String } => {
        /// The track's instrument after the command.
        device: DeviceId,
        /// Its instrument before the command.
        previous: DeviceId,
        /// How many automation lanes of the old instrument were removed with it.
        removed_lanes: u32,
    };

    /// Swap a track's instrument for a device taken out earlier: the inverse of
    /// `track.set_instrument`. `current` is the instrument in the track now, replaced by
    /// `snapshot` under the identifier `device`. Refused when `device` is already in use,
    /// when `current` is not the track's instrument, or when the snapshot is not an
    /// instrument that fits the track (a built-in device must be one of the registry; a
    /// module device is taken as it is). The lanes that target `current` are removed with
    /// it; `lanes` (absent: none) are the lanes that targeted the instrument put back, as
    /// `track.set_instrument` took them out: each must target the restored instrument and
    /// a parameter it has, under an identifier not in use. Undo: the same swap the other
    /// way, which takes `lanes` out again and brings the lanes of `current` back.
    "track.restore_instrument" (kind = "edit", undo = "inverse", destructive = false)
    RestoreInstrument {
        track: TrackId,
        current: DeviceId,
        device: DeviceId,
        snapshot: Device,
        /// The automation lanes of the instrument put back, by identifier.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        lanes: BTreeMap<LaneId, AutomationLane>,
    } => {};

    /// Arm or disarm a track: live input (`live.input_note_on`, the keyboard and MIDI
    /// controllers) reaches the armed tracks as well as the selected one, and recording
    /// captures them. Session state: not undoable, not in the history, not saved with
    /// the project. Refused with `not_found` for a track that does not exist, except to
    /// disarm a track deleted since.
    "track.arm" (kind = "live", undo = "none", destructive = false)
    Arm { track: TrackId, armed: bool } => {
        /// Whether the track was armed before the command.
        previous: bool,
    };

    /// Monitor or stop monitoring a track's live input through its chain. Session
    /// state: not undoable, not in the history, not saved with the project. A MIDI
    /// track's instrument sounds what is played whatever this says; it matters for the
    /// audio input. Refused with `not_found` for a track that does not exist, except to
    /// stop monitoring a track deleted since.
    "track.set_monitor" (kind = "live", undo = "none", destructive = false)
    SetMonitor { track: TrackId, monitor: bool } => {
        /// Whether the track was monitored before the command.
        previous: bool,
    };

    /// Duplicate a track right after the source (after any track that shares its order
    /// key), with new identifiers for the track, its
    /// devices, clips, placements and automation lanes. The clips are new, independent
    /// clips with the same content and the same names.
    /// `name` is optional (trimmed, at most 200
    /// characters): the copy has the source's name and colour by default
    /// (recolour it with `track.set_color`). The copy is never muted
    /// (volume and pan are copied); solo and arming are session state of the source and
    /// are not copied. Refused with `limit` at
    /// the safety limit of tracks, notes or automation points. Undo: `track.delete` of
    /// the copy.
    "track.duplicate" (kind = "edit", undo = "inverse", destructive = false)
    Duplicate { track: TrackId, name: Option<String> } => {
        /// The new track.
        track: TrackId,
    };

    /// List the tracks in display order (the order of the arrangement and the mixer), each
    /// with its name, colour, kind (`midi` or `audio`), mixer (gain in dB, `null` is
    /// silence; pan; mute), solo and arming (session state) and how many devices and clips
    /// it holds. Paged: `offset` (tracks to skip, 0 by default) and `limit` (tracks to
    /// return, 1 to 1000, 100 by default; any other limit is refused with `invalid`, never
    /// clamped); the answer's `page` gives the `total`, the `offset` used and the
    /// `next_offset` of the next page, `null` after the last one. An `offset` past the end
    /// gives an empty page. A query: changes nothing and never enters the history.
    "track.list" (kind = "query", undo = "none", destructive = false)
    List {
        /// Tracks to skip, 0 by default.
        offset: Option<u32>,
        /// Tracks to return, 1 to 1000, 100 by default.
        limit: Option<u32>,
    } => {
        /// The tracks of this page, in display order.
        tracks: Vec<TrackSummary>,
        /// Where this page sits in the list.
        page: PageInfo,
    };

    /// Read one track: its summary, its whole device chain in order (each device with its
    /// source, role, bypass state, English name and parameter values; the chain is bounded
    /// by the project's safety limit), a page of its clips (in the order of their
    /// identifiers, which is creation order) and how many arrangement placements and
    /// automation lanes it has (`clip.get` lists a clip's placements, `automation.lane.list`
    /// the lanes). Paged on the clips: `offset` (clips to skip, 0 by default) and `limit`
    /// (clips to return, 1 to 1000, 100 by default; any other limit is refused with
    /// `invalid`, never clamped); the answer's `page` gives the `total`, the `offset` used
    /// and the `next_offset` of the next page, `null` after the last one. Refused with
    /// `not_found` for an unknown track. A query: changes nothing and never enters the
    /// history.
    "track.get" (kind = "query", undo = "none", destructive = false)
    Get {
        /// The track to read.
        track: TrackId,
        /// Clips to skip, 0 by default.
        offset: Option<u32>,
        /// Clips to return, 1 to 1000, 100 by default.
        limit: Option<u32>,
    } => {
        /// The track's summary.
        track: TrackSummary,
        /// Its device chain, in order.
        devices: Vec<DeviceSummary>,
        /// The clips of this page.
        clips: Vec<ClipSummary>,
        /// Where this page of clips sits in the track's clip list.
        page: PageInfo,
        /// How many arrangement placements the track has.
        placement_count: u32,
        /// How many automation lanes the track has.
        lane_count: u32,
    };
}

type Out = Outcome<Command, Output>;

/// Applies a command of this domain.
pub fn apply(command: &Command, state: &mut State, ctx: &mut Context) -> Result<Out, CommandError> {
    match command {
        Command::List { offset, limit } => {
            let (tracks, page) = crate::query::track_list(state, *offset, *limit)?;
            Ok(crate::query::answer(Output::List { tracks, page }))
        }
        Command::Get {
            track,
            offset,
            limit,
        } => {
            let d = crate::query::track_get(state, track, *offset, *limit)?;
            Ok(crate::query::answer(Output::Get {
                track: d.track,
                devices: d.devices,
                clips: d.clips,
                page: d.page,
                placement_count: d.placement_count,
                lane_count: d.lane_count,
            }))
        }
        Command::Create {
            kind,
            instrument,
            name,
            color,
        } => create(
            state,
            ctx,
            *kind,
            instrument.as_deref(),
            name.as_deref(),
            *color,
        ),
        Command::Delete { track } => delete(state, track),
        Command::Restore { track, snapshot } => restore(state, track, snapshot),
        Command::Rename { track, name } => rename(state, track, name),
        Command::SetColor { track, color } => set_color(state, track, *color),
        Command::Move { track, index } => move_track(state, track, *index),
        Command::RestoreOrder { track, order } => restore_order(state, track, order),
        Command::SetInstrument { track, instrument } => {
            set_instrument(state, ctx, track, instrument)
        }
        Command::RestoreInstrument {
            track,
            current,
            device,
            snapshot,
            lanes,
        } => restore_instrument(state, track, current, device, snapshot, lanes),
        Command::Duplicate { track, name } => duplicate(state, ctx, track, name.as_deref()),
        Command::Arm { track, armed } => {
            let previous = set_flag(state, track, *armed, |s| &mut s.armed)?;
            Ok(session_outcome(
                Output::Arm { previous },
                previous != *armed,
                ProjectEvent::ArmChanged {
                    track: track.clone(),
                    armed: *armed,
                },
            ))
        }
        Command::SetMonitor { track, monitor } => {
            let previous = set_flag(state, track, *monitor, |s| &mut s.monitor)?;
            Ok(session_outcome(
                Output::SetMonitor { previous },
                previous != *monitor,
                ProjectEvent::MonitorChanged {
                    track: track.clone(),
                    monitor: *monitor,
                },
            ))
        }
    }
}

/// Sets or clears `track` in one of the session's track sets and returns whether it was
/// in it. A track that is gone can be cleared (a stale id) but not set.
fn set_flag(
    state: &mut State,
    track: &TrackId,
    on: bool,
    set: impl FnOnce(&mut project::SessionState) -> &mut std::collections::BTreeSet<TrackId>,
) -> Result<bool, CommandError> {
    let exists = state.project.tracks.contains_key(track);
    let set = set(&mut state.session);
    let previous = set.contains(track);
    if !exists && (on || !previous) {
        return Err(not_found(track));
    }
    if on {
        set.insert(track.clone());
    } else {
        set.remove(track);
    }
    Ok(previous)
}

/// The outcome of a session-state command: no inverse, and the event only when it changed.
fn session_outcome(output: Output, changed: bool, event: ProjectEvent) -> Out {
    Outcome {
        output,
        undo: Undo::None,
        events: if changed { vec![event] } else { vec![] },
        control: vec![],
    }
}

fn not_found(track: &TrackId) -> CommandError {
    CommandError::NotFound {
        entity: "track".into(),
        id: track.to_string(),
    }
}

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

fn check_room(state: &State) -> Result<(), CommandError> {
    if state.project.tracks.len() >= MAX_TRACKS {
        return Err(CommandError::Limit {
            entity: "tracks".into(),
            max: MAX_TRACKS as u64,
        });
    }
    Ok(())
}

/// A name, trimmed, at most [`MAX_NAME_CHARS`] characters.
fn clean_name<'a>(name: &'a str, field: &str) -> Result<&'a str, CommandError> {
    let name = name.trim();
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(
            field,
            format!("a name has at most {MAX_NAME_CHARS} characters"),
        ));
    }
    Ok(name)
}

/// The key that puts a track at the end of the list.
fn last_key(state: &State) -> Result<OrderKey, CommandError> {
    let order = state.project.tracks_in_order();
    OrderKey::between(order.last().map(|(_, t)| &t.order), None)
        .map_err(|e| invalid("index", e.to_string()))
}

/// The instrument a MIDI track plays, from the registry: refused when the type is
/// unknown or is not an instrument.
pub(crate) fn instrument_device(kind_id: &str) -> Result<Device, CommandError> {
    instrument_from(crate::device::registry(), kind_id)
}

fn instrument_from(registry: &[Builtin], kind_id: &str) -> Result<Device, CommandError> {
    let builtin = registry
        .iter()
        .find(|b| b.descriptor.kind == kind_id)
        .ok_or_else(|| CommandError::NotFound {
            entity: "instrument".into(),
            id: kind_id.into(),
        })?;
    let descriptor = builtin.descriptor;
    if descriptor.role != engine::device::DeviceRole::Instrument {
        return Err(invalid(
            "instrument",
            format!("{kind_id} is not an instrument"),
        ));
    }
    Ok(crate::device::built(
        builtin,
        DeviceRole::Instrument,
        OrderKey::first(),
    ))
}

fn clip_player() -> Device {
    Device {
        order: OrderKey::first(),
        role: DeviceRole::Instrument,
        enabled: true,
        source: DeviceSource::Builtin {
            id: CLIP_PLAYER.into(),
            version: 1,
        },
        params: BTreeMap::new(),
        state: None,
    }
}

fn created(track: &TrackId) -> ProjectEvent {
    ProjectEvent::TrackCreated {
        track: track.clone(),
    }
}

fn create(
    state: &mut State,
    ctx: &mut Context,
    kind: TrackKind,
    instrument: Option<&str>,
    name: Option<&str>,
    color: Option<Rgb>,
) -> Result<Out, CommandError> {
    check_room(state)?;
    let name = clean_name(name.unwrap_or_default(), "name")?;
    let order = last_key(state)?;
    let device = match (kind, instrument) {
        (TrackKind::Midi, instrument) => {
            instrument_device(instrument.unwrap_or(DEFAULT_INSTRUMENT))?
        }
        (TrackKind::Audio, None) => clip_player(),
        (TrackKind::Audio, Some(_)) => {
            return Err(invalid(
                "instrument",
                "an audio track plays the clip player and takes no instrument",
            ));
        }
    };
    let track_id = TrackId::generate(&mut ctx.ids);
    let device_id = DeviceId::generate(&mut ctx.ids);
    let mut track = Track::new(kind, name, order);
    track.color = Some(color.unwrap_or_else(|| state.project.next_track_color()));
    track.devices.insert(device_id.clone(), device);
    state.project.tracks.insert(track_id.clone(), track);
    Ok(Outcome {
        output: Output::Create {
            track: track_id.clone(),
            device: device_id,
        },
        undo: Undo::Inverse(Box::new(Command::Delete {
            track: track_id.clone(),
        })),
        events: vec![created(&track_id)],
        control: vec![],
    })
}

fn delete(state: &mut State, track_id: &TrackId) -> Result<Out, CommandError> {
    let track = state
        .project
        .tracks
        .remove(track_id)
        .ok_or_else(|| not_found(track_id))?;
    let mut events = vec![ProjectEvent::TrackDeleted {
        track: track_id.clone(),
    }];
    // Session state follows the track out: a selection or a solo on a track that is gone
    // would send live MIDI nowhere. Undo does not bring it back (session, not project).
    if state.session.selected_track.as_ref() == Some(track_id) {
        state.session.selected_track = None;
        events.push(ProjectEvent::TrackSelected { track: None });
    }
    if state.session.solo.remove(track_id) {
        events.push(ProjectEvent::SoloChanged {
            track: track_id.clone(),
            solo: false,
        });
    }
    if state.session.armed.remove(track_id) {
        events.push(ProjectEvent::ArmChanged {
            track: track_id.clone(),
            armed: false,
        });
    }
    if state.session.monitor.remove(track_id) {
        events.push(ProjectEvent::MonitorChanged {
            track: track_id.clone(),
            monitor: false,
        });
    }
    Ok(Outcome {
        output: Output::Delete {
            devices: u32::try_from(track.devices.len()).unwrap_or(u32::MAX),
            clips: u32::try_from(track.clips.len()).unwrap_or(u32::MAX),
        },
        undo: Undo::Inverse(Box::new(Command::Restore {
            track: track_id.clone(),
            snapshot: track,
        })),
        events,
        control: vec![],
    })
}

fn restore(state: &mut State, track_id: &TrackId, snap: &Track) -> Result<Out, CommandError> {
    if state.project.contains_id(&track_id.0) {
        return Err(invalid(
            "track",
            format!("the identifier {track_id} is already in use"),
        ));
    }
    check_room(state)?;
    check_devices(snap)?;
    state.project.tracks.insert(track_id.clone(), snap.clone());
    // The snapshot is data from outside: the whole project must stay valid with it
    // (identifiers unique across the project, limits, device chain, clip kinds). A
    // refusal takes the track back out, so the state is untouched.
    if let Err(violations) = state.project.validate() {
        state.project.tracks.remove(track_id);
        let first = violations
            .first()
            .map_or_else(String::new, ToString::to_string);
        return Err(invalid("snapshot", first));
    }
    Ok(Outcome {
        output: Output::Restore {},
        undo: Undo::Inverse(Box::new(Command::Delete {
            track: track_id.clone(),
        })),
        events: vec![created(track_id)],
        control: vec![],
    })
}

/// The devices of a snapshot are devices Faulk has: a built-in id of the registry (or the
/// clip player), with only parameters its descriptor declares. `track.create` builds its
/// devices from the registry; a restore from outside must not bring in what it cannot.
fn check_devices(snap: &Track) -> Result<(), CommandError> {
    for (id, device) in &snap.devices {
        crate::device::check_device(crate::device::registry(), id, device)?;
    }
    Ok(())
}

/// The track's instrument: its id and device (a chain holds at most one).
fn instrument_of(track: &Track) -> Option<(&DeviceId, &Device)> {
    track
        .devices
        .iter()
        .find(|(_, d)| d.role == DeviceRole::Instrument)
}

fn chain_changed(track: &TrackId) -> Vec<ProjectEvent> {
    vec![ProjectEvent::DeviceChainChanged {
        track: Some(track.clone()),
    }]
}

fn set_instrument(
    state: &mut State,
    ctx: &mut Context,
    track_id: &TrackId,
    instrument: &str,
) -> Result<Out, CommandError> {
    let track = track_mut(state, track_id)?;
    if track.kind != TrackKind::Midi {
        return Err(invalid(
            "track",
            "an audio track plays the clip player, whose instrument is not replaced",
        ));
    }
    let mut device = instrument_device(instrument)?;
    let (old_id, old) = instrument_of(track)
        .map(|(id, d)| (id.clone(), d.clone()))
        .ok_or_else(|| invalid("track", "the track has no instrument to replace"))?;
    let same = matches!(&old.source, DeviceSource::Builtin { id, .. } if id == instrument);
    if same {
        return Ok(unchanged(Output::SetInstrument {
            device: old_id.clone(),
            previous: old_id,
            removed_lanes: 0,
        }));
    }
    // The new instrument takes the old one's place, first in the chain.
    device.order = old.order.clone();
    let new_id = DeviceId::generate(&mut ctx.ids);
    track.devices.remove(&old_id);
    track.devices.insert(new_id.clone(), device);
    // The lanes of the old instrument go with it, and come back with it on undo.
    let lanes = take_lanes(track, &old_id);
    let mut events = lane_deletions(track_id, &lanes);
    events.extend(chain_changed(track_id));
    Ok(Outcome {
        output: Output::SetInstrument {
            device: new_id.clone(),
            previous: old_id.clone(),
            removed_lanes: u32::try_from(lanes.len()).unwrap_or(u32::MAX),
        },
        undo: Undo::Inverse(Box::new(Command::RestoreInstrument {
            track: track_id.clone(),
            current: new_id,
            device: old_id,
            snapshot: old,
            lanes,
        })),
        events,
        control: vec![],
    })
}

/// Takes the automation lanes that target `device` out of the track.
fn take_lanes(track: &mut Track, device: &DeviceId) -> BTreeMap<LaneId, AutomationLane> {
    let ids: Vec<LaneId> = track
        .automation
        .iter()
        .filter(|(_, lane)| {
            matches!(&lane.target, AutomationTarget::Device { device: d, .. } if d == device)
        })
        .map(|(id, _)| id.clone())
        .collect();
    ids.into_iter()
        .filter_map(|id| track.automation.remove(&id).map(|lane| (id, lane)))
        .collect()
}

fn lane_deletions(track: &TrackId, lanes: &BTreeMap<LaneId, AutomationLane>) -> Vec<ProjectEvent> {
    lanes
        .keys()
        .map(|lane| ProjectEvent::AutomationLaneDeleted {
            track: Some(track.clone()),
            lane: lane.clone(),
        })
        .collect()
}

fn restore_instrument(
    state: &mut State,
    track_id: &TrackId,
    current: &DeviceId,
    device_id: &DeviceId,
    snapshot: &Device,
    lanes: &BTreeMap<LaneId, AutomationLane>,
) -> Result<Out, CommandError> {
    if snapshot.role != DeviceRole::Instrument {
        return Err(invalid("snapshot", "only an instrument is swapped in"));
    }
    if state.project.contains_id(&device_id.0) {
        return Err(invalid(
            "device",
            format!("the identifier {device_id} is already in use"),
        ));
    }
    crate::device::check_device(crate::device::registry(), device_id, snapshot)?;
    let track = track_mut(state, track_id)?;
    let out = track
        .devices
        .get(current)
        .filter(|d| d.role == DeviceRole::Instrument)
        .cloned()
        .ok_or_else(|| invalid("current", "that device is not the track's instrument"))?;
    // The lanes of the instrument that leaves go with it. What follows can refuse: the
    // track is kept as it was, to be put back.
    let kept = track.automation.clone();
    let taken = take_lanes(track, current);
    track.devices.remove(current);
    track.devices.insert(device_id.clone(), snapshot.clone());
    let put_back = |state: &mut State| {
        if let Some(track) = state.project.tracks.get_mut(track_id) {
            track.devices.remove(device_id);
            track.devices.insert(current.clone(), out.clone());
            track.automation = kept.clone();
        }
    };
    // The lanes are checked after the instrument left, so that their identifiers can be
    // the ones just taken (a swap and its undo).
    if let Err(error) = crate::device::check_lanes(
        state,
        &crate::device::Place::Track(track_id.clone()),
        device_id,
        snapshot,
        lanes,
    ) {
        put_back(state);
        return Err(error);
    }
    if let Some(track) = state.project.tracks.get_mut(track_id) {
        track
            .automation
            .extend(lanes.iter().map(|(id, lane)| (id.clone(), lane.clone())));
    }
    // The snapshot is data from outside: the whole project must stay valid with it (the
    // instrument first, the clip player only on an audio track). A refusal puts the
    // chain back, so the state is untouched.
    if let Err(violations) = state.project.validate() {
        put_back(state);
        let first = violations
            .first()
            .map_or_else(String::new, ToString::to_string);
        return Err(invalid("snapshot", first));
    }
    let mut events = lane_deletions(track_id, &taken);
    events.extend(
        lanes
            .keys()
            .map(|lane| ProjectEvent::AutomationLaneCreated {
                track: Some(track_id.clone()),
                lane: lane.clone(),
            }),
    );
    events.extend(chain_changed(track_id));
    Ok(Outcome {
        output: Output::RestoreInstrument {},
        undo: Undo::Inverse(Box::new(Command::RestoreInstrument {
            track: track_id.clone(),
            current: device_id.clone(),
            device: current.clone(),
            snapshot: out,
            lanes: taken,
        })),
        events,
        control: vec![],
    })
}

fn track_mut<'a>(state: &'a mut State, track: &TrackId) -> Result<&'a mut Track, CommandError> {
    state
        .project
        .tracks
        .get_mut(track)
        .ok_or_else(|| not_found(track))
}

fn changed(track: &TrackId, part: TrackPart) -> Vec<ProjectEvent> {
    vec![ProjectEvent::TrackChanged {
        track: track.clone(),
        part,
    }]
}

fn unchanged(output: Output) -> Out {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

fn rename(state: &mut State, track_id: &TrackId, name: &str) -> Result<Out, CommandError> {
    let name = clean_name(name, "name")?;
    let track = track_mut(state, track_id)?;
    let previous = track.name.clone();
    let output = Output::Rename {
        previous: previous.clone(),
    };
    if previous == name {
        return Ok(unchanged(output));
    }
    track.name = name.to_owned();
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::Rename {
            track: track_id.clone(),
            name: previous,
        })),
        events: changed(track_id, TrackPart::Name),
        control: vec![],
    })
}

fn set_color(
    state: &mut State,
    track_id: &TrackId,
    color: Option<Rgb>,
) -> Result<Out, CommandError> {
    let track = track_mut(state, track_id)?;
    let previous = track.color;
    let output = Output::SetColor { previous };
    if previous == color {
        return Ok(unchanged(output));
    }
    track.color = color;
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::SetColor {
            track: track_id.clone(),
            color: previous,
        })),
        events: changed(track_id, TrackPart::Color),
        control: vec![],
    })
}

fn move_track(state: &mut State, track_id: &TrackId, index: u32) -> Result<Out, CommandError> {
    let order = state.project.tracks_in_order();
    let previous_index = order
        .iter()
        .position(|(id, _)| *id == track_id)
        .ok_or_else(|| not_found(track_id))?;
    let output = Output::Move {
        previous_index: u32::try_from(previous_index).unwrap_or(u32::MAX),
    };
    let rest: Vec<(&TrackId, &OrderKey)> = order
        .iter()
        .filter(|(id, _)| *id != track_id)
        .map(|(id, t)| (*id, &t.order))
        .collect();
    let others: Vec<&OrderKey> = rest.iter().map(|(_, k)| *k).collect();
    let index = usize::try_from(index)
        .unwrap_or(usize::MAX)
        .min(others.len());
    if index == previous_index {
        return Ok(unchanged(output));
    }
    let key = OrderKey::for_index(&others, index).map_err(|e| invalid("index", e.to_string()))?;
    // Tracks that share a key (two branches made the same one) leave no key between them:
    // refuse rather than report a move that does not happen.
    let lands = rest
        .iter()
        .filter(|(id, k)| (*k, *id) < (&key, track_id))
        .count();
    if lands != index {
        return Err(invalid(
            "index",
            "the tracks around that position share an order key, so none fits between them",
        ));
    }
    let track = track_mut(state, track_id)?;
    let before = std::mem::replace(&mut track.order, key);
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::RestoreOrder {
            track: track_id.clone(),
            order: before,
        })),
        events: changed(track_id, TrackPart::Order),
        control: vec![],
    })
}

fn restore_order(
    state: &mut State,
    track_id: &TrackId,
    order: &OrderKey,
) -> Result<Out, CommandError> {
    let track = track_mut(state, track_id)?;
    let output = Output::RestoreOrder {};
    if track.order == *order {
        return Ok(unchanged(output));
    }
    let before = std::mem::replace(&mut track.order, order.clone());
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::RestoreOrder {
            track: track_id.clone(),
            order: before,
        })),
        events: changed(track_id, TrackPart::Order),
        control: vec![],
    })
}

fn duplicate(
    state: &mut State,
    ctx: &mut Context,
    track_id: &TrackId,
    name: Option<&str>,
) -> Result<Out, CommandError> {
    duplicate_within(state, ctx, track_id, name, (MAX_NOTES, MAX_POINTS))
}

/// [`duplicate`] with the project's caps of notes and automation points given, so that a
/// test reaches them without building millions of notes.
fn duplicate_within(
    state: &mut State,
    ctx: &mut Context,
    track_id: &TrackId,
    name: Option<&str>,
    (max_notes, max_points): (usize, usize),
) -> Result<Out, CommandError> {
    check_room(state)?;
    let source = state
        .project
        .tracks
        .get(track_id)
        .ok_or_else(|| not_found(track_id))?;
    let name = name.map(|n| clean_name(n, "name")).transpose()?;
    // The copy adds its notes and points to the project's.
    check_copy_fits(
        (max_notes, max_points),
        state.project.tracks.values().map(track_notes).sum(),
        state.project.tracks.values().map(track_points).sum(),
        track_notes(source),
        track_points(source),
    )?;
    check_counts_fit(&state.project, source)?;
    let order = state.project.tracks_in_order();
    let at = order
        .iter()
        .position(|(id, _)| *id == track_id)
        .ok_or_else(|| not_found(track_id))?;
    let keys: Vec<&OrderKey> = order.iter().map(|(_, t)| &t.order).collect();
    let key = OrderKey::for_index(&keys, at + 1).map_err(|e| invalid("index", e.to_string()))?;
    let mut copy = source.clone();
    copy.order = key;
    // A duplicate is there to be heard: a muted source does not make a silent copy.
    copy.mixer.mute = false;
    if let Some(name) = name {
        copy.name = name.to_owned();
    }
    // New identifiers everywhere: devices (lanes name their device), clips (placements
    // name their clip), placements and lanes.
    let devices: BTreeMap<DeviceId, DeviceId> = source
        .devices
        .keys()
        .map(|d| (d.clone(), DeviceId::generate(&mut ctx.ids)))
        .collect();
    copy.devices = std::mem::take(&mut copy.devices)
        .into_iter()
        .map(|(id, d): (DeviceId, Device)| (devices[&id].clone(), d))
        .collect();
    let clips: BTreeMap<ClipId, ClipId> = source
        .clips
        .keys()
        .map(|c| (c.clone(), ClipId::generate(&mut ctx.ids)))
        .collect();
    // Each copied clip keeps its source's name (#473).
    copy.clips = std::mem::take(&mut copy.clips)
        .into_iter()
        .map(|(id, c): (ClipId, Clip)| (clips[&id].clone(), c))
        .collect();
    copy.placements = std::mem::take(&mut copy.placements)
        .into_values()
        .map(|mut p: Placement| {
            if let Some(clip) = clips.get(&p.clip) {
                p.clip = clip.clone();
            }
            (PlacementId::generate(&mut ctx.ids), p)
        })
        .collect();
    copy.automation = std::mem::take(&mut copy.automation)
        .into_values()
        .map(|mut lane: AutomationLane| {
            if let AutomationTarget::Device { device, .. } = &mut lane.target
                && let Some(new) = devices.get(device)
            {
                *device = new.clone();
            }
            (LaneId::generate(&mut ctx.ids), lane)
        })
        .collect();
    let new_id = TrackId::generate(&mut ctx.ids);
    state.project.tracks.insert(new_id.clone(), copy);
    Ok(Outcome {
        output: Output::Duplicate {
            track: new_id.clone(),
        },
        undo: Undo::Inverse(Box::new(Command::Delete {
            track: new_id.clone(),
        })),
        events: vec![created(&new_id)],
        control: vec![],
    })
}

/// Refuses a copy that would take the project past the safety limits of notes or
/// automation points.
fn check_copy_fits(
    (max_notes, max_points): (usize, usize),
    notes: usize,
    points: usize,
    copy_notes: usize,
    copy_points: usize,
) -> Result<(), CommandError> {
    if notes.saturating_add(copy_notes) > max_notes {
        return Err(CommandError::Limit {
            entity: "notes".into(),
            max: max_notes as u64,
        });
    }
    if points.saturating_add(copy_points) > max_points {
        return Err(CommandError::Limit {
            entity: "automation points".into(),
            max: max_points as u64,
        });
    }
    Ok(())
}

/// Refuses a copy of `source` that would take the project past its limits of clips,
/// placements or lanes (project-wide, as the engine's slots are).
fn check_counts_fit(project: &project::Project, source: &Track) -> Result<(), CommandError> {
    counts_fit(
        project.counts(),
        source,
        (MAX_CLIPS, MAX_PLACEMENTS, MAX_LANES),
    )
}

/// [`check_counts_fit`] with the caps given, so a test reaches them.
fn counts_fit(
    counts: project::Counts,
    source: &Track,
    (max_clips, max_placements, max_lanes): (usize, usize, usize),
) -> Result<(), CommandError> {
    for (entity, count, more, max) in [
        ("clips", counts.clips, source.clips.len(), max_clips),
        (
            "placements",
            counts.placements,
            source.placements.len(),
            max_placements,
        ),
        ("lanes", counts.lanes, source.automation.len(), max_lanes),
    ] {
        if count.saturating_add(more) > max {
            return Err(CommandError::Limit {
                entity: entity.into(),
                max: max as u64,
            });
        }
    }
    Ok(())
}

fn track_notes(track: &Track) -> usize {
    track
        .clips
        .values()
        .map(|c| match &c.content {
            ClipContent::Midi { notes, .. } => notes.len(),
            ClipContent::Audio { .. } => 0,
        })
        .fold(0, usize::saturating_add)
}

fn track_points(track: &Track) -> usize {
    track
        .automation
        .values()
        .map(|l| l.points.len())
        .fold(0, usize::saturating_add)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{check, context, fixture};
    use engine::device::{DeviceDescriptor, DeviceRole as EngineRole};
    use engine::msg::DeviceKind;

    const CAPS: (usize, usize) = (MAX_NOTES, MAX_POINTS);

    #[test]
    fn a_copy_is_refused_past_the_project_wide_clips_placements_or_lanes() {
        let f = fixture();
        let mut source = f.state.project.tracks[&f.track].clone();
        let clip = ClipId::parse("00000000000000000000000077").unwrap();
        source.clips.insert(
            clip.clone(),
            project::Clip {
                name: String::new(),
                color: None,
                slot: None,
                launch_quantize: None,
                content: ClipContent::Midi {
                    length: project::Tick(960),
                    looping: None,
                    notes: Default::default(),
                },
            },
        );
        let counts = |clips, placements, lanes| project::Counts {
            clips,
            placements,
            lanes,
        };
        assert_eq!(counts_fit(counts(9, 0, 0), &source, (10, 10, 10)), Ok(()));
        assert_eq!(
            counts_fit(counts(10, 0, 0), &source, (10, 10, 10)),
            Err(CommandError::Limit {
                entity: "clips".into(),
                max: 10
            })
        );
        let empty = Track::new(project::TrackKind::Midi, "", OrderKey::first());
        assert_eq!(counts_fit(counts(10, 10, 10), &empty, (10, 10, 10)), Ok(()));
        let mut one_each = empty.clone();
        one_each.placements.insert(
            project::PlacementId::parse("00000000000000000000000078").unwrap(),
            project::Placement {
                clip,
                start: project::Tick(0),
                span: project::Span::Ticks {
                    length: project::Tick(960),
                    offset: project::Tick(0),
                },
            },
        );
        assert!(matches!(
            counts_fit(counts(0, 10, 0), &one_each, (10, 10, 10)),
            Err(CommandError::Limit { entity, .. }) if entity == "placements"
        ));
    }

    #[test]
    fn a_copy_is_refused_exactly_past_the_notes_or_points_limit() {
        assert!(check_copy_fits(CAPS, MAX_NOTES - 5, 0, 5, 0).is_ok());
        assert_eq!(
            check_copy_fits(CAPS, MAX_NOTES - 5, 0, 6, 0),
            Err(CommandError::Limit {
                entity: "notes".into(),
                max: MAX_NOTES as u64
            })
        );
        assert!(check_copy_fits(CAPS, 0, MAX_POINTS - 5, 0, 5).is_ok());
        assert_eq!(
            check_copy_fits(CAPS, 0, MAX_POINTS - 5, 0, 6),
            Err(CommandError::Limit {
                entity: "automation points".into(),
                max: MAX_POINTS as u64
            })
        );
        // Sums never overflow.
        let all = usize::MAX;
        assert!(check_copy_fits(CAPS, all, all, all, all).is_err());
    }

    /// A project with two notes: the copy fits under a cap of four, is refused under a
    /// cap of three, and the refusal leaves the state untouched.
    #[test]
    fn duplicate_refuses_past_the_caps_and_changes_nothing() {
        let f = fixture();
        let mut state = f.state.clone();
        let track = state.project.tracks.get_mut(&f.track).unwrap();
        let mut ids = project::IdGenerator::new(1_760_000_000_000, 6);
        track.clips.insert(
            ClipId::generate(&mut ids),
            Clip {
                name: String::new(),
                color: None,
                slot: None,
                launch_quantize: None,
                content: ClipContent::Midi {
                    length: project::Tick(960),
                    looping: None,
                    notes: [
                        project::Note::new(project::Tick(0), 60, project::Tick(10), 100),
                        project::Note::new(project::Tick(10), 62, project::Tick(10), 100),
                    ]
                    .into(),
                },
            },
        );
        let mut ctx = context();
        let mut fits = state.clone();
        assert!(duplicate_within(&mut fits, &mut ctx, &f.track, None, (4, 10)).is_ok());
        let mut full = state.clone();
        let before = full.clone();
        let err = duplicate_within(&mut full, &mut ctx, &f.track, None, (3, 10)).unwrap_err();
        assert_eq!(
            err,
            CommandError::Limit {
                entity: "notes".into(),
                max: 3
            }
        );
        assert_eq!(full, before);
        // The command at the real caps copies it.
        let command = crate::Command::Track(Command::Duplicate {
            track: f.track.clone(),
            name: None,
        });
        check(&command, &state, &context()).unwrap();
    }

    /// A muted source gives an audible copy; volume and pan are copied, and the copy
    /// takes the name it is given.
    #[test]
    fn a_duplicate_is_never_muted_but_keeps_volume_and_pan() {
        let f = fixture();
        let mut state = f.state.clone();
        let mixer = &mut state.project.tracks.get_mut(&f.track).unwrap().mixer;
        mixer.mute = true;
        mixer.gain_db = -6.0;
        mixer.pan = 0.25;
        let mut ctx = context();
        let out = duplicate(&mut state, &mut ctx, &f.track, Some(" Lead 2 ")).unwrap();
        let Output::Duplicate { track } = out.output else {
            panic!("expected a duplicate output")
        };
        let copy = &state.project.tracks[&track];
        assert!(!copy.mixer.mute);
        assert_eq!(copy.mixer.gain_db, -6.0);
        assert_eq!(copy.mixer.pan, 0.25);
        assert_eq!(copy.name, "Lead 2");
        // The source is left as it was.
        assert!(state.project.tracks[&f.track].mixer.mute);
    }

    #[test]
    fn a_device_that_is_not_an_instrument_cannot_be_a_tracks_instrument() {
        static EFFECT: DeviceDescriptor = DeviceDescriptor {
            kind: "faulk.test_effect",
            kind_id: DeviceKind(900),
            version: 1,
            name: "Test effect",
            role: EngineRole::Effect,
            memory_bytes: 0,
            midi_output: None,
            named_keys: &[],
            params: &[],
        };
        let registry = [Builtin {
            descriptor: &EFFECT,
            factory: engine::devices::osc_synth::factory,
        }];
        let err = instrument_from(&registry, "faulk.test_effect").unwrap_err();
        assert!(matches!(err, CommandError::Invalid { ref field, .. } if field == "instrument"));
        // An instrument of the registry is accepted, one outside the given registry is
        // not found.
        assert!(instrument_from(crate::device::registry(), DEFAULT_INSTRUMENT).is_ok());
        assert_eq!(
            instrument_from(&registry, "faulk.osc_synth")
                .unwrap_err()
                .code(),
            "not_found"
        );
    }
}
