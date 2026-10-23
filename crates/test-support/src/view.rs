//! What an engine holds after a message stream, by the project's ids (PR-3): a model of
//! the engine's control state, folded from the [`ControlMsg`]s a scenario pushed, then
//! named by ids through the project's slot table; and the same view built straight from a
//! project, without the compiler. Two streams that leave an engine in the same state
//! (commands applied live, or the snapshot of the final project) give equal views, and
//! both equal the project's.
//!
//! The model is written apart from `commands::engine_msgs`: it reads messages, it never
//! calls the compiler, so a slot mix-up of the compiler shows as a difference with
//! [`EngineView::of_project`].

use std::collections::BTreeMap;

use commands::engine_msgs::{db_to_gain, finite_f32};
use engine::devices::REGISTRY;
use engine::msg::{
    AutomationPoint, AutomationTarget, ControlMsg, Curve, DeviceKind, HEADER_BYTES, Header,
    Note as EngineNote, Span as EngineSpan, TrackKind, state_pairs,
};
use engine::time::{LoopRegion, Quantize, Tempo, Tick, TimeSignature};
use project::{
    ClipContent, ClipId, DeviceId, DeviceSource, EngineSlots, LaneId, PlacementId, Project,
    SceneId, SessionState, Span, TrackId,
};

use crate::scenario::{Scenario, Step};

/// A device as the engine holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceView {
    pub kind: DeviceKind,
    pub enabled: bool,
    /// Parameter id → value: the initial state, then every `ParamSet`.
    pub params: BTreeMap<u32, f32>,
}

/// A track as the engine holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackView {
    pub kind: TrackKind,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub monitor: bool,
    pub devices: BTreeMap<DeviceId, DeviceView>,
    /// Scene → the clip in that launch-grid slot.
    pub scenes: BTreeMap<SceneId, ClipId>,
    /// The arrangement: placement, clip, start, offset, length (ticks), in the order sent.
    pub placements: Vec<(PlacementId, ClipId, Tick, Tick, Tick)>,
}

/// A MIDI clip as the engine holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipView {
    pub length: Tick,
    pub looping: Option<LoopRegion>,
    pub notes: Vec<EngineNote>,
}

/// What a lane drives, by ids.
#[derive(Clone, Debug, PartialEq)]
pub enum TargetView {
    Device { device: DeviceId, param: u32 },
    MixerGain,
    MixerPan,
}

/// An automation lane as the engine holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct LaneView {
    pub track: TrackId,
    pub target: TargetView,
    pub points: Vec<AutomationPoint>,
}

/// An engine's control state, by the project's ids.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineView {
    pub transport: Option<(Tempo, TimeSignature, Quantize)>,
    pub loop_region: Option<LoopRegion>,
    pub metronome: Option<(bool, f32, u8)>,
    pub master: Option<(f32, f32, bool)>,
    pub tracks: BTreeMap<TrackId, TrackView>,
    pub clips: BTreeMap<ClipId, ClipView>,
    pub lanes: BTreeMap<LaneId, LaneView>,
    pub selected: Option<TrackId>,
}

/// The messages of a scenario's pushes, in order.
pub fn messages(scenario: &Scenario) -> Vec<ControlMsg> {
    let mut out = Vec::new();
    for step in &scenario.steps {
        let Step::Push(bytes) = step else { continue };
        let mut at = 0;
        while at < bytes.len() {
            let epoch = u16::from_le_bytes([bytes[at + 4], bytes[at + 5]]);
            // A `Reset` carries a newer epoch than the engine's; any other message the same.
            let header = Header::decode(&bytes[at..], epoch)
                .or_else(|_| Header::decode(&bytes[at..], epoch.wrapping_sub(1)))
                .expect("a valid header");
            let end = at + HEADER_BYTES + header.len as usize;
            out.push(
                ControlMsg::decode(&header, &bytes[at + HEADER_BYTES..end])
                    .expect("a valid payload"),
            );
            at += Header::frame_len(header.len);
        }
    }
    out
}

#[derive(Default)]
struct SlotTrack {
    kind: Option<TrackKind>,
    mix: (f32, f32, bool),
    solo: bool,
    armed: bool,
    monitor: bool,
    devices: BTreeMap<u16, DeviceView>,
    scenes: BTreeMap<u16, u32>,
    placements: Vec<engine::msg::Placement>,
}

#[derive(Default)]
struct Model {
    transport: Option<(Tempo, TimeSignature, Quantize)>,
    loop_region: Option<LoopRegion>,
    metronome: Option<(bool, f32, u8)>,
    master: Option<(f32, f32, bool)>,
    tracks: BTreeMap<u32, SlotTrack>,
    clips: BTreeMap<u32, ClipView>,
    lanes: BTreeMap<u32, (u32, AutomationTarget, Vec<AutomationPoint>)>,
    selected: Option<u32>,
    staged_notes: BTreeMap<u32, Vec<EngineNote>>,
    staged_placements: BTreeMap<u32, Vec<engine::msg::Placement>>,
    staged_points: BTreeMap<u32, Vec<AutomationPoint>>,
}

impl Model {
    fn apply(&mut self, msg: ControlMsg) {
        match msg {
            ControlMsg::Reset { .. } => *self = Model::default(),
            ControlMsg::SetTransport {
                tempo,
                signature,
                launch_quantize,
            } => self.transport = Some((tempo, signature, launch_quantize)),
            ControlMsg::SetLoop { region } => self.loop_region = region,
            ControlMsg::SetMetronome {
                enabled,
                volume,
                count_in_bars,
            } => self.metronome = Some((enabled, volume, count_in_bars)),
            ControlMsg::SetMaster { gain, pan, mute } => self.master = Some((gain, pan, mute)),
            ControlMsg::TrackCreate { track, kind } => {
                self.tracks.insert(
                    track.0,
                    SlotTrack {
                        kind: Some(kind),
                        mix: (1.0, 0.0, false),
                        ..SlotTrack::default()
                    },
                );
            }
            ControlMsg::TrackRemove { track } => {
                self.tracks.remove(&track.0);
            }
            ControlMsg::TrackSetMix {
                track,
                gain,
                pan,
                mute,
            } => self.track(track.0).mix = (gain, pan, mute),
            ControlMsg::TrackSetSolo { track, solo } => self.track(track.0).solo = solo,
            ControlMsg::TrackArm { track, armed } => self.track(track.0).armed = armed,
            ControlMsg::TrackSetMonitor { track, monitor } => {
                self.track(track.0).monitor = monitor;
            }
            ControlMsg::SelectTrack { track } => self.selected = track.map(|t| t.0),
            ControlMsg::DeviceInsert {
                track,
                device,
                kind,
                state,
            } => {
                let params = state_pairs(&state)
                    .map(|(id, v)| (id.0, v))
                    .collect::<BTreeMap<_, _>>();
                self.track(track.0).devices.insert(
                    device.0,
                    DeviceView {
                        kind,
                        enabled: true,
                        params,
                    },
                );
            }
            ControlMsg::DeviceRemove { track, device } => {
                self.track(track.0).devices.remove(&device.0);
            }
            ControlMsg::DeviceSetEnabled {
                track,
                device,
                enabled,
            } => self.device(track.0, device.0).enabled = enabled,
            ControlMsg::ParamSet {
                track,
                device,
                param,
                value,
            } => {
                self.device(track.0, device.0).params.insert(param.0, value);
            }
            ControlMsg::ClipNotesUpload {
                clip,
                length,
                looping,
                first,
                total,
                notes,
            } => {
                let staged = self.staged_notes.entry(clip.0).or_default();
                assert_eq!(staged.len() as u32, first, "chunks in order");
                staged.extend(notes);
                if staged.len() as u32 == total {
                    let notes = self.staged_notes.remove(&clip.0).unwrap_or_default();
                    self.clips.insert(
                        clip.0,
                        ClipView {
                            length,
                            looping,
                            notes,
                        },
                    );
                }
            }
            ControlMsg::ClipRemove { clip } => {
                self.clips.remove(&clip.0);
            }
            ControlMsg::TrackPlacementsUpload {
                track,
                first,
                total,
                placements,
            } => {
                let staged = self.staged_placements.entry(track.0).or_default();
                assert_eq!(staged.len() as u32, first, "chunks in order");
                staged.extend(placements);
                if staged.len() as u32 == total {
                    let all = self.staged_placements.remove(&track.0).unwrap_or_default();
                    self.track(track.0).placements = all;
                }
            }
            ControlMsg::AutomationLaneUpload {
                lane,
                track,
                target,
                first,
                total,
                points,
            } => {
                let staged = self.staged_points.entry(lane.0).or_default();
                assert_eq!(staged.len() as u32, first, "chunks in order");
                staged.extend(points);
                if staged.len() as u32 == total {
                    let all = self.staged_points.remove(&lane.0).unwrap_or_default();
                    self.lanes.insert(lane.0, (track.0, target, all));
                }
            }
            ControlMsg::AutomationLaneRemove { lane } => {
                self.lanes.remove(&lane.0);
            }
            ControlMsg::SceneSetSlot { track, scene, clip } => {
                let row = &mut self.track(track.0).scenes;
                match clip {
                    Some(clip) => row.insert(scene.0, clip.0),
                    None => row.remove(&scene.0),
                };
            }
            // Playback orders and live notes leave the control state as it is.
            _ => {}
        }
    }

    fn track(&mut self, slot: u32) -> &mut SlotTrack {
        self.tracks
            .get_mut(&slot)
            .unwrap_or_else(|| panic!("a message names track slot {slot}, which is empty"))
    }

    fn device(&mut self, track: u32, device: u16) -> &mut DeviceView {
        self.track(track)
            .devices
            .get_mut(&device)
            .unwrap_or_else(|| panic!("no device in slot {device} of track {track}"))
    }
}

impl EngineView {
    /// The engine state the messages of `scenario` leave, named by `slots` (the slot
    /// table of the context that produced the stream). Panics when a message names an
    /// empty slot, or a slot the table does not know.
    pub fn of_stream(scenario: &Scenario, slots: &EngineSlots) -> EngineView {
        let mut model = Model::default();
        for msg in messages(scenario) {
            model.apply(msg);
        }
        let track_id = |slot: u32| {
            slots
                .tracks
                .id_at(slot)
                .cloned()
                .unwrap_or_else(|| panic!("track slot {slot} is not in the table"))
        };
        let clip_id = |slot: u32| {
            slots
                .clips
                .id_at(slot)
                .cloned()
                .unwrap_or_else(|| panic!("clip slot {slot} is not in the table"))
        };
        let device_id = |track: &TrackId, slot: u16| {
            slots.track[track]
                .devices
                .id_at(u32::from(slot))
                .cloned()
                .unwrap_or_else(|| panic!("device slot {slot} of {track} is not in the table"))
        };
        let mut tracks = BTreeMap::new();
        for (slot, t) in &model.tracks {
            let id = track_id(*slot);
            let view = TrackView {
                kind: t.kind.expect("created"),
                gain: t.mix.0,
                pan: t.mix.1,
                mute: t.mix.2,
                solo: t.solo,
                armed: t.armed,
                monitor: t.monitor,
                devices: t
                    .devices
                    .iter()
                    .map(|(d, v)| (device_id(&id, *d), v.clone()))
                    .collect(),
                scenes: t
                    .scenes
                    .iter()
                    .map(|(s, c)| {
                        let scene = slots
                            .scenes
                            .id_at(u32::from(*s))
                            .cloned()
                            .unwrap_or_else(|| panic!("scene slot {s} is not in the table"));
                        (scene, clip_id(*c))
                    })
                    .collect(),
                placements: t
                    .placements
                    .iter()
                    .map(|p| {
                        let EngineSpan::Ticks { offset, length } = p.span else {
                            panic!("a MIDI placement spans ticks");
                        };
                        let pid = slots
                            .placements
                            .id_at(p.placement.0)
                            .cloned()
                            .expect("a placement slot in the table");
                        (pid, clip_id(p.clip.0), p.start, offset, length)
                    })
                    .collect(),
            };
            tracks.insert(id, view);
        }
        let clips = model
            .clips
            .iter()
            .map(|(slot, c)| (clip_id(*slot), c.clone()))
            .collect();
        let lanes = model
            .lanes
            .iter()
            .map(|(slot, (track, target, points))| {
                let track = track_id(*track);
                let target = match target {
                    AutomationTarget::DeviceParam { device, param } => TargetView::Device {
                        device: device_id(&track, device.0),
                        param: param.0,
                    },
                    AutomationTarget::MixerGain => TargetView::MixerGain,
                    AutomationTarget::MixerPan => TargetView::MixerPan,
                };
                let lane = slots
                    .lanes
                    .id_at(*slot)
                    .cloned()
                    .expect("a lane slot in the table");
                (
                    lane,
                    LaneView {
                        track,
                        target,
                        points: points.clone(),
                    },
                )
            })
            .collect();
        EngineView {
            transport: model.transport,
            loop_region: model.loop_region,
            metronome: model.metronome,
            master: model.master,
            tracks,
            clips,
            lanes,
            selected: model.selected.map(track_id),
        }
    }

    /// The engine state `project` (with the session's solo and selection) should leave,
    /// built from the project alone: what the engine can build of it (built-in devices,
    /// MIDI clips and placements, lanes whose target it has).
    pub fn of_project(project: &Project, session: &SessionState) -> EngineView {
        let builtin = |source: &DeviceSource| match source {
            DeviceSource::Builtin { id, .. } => {
                REGISTRY.iter().find(|b| b.descriptor.kind == id.as_str())
            }
            _ => None,
        };
        let param = |source: &DeviceSource, key: &str| {
            builtin(source)?
                .descriptor
                .params
                .iter()
                .find(|p| p.key == key)
                .map(|p| p.id.0)
        };
        let mut tracks = BTreeMap::new();
        let mut clips = BTreeMap::new();
        let mut lanes = BTreeMap::new();
        for (id, t) in &project.tracks {
            let devices: BTreeMap<DeviceId, DeviceView> = t
                .devices
                .iter()
                .filter_map(|(d, dev)| {
                    let b = builtin(&dev.source)?;
                    Some((
                        d.clone(),
                        DeviceView {
                            kind: b.descriptor.kind_id,
                            enabled: dev.enabled,
                            params: dev
                                .params
                                .iter()
                                .filter_map(|(k, v)| Some((param(&dev.source, k)?, finite_f32(*v))))
                                .collect(),
                        },
                    ))
                })
                .collect();
            let mut placements: Vec<_> = t
                .placements
                .iter()
                .filter(|(_, p)| t.clips.get(&p.clip).is_some_and(|c| c.content.is_midi()))
                .filter_map(|(pid, p)| match p.span {
                    Span::Ticks { length, offset } => {
                        Some((pid.clone(), p.clip.clone(), p.start, offset, length))
                    }
                    Span::Frames { .. } => None,
                })
                .collect();
            placements.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
            let mut scenes = BTreeMap::new();
            for (cid, c) in &t.clips {
                let ClipContent::Midi {
                    length,
                    looping,
                    notes,
                } = &c.content
                else {
                    continue;
                };
                if let Some(scene) = &c.slot {
                    scenes.insert(scene.clone(), cid.clone());
                }
                clips.insert(
                    cid.clone(),
                    ClipView {
                        length: *length,
                        looping: looping.and_then(|l| LoopRegion::new(l.start, l.end)),
                        notes: notes
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
                            .collect(),
                    },
                );
            }
            for (lid, lane) in &t.automation {
                let target = match &lane.target {
                    project::AutomationTarget::Mixer {
                        param: project::MixerParam::Gain,
                    } => TargetView::MixerGain,
                    project::AutomationTarget::Mixer {
                        param: project::MixerParam::Pan,
                    } => TargetView::MixerPan,
                    project::AutomationTarget::Device { device, param: key } => {
                        let Some(p) = t
                            .devices
                            .get(device)
                            .filter(|_| devices.contains_key(device))
                            .and_then(|d| param(&d.source, key))
                        else {
                            continue;
                        };
                        TargetView::Device {
                            device: device.clone(),
                            param: p,
                        }
                    }
                };
                lanes.insert(
                    lid.clone(),
                    LaneView {
                        track: id.clone(),
                        target,
                        points: lane
                            .points
                            .iter()
                            .map(|(tick, p)| AutomationPoint {
                                tick: *tick,
                                value: finite_f32(p.value),
                                curve: match p.shape {
                                    project::Shape::Step => Curve::Step,
                                    project::Shape::Linear => Curve::Linear,
                                },
                            })
                            .collect(),
                    },
                );
            }
            tracks.insert(
                id.clone(),
                TrackView {
                    kind: match t.kind {
                        project::TrackKind::Midi => TrackKind::Midi,
                        project::TrackKind::Audio => TrackKind::Audio,
                    },
                    gain: db_to_gain(t.mixer.gain_db),
                    pan: t.mixer.pan as f32,
                    mute: t.mixer.mute,
                    solo: session.solo.contains(id),
                    armed: session.armed.contains(id),
                    monitor: session.monitor.contains(id),
                    devices,
                    scenes,
                    placements,
                },
            );
        }
        let tr = &project.transport;
        EngineView {
            transport: Some((
                project.tempo(),
                project.time_signature(),
                tr.launch_quantize,
            )),
            loop_region: tr.loop_region.filter(|_| tr.loop_enabled),
            metronome: Some((
                tr.metronome.enabled,
                db_to_gain(tr.metronome.gain_db).clamp(0.0, 4.0),
                tr.metronome.count_in_bars,
            )),
            master: Some((
                db_to_gain(project.master.gain_db),
                project.master.pan as f32,
                project.master.mute,
            )),
            tracks,
            clips,
            lanes,
            selected: session
                .selected_track
                .clone()
                .filter(|t| project.tracks.contains_key(t)),
        }
    }
}
