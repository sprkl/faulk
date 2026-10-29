//! Saving: the dirty-file writer (§ 2.4, § 7). Only the files whose canonical bytes
//! changed are rewritten; the files of an entity that is gone are removed.
//!
//! Order within a save, so that an interrupted one leaves a folder that opens or that
//! the validation names precisely: blobs first, then the files of each unit (a unit
//! writes what the others refer to before them: devices and clips before the arrangement
//! and the lanes that point at them), `project.json` last, removals after everything.

use super::cache;
use super::canon::fast_hash;
use super::error::FileError;
use super::fs::FileSystem;
use super::unit::{EmitCtx, GITATTRIBUTES_FILE, GITIGNORE_FILE, Layout, Out, Unit, emit};
use crate::blob::Hash;
use crate::ids::{LaneId, TrackId};
use crate::model::Project;
use std::collections::{BTreeMap, BTreeSet};

/// What `.gitattributes` holds (§ 3.2). Written when absent, never overwritten.
pub const GITATTRIBUTES: &str = "blobs/** filter=lfs diff=lfs merge=lfs -text\n*.json text eol=lf merge=faulk diff=faulk\n*.tsv text eol=lf merge=faulk diff=faulk\n";
/// What `.gitignore` holds: the cache, the session and the temporary files stay out of git.
pub const GITIGNORE: &str = ".faulk/\n";

/// The units a save must look at: what the commands marked since the last save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dirty {
    units: BTreeSet<Unit>,
}

impl Dirty {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark(&mut self, unit: Unit) {
        self.units.insert(unit);
    }

    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    pub fn units(&self) -> impl Iterator<Item = &Unit> {
        self.units.iter()
    }

    pub fn clear(&mut self) {
        self.units.clear();
    }
}

/// What a save remembers of the folder: the hash of every file as last read or written
/// (the lazy write skips a file whose bytes did not change) and which lanes are blobs
/// (the hysteresis of [`Layout`]). Made by [`super::open`] or [`SaveState::new`].
///
/// It cannot see an edit made outside Faulk since: reopen the folder to take it in.
#[derive(Clone, Debug, Default)]
pub struct SaveState {
    pub(crate) known: BTreeMap<String, u64>,
    /// The (size, stamp) each file had when it was last read or written: what the cache
    /// snapshot may vouch for without reading the file again.
    pub(crate) stamps: BTreeMap<String, (u64, u64)>,
    pub(crate) blob_lanes: BTreeMap<(Option<TrackId>, LaneId), Hash>,
    support_checked: bool,
}

/// Two states are equal when they know the same files and the same blob lanes.
impl PartialEq for SaveState {
    fn eq(&self, other: &Self) -> bool {
        self.known == other.known && self.blob_lanes == other.blob_lanes
    }
}

impl Eq for SaveState {}

impl SaveState {
    /// The state of a folder that has never been written: every file is new.
    pub fn new() -> Self {
        Self::default()
    }

    /// The files of the project the folder is known to hold.
    pub fn known_files(&self) -> impl Iterator<Item = &str> {
        self.known.keys().map(String::as_str)
    }

    /// Whether a lane is stored as a blob.
    pub fn is_blob_lane(&self, track: &TrackId, lane: &LaneId) -> bool {
        self.blob_lanes
            .contains_key(&(Some(track.clone()), lane.clone()))
    }

    /// Whether a lane of the master is stored as a blob.
    pub fn is_blob_master_lane(&self, lane: &LaneId) -> bool {
        self.blob_lanes.contains_key(&(None, lane.clone()))
    }
}

/// How to save.
#[derive(Clone, Copy, Debug)]
pub struct SaveOptions {
    pub layout: Layout,
    /// Writes the cache snapshot (`.faulk/cache/`) after the files: the browser build,
    /// where opening thousands of small files is the cost (QA-7). Not in native.
    pub write_cache: bool,
    /// Writes `.gitattributes` and `.gitignore` when they are absent.
    pub support_files: bool,
}

impl Default for SaveOptions {
    fn default() -> Self {
        Self {
            layout: Layout::QA7,
            write_cache: false,
            support_files: true,
        }
    }
}

/// What a save did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveReport {
    /// The text files rewritten.
    pub written: Vec<String>,
    /// The files removed (of a track, a clip, a lane that is gone).
    pub removed: Vec<String>,
    /// The blobs written (the ones already there are not).
    pub blobs: Vec<String>,
    /// The text files looked at and found unchanged.
    pub unchanged: usize,
    /// The cache snapshot was written.
    pub cache_written: bool,
}

fn io(path: &str) -> impl Fn(super::fs::FsError) -> FileError + '_ {
    move |e| FileError::io(path, e)
}

/// Saves `project` into `fs`. With `dirty`, only those units are looked at; without, the
/// whole project is, and any file the state knows that the project no longer has is
/// removed. Blobs are never removed (§ 3.4).
///
/// On an error the files written so far stay (and are remembered by `state`): saving again
/// resumes.
pub fn save(
    fs: &mut dyn FileSystem,
    project: &Project,
    state: &mut SaveState,
    dirty: Option<&Dirty>,
    options: &SaveOptions,
) -> Result<SaveReport, FileError> {
    let mut report = SaveReport::default();
    let mut units: Vec<Unit> = match dirty {
        Some(d) => d.units.iter().cloned().collect(),
        None => project
            .tracks
            .keys()
            .cloned()
            .map(Unit::Track)
            .chain(
                project
                    .master_automation
                    .keys()
                    .cloned()
                    .map(Unit::MasterLane),
            )
            .collect(),
    };
    // `project.json` last.
    units.retain(|u| *u != Unit::Project);
    if dirty.is_none_or(|d| d.units.contains(&Unit::Project)) {
        units.push(Unit::Project);
    }

    // What the cache needs of the files this save emitted: path -> (hash, payload).
    let mut cache_entries: BTreeMap<String, (u64, Vec<u8>)> = BTreeMap::new();
    let mut emitted_all: BTreeSet<String> = BTreeSet::new();
    let mut to_remove: BTreeSet<String> = BTreeSet::new();

    for unit in &units {
        let mut emitted: BTreeSet<String> = BTreeSet::new();
        let mut ctx = EmitCtx {
            layout: options.layout,
            blob_lanes: &mut state.blob_lanes,
            with_payload: options.write_cache,
        };
        let known = &mut state.known;
        let mut sink = |out: Out| -> Result<(), FileError> {
            if out.blob {
                if !fs.exists(&out.path) {
                    fs.write(&out.path, &out.bytes).map_err(io(&out.path))?;
                    report.blobs.push(out.path);
                }
                return Ok(());
            }
            let hash = fast_hash(&out.bytes);
            if known.get(&out.path) == Some(&hash) {
                report.unchanged += 1;
            } else {
                fs.write(&out.path, &out.bytes).map_err(io(&out.path))?;
                known.insert(out.path.clone(), hash);
                report.written.push(out.path.clone());
            }
            if options.write_cache {
                let payload = out.payload.unwrap_or(out.bytes);
                cache_entries.insert(out.path.clone(), (hash, payload));
            }
            emitted.insert(out.path);
            Ok(())
        };
        emit(project, unit, &mut ctx, &mut sink)?;
        // The files this unit owns that it did not emit any more.
        if dirty.is_some() {
            for path in state.known.keys() {
                if unit.owns(path) && !emitted.contains(path) {
                    to_remove.insert(path.clone());
                }
            }
            if let Unit::Track(t) = unit
                && !project.tracks.contains_key(t)
            {
                state.blob_lanes.retain(|(tt, _), _| tt.as_ref() != Some(t));
            }
        }
        emitted_all.extend(emitted);
    }
    if dirty.is_none() {
        for path in state.known.keys() {
            if !emitted_all.contains(path) {
                to_remove.insert(path.clone());
            }
        }
        state.blob_lanes.retain(|(t, l), _| {
            project
                .lanes(t.as_ref())
                .is_some_and(|lanes| lanes.contains_key(l))
        });
    }

    for path in to_remove {
        fs.remove(&path).map_err(io(&path))?;
        state.known.remove(&path);
        report.removed.push(path);
    }

    if options.support_files && !state.support_checked {
        for (path, content) in [
            (GITATTRIBUTES_FILE, GITATTRIBUTES),
            (GITIGNORE_FILE, GITIGNORE),
        ] {
            if !fs.exists(path) {
                fs.write(path, content.as_bytes()).map_err(io(path))?;
                report.written.push(path.to_owned());
            }
        }
        state.support_checked = true;
    }

    if options.write_cache {
        report.cache_written =
            cache::write(fs, project, state, options, cache_entries, &report.written)?;
    }
    Ok(report)
}
