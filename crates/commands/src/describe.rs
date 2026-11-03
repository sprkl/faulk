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
    /// The value a switch was set to: mute, solo, bypass, the loop, the metronome.
    pub on: Option<bool>,
    /// What a rename or a value change set: the new name, the tempo, the time signature.
    pub to: Option<String>,
}

impl Subject {
    fn one(object: Option<String>, within: Option<String>) -> Self {
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

    fn add(&mut self, object: Option<String>, within: Option<String>) {
        push(&mut self.objects, object);
        push(&mut self.within, within);
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
            push(&mut self.objects, Some(name.clone()));
        }
        for name in &other.within {
            push(&mut self.within, Some(name.clone()));
        }
        if self.on != other.on {
            self.on = None;
        }
        if self.to != other.to {
            self.to = None;
        }
    }
}

fn push(names: &mut Vec<String>, name: Option<String>) {
    if let Some(name) = name
        && names.len() < MAX_NAMES
        && !names.contains(&name)
    {
        names.push(name);
    }
}

fn track(p: &Project, id: &TrackId) -> Option<String> {
    p.tracks.get(id).map(|t| t.name.clone())
}

fn owner(p: &Project, id: Option<&TrackId>) -> Option<String> {
    match id {
        Some(id) => track(p, id),
        None => Some(MASTER.to_owned()),
    }
}

/// The clip and its track's names.
fn clip(p: &Project, id: &ClipId) -> Option<(String, String)> {
    p.tracks
        .values()
        .find_map(|t| t.clips.get(id).map(|c| (c.name.clone(), t.name.clone())))
}

fn clip_subject(p: &Project, id: &ClipId) -> Option<Subject> {
    clip(p, id).map(|(c, t)| Subject::one(Some(c), Some(t)))
}

/// The name of the clip a placement plays, and its track's.
fn placement(p: &Project, id: &PlacementId) -> Option<(String, String)> {
    p.tracks.values().find_map(|t| {
        t.placements.get(id).map(|pl| {
            let name = t.clips.get(&pl.clip).map(|c| c.name.clone());
            (name.unwrap_or_default(), t.name.clone())
        })
    })
}

fn placements(p: &Project, ids: &[PlacementId]) -> Option<Subject> {
    let mut subject = Subject::many();
    for id in ids {
        let (clip, track) = placement(p, id)?;
        subject.count += 1;
        subject.add(Some(clip).filter(|c| !c.is_empty()), Some(track));
    }
    Some(subject)
}

fn marker(p: &Project, id: &project::MarkerId) -> Option<String> {
    p.markers.get(id).map(|m| m.name.clone())
}

fn scene(p: &Project, id: &SceneId) -> Option<String> {
    p.scenes.get(id).map(|s| s.name.clone())
}

fn device_kind_name(kind: &str) -> Option<String> {
    crate::device::registry()
        .iter()
        .find(|b| b.descriptor.kind == kind)
        .map(|b| b.descriptor.name.to_owned())
}

/// A device's name and its owner's (a track, or the master).
fn device(p: &Project, id: &DeviceId) -> Option<(String, String)> {
    let named = |d: &project::Device| match &d.source {
        project::DeviceSource::Builtin { id, .. } => device_kind_name(id),
        project::DeviceSource::Module { name, .. } => Some(name.clone()),
    };
    if let Some(d) = p.master_devices.get(id) {
        return Some((named(d)?, MASTER.to_owned()));
    }
    p.tracks
        .values()
        .find_map(|t| t.devices.get(id).map(|d| (named(d), t.name.clone())))
        .and_then(|(d, t)| Some((d?, t)))
}

fn device_subject(p: &Project, id: &DeviceId) -> Option<Subject> {
    device(p, id).map(|(d, t)| Subject::one(Some(d), Some(t)))
}

/// The owner of a lane, as a subject that holds `count` points.
fn lane(p: &Project, id: &LaneId, count: usize) -> Option<Subject> {
    let held_by = p.lane_owner(id)?;
    let mut subject = Subject::one(None, owner(p, held_by.as_ref()));
    subject.count = u32::try_from(count).unwrap_or(u32::MAX);
    Some(subject)
}

/// Notes of a clip: `count` of them, held by the clip.
fn notes(p: &Project, id: &ClipId, count: usize) -> Option<Subject> {
    let (clip, _) = clip(p, id)?;
    let mut subject = Subject::one(None, Some(clip));
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
            } => Some(Subject::one(device_kind_name(kind), owner(p, id.as_ref()))),
            de::Command::Remove { device: id }
            | de::Command::Move { device: id, .. }
            | de::Command::SetParam { device: id, .. } => device_subject(p, id),
            de::Command::SetBypass { device: id, bypass } => {
                device_subject(p, id).map(|s| s.on(*bypass))
            }
            _ => None,
        },
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
            | ar::Command::Split { placement: id, .. } => placement(p, id)
                .map(|(c, t)| Subject::one(Some(c).filter(|c| !c.is_empty()), Some(t))),
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
            au::Command::LaneCreate { track: id, .. } => {
                Some(Subject::one(None, owner(p, id.as_ref())))
            }
            _ => None,
        },
        Command::Transport(c) => match c {
            tp::Command::ToggleLoop {} => Some(Subject::default().on(!p.transport.loop_enabled)),
            tp::Command::SetMetronome { enabled, .. } => Some(Subject {
                on: *enabled,
                ..Subject::default()
            }),
            tp::Command::SetTempo { bpm } => Some(Subject::default().to(format_bpm(*bpm))),
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
            Some(Subject::one(track(p, id), None))
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
