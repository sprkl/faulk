//! The query commands (`x-kind: query`, PR-6): what an agent reads of the project before it
//! edits it. The commands are declared in their own domains (`project.get`, `track.list`,
//! `track.get`, `clip.get`, `clip.notes.list`, `scene.list`, `device.describe`,
//! `automation.lane.list`, `automation.lane.points`, `transport.get`); this module holds
//! what they share: the pagination rule, the output types and the readers.
//!
//! The contract of a query, repeated in each description for agents:
//!
//! - **A query changes nothing.** It borrows the [`State`] immutably, so it cannot change
//!   the project, the session or the history: it returns no inverse and no event, never
//!   enters the history ([`crate::History::record`]) and never marks the project as having
//!   changes to save (the page counts an `edit` that reports an event, nothing else). The
//!   test rule checks `state == before` on every query example, and a test of this crate
//!   checks it again on a rich project.
//! - **Pagination.** A list that has no bound of its own besides the safety limits of the
//!   project (the tracks, the scenes, the lanes, the clips of a track, the notes of a clip,
//!   the points of a lane, the placements of a clip) takes `offset` (items to skip, 0 by
//!   default) and `limit` (items to return, [`DEFAULT_PAGE`] by default, from 1 to
//!   [`MAX_PAGE`]; a `limit` outside that range is refused with `invalid`, never clamped).
//!   The answer carries a [`PageInfo`]: `total` items in the list, the `offset` used and
//!   `next_offset`, the `offset` of the next page or `null` after the last one. An `offset`
//!   at or past the end gives an empty page, not an error. Each list has one fixed order
//!   (stated in its command), so pages of an unchanged project never overlap nor skip.
//! - A query with a `limit` and an `offset` pages exactly one list; the other fields of
//!   its answer are bounded by the safety limits or are counts.

use crate::contract::{CommandError, State};
use crate::device::{descriptor_of, registry};
use engine::device::{
    DeviceDescriptor, DeviceRole as KindRole, ParamDescriptor, Scale as KindScale, Unit,
};
use project::{
    AudioMedia, AutomationLane, AutomationTarget, Clip, ClipContent, ClipId, Device, DeviceId,
    DeviceRole, DeviceSource, FrameLoop, Hash, LaneId, Mixer, Note, Placement, PlacementId,
    Project, Quantize, Rgb, Scene, SceneId, Tick, TickLoop, Track, TrackId, TrackKind,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Items a page holds when the query gives no `limit`.
pub const DEFAULT_PAGE: u32 = 100;
/// Most items a page holds: a larger `limit` is refused.
pub const MAX_PAGE: u32 = 1_000;

/// Where a page sits in its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageInfo {
    /// How many items the whole list holds.
    pub total: u32,
    /// The index of the first item of the page (the `offset` asked for, 0 by default).
    pub offset: u32,
    /// The `offset` to ask for the next page; `null` when this page reaches the end of the
    /// list.
    pub next_offset: Option<u32>,
}

/// One page of a list, in the list's fixed order.
pub(crate) fn page<T>(
    items: impl ExactSizeIterator<Item = T>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<(Vec<T>, PageInfo), CommandError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE);
    if limit == 0 || limit > MAX_PAGE {
        return Err(CommandError::Invalid {
            field: "limit".into(),
            message: format!("the limit is from 1 to {MAX_PAGE}, not {limit}"),
        });
    }
    let offset = offset.unwrap_or(0);
    let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let taken: Vec<T> = items.skip(offset as usize).take(limit as usize).collect();
    // `taken.len() <= limit <= MAX_PAGE`, so the sum cannot overflow past `u32::MAX`
    // without also passing `total`.
    let end = offset.saturating_add(u32::try_from(taken.len()).unwrap_or(u32::MAX));
    let next_offset = (end < total).then_some(end);
    Ok((
        taken,
        PageInfo {
            total,
            offset,
            next_offset,
        },
    ))
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn not_found(entity: &str, id: impl ToString) -> CommandError {
    CommandError::NotFound {
        entity: entity.into(),
        id: id.to_string(),
    }
}

// ------------------------------------------------------------------ output types

/// A track in a list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackSummary {
    /// The track's identifier.
    pub id: TrackId,
    /// The track's name.
    pub name: String,
    /// `#rrggbb`, or `null` for the default colour.
    pub color: Option<Rgb>,
    /// The height of the track's row in the arrangement, in pixels, or `null` for the default.
    pub height: Option<u16>,
    /// `midi` (MIDI clips played by the instrument) or `audio`.
    pub kind: TrackKind,
    /// Gain in dB (`null` is silence), pan from -1 to 1 and mute.
    pub mixer: Mixer,
    /// Whether the track is soloed (session state).
    pub solo: bool,
    /// Whether the track is armed for recording (session state).
    pub armed: bool,
    /// How many devices its chain holds.
    pub device_count: u32,
    /// How many clips it holds.
    pub clip_count: u32,
}

/// A device of a chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceSummary {
    /// The device's identifier.
    pub id: DeviceId,
    /// `instrument` or `effect`.
    pub role: DeviceRole,
    /// `false` when the device is bypassed.
    pub enabled: bool,
    /// Where the device comes from: a built-in kind (`faulk.osc_synth`) or a module.
    pub source: DeviceSource,
    /// The device's English name; `null` for a module and for the clip player.
    pub name: Option<String>,
}

/// A clip in a list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipSummary {
    /// The clip's identifier.
    pub id: ClipId,
    /// The clip's name.
    pub name: String,
    /// The clip's colour, `#rrggbb`, or `null`.
    pub color: Option<Rgb>,
    /// The scene of the launch-grid slot the clip sits in, or `null` when it only lives
    /// in the arrangement.
    pub slot: Option<SceneId>,
    /// `midi` or `audio`, as its track.
    pub kind: TrackKind,
    /// The length in ticks (960 per quarter note) of a MIDI clip; `null` for an audio clip
    /// (its length depends on the tempo).
    pub length: Option<Tick>,
    /// How many notes a MIDI clip holds; `null` for an audio clip.
    pub note_count: Option<u32>,
    /// For an audio clip, whether its file was missing from the project's folder when the
    /// project was opened ("média absent": the clip stays in the project, silent); `null`
    /// for a MIDI clip.
    pub media_missing: Option<bool>,
}

/// What a clip holds, apart from the notes (`clip.notes.list`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipContentInfo {
    /// A MIDI clip: musical time.
    Midi {
        /// Length in ticks.
        length: Tick,
        /// The loop `[start, end)` in ticks inside the clip, if one is set.
        looping: Option<TickLoop>,
        /// How many notes it holds.
        note_count: u32,
    },
    /// An audio clip: a range of a file.
    Audio {
        /// The audio file the clip plays.
        media: AudioMedia,
        /// First frame of the file played.
        start: u64,
        /// Frames played.
        frames: u64,
        /// Clip gain in dB; `null` is silence.
        gain_db: Option<f64>,
        /// The loop `[start, end)` in frames of the file, if one is set.
        looping: Option<FrameLoop>,
    },
}

/// A clip set in a track's arrangement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacementInfo {
    /// The placement's identifier.
    pub id: PlacementId,
    /// The placement: clip, start and span.
    pub placement: Placement,
}

/// A scene of the launch grid with the clips of its slots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneInfo {
    /// The scene's identifier.
    pub id: SceneId,
    /// The scene's name.
    pub name: String,
    /// The clips in this scene's slots, one per track at most, in the tracks' display order.
    pub clips: Vec<SceneSlot>,
}

/// A filled slot of a scene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneSlot {
    /// The track whose slot it is.
    pub track: TrackId,
    /// The clip in the slot.
    pub clip: ClipId,
}

/// An automation lane without its points (`automation.lane.points` pages them).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LaneSummary {
    /// The lane's identifier.
    pub id: LaneId,
    /// The track that owns the lane; `null` for a lane of the master.
    pub track: Option<TrackId>,
    /// The parameter the lane automates.
    /// The parameter the lane automates.
    pub target: AutomationTarget,
    /// How many points the lane holds.
    pub point_count: u32,
}

/// The unit of a device parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParamUnit {
    /// A plain number.
    None,
    /// A linear gain (1 is unity).
    Gain,
    Seconds,
    Hertz,
    Decibels,
    Semitones,
    Cents,
    Percent,
    /// An angle, in degrees.
    Degrees,
    /// A frequency ratio, in octaves.
    Octaves,
}

/// How a control maps its travel to a parameter's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParamScale {
    Linear,
    Log,
}

/// What a device kind does in a chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KindRoleInfo {
    Instrument,
    Effect,
    MidiEffect,
}

/// One parameter a device kind declares.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParamSpec {
    /// The key of the parameter: what `device.set_param` and the automation targets name.
    pub key: String,
    /// What the parameter does and what its range means.
    pub description: String,
    /// The unit of the value.
    pub unit: ParamUnit,
    /// The lowest value.
    pub min: f64,
    /// The highest value.
    pub max: f64,
    /// The value a new device starts with.
    pub default: f64,
    /// `linear` or `log` (logarithmic travel, for frequencies and times).
    pub scale: ParamScale,
    /// The smallest change of the value; 0 for a continuous parameter.
    pub step: f64,
    /// For an enumerated parameter, the key of each value from `min` to `max`; empty
    /// otherwise.
    pub values: Vec<String>,
}

/// A MIDI key an instrument gives a name to (a drum kit's sounds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyName {
    /// The MIDI key, 0 to 127.
    pub key: u8,
    /// The name of the sound on that key.
    pub name: String,
}

/// The static description of a built-in device kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceKindInfo {
    /// The kind, `faulk.<name>`: the `id` of a `builtin` device source.
    pub kind: String,
    /// The kind's version, raised when its parameters change meaning.
    pub version: u32,
    /// The device's one English name.
    pub name: String,
    /// What the kind does in a chain.
    pub role: KindRoleInfo,
    /// The kind's parameters, in declaration order.
    pub params: Vec<ParamSpec>,
    /// The notes an instrument answers to, with their names; empty for a pitched
    /// instrument and for effects.
    pub named_keys: Vec<KeyName>,
}

// ------------------------------------------------------------------ readers

pub(crate) struct ProjectData {
    pub id: project::ProjectId,
    pub name: String,
    pub format: u32,
    pub tempo_bpm: f64,
    pub numerator: u8,
    pub denominator: u8,
    pub master: Mixer,
    pub track_count: u32,
    pub scene_count: u32,
    pub clip_count: u32,
    pub placement_count: u32,
    pub lane_count: u32,
    pub master_devices: Vec<DeviceSummary>,
    pub selected_track: Option<TrackId>,
    pub can_undo: bool,
    pub can_redo: bool,
}

pub(crate) fn project_data(state: &State) -> ProjectData {
    let p = &state.project;
    let counts = p.counts();
    let signature = p.time_signature();
    ProjectData {
        id: p.id.clone(),
        name: p.name.clone(),
        format: p.format,
        tempo_bpm: p.tempo().bpm(),
        numerator: signature.numerator(),
        denominator: signature.denominator(),
        master: p.master,
        track_count: count(p.tracks.len()),
        scene_count: count(p.scenes.len()),
        clip_count: count(counts.clips),
        placement_count: count(counts.placements),
        lane_count: count(counts.lanes),
        master_devices: p
            .master_devices_in_order()
            .into_iter()
            .map(|(id, d)| device_summary(id, d))
            .collect(),
        selected_track: state.session.selected_track.clone(),
        can_undo: state.history.can_undo(),
        can_redo: state.history.can_redo(),
    }
}

fn track_summary(state: &State, id: &TrackId, track: &Track) -> TrackSummary {
    TrackSummary {
        id: id.clone(),
        name: track.name.clone(),
        color: track.color,
        height: track.height,
        kind: track.kind,
        mixer: track.mixer,
        solo: state.session.solo.contains(id),
        armed: state.session.armed.contains(id),
        device_count: count(track.devices.len()),
        clip_count: count(track.clips.len()),
    }
}

pub(crate) fn track_list(
    state: &State,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<(Vec<TrackSummary>, PageInfo), CommandError> {
    let ordered = state.project.tracks_in_order();
    let (items, info) = page(ordered.into_iter(), offset, limit)?;
    let tracks = items
        .into_iter()
        .map(|(id, track)| track_summary(state, id, track))
        .collect();
    Ok((tracks, info))
}

pub(crate) struct TrackData {
    pub track: TrackSummary,
    pub devices: Vec<DeviceSummary>,
    pub clips: Vec<ClipSummary>,
    pub page: PageInfo,
    pub placement_count: u32,
    pub lane_count: u32,
}

pub(crate) fn track_get(
    state: &State,
    missing: &BTreeSet<Hash>,
    id: &TrackId,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<TrackData, CommandError> {
    let track = state
        .project
        .tracks
        .get(id)
        .ok_or_else(|| not_found("track", id))?;
    let (clips, page_info) = page(track.clips.iter(), offset, limit)?;
    Ok(TrackData {
        track: track_summary(state, id, track),
        devices: track
            .devices_in_order()
            .into_iter()
            .map(|(id, device)| device_summary(id, device))
            .collect(),
        clips: clips
            .into_iter()
            .map(|(id, clip)| clip_summary(id, clip, missing))
            .collect(),
        page: page_info,
        placement_count: count(track.placements.len()),
        lane_count: count(track.automation.len()),
    })
}

fn device_name(device: &Device) -> Option<String> {
    descriptor_of(registry(), device).map(|d| d.name.to_owned())
}

fn device_summary(id: &DeviceId, device: &Device) -> DeviceSummary {
    DeviceSummary {
        id: id.clone(),
        role: device.role,
        enabled: device.enabled,
        source: device.source.clone(),
        name: device_name(device),
    }
}

fn clip_summary(id: &ClipId, clip: &Clip, missing: &BTreeSet<Hash>) -> ClipSummary {
    let (kind, length, note_count, media_missing) = match &clip.content {
        ClipContent::Midi { length, notes, .. } => (
            TrackKind::Midi,
            Some(*length),
            Some(count(notes.len())),
            None,
        ),
        ClipContent::Audio { media, .. } => (
            TrackKind::Audio,
            None,
            None,
            Some(missing.contains(&media.hash)),
        ),
    };
    ClipSummary {
        id: id.clone(),
        name: clip.name.clone(),
        color: clip.color,
        slot: clip.slot.clone(),
        kind,
        length,
        note_count,
        media_missing,
    }
}

/// The track that holds a clip, with the clip.
fn find_clip<'a>(
    project: &'a Project,
    id: &ClipId,
) -> Result<(&'a TrackId, &'a Track, &'a Clip), CommandError> {
    project
        .tracks
        .iter()
        .find_map(|(tid, t)| t.clips.get(id).map(|c| (tid, t, c)))
        .ok_or_else(|| not_found("clip", id))
}

pub(crate) struct ClipData {
    pub track: TrackId,
    pub summary: ClipSummary,
    pub launch_quantize: Option<Quantize>,
    pub content: ClipContentInfo,
    pub placements: Vec<PlacementInfo>,
    pub page: PageInfo,
}

pub(crate) fn clip_get(
    project: &Project,
    missing: &BTreeSet<Hash>,
    id: &ClipId,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<ClipData, CommandError> {
    let (track_id, track, clip) = find_clip(project, id)?;
    let own = track.placements.iter().filter(|(_, p)| p.clip == *id);
    // The filter has no exact size: collect the clip's placements (bounded by the track's
    // safety limit) so that the page knows its total.
    let own: Vec<_> = own.collect();
    let (items, page_info) = page(own.into_iter(), offset, limit)?;
    let content = match &clip.content {
        ClipContent::Midi {
            length,
            looping,
            notes,
        } => ClipContentInfo::Midi {
            length: *length,
            looping: *looping,
            note_count: count(notes.len()),
        },
        ClipContent::Audio {
            media,
            start,
            frames,
            gain_db,
            looping,
        } => ClipContentInfo::Audio {
            media: media.clone(),
            start: start.0,
            frames: frames.0,
            gain_db: gain_db.is_finite().then_some(*gain_db),
            looping: *looping,
        },
    };
    Ok(ClipData {
        track: track_id.clone(),
        summary: clip_summary(id, clip, missing),
        launch_quantize: clip.launch_quantize,
        content,
        placements: items
            .into_iter()
            .map(|(pid, p)| PlacementInfo {
                id: pid.clone(),
                placement: p.clone(),
            })
            .collect(),
        page: page_info,
    })
}

pub(crate) fn clip_notes(
    project: &Project,
    id: &ClipId,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<(Vec<Note>, PageInfo), CommandError> {
    let (_, _, clip) = find_clip(project, id)?;
    let ClipContent::Midi { notes, .. } = &clip.content else {
        return Err(CommandError::Invalid {
            field: "clip".into(),
            message: format!("clip {id} is an audio clip: it has no notes"),
        });
    };
    let (items, info) = page(notes.iter(), offset, limit)?;
    Ok((items.into_iter().copied().collect(), info))
}

pub(crate) fn scene_list(
    project: &Project,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<(Vec<SceneInfo>, PageInfo), CommandError> {
    let ordered: Vec<(&SceneId, &Scene)> = project.scenes_in_order();
    let (items, info) = page(ordered.into_iter(), offset, limit)?;
    // The filled slots of the scenes on this page, in one pass over the clips.
    let mut slots: BTreeMap<&SceneId, Vec<SceneSlot>> = BTreeMap::new();
    for (track_id, track) in project.tracks_in_order() {
        for (clip_id, clip) in &track.clips {
            if let Some(scene) = &clip.slot {
                slots.entry(scene).or_default().push(SceneSlot {
                    track: track_id.clone(),
                    clip: clip_id.clone(),
                });
            }
        }
    }
    let scenes = items
        .into_iter()
        .map(|(id, scene)| SceneInfo {
            id: id.clone(),
            name: scene.name.clone(),
            clips: slots.remove(id).unwrap_or_default(),
        })
        .collect();
    Ok((scenes, info))
}

fn lane_summary(id: &LaneId, owner: Option<&TrackId>, lane: &AutomationLane) -> LaneSummary {
    LaneSummary {
        id: id.clone(),
        track: owner.cloned(),
        target: lane.target.clone(),
        point_count: count(lane.points.len()),
    }
}

pub(crate) fn lane_list(
    project: &Project,
    track: Option<&TrackId>,
    master: bool,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<(Vec<LaneSummary>, PageInfo), CommandError> {
    if track.is_some() && master {
        return Err(CommandError::Invalid {
            field: "master".into(),
            message: "name a track or the master, not both".into(),
        });
    }
    let mut lanes: Vec<LaneSummary> = Vec::new();
    let mut add = |owner: Option<&TrackId>, map: &BTreeMap<LaneId, AutomationLane>| {
        lanes.extend(map.iter().map(|(id, lane)| lane_summary(id, owner, lane)));
    };
    if let Some(id) = track {
        let t = project
            .tracks
            .get(id)
            .ok_or_else(|| not_found("track", id))?;
        add(Some(id), &t.automation);
    } else if master {
        add(None, &project.master_automation);
    } else {
        add(None, &project.master_automation);
        for (id, t) in project.tracks_in_order() {
            add(Some(id), &t.automation);
        }
    }
    let (items, info) = page(lanes.into_iter(), offset, limit)?;
    Ok((items, info))
}

pub(crate) fn lane_points(
    project: &Project,
    id: &LaneId,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<(LaneSummary, Vec<crate::automation::Point>, PageInfo), CommandError> {
    let owner = project
        .lane_owner(id)
        .ok_or_else(|| not_found("lane", id))?;
    let lane = project
        .lanes(owner.as_ref())
        .and_then(|lanes| lanes.get(id))
        .ok_or_else(|| not_found("lane", id))?;
    let (items, info) = page(lane.points.iter(), offset, limit)?;
    let points = items
        .into_iter()
        .map(|(tick, p)| crate::automation::Point {
            tick: *tick,
            value: p.value,
            shape: p.shape,
        })
        .collect();
    Ok((lane_summary(id, owner.as_ref(), lane), points, info))
}

fn param_spec(p: &ParamDescriptor) -> ParamSpec {
    ParamSpec {
        key: p.key.to_owned(),
        description: p.description.to_owned(),
        unit: match p.unit {
            Unit::None => ParamUnit::None,
            Unit::Gain => ParamUnit::Gain,
            Unit::Seconds => ParamUnit::Seconds,
            Unit::Hertz => ParamUnit::Hertz,
            Unit::Decibels => ParamUnit::Decibels,
            Unit::Semitones => ParamUnit::Semitones,
            Unit::Cents => ParamUnit::Cents,
            Unit::Percent => ParamUnit::Percent,
            Unit::Degrees => ParamUnit::Degrees,
            Unit::Octaves => ParamUnit::Octaves,
        },
        min: p.min,
        max: p.max,
        default: p.default,
        scale: match p.scale {
            KindScale::Linear => ParamScale::Linear,
            KindScale::Log => ParamScale::Log,
        },
        step: p.step,
        values: p.values.iter().map(|v| (*v).to_owned()).collect(),
    }
}

fn kind_info(d: &DeviceDescriptor) -> DeviceKindInfo {
    DeviceKindInfo {
        kind: d.kind.to_owned(),
        version: d.version,
        name: d.name.to_owned(),
        role: match d.role {
            KindRole::Instrument => KindRoleInfo::Instrument,
            KindRole::Effect => KindRoleInfo::Effect,
            KindRole::MidiEffect => KindRoleInfo::MidiEffect,
        },
        params: d.params.iter().map(param_spec).collect(),
        named_keys: d
            .named_keys
            .iter()
            .map(|k| KeyName {
                key: k.key,
                name: k.name.to_owned(),
            })
            .collect(),
    }
}

pub(crate) struct DeviceData {
    pub track: Option<TrackId>,
    pub in_master: bool,
    pub device: Option<DeviceSummary>,
    pub params: BTreeMap<String, f64>,
    pub descriptor: Option<DeviceKindInfo>,
}

pub(crate) fn device_describe(
    project: &Project,
    device: Option<&DeviceId>,
    kind: Option<&str>,
) -> Result<DeviceData, CommandError> {
    match (device, kind) {
        (Some(id), None) => {
            let found = project.device(id).ok_or_else(|| not_found("device", id))?;
            let track = project
                .tracks
                .iter()
                .find(|(_, t)| t.devices.contains_key(id))
                .map(|(tid, _)| tid.clone());
            Ok(DeviceData {
                track,
                in_master: project.master_devices.contains_key(id),
                device: Some(device_summary(id, found)),
                params: found.params.clone(),
                descriptor: descriptor_of(registry(), found).map(kind_info),
            })
        }
        (None, Some(kind)) => {
            let descriptor = registry()
                .iter()
                .find(|b| b.descriptor.kind == kind)
                .map(|b| kind_info(b.descriptor))
                .ok_or_else(|| not_found("device_kind", kind))?;
            Ok(DeviceData {
                track: None,
                in_master: false,
                device: None,
                params: BTreeMap::new(),
                descriptor: Some(descriptor),
            })
        }
        _ => Err(CommandError::Invalid {
            field: "device".into(),
            message: "give exactly one of `device` (a device id) and `kind` (a built-in kind)"
                .into(),
        }),
    }
}

/// The outcome of a query: its answer, and nothing else (no inverse, no event, no engine
/// message).
pub(crate) fn answer<C, O>(output: O) -> crate::contract::Outcome<C, O> {
    crate::contract::Outcome {
        output,
        undo: crate::contract::Undo::None,
        events: vec![],
        control: vec![],
    }
}
