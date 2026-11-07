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
//! Audio clips (AU-3): each media an audio clip on the engine uses is an audio source
//! (`AudioSourceDeclare`, with the blob's hash, before its first clip; `AudioSourceRelease`
//! after its last), each audio clip a `ClipAudioSet` (range, gain, loop), and their
//! placements (`Span::Frames`) go in the track's arrangement with the MIDI ones. The engine's
//! prepare side reads the PCM from disk (`engine::audio_source`); AU-4 plays it.
//!
//! What is not sent yet: a device whose source is not one of the built-in devices
//! of the registry (the clip player, modules, plugins) has no `DeviceInsert`, its place in
//! the chain stays empty. A clip's loop region and its notes' cents, chance and repeat
//! are sent (CL-2, #225); the engine plays the cents, #192 the chance and the repeat.

use std::collections::{BTreeMap, BTreeSet};

use engine::msg::{
    AutomationPoint as EnginePoint, AutomationTarget as EngineTarget, ClipSlot, ControlMsg, Curve,
    DeviceKind, DeviceSlot, LaneSlot, MsgError, Note as EngineNote, ParamId,
    Placement as EnginePlacement, PlacementSlot, SceneSlot, SourceSlot, Span as EngineSpan,
    TrackKind, TrackSlot, encode_state, limits,
};
use engine::time::LoopRegion;
use project::{
    AutomationLane, AutomationTarget, Clip, ClipContent, ClipId, Device, DeviceId, DeviceSource,
    EngineSlots, IdGenerator, LaneId, MixerParam, OrderKey, Placement, PlacementId, Project,
    ProjectId, SessionState, Shape, Span, Track, TrackId, TrackSlots,
};

use crate::contract::{ClipPart, CommandError, Context, ProjectEvent, State, TakeState};
use crate::{Author, Command, launch, live, record, transport};

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
                    at: None,
                });
            }
        }
        Command::Live(live::Command::NoteOff { track: id, note }) => {
            sync.held.remove(&(*note, id.clone()));
            if let Some(track) = track_slot(sync, id) {
                out.push(ControlMsg::NoteOff {
                    track,
                    pitch: *note,
                    at: None,
                });
            }
        }
        Command::Live(live::Command::InputNoteOn {
            note,
            velocity,
            frame,
        }) => {
            if *velocity == 0 {
                release_input(sync, *note, *frame, &mut out);
            } else {
                // Where the note goes is the session as it stands; where it is held is
                // remembered, so its note off reaches the same tracks.
                for track in live::input_targets(state) {
                    if let Some(slot) = track_slot(sync, &track) {
                        out.push(ControlMsg::NoteOn {
                            track: slot,
                            pitch: *note,
                            velocity: *velocity,
                            at: *frame,
                        });
                        sync.held.insert((*note, track));
                    }
                }
            }
        }
        Command::Live(live::Command::InputNoteOff { note, frame }) => {
            release_input(sync, *note, *frame, &mut out);
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
        Command::Live(live::Command::AllSoundOff {}) => {
            // Notes first (the held keys, the MIDI effects' notes), then the cut itself.
            sync.held.clear();
            out.extend(sync.tracks.iter().map(|(slot, _)| ControlMsg::AllNotesOff {
                track: TrackSlot(slot),
            }));
            out.push(ControlMsg::AllSoundOff);
        }
        Command::Transport(transport::Command::Play {}) => start_playback(sync, &mut out),
        Command::Transport(transport::Command::Stop {}) => stop_playback(sync, &mut out),
        Command::Transport(transport::Command::Pause { tick }) => {
            // Stops where the playhead is: the marker stays, only the cue moves, so a resume
            // plays from here and a stop brings the playhead back to the marker.
            sync.playing = false;
            sync.cue = *tick;
            out.push(ControlMsg::Stop);
            out.push(ControlMsg::Seek { tick: *tick });
        }
        Command::Transport(transport::Command::Seek { tick }) => {
            sync.cue = *tick;
            sync.marker = *tick;
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
        Command::Record(record::Command::Start { target, latency_ms }) => {
            // From a stopped transport with a scene target, the scene's clip of each
            // recording track is launched first (the take is played over it), then the
            // capture is asked, then the play: the engine counts in from the stopped clock.
            if !sync.playing
                && let Some(record::RecordTarget::Scene { scene }) = target
                && let Some(scene_slot) = sync.scenes.get(scene)
            {
                for track in record::recording_tracks(state) {
                    let has_clip = state.project.tracks.get(&track).is_some_and(|t| {
                        t.clips.iter().any(|(id, c)| {
                            c.slot.as_ref() == Some(scene)
                                && sync
                                    .track
                                    .get(&track)
                                    .is_some_and(|s| s.clips.contains_key(id))
                        })
                    });
                    if has_clip && let Some(slot) = track_slot(sync, &track) {
                        out.push(ControlMsg::LaunchClip {
                            track: slot,
                            scene: SceneSlot(scene_slot as u16),
                        });
                    }
                }
            }
            // The engine moves the take's notes back by its shift, on its own clock (#560).
            out.push(ControlMsg::SetRecording {
                recording: true,
                shift_us: record::shift_us(record::take_shift_ms(*latency_ms, state)),
            });
            start_playback(sync, &mut out);
        }
        Command::Launch(launch::Command::StopAll {}) => out.push(ControlMsg::StopAll),
        Command::Launch(launch::Command::BackTrackToArrangement { track }) => {
            if let Some(track) = track_slot(sync, track) {
                out.push(ControlMsg::BackTrackToArrangement { track });
            }
        }
        _ => {}
    }
    // The tracks whose table one `ArrangementChanged` sends at the end of the outcome:
    // their clip events send the clips only, so N clips created or removed by one command
    // cost one table, not N.
    let mut batch = Batch {
        tracks: events
            .iter()
            .filter_map(|e| match e {
                ProjectEvent::ArrangementChanged { track } => Some(track.clone()),
                _ => None,
            })
            .collect(),
        removed: Vec::new(),
    };
    for event in events {
        compile_event(event, state, sync, &mut out, &mut batch);
    }
    out
}

/// Releases a live input note on the tracks that hold it; `at` is when it was released (the
/// command's `frame`).
fn release_input(sync: &mut EngineSlots, note: u8, at: Option<u64>, out: &mut Vec<ControlMsg>) {
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
                at,
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

/// Stops, and puts the playhead where `play` restarts (#360): on the marker. A stop while
/// playing, or from a pause (the head off the marker), goes back to the marker; a stop with
/// the head already on it goes on to the start of the project and moves the marker there.
/// The seek is always sent, so the display never depends on what the engine held before (a
/// resync leaves its clock where it was while the slot table starts at 0).
fn stop_playback(sync: &mut EngineSlots, out: &mut Vec<ControlMsg>) {
    if !sync.playing && sync.cue == sync.marker {
        sync.marker = project::Tick(0);
    }
    sync.cue = sync.marker;
    sync.playing = false;
    out.push(ControlMsg::Stop);
    out.push(ControlMsg::Seek { tick: sync.cue });
}

/// What the clip events of one outcome leave to its `ArrangementChanged`.
struct Batch {
    tracks: BTreeSet<TrackId>,
    /// Clips removed from the project, whose engine slot is freed after the table that no
    /// longer uses them.
    removed: Vec<(TrackId, ClipId)>,
}

fn compile_event(
    event: &ProjectEvent,
    state: &State,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
    batch: &mut Batch,
) {
    let project = &state.project;
    match event {
        // The end of a capture (record.stop, record.discard_take, a transport stop or
        // pause during a take): the engine cuts the held notes there.
        ProjectEvent::RecordingChanged {
            state: TakeState::Stopped | TakeState::Discarded,
            ..
        } => out.push(ControlMsg::SetRecording { recording: false, shift_us: 0 }),
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
            if !batch.tracks.contains(track) {
                placements_msgs(track, t, sync, out);
            }
        }
        ProjectEvent::ArrangementChanged { track } => {
            // The table once, after the clips created (their slots are assigned) and
            // before the clips removed are freed.
            if let (Some(t), Some(_)) = (project.tracks.get(track), track_slot(sync, track)) {
                placements_msgs(track, t, sync, out);
            }
            let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut batch.removed)
                .into_iter()
                .partition(|(t, _)| t == track);
            batch.removed = kept;
            for (_, clip) in gone {
                release_clip(&clip, sync, out);
            }
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
            if batch.tracks.contains(track) {
                batch.removed.push((track.clone(), clip.clone()));
                return;
            }
            if let Some(t) = project.tracks.get(track) {
                placements_msgs(track, t, sync, out);
            }
            release_clip(clip, sync, out);
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
            let Some((track, c)) = project
                .tracks
                .iter()
                .find_map(|(id, t)| t.clips.get(clip).map(|c| (id, c)))
            else {
                return;
            };
            if let Some(slot) = sync.clips.get(clip) {
                upload_clip(ClipSlot(slot), &c.content, sync, out);
            }
            // The placements that play the clip whole follow its length.
            if matches!(part, ClipPart::Length | ClipPart::Content)
                && let Some(t) = project
                    .tracks
                    .get(track)
                    .filter(|t| !t.placements.is_empty())
            {
                placements_msgs(track, t, sync, out);
            }
        }
        ProjectEvent::DeviceParamChanged {
            device,
            param,
            value,
        } => {
            if let Some(d) = project.master_devices.get(device) {
                let (Some(dev), Some(param)) = (
                    sync.master_devices
                        .get(device)
                        .map(|s| DeviceSlot(s as u16)),
                    param_id(d, param),
                ) else {
                    return;
                };
                out.push(ControlMsg::ParamSet {
                    track: TrackSlot::MASTER,
                    device: dev,
                    param,
                    value: finite_f32(*value),
                });
                return;
            }
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
        // A gesture that left a driven control as it was (#417): the engine holds the lanes
        // that drive it, with the value the project holds. A fader or pan says so by
        // `HoldMix` (a `TrackSetMix` that moves nothing would look like a mute); a device
        // parameter by the `ParamSet` of its value, which holds its lanes whatever the value.
        ProjectEvent::GestureUnchanged {
            track,
            target: AutomationTarget::Mixer { param },
        } => {
            let (slot, mixer) = match track {
                None => (Some(TrackSlot::MASTER), Some(&project.master)),
                Some(t) => (track_slot(sync, t), project.tracks.get(t).map(|t| &t.mixer)),
            };
            if let (Some(slot), Some(mixer)) = (slot, mixer) {
                out.push(ControlMsg::HoldMix {
                    track: slot,
                    gain: (*param == MixerParam::Gain).then(|| db_to_gain(mixer.gain_db)),
                    pan: (*param == MixerParam::Pan).then(|| finite_f32(mixer.pan)),
                });
            }
        }
        ProjectEvent::GestureUnchanged {
            target: AutomationTarget::Device { device, param },
            ..
        } => {
            let held = project
                .master_devices
                .get(device)
                .or_else(|| project.tracks.values().find_map(|t| t.devices.get(device)))
                .and_then(|d| d.params.get(param));
            if let Some(value) = held {
                let event = ProjectEvent::DeviceParamChanged {
                    device: device.clone(),
                    param: param.clone(),
                    value: *value,
                };
                compile_event(&event, state, sync, out, batch);
            }
        }
        ProjectEvent::DeviceChainChanged { track: Some(track) } => {
            sync_chain(track, project, sync, out, &device_insert);
        }
        ProjectEvent::DeviceChainChanged { track: None } => {
            sync_master_chain(project, sync, out, &device_insert);
        }
        ProjectEvent::DeviceBypassChanged { device, bypass } => {
            if project.master_devices.contains_key(device) {
                if let Some(dev) = sync.master_devices.get(device) {
                    out.push(ControlMsg::DeviceSetEnabled {
                        track: TrackSlot::MASTER,
                        device: DeviceSlot(dev as u16),
                        enabled: !bypass,
                    });
                }
                return;
            }
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
        // The lane goes up whole: a device that was not in the engine yet (a restored
        // device, whose chain event follows) is uploaded with the chain, since a lane
        // names the device's slot. The engine plays it (AM-2, `engine::automation`).
        ProjectEvent::AutomationLaneCreated { track, lane }
        | ProjectEvent::AutomationPointsChanged { track, lane } => {
            let slot = match track {
                None => TrackSlot::MASTER,
                Some(t) => match track_slot(sync, t) {
                    Some(slot) => slot,
                    None => return,
                },
            };
            let Some(l) = project
                .lanes(track.as_ref())
                .and_then(|lanes| lanes.get(lane))
            else {
                return;
            };
            let devices = match track {
                None => &project.master_devices,
                Some(t) => match project.tracks.get(t) {
                    Some(t) => &t.devices,
                    None => return,
                },
            };
            if l.points.is_empty() {
                // A lane with no point does nothing (AM-1): it leaves the engine, as a deleted
                // one does, and its parameter goes back to the stored value (#394). Never
                // uploaded empty: an upload would inherit the hold of the restoring `ParamSet`
                // and stay silent when its points come back.
                if let Some(target) = drop_lane(track.as_ref(), lane, sync, out) {
                    restore_param(target, project, sync, out);
                }
            } else {
                lane_msgs(track.as_ref(), slot, devices, lane, l, sync, out);
            }
        }
        ProjectEvent::AutomationLaneDeleted { track, lane } => {
            // For a device parameter lane, the stored value comes back so that the engine does
            // not keep the last value the lane sent (#394); a mixer lane gives its field back
            // to the fader in the engine (`forget_lane`).
            if let Some(target) = drop_lane(track.as_ref(), lane, sync, out) {
                restore_param(target, project, sync, out);
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
        ProjectEvent::TempoChanged { .. } => {
            out.push(set_transport(project));
            // A tempo change can shorten audio placements (#709): the engine hears the
            // new lengths of every track that holds one.
            for (id, t) in &project.tracks {
                if t.placements
                    .values()
                    .any(|p| matches!(p.span, Span::Frames { .. }))
                {
                    placements_msgs(id, t, sync, out);
                }
            }
        }
        ProjectEvent::TimeSignatureChanged { .. } | ProjectEvent::LaunchQuantizeChanged { .. } => {
            out.push(set_transport(project));
        }
        ProjectEvent::LoopChanged { .. } => out.push(set_loop(project)),
        ProjectEvent::MetronomeChanged { .. } => out.push(set_metronome(project)),
        // `project.open` has no per-command messages: the host sends a snapshot of the
        // new project (`Synced::run`).
        // The MIDI effects that work in the scale read it (#690).
        ProjectEvent::ScaleChanged { .. } => out.push(set_scale(project)),
        ProjectEvent::ProjectRenamed { .. }
        | ProjectEvent::MarkersChanged {}
        | ProjectEvent::ProjectOpened { .. }
        | ProjectEvent::ControlChange { .. }
        | ProjectEvent::TrackChanged { .. }
        | ProjectEvent::SceneChanged { .. }
        | ProjectEvent::MidiInputSelected { .. }
        | ProjectEvent::HistoryChanged { .. }
        | ProjectEvent::RecordingChanged { .. }
        | ProjectEvent::RecordSettingsChanged { .. }
        // A preset is the project's data (or the library's); the values a load sets come as
        // `DeviceParamChanged`.
        | ProjectEvent::PresetChanged { .. }
        | ProjectEvent::DevicePresetChanged { .. } => {}
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
    // A `Reset` leaves the engine without a scale: only a project with one sends it.
    if project.scale.is_some() {
        out.push(set_scale(project));
    }
    if project.transport.loop_enabled && project.transport.loop_region.is_some() {
        out.push(set_loop(project));
    }
    out.push(set_metronome(project));
    out.push(master(project));
    master_chain_msgs(project, &mut sync, &mut out);
    master_lane_msgs(project, &mut sync, &mut out);
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

/// The slot of a device in the chain of a lane's owner: a track's (`Some`) or the master's.
fn owner_device_slot(
    sync: &EngineSlots,
    owner: Option<&TrackId>,
    device: &DeviceId,
) -> Option<DeviceSlot> {
    match owner {
        Some(track) => device_slot(sync, track, device),
        None => sync
            .master_devices
            .get(device)
            .map(|slot| DeviceSlot(slot as u16)),
    }
}

/// The engine slots a track's chain uses before its instrument: one per MIDI effect (#219). The
/// instrument and the audio effects start after them, so that a MIDI effect inserted, moved or
/// removed never moves them (and never builds them again).
pub const MIDI_EFFECT_SLOTS: u32 = project::limits::MAX_MIDI_EFFECTS_PER_TRACK as u32;

/// The engine slot of each device of a chain in order: on a track, a MIDI effect's is its index
/// among the MIDI effects, another device's [`MIDI_EFFECT_SLOTS`] plus its index among the
/// others; on the master (`midi_band` false), its index.
fn chain_slots(chain: &[(&DeviceId, &Device)], midi_band: bool) -> Vec<u32> {
    let (mut midi, mut other) = (0u32, 0u32);
    chain
        .iter()
        .map(|(_, device)| {
            if !midi_band {
                other += 1;
                other - 1
            } else if device.role == project::DeviceRole::MidiEffect {
                midi += 1;
                midi - 1
            } else {
                other += 1;
                MIDI_EFFECT_SLOTS + other - 1
            }
        })
        .collect()
}

/// What the engine needs after a track's chain changed (a device inserted, removed, moved,
/// brought back, or the instrument replaced): the engine runs the chain in slot order and a
/// device's slot is its place in the chain, as in the snapshot ([`chain_slots`]: the MIDI
/// effects in the first slots, the instrument and the audio effects after a fixed band). So a
/// device whose place changed goes to its new slot ([`sync_devices`]): an audio effect is
/// moved there as it is (#800: its tail and state go on), a MIDI effect is built again there
/// (from its project settings; the instrument, first after the MIDI band, never shifts); a freed slot nobody takes is
/// emptied, and a device that keeps its place is not touched: a MIDI effect inserted, moved
/// or removed leaves the instrument and the audio effects alone. A `DeviceInsert` on an
/// occupied slot replaces the device there. The automation lanes that target a device built
/// again or moved are uploaded again, since a lane names a slot. A track the engine was never
/// told about gets nothing. `insert_of` says what the engine can build (the registry's
/// devices).
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
    let chain = track.devices_in_order();
    let built = sync_devices(
        slot,
        &chain,
        true,
        &mut entry.devices,
        &mut sync.overflow,
        out,
        insert_of,
    );
    sync.track.insert(track_id.clone(), entry);
    for (lane_id, lane) in &track.automation {
        if matches!(&lane.target, AutomationTarget::Device { device, .. } if built.contains(device))
        {
            lane_msgs(
                Some(track_id),
                slot,
                &track.devices,
                lane_id,
                lane,
                sync,
                out,
            );
        }
    }
}

/// [`sync_chain`] for the master's chain (#364): the same rule, on the master's slot table.
/// The master's lanes that target a device built again or moved are uploaded again, as a track's are
/// (#378). An effect inserted, removed or moved early in the chain moves the ones that
/// follow it (#800): their tails go on.
fn sync_master_chain(
    project: &Project,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
    insert_of: &dyn Fn(&Device) -> Option<DeviceBuild>,
) {
    let chain = project.master_devices_in_order();
    let mut table = std::mem::take(&mut sync.master_devices);
    let built = sync_devices(
        TrackSlot::MASTER,
        &chain,
        false,
        &mut table,
        &mut sync.overflow,
        out,
        insert_of,
    );
    sync.master_devices = table;
    for (lane_id, lane) in &project.master_automation {
        if matches!(&lane.target, AutomationTarget::Device { device, .. } if built.contains(device))
        {
            lane_msgs(
                None,
                TrackSlot::MASTER,
                &project.master_devices,
                lane_id,
                lane,
                sync,
                out,
            );
        }
    }
}

/// The messages that bring the master's lanes to an empty engine: after its chain, whose
/// slots they name.
fn master_lane_msgs(project: &Project, sync: &mut EngineSlots, out: &mut Vec<ControlMsg>) {
    for (lane_id, lane) in &project.master_automation {
        lane_msgs(
            None,
            TrackSlot::MASTER,
            &project.master_devices,
            lane_id,
            lane,
            sync,
            out,
        );
    }
}

/// The messages that bring the master's chain of an empty engine to the project's.
fn master_chain_msgs(project: &Project, sync: &mut EngineSlots, out: &mut Vec<ControlMsg>) {
    sync_master_chain(project, sync, out, &device_insert);
}

/// What the engine needs after a chain changed, for the chain `chain` in the engine's row
/// `slot` (a track's, or [`TrackSlot::MASTER`]) whose slots `table` holds: see
/// [`sync_chain`]. Returns the devices built (again) or moved, whose lanes name a new slot.
///
/// A device that shifts is moved, not built again (#800), unless it is a MIDI effect (its
/// output is sized from its place, #219; it has no tail): first the devices gone (and the
/// MIDI effects out of place) leave the table, with a `DeviceRemove` unless a `DeviceInsert`
/// lands on their slot (a replacement crossfades); then each device out of place moves into
/// its slot once it is free (`DeviceMove`: a shift is a run of such moves, from the tail for
/// an insert, from the head for a removal); a reorder, where no target is free, builds again
/// the device that goes furthest; then the new devices and those built again take their
/// slots. The engine fades a removed device out in front of the ones moved into its place
/// (`engine::graph` > Device fades), so nothing jumps.
fn sync_devices(
    slot: TrackSlot,
    chain: &[(&DeviceId, &Device)],
    midi_band: bool,
    table: &mut project::SlotTable<DeviceId>,
    overflow: &mut u32,
    out: &mut Vec<ControlMsg>,
    insert_of: &dyn Fn(&Device) -> Option<DeviceBuild>,
) -> BTreeSet<DeviceId> {
    // The devices the engine builds, with the slot each belongs in.
    let slots = chain_slots(chain, midi_band);
    let wanted: Vec<(u32, &DeviceId, &Device, DeviceKind, Vec<u8>)> = chain
        .iter()
        .zip(&slots)
        .filter_map(|((id, device), at)| {
            let (kind, state) = insert_of(device)?;
            Some((*at, *id, *device, kind, state))
        })
        .collect();
    let belongs = |id: &DeviceId| wanted.iter().find(|w| w.1 == id).map(|w| (w.0, w.2));
    // The engine moves an audio effect (`ControlMsg::DeviceMove`); a MIDI effect is sized
    // from its place, and the instrument never shifts (first after the MIDI band).
    let movable = |device: &Device| device.role == project::DeviceRole::Effect;
    let mut changed = BTreeSet::new();
    // Devices that leave the table: gone from the chain, or one the engine does not move out
    // of place.
    let leaving: Vec<(u32, DeviceId)> = table
        .iter()
        .filter(|(at, id)| match belongs(id) {
            None => true,
            Some((to, device)) => to != *at && !movable(device),
        })
        .map(|(at, id)| (at, id.clone()))
        .collect();
    for (_, id) in &leaving {
        table.release(id);
    }
    // A slot gets a `DeviceInsert` when the device that belongs there is not in the table
    // (new, or one that just left it).
    let inserted_at = |at: u32, table: &project::SlotTable<DeviceId>| {
        wanted.iter().any(|w| w.0 == at && table.get(w.1).is_none())
    };
    for (at, _) in &leaving {
        if !inserted_at(*at, table) {
            out.push(ControlMsg::DeviceRemove {
                track: slot,
                device: DeviceSlot(*at as u16),
            });
        }
    }
    // The moves, each into a free slot: each one puts a device in its slot for good. When
    // none is left with a free target, the rest are cycles (a reorder): the device that goes
    // furthest (the one the user moved) leaves and is built again at its place, which frees a
    // slot for the others. So this ends.
    loop {
        let misplaced = || {
            wanted.iter().filter_map(|w| {
                let at = table.get(w.1)?;
                (at != w.0).then_some((at, w.0, w.1))
            })
        };
        let free = misplaced().find(|m| table.id_at(m.1).is_none());
        let Some((from, to, id)) = free else {
            // Furthest first, then the one going toward the head.
            let Some((from, _, id)) = misplaced().max_by_key(|m| (m.0.abs_diff(m.1), m.1 < m.0))
            else {
                break;
            };
            table.release(id);
            out.push(ControlMsg::DeviceRemove {
                track: slot,
                device: DeviceSlot(from as u16),
            });
            continue;
        };
        table.release(id);
        if table
            .insert_at(id, to, limits::MAX_DEVICES_PER_TRACK)
            .is_none()
        {
            // Beyond the engine's limit (the project caps a chain below it): it leaves the
            // engine, and the inserts below count it in `overflow`.
            out.push(ControlMsg::DeviceRemove {
                track: slot,
                device: DeviceSlot(from as u16),
            });
            continue;
        }
        out.push(ControlMsg::DeviceMove {
            track: slot,
            from: DeviceSlot(from as u16),
            to: DeviceSlot(to as u16),
        });
        changed.insert(id.clone());
    }
    for (index, id, device, kind, state) in wanted {
        if table.get(id).is_some() {
            continue;
        }
        let Some(at) = table.insert_at(id, index, limits::MAX_DEVICES_PER_TRACK) else {
            *overflow += 1;
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
        changed.insert(id.clone());
    }
    changed
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
    let chain = track.devices_in_order();
    let slots = chain_slots(&chain, true);
    for ((device_id, device), index) in chain.into_iter().zip(slots) {
        let Some((kind, state)) = device_insert(device) else {
            continue;
        };
        // A device's slot is its place in the chain ([`chain_slots`]: the engine runs the
        // chain in slot order); the project caps the chain below the engine's limit.
        let Some(at) = entry
            .devices
            .insert_at(device_id, index, limits::MAX_DEVICES_PER_TRACK)
        else {
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
        lane_msgs(Some(id), slot, &track.devices, lane_id, lane, sync, out);
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
        release_clip(clip, sync, out);
    }
    for lane in &entry.lanes {
        sync.lane_targets.remove(lane);
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

/// Takes a lane out of the engine tables and tells the engine to drop it, when it is there.
/// Returns the engine target it drove (`None` when the engine did not hold it).
fn drop_lane(
    owner: Option<&TrackId>,
    lane: &LaneId,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
) -> Option<(TrackSlot, EngineTarget)> {
    let target = sync.lane_targets.remove(lane);
    match owner {
        None => {
            sync.master_lanes.remove(lane);
        }
        Some(track) => {
            if let Some(entry) = sync.track.get_mut(track) {
                entry.lanes.remove(lane);
            }
        }
    }
    if let Some(slot) = sync.lanes.release(lane) {
        out.push(ControlMsg::AutomationLaneRemove {
            lane: LaneSlot(slot),
        });
    }
    target
}

/// A lane that drove `target` is gone: sends the project's stored value of a device parameter
/// (a mixer field goes back to the fader in the engine). Nothing when another lane still
/// drives the same parameter (a merged project): the `ParamSet` would hold it.
fn restore_param(
    (track_slot, target): (TrackSlot, EngineTarget),
    project: &Project,
    sync: &EngineSlots,
    out: &mut Vec<ControlMsg>,
) {
    let EngineTarget::DeviceParam { device, param } = target else {
        return;
    };
    if sync
        .lane_targets
        .values()
        .any(|t| *t == (track_slot, target))
    {
        return;
    }
    restore_device_param(track_slot, device, param, project, sync, out);
}

/// Sends the project's stored value for a device parameter to the engine, so that when an
/// automation lane that was driving it is removed or emptied the engine reverts to the
/// project's value (#394 point 1). Does nothing if the device no longer exists in the
/// project (it was removed in the same command, e.g. `device.remove`).
fn restore_device_param(
    track_slot: TrackSlot,
    device_slot: DeviceSlot,
    param: ParamId,
    project: &Project,
    sync: &EngineSlots,
    out: &mut Vec<ControlMsg>,
) {
    // Resolve the DeviceId from the slot the engine holds.
    let device_id = if track_slot.is_master() {
        sync.master_devices.id_at(device_slot.0 as u32)
    } else {
        sync.tracks
            .id_at(track_slot.0)
            .and_then(|tid| sync.track.get(tid))
            .and_then(|ts| ts.devices.id_at(device_slot.0 as u32))
    };
    let Some(device_id) = device_id else {
        return; // device no longer in the engine table (normal for device.remove)
    };
    // Resolve the device's project record to read the stored parameter value.
    let devices = if track_slot.is_master() {
        &project.master_devices
    } else {
        let Some(track_id) = sync.tracks.id_at(track_slot.0) else {
            return;
        };
        let Some(track) = project.tracks.get(track_id) else {
            return;
        };
        &track.devices
    };
    let Some(device) = devices.get(device_id) else {
        return; // device was removed from the project (e.g. device.remove before lane delete)
    };
    // Reverse-resolve the ParamId to its descriptor, then read the stored value; a key the
    // project does not hold is at the descriptor's default, as the engine built the device.
    let DeviceSource::Builtin { id, .. } = &device.source else {
        // Only built-in devices have lanes in the engine today (`lane_msgs` uploads none for a
        // module: no descriptor yet, MO), so no `lane_targets` entry reaches this point for
        // another source. When modules get lanes, they need their own default lookup here.
        return;
    };
    let Some(descriptor) = crate::device::registry()
        .iter()
        .find(|b| b.descriptor.kind == id)
        .map(|b| b.descriptor)
    else {
        return;
    };
    let Some(p) = descriptor.params.iter().find(|p| p.id == param) else {
        return;
    };
    let value = device.params.get(p.key).copied().unwrap_or(p.default);
    out.push(ControlMsg::ParamSet {
        track: track_slot,
        device: device_slot,
        param,
        value: finite_f32(value),
    });
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
    let Some(slot) = sync.clips.assign(id, limits::MAX_CLIPS) else {
        sync.overflow += 1;
        return;
    };
    if let ClipContent::Audio { media, .. } = &clip.content
        && !declare_source(id, media, sync, out)
    {
        // No source slot left: the clip is not sent.
        sync.clips.release(id);
        sync.overflow += 1;
        return;
    }
    upload_clip(ClipSlot(slot), &clip.content, sync, out);
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

/// The source of an audio clip's media: the one it has, or a new slot and its
/// `AudioSourceDeclare`. Records the clip's media. `false` when every source slot is taken.
fn declare_source(
    clip: &ClipId,
    media: &project::AudioMedia,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
) -> bool {
    if sync.sources.get(&media.hash).is_none() {
        let Some(slot) = sync.sources.assign(&media.hash, limits::MAX_AUDIO_SOURCES) else {
            return false;
        };
        out.push(ControlMsg::AudioSourceDeclare {
            source: SourceSlot(slot),
            sample_rate: media.sample_rate.hz(),
            channels: u8::try_from(media.channels).unwrap_or(u8::MAX),
            frames: media.frames,
            media: media.hash.0,
        });
    }
    sync.clip_media.insert(clip.clone(), media.hash);
    true
}

/// Frees a clip's engine slot (`ClipRemove`), then its source when it was the last clip of
/// its media on the engine (`AudioSourceRelease`).
fn release_clip(clip: &ClipId, sync: &mut EngineSlots, out: &mut Vec<ControlMsg>) {
    if let Some(slot) = sync.clips.release(clip) {
        out.push(ControlMsg::ClipRemove {
            clip: ClipSlot(slot),
        });
    }
    let Some(hash) = sync.clip_media.remove(clip) else {
        return;
    };
    if sync.clip_media.values().all(|h| *h != hash)
        && let Some(source) = sync.sources.release(&hash)
    {
        out.push(ControlMsg::AudioSourceRelease {
            source: SourceSlot(source),
        });
    }
}

/// The content of a clip: the notes of a MIDI clip, in chunks, or an audio clip's range,
/// gain and loop.
fn upload_clip(
    clip: ClipSlot,
    content: &ClipContent,
    sync: &EngineSlots,
    out: &mut Vec<ControlMsg>,
) {
    if let ClipContent::Audio {
        media,
        start,
        frames,
        gain_db,
        looping,
    } = content
    {
        let Some(source) = sync.sources.get(&media.hash) else {
            return;
        };
        out.push(ControlMsg::ClipAudioSet {
            clip,
            source: SourceSlot(source),
            start: *start,
            length: *frames,
            gain: db_to_gain(*gain_db),
            looping: looping
                .filter(|l| l.start < l.end && l.end <= *frames)
                .map(|l| (l.start, l.end)),
        });
        return;
    }
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

/// The arrangement of a track: its placements (MIDI and audio) whose clip the engine has, ordered by
/// start, in chunks. Gives slots to new placements and frees those of the gone ones.
fn placements_msgs(id: &TrackId, track: &Track, sync: &mut EngineSlots, out: &mut Vec<ControlMsg>) {
    let Some(slot) = track_slot(sync, id) else {
        return;
    };
    let mut placed: Vec<(&PlacementId, &Placement)> = track
        .placements
        .iter()
        .filter(|(_, p)| sync.clips.get(&p.clip).is_some())
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
        let Some(clip) = sync.clips.get(&p.clip) else {
            continue;
        };
        let span = match p.span {
            Span::Ticks { length, offset } => EngineSpan::Ticks { offset, length },
            Span::Frames { length, offset } => EngineSpan::Frames { offset, length },
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
            span,
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

/// An automation lane of a track (`owner` is `Some`) or of the master, when the engine has
/// its target. `track_slot` is the owner's row: the track's, or [`TrackSlot::MASTER`].
fn lane_msgs(
    owner: Option<&TrackId>,
    track_slot: TrackSlot,
    devices: &BTreeMap<DeviceId, Device>,
    id: &LaneId,
    lane: &AutomationLane,
    sync: &mut EngineSlots,
    out: &mut Vec<ControlMsg>,
) {
    // A lane with no point does nothing (AM-1): the engine does not hold it.
    if lane.points.is_empty() {
        return;
    }
    let target = match &lane.target {
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        } => EngineTarget::MixerGain,
        AutomationTarget::Mixer {
            param: MixerParam::Pan,
        } => EngineTarget::MixerPan,
        AutomationTarget::Device { device, param } => {
            let (Some(slot), Some(param)) = (
                owner_device_slot(sync, owner, device),
                devices.get(device).and_then(|d| param_id(d, param)),
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
    // Store the engine target so that AutomationLaneDeleted can restore the stored value
    // even after the project has already removed the lane's device (#394).
    sync.lane_targets.insert(id.clone(), (track_slot, target));
    match owner {
        Some(track) => {
            if let Some(entry) = sync.track.get_mut(track) {
                entry.lanes.insert(id.clone());
            }
        }
        None => {
            sync.master_lanes.insert(id.clone());
        }
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

/// The project's scale as the engine takes it: a set of pitch classes, 0 for none (#690).
fn set_scale(project: &Project) -> ControlMsg {
    let pitch_classes = project.scale.map_or(0, |scale| {
        scale
            .pitch_classes()
            .iter()
            .fold(0u16, |bits, pc| bits | 1 << (pc % 12))
    });
    ControlMsg::SetScale { pitch_classes }
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
        pan: project.master.pan as f32,
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

/// Whether an applied command replaced the whole project, so that the engine needs a
/// snapshot of the new one rather than messages (`project.open`).
pub fn replaces_project(command: &Command) -> bool {
    matches!(
        command,
        Command::Project(
            crate::project::Command::Open { .. } | crate::project::Command::New { .. }
        )
    )
}

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

    /// Moves to the next epoch, and further when that one is not newer than `other` (the
    /// epoch of an engine this stream did not start, BO-1: the helper's engine outlives a
    /// page): the epoch then is `other + 1`, never 0. Newer is the engine's wrapping order
    /// (`engine::msg::Header::decode`).
    pub fn next_epoch_after(&mut self, other: u16) -> u16 {
        let next = self.next_epoch();
        if (next.wrapping_sub(other) as i16) <= 0 {
            self.epoch = other.wrapping_add(1).max(1);
        }
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
        let mut msgs = resync(&self.state, &mut self.ctx, self.stream.epoch());
        // The reset engine is stopped but keeps its clock where it was: the head goes to the
        // cue the table starts at, so the position shown is where `play` starts (#360).
        msgs.push(ControlMsg::Seek {
            tick: self.ctx.engine.cue,
        });
        self.stream.encode(&msgs)
    }

    /// [`Synced::setup`] in the next epoch: what an engine that stopped taking the stream
    /// (or was switched to) receives to catch up with the project.
    pub fn resync(&mut self) -> Result<Vec<u8>, MsgError> {
        self.stream.next_epoch();
        self.setup()
    }

    /// [`Synced::resync`] for an engine this stream did not start, whose epoch is `other`
    /// (the helper's, announced in its `hello`, BO-1): the `Reset` is in an epoch newer than
    /// both this stream's and `other` ([`EngineStream::next_epoch_after`]), so either engine
    /// takes it.
    pub fn resync_after(&mut self, other: u16) -> Result<Vec<u8>, MsgError> {
        self.stream.next_epoch_after(other);
        self.setup()
    }

    /// Runs a command and encodes its engine messages (empty bytes when it has none). A
    /// command that replaced the project (`project.open`) gives a snapshot of the new one
    /// in the next epoch instead: the engine starts over from it.
    pub fn run(&mut self, command: &Command) -> Result<(crate::Outcome, Vec<u8>), SyncError> {
        let outcome =
            crate::run(command, &mut self.state, &mut self.ctx).map_err(SyncError::Refused)?;
        let bytes = self
            .engine_bytes(command, &outcome)
            .map_err(SyncError::Stream)?;
        Ok((outcome, bytes))
    }

    /// The engine bytes of an applied command: its messages, or a snapshot when it replaced
    /// the project. On `Err` the engine is behind the project until a [`Synced::resync`].
    pub fn engine_bytes(
        &mut self,
        command: &Command,
        outcome: &crate::Outcome,
    ) -> Result<Vec<u8>, MsgError> {
        if replaces_project(command) {
            return self.resync();
        }
        if outcome.control.is_empty() {
            return Ok(Vec::new());
        }
        self.stream.encode(&outcome.control)
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
            crate::project::DEFAULT_NAME,
        );
        let mut track = Track::new(project::TrackKind::Midi, "Synth", OrderKey::first());
        track.color = Some(project::colors::palette_hue(0));
        let synth = crate::track::instrument_device(engine::OSC_SYNTH.kind)
            .expect("the synth is a built-in instrument");
        track.devices.insert(device(), synth);
        project.tracks.insert(self::track(), track);
        State::new(project)
    }
}
