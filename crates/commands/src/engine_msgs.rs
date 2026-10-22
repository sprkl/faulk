//! The project → engine compiler (PR-3, `docs/specs/02-decoupage-mvp.md` § 2.3): every
//! command's engine messages, the snapshot of a whole project, and the hosts' bundle of a
//! project state with its engine stream.
//!
//! The single mapping from the project to the engine's [`ControlMsg`] stream, shared by
//! every target: the page encodes through `crates/project-wasm` and sends the bytes to the
//! web engine's prepare Worker; the helper encodes the commands it receives and submits
//! the same bytes to its prepare side. Nothing maps commands to the engine in TypeScript.
//!
//! - [`compile`]: what one applied command means for the engine, from the command (the
//!   `live.*` notes and the `transport.*` playback orders, which change no state) and from
//!   its [`ProjectEvent`]s read against the state after the command. The root
//!   [`Command::apply`] calls it, so every [`crate::Outcome`] carries its messages, an undo
//!   included (an inverse is applied through the same path).
//! - [`snapshot`]: the whole project, for an empty engine (opening a project, switching
//!   between web and Boost); [`resync`] puts a `Reset` before it and starts a new slot
//!   table.
//! - The slots: the engine knows slots, never ids. The project's [`EngineSlots`], kept in
//!   the [`Context`], gives each entity sent to the engine the lowest free slot of its
//!   kind (freed slots are reused, the table grows without a cap below the engine's
//!   limits). An entity without a slot (an engine never told about it) gets no message:
//!   nothing is sent that the engine would refuse.
//!
//! What is not sent yet: audio clips and their placements (AU-3 streams the PCM an
//! `AudioSourceDeclare` needs); a device whose source is not one of the built-in devices
//! of the registry (the clip player, modules, plugins) has no `DeviceInsert`, its place in
//! the chain stays empty. A clip's loop region and its notes' cents, chance and repeat
//! are sent (CL-2, #225); the engine plays the cents, #192 the chance and the repeat.

use std::collections::BTreeSet;

use engine::msg::{
    AutomationPoint as EnginePoint, AutomationTarget as EngineTarget, ClipSlot, ControlMsg, Curve,
    DeviceKind, DeviceSlot, LaneSlot, MsgError, Note as EngineNote, ParamId,
    Placement as EnginePlacement, PlacementSlot, SceneSlot, Span as EngineSpan, TrackKind,
    TrackSlot, encode_state, limits,
};
use engine::time::LoopRegion;
use project::{
    AutomationLane, AutomationTarget, Clip, ClipContent, ClipId, Device, DeviceId, DeviceSource,
    EngineSlots, IdGenerator, LaneId, MixerParam, OrderKey, Placement, PlacementId, Project,
    ProjectId, SessionState, Shape, Span, Track, TrackId, TrackSlots,
};

use crate::contract::{ClipPart, CommandError, Context, ProjectEvent, State};
use crate::{Author, Command, launch, live, transport};

/// Items (notes, placements, points) per upload message: at most 2.2 MB of payload, well
/// under the 16 MiB a message may carry.
pub const CHUNK_ITEMS: usize = 65_536;

// ---------------------------------------------------------------- compile

/// The engine messages of a command that was just applied, with its events, against the
/// state after it; updates the slot table. `history.*` gives nothing here: undo and redo
/// apply their commands through [`Command::apply`], which compiled each.
pub fn compile(
    command: &Command,
    events: &[ProjectEvent],
    state: &State,
    sync: &mut EngineSlots,
) -> Vec<ControlMsg> {
    let mut out = Vec::new();
    match command {
        Command::History(_) => return out,
        Command::Live(live::Command::NoteOn {
            track,
            note,
            velocity,
        }) => {
            if let Some(track) = track_slot(sync, track) {
                out.push(ControlMsg::NoteOn {
                    track,
                    pitch: *note,
                    velocity: *velocity,
                });
            }
        }
        Command::Live(live::Command::NoteOff { track: id, note }) => {
            sync.held.remove(&(*note, id.clone()));
            if let Some(track) = track_slot(sync, id) {
                out.push(ControlMsg::NoteOff {
                    track,
                    pitch: *note,
                });
            }
        }
        Command::Live(live::Command::InputNoteOn { note, velocity }) => {
            if *velocity == 0 {
                release_input(sync, *note, &mut out);
            } else {
                // Where the note goes is the session as it stands; where it is held is
                // remembered, so its note off reaches the same tracks.
                for track in live::input_targets(state) {
                    if let Some(slot) = track_slot(sync, &track) {
                        out.push(ControlMsg::NoteOn {
                            track: slot,
                            pitch: *note,
                            velocity: *velocity,
                        });
                        sync.held.insert((*note, track));
                    }
                }
            }
        }
        Command::Live(live::Command::InputNoteOff { note }) => {
            release_input(sync, *note, &mut out);
        }
        Command::Launch(launch::Command::BackToArrangement {}) => {
            out.push(ControlMsg::BackToArrangement);
        }
        Command::Live(live::Command::AllNotesOff {}) => {
            sync.held.clear();
            out.extend(sync.tracks.iter().map(|(slot, _)| ControlMsg::AllNotesOff {
                track: TrackSlot(slot),
            }));
        }
        Command::Transport(transport::Command::Play {}) => start_playback(sync, &mut out),
        Command::Transport(transport::Command::Stop {}) => {
            sync.playing = false;
            out.push(ControlMsg::Stop);
        }
        Command::Transport(transport::Command::Seek { tick }) => {
            sync.cue = *tick;
            out.push(ControlMsg::Seek { tick: *tick });
        }
        Command::Launch(launch::Command::Clip { clip }) => {
            // The slot the engine holds the clip in: a clip it was never told about (an
            // audio clip) launches nothing.
            let launched = state
                .project
                .tracks
                .iter()
                .find(|(_, t)| t.clips.contains_key(clip))
                .and_then(|(id, _)| {
                    let scene = sync.track.get(id)?.clips.get(clip)?.as_ref()?;
                    Some(ControlMsg::LaunchClip {
                        track: track_slot(sync, id)?,
                        scene: SceneSlot(sync.scenes.get(scene)? as u16),
                    })
                });
            if let Some(launch) = launched {
                out.push(launch);
                start_playback(sync, &mut out);
            }
        }
        Command::Launch(launch::Command::Scene { scene }) => {
            if let Some(slot) = sync.scenes.get(scene) {
                out.push(ControlMsg::LaunchScene {
                    scene: SceneSlot(slot as u16),
                });
                start_playback(sync, &mut out);
            }
        }
        Command::Launch(launch::Command::StopTrack { track }) => {
            if let Some(track) = track_slot(sync, track) {
                out.push(ControlMsg::StopTrack { track });
            }
        }
        Command::Launch(launch::Command::StopAll {}) => out.push(ControlMsg::StopAll),
        _ => {}
    }
    for event in events {
        compile_event(event, state, sync, &mut out);
    }
    out
}

/// Releases a live input note on the tracks that hold it.
fn release_input(sync: &mut EngineSlots, note: u8, out: &mut Vec<ControlMsg>) {
    let tracks: Vec<TrackId> = sync
        .held
        .iter()
        .filter(|(n, _)| *n == note)
        .map(|(_, t)| t.clone())
        .collect();
    for track in tracks {
        sync.held.remove(&(note, track.clone()));
        if let Some(slot) = track_slot(sync, &track) {
            out.push(ControlMsg::NoteOff {
                track: slot,
                pitch: note,
            });
        }
    }
}

/// Plays from the cue unless the engine already plays: its play always starts at a
/// position, so a second play would jump back to the cue. A launch while stopped ends
/// with it (GR-2): the engine arms the launch while stopped and takes its boundary from the
/// first segment played, whichever block the two messages land in. Play first would let
/// the playhead leave the cue before the launch arrives, and a clip launched from a bar
/// line would then start a bar late.
fn start_playback(sync: &mut EngineSlots, out: &mut Vec<ControlMsg>) {
    if !sync.playing {
        sync.playing = true;
        out.push(ControlMsg::Play { from: sync.cue });
    }
}

fn compile_event(
    event: &ProjectEvent,
    state: &State,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
) {
    let project = &state.project;
    match event {
        ProjectEvent::TrackCreated { track } => {
            if let Some(t) = project.tracks.get(track) {
                remove_track(track, sync, out);
                track_msgs(track, t, &state.session, sync, out);
            }
        }
        ProjectEvent::TrackDeleted { track } => remove_track(track, sync, out),
        ProjectEvent::ClipCreated { track, clip, .. } => {
            let (Some(t), Some(slot)) = (project.tracks.get(track), track_slot(sync, track)) else {
                return;
            };
            if let Some(c) = t.clips.get(clip) {
                clip_msgs(track, slot, clip, c, sync, out);
            }
            placements_msgs(track, t, sync, out);
        }
        ProjectEvent::ClipDeleted { track, clip, .. } => {
            let Some(slot) = track_slot(sync, track) else {
                return;
            };
            let scene = sync
                .track
                .get_mut(track)
                .and_then(|entry| entry.clips.remove(clip))
                .flatten()
                .and_then(|scene| sync.scenes.get(&scene));
            if let Some(scene) = scene {
                out.push(ControlMsg::SceneSetSlot {
                    track: slot,
                    scene: SceneSlot(scene as u16),
                    clip: None,
                });
            }
            if let Some(t) = project.tracks.get(track) {
                placements_msgs(track, t, sync, out);
            }
            if let Some(clip) = sync.clips.release(clip) {
                out.push(ControlMsg::ClipRemove {
                    clip: ClipSlot(clip),
                });
            }
        }
        ProjectEvent::SceneCreated { scene } => {
            // `assign` returns the slot a scene already holds (a clip restored into it
            // before this event, or a snapshot).
            if sync.scenes.assign(scene, limits::MAX_SCENES).is_none() {
                sync.overflow += 1;
            }
        }
        ProjectEvent::SceneDeleted { scene } => {
            // The clips of its slots were deleted before (their rows emptied), so the
            // slot is free for the next scene.
            sync.scenes.release(scene);
        }
        ProjectEvent::ClipChanged { clip, part } => {
            if *part == ClipPart::Name {
                return;
            }
            let Some(c) = project.tracks.values().find_map(|t| t.clips.get(clip)) else {
                return;
            };
            if let Some(slot) = sync.clips.get(clip) {
                upload_clip(ClipSlot(slot), &c.content, out);
            }
        }
        ProjectEvent::DeviceParamChanged {
            device,
            param,
            value,
        } => {
            let Some((track, d)) = project
                .tracks
                .iter()
                .find_map(|(id, t)| t.devices.get(device).map(|d| (id, d)))
            else {
                return;
            };
            let (Some(slot), Some(dev), Some(param)) = (
                track_slot(sync, track),
                device_slot(sync, track, device),
                param_id(d, param),
            ) else {
                return;
            };
            out.push(ControlMsg::ParamSet {
                track: slot,
                device: dev,
                param,
                value: finite_f32(*value),
            });
        }
        ProjectEvent::DeviceChainChanged { track } => {
            sync_chain(track, project, sync, out, &device_insert);
        }
        ProjectEvent::DeviceBypassChanged { device, bypass } => {
            let Some(track) = project
                .tracks
                .iter()
                .find_map(|(id, t)| t.devices.contains_key(device).then_some(id))
            else {
                return;
            };
            if let (Some(slot), Some(dev)) =
                (track_slot(sync, track), device_slot(sync, track, device))
            {
                out.push(ControlMsg::DeviceSetEnabled {
                    track: slot,
                    device: dev,
                    enabled: !bypass,
                });
            }
        }
        ProjectEvent::MixerChanged { track: None, .. } => out.push(master(project)),
        ProjectEvent::MixerChanged {
            track: Some(track), ..
        } => {
            if let (Some(slot), Some(t)) = (track_slot(sync, track), project.tracks.get(track)) {
                out.push(mix(slot, t));
            }
        }
        ProjectEvent::SoloChanged { track, solo } => {
            if let Some(track) = track_slot(sync, track) {
                out.push(ControlMsg::TrackSetSolo { track, solo: *solo });
            }
        }
        ProjectEvent::ArmChanged { track, armed } => {
            if let Some(track) = track_slot(sync, track) {
                out.push(ControlMsg::TrackArm {
                    track,
                    armed: *armed,
                });
            }
        }
        ProjectEvent::MonitorChanged { track, monitor } => {
            if let Some(track) = track_slot(sync, track) {
                out.push(ControlMsg::TrackSetMonitor {
                    track,
                    monitor: *monitor,
                });
            }
        }
        ProjectEvent::TrackSelected { track } => out.push(ControlMsg::SelectTrack {
            track: track.as_ref().and_then(|t| track_slot(sync, t)),
        }),
        ProjectEvent::TempoChanged { .. }
        | ProjectEvent::TimeSignatureChanged { .. }
        | ProjectEvent::LaunchQuantizeChanged { .. } => {
            out.push(set_transport(project));
        }
        ProjectEvent::LoopChanged { .. } => out.push(set_loop(project)),
        ProjectEvent::MetronomeChanged { .. } => out.push(set_metronome(project)),
        ProjectEvent::ProjectRenamed { .. }
        | ProjectEvent::ControlChange { .. }
        | ProjectEvent::TrackChanged { .. }
        | ProjectEvent::SceneChanged { .. }
        | ProjectEvent::MidiInputSelected { .. }
        | ProjectEvent::HistoryChanged { .. } => {}
    }
}

// ---------------------------------------------------------------- snapshot

/// The messages that bring an empty engine to `project` (with the session's solo and
/// selection), and the slot table they assign: the transport, the metronome and the
/// master, then each track in display order with its chain, clips, launch-grid slots,
/// arrangement and lanes, then the selected track. No `Reset`: [`resync`] adds it.
pub fn snapshot(project: &Project, session: &SessionState) -> (Vec<ControlMsg>, EngineSlots) {
    let mut sync = EngineSlots::default();
    let mut out = vec![set_transport(project)];
    if project.transport.loop_enabled && project.transport.loop_region.is_some() {
        out.push(set_loop(project));
    }
    out.push(set_metronome(project));
    out.push(master(project));
    for (scene, _) in project.scenes_in_order() {
        if sync.scenes.assign(scene, limits::MAX_SCENES).is_none() {
            sync.overflow += 1;
        }
    }
    for (id, track) in project.tracks_in_order() {
        track_msgs(id, track, session, &mut sync, &mut out);
    }
    if let Some(track) = session
        .selected_track
        .as_ref()
        .and_then(|t| sync.tracks.get(t))
    {
        out.push(ControlMsg::SelectTrack {
            track: Some(TrackSlot(track)),
        });
    }
    (out, sync)
}

/// A `Reset` to `epoch` and the [`snapshot`] of the state: what an engine needs before
/// the commands, at start, after a mode switch or to recover a stream it refused. The
/// context's slot table becomes the snapshot's (playback stopped, cue at 0).
pub fn resync(state: &State, ctx: &mut Context, epoch: u16) -> Vec<ControlMsg> {
    let (msgs, sync) = snapshot(&state.project, &state.session);
    ctx.engine = sync;
    let mut out = Vec::with_capacity(msgs.len() + 1);
    out.push(ControlMsg::Reset { epoch });
    out.extend(msgs);
    out
}

// ---------------------------------------------------------------- tracks

fn track_slot(sync: &EngineSlots, track: &TrackId) -> Option<TrackSlot> {
    sync.tracks.get(track).map(TrackSlot)
}

fn device_slot(sync: &EngineSlots, track: &TrackId, device: &DeviceId) -> Option<DeviceSlot> {
    let slot = sync.track.get(track)?.devices.get(device)?;
    Some(DeviceSlot(slot as u16))
}

/// What the engine needs after a track's chain changed (a device inserted, removed, moved,
/// brought back, or the instrument replaced): the engine runs the chain in slot order and a
/// device's slot is its index in the chain, as in the snapshot. So a device whose index
/// changed leaves its slot and is built again at the new one (from its project settings:
/// its tail and voices restart), a freed slot nobody takes is emptied, and a device that
/// keeps its index is not touched. A `DeviceInsert` on an occupied slot replaces the
/// device there. The automation lanes that target a device built again are uploaded again,
/// since a lane names a slot. A track the engine was never told about gets nothing.
/// `insert_of` says what the engine can build (the registry's devices).
fn sync_chain(
    track_id: &TrackId,
    project: &Project,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
    insert_of: &dyn Fn(&Device) -> Option<DeviceBuild>,
) {
    let (Some(track), Some(slot)) = (project.tracks.get(track_id), track_slot(sync, track_id))
    else {
        return;
    };
    let Some(mut entry) = sync.track.remove(track_id) else {
        return;
    };
    // The devices the engine builds, with the slot each belongs in.
    let wanted: Vec<(u32, &DeviceId, &Device, DeviceKind, Vec<u8>)> = track
        .devices_in_order()
        .into_iter()
        .enumerate()
        .filter_map(|(index, (id, device))| {
            let (kind, state) = insert_of(device)?;
            Some((u32::try_from(index).ok()?, id, device, kind, state))
        })
        .collect();
    let belongs = |id: &DeviceId| wanted.iter().find(|w| w.1 == id).map(|w| w.0);
    // Devices out of place: gone from the chain, or at another index than their slot.
    let stale: Vec<(u32, DeviceId)> = entry
        .devices
        .iter()
        .filter(|(at, id)| belongs(id) != Some(*at))
        .map(|(at, id)| (at, id.clone()))
        .collect();
    for (at, id) in &stale {
        entry.devices.release(id);
        if !wanted.iter().any(|w| w.0 == *at) {
            out.push(ControlMsg::DeviceRemove {
                track: slot,
                device: DeviceSlot(*at as u16),
            });
        }
    }
    let mut built = BTreeSet::new();
    for (index, id, device, kind, state) in wanted {
        if entry.devices.get(id) == Some(index) {
            continue;
        }
        let Some(at) = entry
            .devices
            .insert_at(id, index, limits::MAX_DEVICES_PER_TRACK)
        else {
            sync.overflow += 1;
            continue;
        };
        out.push(ControlMsg::DeviceInsert {
            track: slot,
            device: DeviceSlot(at as u16),
            kind,
            state,
        });
        if !device.enabled {
            out.push(ControlMsg::DeviceSetEnabled {
                track: slot,
                device: DeviceSlot(at as u16),
                enabled: false,
            });
        }
        built.insert(id.clone());
    }
    sync.track.insert(track_id.clone(), entry);
    for (lane_id, lane) in &track.automation {
        if matches!(&lane.target, AutomationTarget::Device { device, .. } if built.contains(device))
        {
            lane_msgs(track_id, slot, track, lane_id, lane, sync, out);
        }
    }
}

/// A whole track, given a slot: its mix, solo, chain, clips, arrangement and lanes.
fn track_msgs(
    id: &TrackId,
    track: &Track,
    session: &SessionState,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
) {
    let Some(slot) = sync.tracks.assign(id, limits::MAX_TRACKS) else {
        sync.overflow += 1;
        return;
    };
    let slot = TrackSlot(slot);
    out.push(ControlMsg::TrackCreate {
        track: slot,
        kind: match track.kind {
            project::TrackKind::Midi => TrackKind::Midi,
            project::TrackKind::Audio => TrackKind::Audio,
        },
    });
    out.push(mix(slot, track));
    if session.solo.contains(id) {
        out.push(ControlMsg::TrackSetSolo {
            track: slot,
            solo: true,
        });
    }
    if session.armed.contains(id) {
        out.push(ControlMsg::TrackArm {
            track: slot,
            armed: true,
        });
    }
    if session.monitor.contains(id) {
        out.push(ControlMsg::TrackSetMonitor {
            track: slot,
            monitor: true,
        });
    }
    let mut entry = TrackSlots::default();
    for (index, (device_id, device)) in track.devices_in_order().into_iter().enumerate() {
        let Some((kind, state)) = device_insert(device) else {
            continue;
        };
        // A device's slot is its place in the chain (the engine runs the chain in slot
        // order); the project caps the chain below the engine's limit.
        let Some(at) = u32::try_from(index).ok().and_then(|i| {
            entry
                .devices
                .insert_at(device_id, i, limits::MAX_DEVICES_PER_TRACK)
        }) else {
            sync.overflow += 1;
            continue;
        };
        let device_slot = DeviceSlot(at as u16);
        out.push(ControlMsg::DeviceInsert {
            track: slot,
            device: device_slot,
            kind,
            state,
        });
        if !device.enabled {
            out.push(ControlMsg::DeviceSetEnabled {
                track: slot,
                device: device_slot,
                enabled: false,
            });
        }
    }
    sync.track.insert(id.clone(), entry);
    for (clip_id, clip) in &track.clips {
        clip_msgs(id, slot, clip_id, clip, sync, out);
    }
    if track
        .placements
        .values()
        .any(|p| sync.clips.get(&p.clip).is_some())
    {
        placements_msgs(id, track, sync, out);
    }
    for (lane_id, lane) in &track.automation {
        lane_msgs(id, slot, track, lane_id, lane, sync, out);
    }
}

/// Removes a track from the engine with everything it holds there, and frees its slots.
fn remove_track(id: &TrackId, sync: &mut EngineSlots, out: &mut Vec<ControlMsg>) {
    // The notes it held went with it: nothing to release on a slot that may be reused.
    sync.held.retain(|(_, track)| track != id);
    let Some(slot) = sync.tracks.release(id) else {
        return;
    };
    out.push(ControlMsg::TrackRemove {
        track: TrackSlot(slot),
    });
    let Some(entry) = sync.track.remove(id) else {
        return;
    };
    for clip in entry.clips.keys() {
        if let Some(clip) = sync.clips.release(clip) {
            out.push(ControlMsg::ClipRemove {
                clip: ClipSlot(clip),
            });
        }
    }
    for lane in &entry.lanes {
        if let Some(lane) = sync.lanes.release(lane) {
            out.push(ControlMsg::AutomationLaneRemove {
                lane: LaneSlot(lane),
            });
        }
    }
    for placement in &entry.placements {
        sync.placements.release(placement);
    }
}

fn mix(slot: TrackSlot, track: &Track) -> ControlMsg {
    ControlMsg::TrackSetMix {
        track: slot,
        gain: db_to_gain(track.mixer.gain_db),
        pan: track.mixer.pan as f32,
        mute: track.mixer.mute,
    }
}

// ---------------------------------------------------------------- devices

/// What a `DeviceInsert` carries: the engine kind and the initial state of a device.
type DeviceBuild = (DeviceKind, Vec<u8>);

/// The engine kind and the initial state of a device, when the engine can build it (a
/// built-in device of the registry).
fn device_insert(device: &Device) -> Option<DeviceBuild> {
    let DeviceSource::Builtin { id, .. } = &device.source else {
        return None;
    };
    let descriptor = crate::device::registry()
        .iter()
        .find(|b| b.descriptor.kind == id)?
        .descriptor;
    let values: Vec<(ParamId, f32)> = device
        .params
        .iter()
        .filter_map(|(key, value)| Some((param_id(device, key)?, finite_f32(*value))))
        .collect();
    Some((descriptor.kind_id, encode_state(&values)))
}

/// The engine id of a device's parameter, from its built-in descriptor.
fn param_id(device: &Device, key: &str) -> Option<ParamId> {
    let DeviceSource::Builtin { id, .. } = &device.source else {
        return None;
    };
    crate::device::registry()
        .iter()
        .find(|b| b.descriptor.kind == id)?
        .descriptor
        .params
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.id)
}

// ---------------------------------------------------------------- clips and arrangement

/// A MIDI clip of a track: its slot, its notes, its launch-grid slot.
fn clip_msgs(
    track_id: &TrackId,
    track: TrackSlot,
    id: &ClipId,
    clip: &Clip,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
) {
    if !clip.content.is_midi() {
        return;
    }
    let Some(slot) = sync.clips.assign(id, limits::MAX_CLIPS) else {
        sync.overflow += 1;
        return;
    };
    upload_clip(ClipSlot(slot), &clip.content, out);
    let mut scene = None;
    if let Some(scene_id) = &clip.slot {
        match sync.scenes.assign(scene_id, limits::MAX_SCENES) {
            Some(at) => {
                out.push(ControlMsg::SceneSetSlot {
                    track,
                    scene: SceneSlot(at as u16),
                    clip: Some(ClipSlot(slot)),
                });
                scene = Some(scene_id.clone());
            }
            None => sync.overflow += 1,
        }
    }
    if let Some(entry) = sync.track.get_mut(track_id) {
        entry.clips.insert(id.clone(), scene);
    }
}

/// The notes of a MIDI clip, in chunks.
fn upload_clip(clip: ClipSlot, content: &ClipContent, out: &mut Vec<ControlMsg>) {
    let ClipContent::Midi {
        length,
        looping,
        notes,
    } = content
    else {
        return;
    };
    let notes: Vec<EngineNote> = notes
        .iter()
        .map(|n| EngineNote {
            start: n.start,
            length: n.length,
            pitch: n.pitch,
            velocity: n.velocity,
            cents: n.cents,
            chance: n.chance,
            repeat: n.repeat,
        })
        .collect();
    out.extend(chunked(&notes, |first, total, notes| {
        ControlMsg::ClipNotesUpload {
            clip,
            length: *length,
            looping: looping.and_then(|l| LoopRegion::new(l.start, l.end)),
            first,
            total,
            notes,
        }
    }));
}

/// The arrangement of a track: its MIDI placements whose clip the engine has, ordered by
/// start, in chunks. Gives slots to new placements and frees those of the gone ones.
fn placements_msgs(id: &TrackId, track: &Track, sync: &mut EngineSlots, out: &mut Vec<ControlMsg>) {
    let Some(slot) = track_slot(sync, id) else {
        return;
    };
    let mut placed: Vec<(&PlacementId, &Placement)> = track
        .placements
        .iter()
        .filter(|(_, p)| matches!(p.span, Span::Ticks { .. }) && sync.clips.get(&p.clip).is_some())
        .collect();
    placed.sort_by(|a, b| (a.1.start, a.0).cmp(&(b.1.start, b.0)));
    let previous = sync
        .track
        .get(id)
        .map(|e| e.placements.clone())
        .unwrap_or_default();
    // Free first, so the new placements may reuse those slots.
    let current: BTreeSet<&PlacementId> = placed.iter().map(|(p, _)| *p).collect();
    for gone in previous.iter().filter(|p| !current.contains(p)) {
        sync.placements.release(gone);
    }
    let mut kept = BTreeSet::new();
    let mut items = Vec::with_capacity(placed.len());
    for (pid, p) in placed {
        let (Some(clip), Span::Ticks { length, offset }) = (sync.clips.get(&p.clip), p.span) else {
            continue;
        };
        let Some(at) = sync.placements.assign(pid, limits::MAX_PLACEMENTS) else {
            sync.overflow += 1;
            continue;
        };
        kept.insert(pid.clone());
        items.push(EnginePlacement {
            placement: PlacementSlot(at),
            clip: ClipSlot(clip),
            start: p.start,
            span: EngineSpan::Ticks { offset, length },
        });
    }
    if let Some(entry) = sync.track.get_mut(id) {
        entry.placements = kept;
    }
    out.extend(chunked(&items, |first, total, placements| {
        ControlMsg::TrackPlacementsUpload {
            track: slot,
            first,
            total,
            placements,
        }
    }));
}

/// An automation lane of a track, when the engine has its target.
fn lane_msgs(
    track_id: &TrackId,
    track_slot: TrackSlot,
    track: &Track,
    id: &LaneId,
    lane: &AutomationLane,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
) {
    let target = match &lane.target {
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        } => EngineTarget::MixerGain,
        AutomationTarget::Mixer {
            param: MixerParam::Pan,
        } => EngineTarget::MixerPan,
        AutomationTarget::Device { device, param } => {
            let (Some(slot), Some(param)) = (
                device_slot(sync, track_id, device),
                track.devices.get(device).and_then(|d| param_id(d, param)),
            ) else {
                return;
            };
            EngineTarget::DeviceParam {
                device: slot,
                param,
            }
        }
    };
    let Some(at) = sync.lanes.assign(id, limits::MAX_LANES) else {
        sync.overflow += 1;
        return;
    };
    if let Some(entry) = sync.track.get_mut(track_id) {
        entry.lanes.insert(id.clone());
    }
    let points: Vec<EnginePoint> = lane
        .points
        .iter()
        .map(|(tick, p)| EnginePoint {
            tick: *tick,
            value: finite_f32(p.value),
            curve: match p.shape {
                Shape::Step => Curve::Step,
                Shape::Linear => Curve::Linear,
            },
        })
        .collect();
    out.extend(chunked(&points, |first, total, points| {
        ControlMsg::AutomationLaneUpload {
            lane: LaneSlot(at),
            track: track_slot,
            target,
            first,
            total,
            points,
        }
    }));
}

/// The messages of an upload in chunks of [`CHUNK_ITEMS`]; one empty message for none.
fn chunked<T: Clone>(
    items: &[T],
    make: impl Fn(u32, u32, Vec<T>) -> ControlMsg,
) -> Vec<ControlMsg> {
    let total = items.len() as u32;
    if items.is_empty() {
        return vec![make(0, 0, Vec::new())];
    }
    items
        .chunks(CHUNK_ITEMS)
        .enumerate()
        .map(|(i, chunk)| make((i * CHUNK_ITEMS) as u32, total, chunk.to_vec()))
        .collect()
}

// ---------------------------------------------------------------- project settings

fn set_transport(project: &Project) -> ControlMsg {
    ControlMsg::SetTransport {
        tempo: project.tempo(),
        signature: project.time_signature(),
        launch_quantize: project.transport.launch_quantize,
    }
}

fn set_loop(project: &Project) -> ControlMsg {
    let t = &project.transport;
    ControlMsg::SetLoop {
        region: t.loop_region.filter(|_| t.loop_enabled),
    }
}

/// The metronome's level is a linear gain from 0 to 4 in the engine.
fn set_metronome(project: &Project) -> ControlMsg {
    let m = &project.transport.metronome;
    ControlMsg::SetMetronome {
        enabled: m.enabled,
        volume: db_to_gain(m.gain_db).clamp(0.0, 4.0),
        count_in_bars: m.count_in_bars,
    }
}

fn master(project: &Project) -> ControlMsg {
    ControlMsg::SetMaster {
        gain: db_to_gain(project.master.gain_db),
        mute: project.master.mute,
    }
}

/// A finite project value as an engine `f32`, saturated to the finite `f32` range: a value
/// beyond it would become infinite, which the codec refuses (the device clamps to its range).
pub fn finite_f32(value: f64) -> f32 {
    value.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32
}

/// A level in dB as a linear gain; `-inf` (silence) is 0.
pub fn db_to_gain(db: f64) -> f32 {
    if db == f64::NEG_INFINITY {
        0.0
    } else {
        10f64.powf(db / 20.0) as f32
    }
}

// ---------------------------------------------------------------- hosts

/// The encoder of one engine channel's stream: its epoch and the next sequence number
/// (+1 per message, wrapping), the header fields of `engine::msg`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineStream {
    epoch: u16,
    seq: u32,
}

impl EngineStream {
    /// A stream for a fresh engine, in `epoch` (the first `Reset` adopts it).
    pub fn new(epoch: u16) -> Self {
        EngineStream { epoch, seq: 0 }
    }

    pub fn epoch(&self) -> u16 {
        self.epoch
    }

    /// Moves to the next epoch, never 0 (a fresh engine is in epoch 0 and takes only a
    /// newer one).
    pub fn next_epoch(&mut self) -> u16 {
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.epoch
    }

    /// The bytes of `msgs`, in this stream's epoch, numbered after the previous ones.
    pub fn encode(&mut self, msgs: &[ControlMsg]) -> Result<Vec<u8>, MsgError> {
        let bytes = engine::msg::encode_stream(self.epoch, self.seq, msgs)?;
        self.seq = self.seq.wrapping_add(msgs.len() as u32);
        Ok(bytes)
    }
}

/// Why [`Synced::run`] gave no bytes.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncError {
    /// The command was refused: nothing changed.
    Refused(CommandError),
    /// The command applied but its messages did not encode: the engine is behind the
    /// project until a [`Synced::resync`].
    Stream(MsgError),
}

/// A project state with its command context and the stream of one engine: what a host
/// runs commands with (the page through `crates/project-wasm`, the helper, the render
/// harness). Every command goes through [`crate::run`] (history included) and its
/// outcome's engine messages are encoded for the engine.
#[derive(Clone, Debug)]
pub struct Synced {
    pub state: State,
    pub ctx: Context,
    stream: EngineStream,
}

impl Synced {
    /// A host for `state`, whose engine has nothing yet: send [`Synced::setup`] first.
    pub fn new(state: State, ctx: Context, epoch: u16) -> Self {
        Synced {
            state,
            ctx,
            stream: EngineStream::new(epoch),
        }
    }

    /// The page's project until PR-4 loads real ones: one MIDI track playing the
    /// built-in synth ([`single_synth`]). The ids of a command that creates something
    /// come from a fixed seed until PR-4 seeds them with the page's clock.
    pub fn single_synth(epoch: u16) -> Self {
        let ids = IdGenerator::new(1_700_000_000_000, 0x5EED);
        Synced::new(
            single_synth::state(),
            Context::new(Author::User, ids),
            epoch,
        )
    }

    pub fn epoch(&self) -> u16 {
        self.stream.epoch()
    }

    /// `Reset` and the snapshot of the project, encoded in the current epoch: what a new
    /// engine receives first.
    pub fn setup(&mut self) -> Result<Vec<u8>, MsgError> {
        let msgs = resync(&self.state, &mut self.ctx, self.stream.epoch());
        self.stream.encode(&msgs)
    }

    /// [`Synced::setup`] in the next epoch: what an engine that stopped taking the stream
    /// (or was switched to) receives to catch up with the project.
    pub fn resync(&mut self) -> Result<Vec<u8>, MsgError> {
        self.stream.next_epoch();
        self.setup()
    }

    /// Runs a command and encodes its engine messages (empty bytes when it has none).
    pub fn run(&mut self, command: &Command) -> Result<(crate::Outcome, Vec<u8>), SyncError> {
        let outcome =
            crate::run(command, &mut self.state, &mut self.ctx).map_err(SyncError::Refused)?;
        let bytes = if outcome.control.is_empty() {
            Vec::new()
        } else {
            self.stream
                .encode(&outcome.control)
                .map_err(SyncError::Stream)?
        };
        Ok((outcome, bytes))
    }

    /// Encodes raw engine messages in this stream (what no command maps, in tests).
    pub fn encode(&mut self, msgs: &[ControlMsg]) -> Result<Vec<u8>, MsgError> {
        self.stream.encode(msgs)
    }
}

/// The page's one-synth project (`protocol::single_synth` exports its ids to the page).
pub mod single_synth {
    use super::*;

    /// The track of the one synth.
    pub const TRACK: &str = "00000000000000000000000001";
    /// The one synth, instrument of [`TRACK`].
    pub const DEVICE: &str = "00000000000000000000000002";
    /// The project holding them.
    pub const PROJECT: &str = "00000000000000000000000003";

    pub fn track() -> TrackId {
        TrackId::parse(TRACK).expect("a valid track id")
    }

    pub fn device() -> DeviceId {
        DeviceId::parse(DEVICE).expect("a valid device id")
    }

    /// The project with its one MIDI track and the synth at its default parameters, an
    /// empty session and history.
    pub fn state() -> State {
        let mut project = Project::new(
            ProjectId::parse(PROJECT).expect("a valid project id"),
            "Untitled",
        );
        let mut track = Track::new(project::TrackKind::Midi, "Synth", OrderKey::first());
        let synth = crate::track::instrument_device(engine::OSC_SYNTH.kind)
            .expect("the synth is a built-in instrument");
        track.devices.insert(device(), synth);
        project.tracks.insert(self::track(), track);
        State::new(project)
    }
}
