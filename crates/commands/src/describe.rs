//! What a command acted on, by name, for the history and its activity log (#473): "Mute
//! Lead", "Delete 17 clips (Bass, Drums, Lead)". The page builds the words from a
//! [`Subject`] in the user's language; this module only gathers the names.
//!
//! A subject is read from the project **before** the command applies ([`before`]): a delete
//! removes the names it needs. What a command creates has no name before it applies, so
//! [`after`] reads it from the command's output. A command neither describes gives `None`,
//! and the page shows its generic label.

use crate::Command;
use project::{ClipId, DeviceId, LaneId, PlacementId, Project, SceneId, TrackId};
use schemars::JsonSchema;
use serde::Serialize;

/// The most names a subject keeps in each list: the page shows the first few.
pub const MAX_NAMES: usize = 8;

/// The name the master's chain and lanes go by in a subject: one English word, like a
/// device name.
pub const MASTER: &str = "Master";

/// What an entry of the history (or of its activity log) acted on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Subject {
    /// How many objects it acted on: placements deleted, notes added, tracks muted.
    pub count: u32,
    /// Their names, distinct, in the order met (at most [`MAX_NAMES`]): a track, a clip, a
    /// device, a scene. Empty for objects without a name (notes, points).
    pub objects: Vec<String>,
    /// The names of what holds them, distinct, in the order met (at most [`MAX_NAMES`]): the
    /// track of a clip, placement or device, the clip of notes.
    pub within: Vec<String>,
    /// Which of the objects have no name (a track or a scene the user did not name): their
    /// 1-based position among the tracks or the scenes, distinct, in the order met. The page
    /// names them as it does everywhere ("Track 2"), in the user's language.
    pub unnamed: Vec<u32>,
    /// The same for what holds them: the 1-based positions of the unnamed tracks.
    pub within_unnamed: Vec<u32>,
    /// The value a switch was set to: mute, solo, bypass, the loop, the metronome.
    pub on: Option<bool>,
    /// What a rename or a value change set: the new name, the tempo, the time signature.
    pub to: Option<String>,
    /// The parameter a command set or an automation lane drives, kept only when every
    /// command of an entry agrees.
    pub param: Option<ParamRef>,
    /// The value a parameter was set to, in the unit the device declares.
    pub value: Option<f64>,
}

/// A parameter, by what the page needs to name it in its language.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParamRef {
    /// The device kind (`"faulk.filter"`) when it is a built-in one: the page looks up the
    /// parameter's translated label with it. Absent for a mixer parameter or a module.
    pub kind: Option<String>,
    /// The device's name (one English word, like the device panel shows it); absent for a
    /// mixer parameter.
    pub device: Option<String>,
    /// The parameter's id (`"cutoff"`), or `"gain"` or `"pan"` for the mixer.
    pub key: String,
}

/// How a subject names an object: by its name, or by its position when it has none.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Label {
    Named(String),
    Nth(u32),
}

impl From<String> for Label {
    fn from(name: String) -> Self {
        Label::Named(name)
    }
}

impl Subject {
    fn one(object: Option<Label>, within: Option<Label>) -> Self {
        let mut subject = Self {
            count: 1,
            ..Self::default()
        };
        subject.add(object, within);
        subject
    }

    fn many() -> Self {
        Self::default()
    }

    fn add(&mut self, object: Option<Label>, within: Option<Label>) {
        push(&mut self.objects, &mut self.unnamed, object);
        push(&mut self.within, &mut self.within_unnamed, within);
    }

    fn param(mut self, param: Option<ParamRef>) -> Self {
        self.param = param;
        self
    }

    fn value(mut self, value: f64) -> Self {
        self.value = Some(value);
        self
    }

    fn on(mut self, on: bool) -> Self {
        self.on = Some(on);
        self
    }

    fn to(mut self, to: impl Into<String>) -> Self {
        self.to = Some(to.into());
        self
    }

    /// Joins a later command of the same name into this subject (an entry of a history
    /// group): counts add up, names gather, a switch or a value is kept only when both agree.
    pub fn merge(&mut self, other: &Subject) {
        self.count = self.count.saturating_add(other.count);
        for name in &other.objects {
            push_name(&mut self.objects, name);
        }
        for name in &other.within {
            push_name(&mut self.within, name);
        }
        for n in &other.unnamed {
            push_nth(&mut self.unnamed, *n);
        }
        for n in &other.within_unnamed {
            push_nth(&mut self.within_unnamed, *n);
        }
        if self.on != other.on {
            self.on = None;
        }
        if self.to != other.to {
            self.to = None;
        }
        if self.param != other.param {
            self.param = None;
        }
        if self.value != other.value {
            self.value = None;
        }
    }
}

fn push_name(names: &mut Vec<String>, name: &str) {
    if !name.is_empty() && names.len() < MAX_NAMES && !names.iter().any(|n| n == name) {
        names.push(name.to_owned());
    }
}

fn push_nth(unnamed: &mut Vec<u32>, n: u32) {
    if unnamed.len() < MAX_NAMES && !unnamed.contains(&n) {
        unnamed.push(n);
    }
}

fn push(names: &mut Vec<String>, unnamed: &mut Vec<u32>, label: Option<Label>) {
    match label {
        Some(Label::Named(name)) => push_name(names, &name),
        Some(Label::Nth(n)) => push_nth(unnamed, n),
        None => {}
    }
}

/// A track's name, or its position when it has none.
fn track(p: &Project, id: &TrackId) -> Option<Label> {
    let t = p.tracks.get(id)?;
    // Its place in the display order (order key, then id), counted without sorting.
    let index = p
        .tracks
        .iter()
        .filter(|(tid, o)| (&o.order, *tid) < (&t.order, id))
        .count();
    Some(label(&t.name, index))
}

/// A scene's name, or its position when it has none.
fn scene(p: &Project, id: &SceneId) -> Option<Label> {
    let sc = p.scenes.get(id)?;
    let index = p
        .scenes
        .iter()
        .filter(|(sid, o)| (&o.order, *sid) < (&sc.order, id))
        .count();
    Some(label(&sc.name, index))
}

fn label(name: &str, index: usize) -> Label {
    if name.trim().is_empty() {
        Label::Nth(u32::try_from(index + 1).unwrap_or(u32::MAX))
    } else {
        Label::Named(name.to_owned())
    }
}

fn owner(p: &Project, id: Option<&TrackId>) -> Option<Label> {
    match id {
        Some(id) => track(p, id),
        None => Some(Label::Named(MASTER.to_owned())),
    }
}

/// The clip's name and its track's label.
fn clip(p: &Project, id: &ClipId) -> Option<(String, Label)> {
    let (track_id, c) = p
        .tracks
        .iter()
        .find_map(|(tid, t)| t.clips.get(id).map(|c| (tid, c)))?;
    Some((c.name.clone(), track(p, track_id)?))
}

fn clip_subject(p: &Project, id: &ClipId) -> Option<Subject> {
    clip(p, id).map(|(c, t)| Subject::one(Some(c.into()), Some(t)))
}

/// The name of the clip a placement plays, and its track's label.
fn placement(p: &Project, id: &PlacementId) -> Option<(String, Label)> {
    let (track_id, pl, t) = p
        .tracks
        .iter()
        .find_map(|(tid, t)| t.placements.get(id).map(|pl| (tid, pl, t)))?;
    let name = t.clips.get(&pl.clip).map(|c| c.name.clone());
    Some((name.unwrap_or_default(), track(p, track_id)?))
}

fn placements(p: &Project, ids: &[PlacementId]) -> Option<Subject> {
    let mut subject = Subject::many();
    for id in ids {
        let (clip, track) = placement(p, id)?;
        subject.count += 1;
        subject.add(Some(clip.into()), Some(track));
    }
    Some(subject)
}

fn marker(p: &Project, id: &project::MarkerId) -> Option<Label> {
    p.markers.get(id).map(|m| Label::Named(m.name.clone()))
}

fn device_kind_name(kind: &str) -> Option<String> {
    crate::device::registry()
        .iter()
        .find(|b| b.descriptor.kind == kind)
        .map(|b| b.descriptor.name.to_owned())
}

/// A device's name.
fn device_name(d: &project::Device) -> Option<String> {
    match &d.source {
        project::DeviceSource::Builtin { id, .. } => device_kind_name(id),
        project::DeviceSource::Module { name, .. } => Some(name.clone()),
    }
}

/// A device's name and its owner's (a track, or the master).
fn device(p: &Project, id: &DeviceId) -> Option<(String, Label)> {
    if let Some(d) = p.master_devices.get(id) {
        return Some((device_name(d)?, Label::Named(MASTER.to_owned())));
    }
    let (track_id, d) = p
        .tracks
        .iter()
        .find_map(|(tid, t)| t.devices.get(id).map(|d| (tid, d)))?;
    Some((device_name(d)?, track(p, track_id)?))
}

fn device_subject(p: &Project, id: &DeviceId) -> Option<Subject> {
    device(p, id).map(|(d, t)| Subject::one(Some(d.into()), Some(t)))
}

/// A device parameter, as the page names it.
fn device_param(p: &Project, id: &DeviceId, key: &str) -> Option<ParamRef> {
    // A module has no kind to look the label up with, but it has a device name: only the
    // mixer's parameters have neither.
    let d = p.device(id)?;
    let kind = match &d.source {
        project::DeviceSource::Builtin { id, .. } => Some(id.clone()),
        project::DeviceSource::Module { .. } => None,
    };
    Some(ParamRef {
        kind,
        device: device_name(d),
        key: key.to_owned(),
    })
}

/// What an automation lane drives.
fn target_param(p: &Project, target: &project::AutomationTarget) -> Option<ParamRef> {
    match target {
        project::AutomationTarget::Device { device, param } => device_param(p, device, param),
        project::AutomationTarget::Mixer { param } => Some(ParamRef {
            kind: None,
            device: None,
            key: match param {
                project::MixerParam::Gain => "gain",
                project::MixerParam::Pan => "pan",
            }
            .to_owned(),
        }),
    }
}

/// The lane's owner and parameter, as a subject that holds `count` points.
fn lane(p: &Project, id: &LaneId, count: usize) -> Option<Subject> {
    let held_by = p.lane_owner(id)?;
    let lane = match &held_by {
        Some(track) => p.tracks.get(track)?.automation.get(id)?,
        None => p.master_automation.get(id)?,
    };
    let mut subject =
        Subject::one(None, owner(p, held_by.as_ref())).param(target_param(p, &lane.target));
    subject.count = u32::try_from(count).unwrap_or(u32::MAX);
    Some(subject)
}

/// Notes of a clip: `count` of them, held by the clip.
fn notes(p: &Project, id: &ClipId, count: usize) -> Option<Subject> {
    let (clip, _) = clip(p, id)?;
    let mut subject = Subject::one(None, Some(clip.into()));
    subject.count = u32::try_from(count).unwrap_or(u32::MAX);
    Some(subject)
}

fn trimmed(name: &str) -> String {
    name.trim().to_owned()
}

/// What `command` will act on, read from the project before it applies. `None` when the
/// command does not name its objects that way (see [`after`]) or names one the project
/// does not have (the command will be refused).
pub fn before(command: &Command, p: &Project) -> Option<Subject> {
    use crate::{arrangement as ar, automation as au, clip as cl, device as de};
    use crate::{launch as la, mixer as mx, scene as sc, track as tr, transport as tp};
    match command {
        Command::Track(c) => match c {
            tr::Command::Delete { track: id }
            | tr::Command::SetColor { track: id, .. }
            | tr::Command::SetHeight { track: id, .. }
            | tr::Command::Move { track: id, .. }
            | tr::Command::Duplicate { track: id, .. } => Some(Subject::one(track(p, id), None)),
            tr::Command::Rename { track: id, name } => {
                Some(Subject::one(track(p, id)?.into(), None).to(trimmed(name)))
            }
            tr::Command::SetInstrument {
                track: id,
                instrument,
            } => {
                let subject = Subject::one(track(p, id), None);
                Some(match device_kind_name(instrument) {
                    Some(name) => subject.to(name),
                    None => subject,
                })
            }
            tr::Command::Arm { track: id, armed } => {
                Some(Subject::one(track(p, id), None).on(*armed))
            }
            tr::Command::SetMonitor { track: id, monitor } => {
                Some(Subject::one(track(p, id), None).on(*monitor))
            }
            _ => None,
        },
        Command::Mixer(c) => match c {
            mx::Command::SetVolume { track: id, .. } | mx::Command::SetPan { track: id, .. } => {
                Some(Subject::one(track(p, id), None))
            }
            mx::Command::SetMute { track: id, mute } => {
                Some(Subject::one(track(p, id), None).on(*mute))
            }
            mx::Command::SetSolo { track: id, solo } => {
                Some(Subject::one(track(p, id), None).on(*solo))
            }
            _ => None,
        },
        Command::Device(c) => match c {
            de::Command::Insert {
                track: id, kind, ..
            } => Some(Subject::one(
                device_kind_name(kind).map(Label::from),
                owner(p, id.as_ref()),
            )),
            de::Command::Remove { device: id } | de::Command::Move { device: id, .. } => {
                device_subject(p, id)
            }
            de::Command::SetParam {
                device: id,
                param,
                value,
            } => device_subject(p, id).map(|s| s.param(device_param(p, id, param)).value(*value)),
            de::Command::SetBypass { device: id, bypass } => {
                device_subject(p, id).map(|s| s.on(*bypass))
            }
            // The track it will listen to, by name (#952); its own track says "off".
            de::Command::SetSidechain { device: id, source } => {
                let subject = device_subject(p, id)?.on(source.is_some());
                let name = source
                    .as_ref()
                    .and_then(|s| p.tracks.get(s))
                    .map(|t| t.name.clone())
                    .filter(|n| !n.is_empty());
                Some(match name {
                    Some(name) => subject.to(name),
                    None => subject,
                })
            }
            _ => None,
        },
        Command::Preset(
            crate::preset::Command::Save { device: id, .. }
            | crate::preset::Command::Load { device: id, .. },
        ) => device_subject(p, id),
        Command::Scene(c) => match c {
            sc::Command::Delete { scene: id } | sc::Command::Move { scene: id, .. } => {
                Some(Subject::one(scene(p, id), None))
            }
            sc::Command::Rename { scene: id, name } => {
                Some(Subject::one(scene(p, id), None).to(trimmed(name)))
            }
            _ => None,
        },
        Command::Clip(c) => match c {
            cl::Command::Delete { clip: id }
            | cl::Command::Duplicate { clip: id, .. }
            | cl::Command::Move { clip: id, .. }
            | cl::Command::SetLength { clip: id, .. }
            | cl::Command::SetLoop { clip: id, .. }
            | cl::Command::SetContent { clip: id, .. } => clip_subject(p, id),
            cl::Command::Rename { clip: id, name } => {
                clip_subject(p, id).map(|s| s.to(trimmed(name)))
            }
            cl::Command::NotesAdd { clip: id, notes: n }
            | cl::Command::NotesRemove { clip: id, notes: n }
            | cl::Command::NotesTranspose {
                clip: id, notes: n, ..
            }
            | cl::Command::NotesDuplicate {
                clip: id, notes: n, ..
            }
            | cl::Command::NotesRepeat {
                clip: id, notes: n, ..
            } => notes(p, id, n.len()),
            cl::Command::NotesUpdate { clip: id, changes } => notes(p, id, changes.len()),
            cl::Command::NotesQuantize {
                clip: id, notes: n, ..
            } => {
                let count = match n {
                    Some(n) => n.len(),
                    None => p
                        .tracks
                        .values()
                        .find_map(|t| t.clips.get(id))
                        .map_or(0, |c| match &c.content {
                            project::ClipContent::Midi { notes, .. } => notes.len(),
                            project::ClipContent::Audio { .. } => 0,
                        }),
                };
                notes(p, id, count)
            }
            _ => None,
        },
        Command::Arrangement(c) => match c {
            ar::Command::Place { clip: id, .. } | ar::Command::PlaceFromSlot { clip: id, .. } => {
                clip_subject(p, id)
            }
            ar::Command::Move {
                placements: ids, ..
            }
            | ar::Command::Delete { placements: ids }
            | ar::Command::Duplicate {
                placements: ids, ..
            } => placements(p, ids),
            ar::Command::MoveToTrack { moves, .. } => {
                let ids: Vec<_> = moves.iter().map(|m| m.placement.clone()).collect();
                placements(p, &ids)
            }
            ar::Command::Resize { placement: id, .. }
            | ar::Command::Repeat { placement: id, .. }
            | ar::Command::Split { placement: id, .. } => {
                placement(p, id).map(|(c, t)| Subject::one(Some(c.into()), Some(t)))
            }
            ar::Command::PlaceScene { scene: id, .. } => Some(Subject::one(scene(p, id), None)),
            ar::Command::RenameMarker { marker: id, name } => {
                marker(p, id).map(|n| Subject::one(Some(n), None).to(trimmed(name)))
            }
            ar::Command::MoveMarker { marker: id, .. }
            | ar::Command::DeleteMarker { marker: id } => {
                marker(p, id).map(|n| Subject::one(Some(n), None))
            }
            _ => None,
        },
        Command::Automation(c) => match c {
            au::Command::LaneDelete { lane: id } | au::Command::LaneClear { lane: id } => {
                lane(p, id, 1)
            }
            au::Command::PointsAdd { lane: id, points }
            | au::Command::PointsUpdate { lane: id, points } => lane(p, id, points.len()),
            au::Command::PointsRemove { lane: id, ticks } => lane(p, id, ticks.len()),
            // A lane created has no identifier before it applies: `after` reads it.
            au::Command::PointsRestore {
                lane: id,
                points,
                remove,
            } => lane(p, id, points.len() + remove.len()),
            _ => None,
        },
        Command::Transport(c) => match c {
            tp::Command::ToggleLoop {} => Some(Subject::default().on(!p.transport.loop_enabled)),
            tp::Command::SetMetronome { enabled, .. } => Some(Subject {
                on: *enabled,
                ..Subject::default()
            }),
            tp::Command::SetTempo { bpm, .. } => Some(Subject::default().to(format_bpm(*bpm))),
            tp::Command::SetTimeSignature {
                numerator,
                denominator,
            } => Some(Subject::default().to(format!("{numerator}/{denominator}"))),
            _ => None,
        },
        // One take is one entry (#470): its notes, counted, in the tracks it wrote to.
        Command::Record(crate::record::Command::CommitTake { tracks, .. }) => {
            let mut subject = Subject::many();
            for take in tracks {
                subject.count = subject
                    .count
                    .saturating_add(u32::try_from(take.notes.len()).unwrap_or(u32::MAX));
                subject.add(None, track(p, &take.track));
            }
            Some(subject)
        }
        Command::Launch(c) => match c {
            la::Command::Clip { clip: id } => clip_subject(p, id),
            la::Command::Scene { scene: id } => Some(Subject::one(scene(p, id), None)),
            la::Command::StopTrack { track: id }
            | la::Command::BackTrackToArrangement { track: id } => {
                Some(Subject::one(track(p, id), None))
            }
            _ => None,
        },
        _ => None,
    }
}

/// A tempo as the transport shows it: at most two decimals, none when whole.
fn format_bpm(bpm: f64) -> String {
    let rounded = (bpm * 100.0).round() / 100.0;
    let text = format!("{rounded:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// What a command created, read from its output once it applied: a track, a scene, a
/// clip, a lane. `None` for any other command.
pub fn after(output: &crate::Output, p: &Project) -> Option<Subject> {
    use crate::{automation as au, clip as cl, scene as sc, track as tr};
    match output {
        crate::Output::Track(tr::Output::Create { track: id, .. }) => {
            // The instrument it plays tells the new track apart from the last one.
            let instrument = p.tracks.get(id).and_then(|t| {
                t.devices
                    .values()
                    .find(|d| d.role == project::DeviceRole::Instrument)
                    .and_then(device_name)
            });
            Some(Subject::one(track(p, id), instrument.map(Label::from)))
        }
        crate::Output::Scene(sc::Output::Create { scene: id }) => {
            Some(Subject::one(scene(p, id), None))
        }
        crate::Output::Clip(cl::Output::Create { clip: id, .. }) => clip_subject(p, id),
        crate::Output::Automation(au::Output::LaneCreate { lane: id }) => lane(p, id, 1),
        crate::Output::Arrangement(crate::arrangement::Output::AddMarker { marker: id }) => {
            marker(p, id).map(|n| Subject::one(Some(n), None))
        }
        _ => None,
    }
}
