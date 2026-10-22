//! The files of a project and which entity each one holds: the paths of § 2.2, the
//! dirty-tracking units, and the canonical bytes each unit produces.

use super::canon;
use super::dto::{ClipDoc, ProjectDoc, TrackDoc};
use super::error::FileError;
use super::text;
use crate::blob::Hash;
use crate::ids::{ClipId, DeviceId, LaneId, TrackId};
use crate::model::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PROJECT_FILE: &str = "project.json";
pub const SESSION_FILE: &str = ".faulk/session.json";
pub const SNAPSHOT_FILE: &str = ".faulk/cache/project.snap";
pub const GITATTRIBUTES_FILE: &str = ".gitattributes";
pub const GITIGNORE_FILE: &str = ".gitignore";

pub fn track_file(t: &TrackId) -> String {
    format!("tracks/{t}/track.json")
}

pub fn device_file(t: &TrackId, d: &DeviceId) -> String {
    format!("tracks/{t}/devices/{d}.json")
}

pub fn clip_file(t: &TrackId, c: &ClipId) -> String {
    format!("tracks/{t}/clips/{c}.json")
}

pub fn notes_file(t: &TrackId, c: &ClipId) -> String {
    format!("tracks/{t}/clips/{c}.notes.tsv")
}

pub fn arrangement_file(t: &TrackId) -> String {
    format!("tracks/{t}/arrangement.tsv")
}

pub fn lane_file(t: &TrackId, l: &LaneId) -> String {
    format!("tracks/{t}/automation/{l}.tsv")
}

pub fn blob_file(h: &Hash) -> String {
    format!("blobs/{h}")
}

/// What the dirty tracking names: the set of files one entity of the project owns. A
/// command that changes an entity marks the unit of the file that holds the change; a
/// unit whose entity is gone removes its files.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Unit {
    /// `project.json`: name, tempo, meter, transport, scenes, master.
    Project,
    /// Everything of a track: its header, devices, clips, notes, arrangement, lanes.
    Track(TrackId),
    /// `track.json` only.
    TrackHeader(TrackId),
    Device(TrackId, DeviceId),
    /// A clip's header and its notes.
    Clip(TrackId, ClipId),
    /// A MIDI clip's notes only: editing a note rewrites this one file.
    Notes(TrackId, ClipId),
    Arrangement(TrackId),
    Lane(TrackId, LaneId),
}

impl Unit {
    /// Whether `path` is a file of this unit (its files, present or not).
    pub fn owns(&self, path: &str) -> bool {
        match self {
            Unit::Project => path == PROJECT_FILE,
            Unit::Track(t) => path
                .strip_prefix("tracks/")
                .and_then(|p| p.strip_prefix(t.0.as_str()))
                .is_some_and(|p| p.starts_with('/')),
            Unit::TrackHeader(t) => path == track_file(t),
            Unit::Device(t, d) => path == device_file(t, d),
            Unit::Clip(t, c) => path == clip_file(t, c) || path == notes_file(t, c),
            Unit::Notes(t, c) => path == notes_file(t, c),
            Unit::Arrangement(t) => path == arrangement_file(t),
            Unit::Lane(t, l) => path == lane_file(t, l),
        }
    }
}

/// When a lane goes to a blob (QA-7, `docs/spikes/QA-7-banc-de-charge-format-projet.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// A lane with more points than this is written as a blob.
    pub blob_above: usize,
    /// A lane with fewer points than this is written as text. Between the two it keeps
    /// the form it had (hysteresis: a lane that hovers around the threshold does not
    /// flip between text and a new blob at every save).
    pub text_below: usize,
}

impl Layout {
    /// The thresholds QA-7 measured and SV-1 confirms.
    pub const QA7: Layout = Layout {
        blob_above: 100_000,
        text_below: 50_000,
    };

    pub fn wants_blob(&self, points: usize, was_blob: bool) -> bool {
        if points > self.blob_above {
            true
        } else if points < self.text_below {
            false
        } else {
            was_blob
        }
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self::QA7
    }
}

/// A file the writer produces.
pub struct Out {
    pub path: String,
    pub bytes: Vec<u8>,
    /// A content-addressed blob: written once, never rewritten, not tracked.
    pub blob: bool,
    /// What the cache snapshot stores for this file instead of its text, when it is
    /// asked for (packed notes, packed lane); `None` means the text itself.
    pub payload: Option<Vec<u8>>,
}

impl Out {
    fn text(path: String, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            blob: false,
            payload: None,
        }
    }
}

/// What emitting needs from the saver.
pub struct EmitCtx<'a> {
    pub layout: Layout,
    /// The lanes written as blobs: read for the hysteresis and kept up to date.
    pub blob_lanes: &'a mut BTreeMap<(TrackId, LaneId), Hash>,
    pub with_payload: bool,
}

fn json<T: serde::Serialize>(path: &str, value: &T) -> Result<Vec<u8>, FileError> {
    canon::json(value).map_err(|message| FileError::Unwritable {
        path: path.to_owned(),
        message,
    })
}

pub fn sha256(bytes: &[u8]) -> Hash {
    Hash(Sha256::digest(bytes).into())
}

/// The packed notes of a clip, as the cache snapshot stores them: a count, then 21 bytes a
/// note.
pub fn notes_payload(notes: &BTreeSet<Note>) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + notes.len() * 21);
    out.extend_from_slice(&(notes.len() as u32).to_le_bytes());
    for n in notes {
        out.extend_from_slice(&n.start.get().to_le_bytes());
        out.extend_from_slice(&n.length.get().to_le_bytes());
        out.extend_from_slice(&[n.pitch, n.velocity, n.cents as u8, n.chance, n.repeat]);
    }
    out
}

pub fn parse_notes_payload(bytes: &[u8]) -> Result<BTreeSet<Note>, String> {
    let (count, rest) = bytes.split_at_checked(4).ok_or("truncated")?;
    let count = u32::from_le_bytes([count[0], count[1], count[2], count[3]]) as usize;
    let (records, tail) = rest.as_chunks::<21>();
    if records.len() != count || !tail.is_empty() {
        return Err("the note count does not match".into());
    }
    let mut out = BTreeSet::new();
    for r in records {
        let le = |i: usize| {
            i64::from_le_bytes([
                r[i],
                r[i + 1],
                r[i + 2],
                r[i + 3],
                r[i + 4],
                r[i + 5],
                r[i + 6],
                r[i + 7],
            ])
        };
        out.insert(Note {
            start: engine::time::Tick(le(0)),
            length: engine::time::Tick(le(8)),
            pitch: r[16],
            velocity: r[17],
            cents: r[18] as i8,
            chance: r[19],
            repeat: r[20],
        });
    }
    if out.len() != count {
        return Err("a note is there twice".into());
    }
    Ok(out)
}

/// A lane as the cache snapshot stores it: whether it is a blob, its target, its blob
/// hash, then its points packed.
pub fn lane_payload(
    target: &AutomationTarget,
    blob: Option<&Hash>,
    points: &std::collections::BTreeMap<engine::time::Tick, AutomationPoint>,
) -> Result<Vec<u8>, String> {
    let target = text::target_text(target)?;
    let mut out = Vec::with_capacity(48 + target.len() + points.len() * text::POINT_BYTES);
    out.push(u8::from(blob.is_some()));
    out.extend_from_slice(&(target.len() as u16).to_le_bytes());
    out.extend_from_slice(target.as_bytes());
    if let Some(h) = blob {
        out.extend_from_slice(&h.0);
    }
    out.extend_from_slice(&(points.len() as u32).to_le_bytes());
    text::pack_points(points, &mut out);
    Ok(out)
}

/// Emits the files of one unit, blobs before the text that names them. An entity that is
/// not in the project emits nothing.
pub fn emit(
    project: &Project,
    unit: &Unit,
    ctx: &mut EmitCtx<'_>,
    sink: &mut dyn FnMut(Out) -> Result<(), FileError>,
) -> Result<(), FileError> {
    match unit {
        Unit::Project => {
            sink(Out::text(
                PROJECT_FILE.to_owned(),
                json(PROJECT_FILE, &ProjectDoc::of(project))?,
            ))?;
        }
        Unit::Track(t) => {
            let Some(track) = project.tracks.get(t) else {
                return Ok(());
            };
            sink(Out::text(
                track_file(t),
                json(&track_file(t), &TrackDoc::of(track))?,
            ))?;
            for d in track.devices.keys() {
                emit(project, &Unit::Device(t.clone(), d.clone()), ctx, sink)?;
            }
            for c in track.clips.keys() {
                emit(project, &Unit::Clip(t.clone(), c.clone()), ctx, sink)?;
            }
            emit(project, &Unit::Arrangement(t.clone()), ctx, sink)?;
            for l in track.automation.keys() {
                emit(project, &Unit::Lane(t.clone(), l.clone()), ctx, sink)?;
            }
        }
        Unit::TrackHeader(t) => {
            if let Some(track) = project.tracks.get(t) {
                sink(Out::text(
                    track_file(t),
                    json(&track_file(t), &TrackDoc::of(track))?,
                ))?;
            }
        }
        Unit::Device(t, d) => {
            if let Some(device) = project.tracks.get(t).and_then(|t| t.devices.get(d)) {
                sink(Out::text(
                    device_file(t, d),
                    json(&device_file(t, d), device)?,
                ))?;
            }
        }
        Unit::Clip(t, c) => {
            if let Some(clip) = project.tracks.get(t).and_then(|t| t.clips.get(c)) {
                sink(Out::text(
                    clip_file(t, c),
                    json(&clip_file(t, c), &ClipDoc::of(clip))?,
                ))?;
                emit(project, &Unit::Notes(t.clone(), c.clone()), ctx, sink)?;
            }
        }
        Unit::Notes(t, c) => {
            if let Some(ClipContent::Midi { notes, .. }) = project
                .tracks
                .get(t)
                .and_then(|t| t.clips.get(c))
                .map(|c| &c.content)
            {
                let mut out = Out::text(notes_file(t, c), text::notes_tsv(notes));
                if ctx.with_payload {
                    out.payload = Some(notes_payload(notes));
                }
                sink(out)?;
            }
        }
        Unit::Arrangement(t) => {
            if let Some(track) = project.tracks.get(t)
                && !track.placements.is_empty()
            {
                sink(Out::text(
                    arrangement_file(t),
                    text::arrangement_tsv(&track.placements),
                ))?;
            }
        }
        Unit::Lane(t, l) => {
            let Some(lane) = project.tracks.get(t).and_then(|t| t.automation.get(l)) else {
                ctx.blob_lanes.remove(&(t.clone(), l.clone()));
                return Ok(());
            };
            let path = lane_file(t, l);
            let unwritable = |message: String| FileError::Unwritable {
                path: path.clone(),
                message,
            };
            let key = (t.clone(), l.clone());
            let blob_mode = ctx
                .layout
                .wants_blob(lane.points.len(), ctx.blob_lanes.contains_key(&key));
            if blob_mode {
                let bytes = text::lane_blob(&lane.points);
                let hash = sha256(&bytes);
                sink(Out {
                    path: blob_file(&hash),
                    bytes,
                    blob: true,
                    payload: None,
                })?;
                let mut out = Out::text(
                    path.clone(),
                    text::lane_blob_header(&lane.target, &hash, lane.points.len())
                        .map_err(unwritable)?,
                );
                if ctx.with_payload {
                    out.payload = Some(
                        lane_payload(&lane.target, Some(&hash), &lane.points).map_err(|m| {
                            FileError::Unwritable {
                                path: path.clone(),
                                message: m,
                            }
                        })?,
                    );
                }
                ctx.blob_lanes.insert(key, hash);
                sink(out)?;
            } else {
                let mut out = Out::text(
                    path.clone(),
                    text::lane_tsv(&lane.target, &lane.points).map_err(unwritable)?,
                );
                if ctx.with_payload {
                    out.payload =
                        Some(lane_payload(&lane.target, None, &lane.points).map_err(|m| {
                            FileError::Unwritable {
                                path: path.clone(),
                                message: m,
                            }
                        })?);
                }
                ctx.blob_lanes.remove(&key);
                sink(out)?;
            }
        }
    }
    Ok(())
}

/// Reads what [`lane_payload`] wrote.
pub fn parse_lane_payload(
    bytes: &[u8],
) -> Result<
    (
        text::LaneHeader,
        BTreeMap<engine::time::Tick, AutomationPoint>,
    ),
    String,
> {
    let mode = *bytes.first().ok_or("truncated")?;
    let len_bytes = bytes.get(1..3).ok_or("truncated")?;
    let len = usize::from(u16::from_le_bytes([len_bytes[0], len_bytes[1]]));
    let target_end = 3usize.checked_add(len).ok_or("truncated")?;
    let target = std::str::from_utf8(bytes.get(3..target_end).ok_or("truncated")?)
        .map_err(|e| e.to_string())?;
    let target = text::parse_target_text(target).map_err(|e| e.to_string())?;
    let mut at = target_end;
    let blob = if mode == 1 {
        let h = bytes.get(at..at + 32).ok_or("truncated")?;
        at += 32;
        let mut a = [0u8; 32];
        a.copy_from_slice(h);
        Some(Hash(a))
    } else if mode == 0 {
        None
    } else {
        return Err("bad mode".into());
    };
    let n = bytes.get(at..at + 4).ok_or("truncated")?;
    let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
    let points = text::unpack_points(bytes.get(at + 4..).ok_or("truncated")?)?;
    if points.len() != n {
        return Err("the point count does not match".into());
    }
    Ok((
        text::LaneHeader {
            target,
            blob: blob.map(|h| (h, n)),
        },
        points,
    ))
}
