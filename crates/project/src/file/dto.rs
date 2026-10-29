//! The JSON files: what each one holds. The project is split in small documents, one per
//! file (§ 2.2); these are their shapes. They refuse unknown fields (a newer format is
//! detected by its `format` first, never by a parse error).

use crate::ids::{ProjectId, SceneId};
use crate::model::*;
use crate::serde_util::{Rgb, db, unique_map};
use engine::time::{Frame, Tick};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// `project.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDoc {
    pub format: u32,
    pub id: ProjectId,
    pub name: String,
    pub tempo: Vec<TempoPoint>,
    pub meter: Vec<MeterPoint>,
    pub transport: TransportSettings,
    #[serde(deserialize_with = "unique_map")]
    pub scenes: BTreeMap<SceneId, Scene>,
    pub master: Mixer,
    /// The master's chain (#364): effects only; left out when empty, so that a project
    /// without one writes the same file as before.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "unique_map"
    )]
    pub master_devices: BTreeMap<crate::ids::DeviceId, Device>,
}

impl ProjectDoc {
    pub fn of(p: &Project) -> Self {
        Self {
            format: p.format,
            id: p.id.clone(),
            name: p.name.clone(),
            tempo: p.tempo.clone(),
            meter: p.meter.clone(),
            transport: p.transport,
            scenes: p.scenes.clone(),
            master: p.master,
            master_devices: p.master_devices.clone(),
        }
    }

    pub fn into_project(self) -> Project {
        Project {
            format: self.format,
            id: self.id,
            name: self.name,
            tempo: self.tempo,
            meter: self.meter,
            transport: self.transport,
            scenes: self.scenes,
            master: self.master,
            master_devices: self.master_devices,
            master_automation: BTreeMap::new(),
            tracks: BTreeMap::new(),
        }
    }
}

/// `tracks/<track>/track.json`: the track without what has its own files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackDoc {
    pub name: String,
    pub color: Option<Rgb>,
    pub order: crate::order::OrderKey,
    pub kind: TrackKind,
    pub mixer: Mixer,
    pub output: Output,
}

impl TrackDoc {
    pub fn of(t: &Track) -> Self {
        Self {
            name: t.name.clone(),
            color: t.color,
            order: t.order.clone(),
            kind: t.kind,
            mixer: t.mixer,
            output: t.output,
        }
    }

    pub fn into_track(self) -> Track {
        Track {
            name: self.name,
            color: self.color,
            order: self.order,
            kind: self.kind,
            mixer: self.mixer,
            output: self.output,
            devices: BTreeMap::new(),
            clips: BTreeMap::new(),
            placements: BTreeMap::new(),
            automation: BTreeMap::new(),
        }
    }
}

/// `tracks/<track>/clips/<clip>.json`: the clip header; a MIDI clip's notes are in
/// `<clip>.notes.tsv`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipDoc {
    pub name: String,
    pub color: Option<Rgb>,
    pub slot: Option<SceneId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_quantize: Option<engine::time::Quantize>,
    pub content: ClipHead,
}

/// A clip's content without its notes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipHead {
    Midi {
        length: Tick,
        looping: Option<TickLoop>,
    },
    Audio {
        media: AudioMedia,
        start: Frame,
        frames: Frame,
        #[serde(with = "db")]
        gain_db: f64,
        looping: Option<FrameLoop>,
    },
}

impl ClipDoc {
    pub fn of(c: &Clip) -> Self {
        Self {
            name: c.name.clone(),
            color: c.color,
            slot: c.slot.clone(),
            launch_quantize: c.launch_quantize,
            content: match &c.content {
                ClipContent::Midi {
                    length, looping, ..
                } => ClipHead::Midi {
                    length: *length,
                    looping: *looping,
                },
                ClipContent::Audio {
                    media,
                    start,
                    frames,
                    gain_db,
                    looping,
                } => ClipHead::Audio {
                    media: media.clone(),
                    start: *start,
                    frames: *frames,
                    gain_db: *gain_db,
                    looping: *looping,
                },
            },
        }
    }

    pub fn is_midi(&self) -> bool {
        matches!(self.content, ClipHead::Midi { .. })
    }

    /// The clip, with `notes` for a MIDI clip (ignored for an audio clip).
    pub fn into_clip(self, notes: BTreeSet<Note>) -> Clip {
        Clip {
            name: self.name,
            color: self.color,
            slot: self.slot,
            launch_quantize: self.launch_quantize,
            content: match self.content {
                ClipHead::Midi { length, looping } => ClipContent::Midi {
                    length,
                    looping,
                    notes,
                },
                ClipHead::Audio {
                    media,
                    start,
                    frames,
                    gain_db,
                    looping,
                } => ClipContent::Audio {
                    media,
                    start,
                    frames,
                    gain_db,
                    looping,
                },
            },
        }
    }
}
