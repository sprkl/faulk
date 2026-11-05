//! Opening: the canonical reader (§ 2.2, § 6.3). It lists the folder, reads each file
//! (from the cache snapshot when the file did not change), builds the project, and refuses
//! with a typed error whatever is not a valid project of this format.
//!
//! Reading never panics and never allocates more than what the files hold: sizes are
//! checked before a read, counts while parsing, and the result is validated
//! ([`Project::validate`]) before it is returned.

use super::MAX_TEXT_FILE_BYTES;
use super::cache::{self, CacheUse, Snapshot};
use super::canon::fast_hash;
use super::dto::{ClipDoc, ProjectDoc, TrackDoc};
use super::error::FileError;
use super::fs::{DirEntry, FileSystem, FsError, MemFs};
use super::migrate::{MIGRATIONS, Migration, migrate};
use super::text;
use super::unit::{PROJECT_FILE, blob_file, parse_lane_payload, parse_notes_payload, sha256};
use super::write::SaveState;
use crate::blob::Hash;
use crate::ids::{ClipId, DeviceId, LaneId, TrackId};
use crate::limits::*;
use crate::model::*;
use crate::preset::{MAX_PRESET_BYTES, MAX_PROJECT_PRESETS, Preset, PresetId};
use std::collections::{BTreeMap, BTreeSet};

/// How to open.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenOptions {
    /// Uses the cache snapshot when there is a current one (the browser build, § 7). Not
    /// in native, where it gains nothing (QA-7).
    pub use_cache: bool,
}

/// What opening found besides the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenReport {
    /// The format the folder was in, when it was migrated in memory: the host saves it
    /// as a version of its own (§ 2.6).
    pub migrated_from: Option<u32>,
    /// Files and folders in the structural folders that are not part of the format
    /// (a `.DS_Store`, an orphan notes file): left alone, never deleted.
    pub ignored: Vec<String>,
    /// Blobs the project refers to (audio, instrument state, modules) that the folder
    /// does not have: the track opens and shows "média absent" (§ 3.3).
    pub missing_blobs: Vec<Hash>,
    pub cache: CacheUse,
    /// MIDI clips the folder holds without a name, named in memory as they were read
    /// ([`Project::name_unnamed_clips`]). Their files on disk stay as they are until a
    /// save rewrites them (the save compares every file with what was read), and the
    /// project is not dirty: opening is not an edit.
    pub named_clips: usize,
    /// Tracks the folder holds without a colour, given the hue of their position as they
    /// were read ([`Project::color_uncolored_tracks`]); same rules as `named_clips`.
    pub colored_tracks: usize,
    /// Text files read from the folder.
    pub files_read: usize,
    /// Files taken from the cache snapshot instead.
    pub files_from_cache: usize,
}

/// A project read from a folder.
#[derive(Debug)]
pub struct Opened {
    pub project: Project,
    /// What a later save needs: the hash of each file as read, and the lanes that are
    /// blobs. Belongs to this folder: another destination starts from
    /// [`SaveState::new`].
    pub state: SaveState,
    pub report: OpenReport,
}

fn fs_err(path: &str, e: FsError) -> FileError {
    match e {
        FsError::NotFound => FileError::Missing {
            path: path.to_owned(),
        },
        FsError::Other(m) => FileError::io(path, m),
    }
}

fn json<T: serde::de::DeserializeOwned>(path: &str, bytes: &[u8]) -> Result<T, FileError> {
    serde_json::from_slice(bytes).map_err(|e| FileError::corrupt(path, e))
}

/// Opens the project in `fs`.
pub fn open(fs: &dyn FileSystem, options: &OpenOptions) -> Result<Opened, FileError> {
    open_with(fs, options, MIGRATIONS)
}

/// [`open`] with another table of migrations (the tests').
pub fn open_with(
    fs: &dyn FileSystem,
    options: &OpenOptions,
    table: &[Migration],
) -> Result<Opened, FileError> {
    let root = fs.list("").map_err(|e| fs_err("", e))?;
    let entry = root
        .iter()
        .find(|e| e.name == PROJECT_FILE && !e.is_dir)
        .ok_or(FileError::NotAProject)?;
    check_size(PROJECT_FILE, entry.size)?;
    let bytes = fs.read(PROJECT_FILE).map_err(|e| fs_err(PROJECT_FILE, e))?;
    check_size(PROJECT_FILE, bytes.len() as u64)?;
    let probe: FormatProbe = json(PROJECT_FILE, &bytes)?;
    if probe.format > FORMAT_VERSION {
        return Err(FileError::NewerFormat {
            found: probe.format,
            supported: FORMAT_VERSION,
        });
    }
    if probe.format < FORMAT_VERSION {
        return open_migrated(fs, table, probe.format);
    }
    match read_folder(fs, &bytes, options) {
        // A snapshot that checks out but does not decode, or lies about a file, is no
        // reason to refuse a folder whose text is fine: read the text.
        Err(_) if options.use_cache => read_folder(fs, &bytes, &OpenOptions { use_cache: false }),
        result => result,
    }
}

fn check_size(path: &str, size: u64) -> Result<(), FileError> {
    if size > MAX_TEXT_FILE_BYTES {
        Err(FileError::TooLarge {
            path: path.to_owned(),
            size,
            max: MAX_TEXT_FILE_BYTES,
        })
    } else {
        Ok(())
    }
}

/// A folder of older format: its text files are copied into memory, migrated there, and
/// read from the copy. The state remembers the bytes that are on disk, so that saving
/// rewrites every file the migration changed.
fn open_migrated(
    fs: &dyn FileSystem,
    table: &[Migration],
    found: u32,
) -> Result<Opened, FileError> {
    let mut mem = MemFs::new();
    let mut on_disk = BTreeMap::new();
    for (path, entry) in project_files(fs).map_err(|e| fs_err("", e))? {
        check_size(&path, entry.size)?;
        let bytes = fs.read(&path).map_err(|e| fs_err(&path, e))?;
        check_size(&path, bytes.len() as u64)?;
        on_disk.insert(path.clone(), fast_hash(&bytes));
        mem.insert(path, bytes);
    }
    migrate(table, found, FORMAT_VERSION, &mut mem)?;
    let bytes = mem
        .read(PROJECT_FILE)
        .map_err(|e| fs_err(PROJECT_FILE, e))?;
    let view = Overlay { mem, base: fs };
    let mut opened = read_folder(&view, &bytes, &OpenOptions::default())?;
    opened.state.known = on_disk;
    opened.report.migrated_from = Some(found);
    Ok(opened)
}

/// Whether a path of a folder is one of the project's text files (as opposed to a blob, the
/// session, git's files, or anything else): every component is the name the format gives.
pub(crate) fn text_path(path: &str) -> bool {
    if path == PROJECT_FILE {
        return true;
    }
    if let Some(f) = path.strip_prefix("master/automation/") {
        return f
            .strip_suffix(".tsv")
            .is_some_and(|id| LaneId::parse(id).is_ok());
    }
    if let Some(f) = path.strip_prefix("presets/") {
        return f
            .strip_suffix(".json")
            .is_some_and(|id| PresetId::parse(id).is_ok());
    }
    let mut parts = path.split('/');
    if parts.next() != Some("tracks") || TrackId::parse(parts.next().unwrap_or("")).is_err() {
        return false;
    }
    let (a, b) = (parts.next(), parts.next());
    if parts.next().is_some() {
        return false;
    }
    match (a, b) {
        (Some("track.json" | "arrangement.tsv"), None) => true,
        (Some("devices"), Some(f)) => f
            .strip_suffix(".json")
            .is_some_and(|id| DeviceId::parse(id).is_ok()),
        (Some("clips"), Some(f)) => f
            .strip_suffix(".json")
            .or_else(|| f.strip_suffix(".notes.tsv"))
            .is_some_and(|id| ClipId::parse(id).is_ok()),
        (Some("automation"), Some(f)) => f
            .strip_suffix(".tsv")
            .is_some_and(|id| LaneId::parse(id).is_ok()),
        _ => false,
    }
}

/// The project's own text files with their size and stamp, found by listing the folders the
/// format has (the root, `tracks/`, and each track's four): nothing else is entered, so a
/// sample library the user keeps in the folder costs nothing and cannot break a save.
pub(crate) fn project_files(fs: &dyn FileSystem) -> Result<Vec<(String, DirEntry)>, FsError> {
    let mut out = Vec::new();
    for e in fs.list("")? {
        if !e.is_dir && e.name == PROJECT_FILE {
            out.push((e.name.clone(), e));
        }
    }
    for t in fs.list("tracks")? {
        if !t.is_dir || TrackId::parse(&t.name).is_err() {
            continue;
        }
        let base = format!("tracks/{}", t.name);
        for e in fs.list(&base)? {
            let path = format!("{base}/{}", e.name);
            if e.is_dir && matches!(e.name.as_str(), "devices" | "clips" | "automation") {
                for f in fs.list(&path)? {
                    let file = format!("{path}/{}", f.name);
                    if !f.is_dir && text_path(&file) {
                        out.push((file, f));
                    }
                }
            } else if !e.is_dir && text_path(&path) {
                out.push((path, e));
            }
        }
    }
    for f in fs.list("master/automation")? {
        let file = format!("master/automation/{}", f.name);
        if !f.is_dir && text_path(&file) {
            out.push((file, f));
        }
    }
    for f in fs.list("presets")? {
        let file = format!("presets/{}", f.name);
        if !f.is_dir && text_path(&file) {
            out.push((file, f));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The migrated text files over the real folder: text from memory, blobs from the folder.
struct Overlay<'a> {
    mem: MemFs,
    base: &'a dyn FileSystem,
}

impl FileSystem for Overlay<'_> {
    fn read(&self, path: &str) -> Result<Vec<u8>, FsError> {
        if path.starts_with("blobs/") {
            self.base.read(path)
        } else {
            self.mem.read(path)
        }
    }

    fn write(&mut self, _: &str, _: &[u8]) -> Result<(), FsError> {
        Err(FsError::Other("read-only".into()))
    }

    fn remove(&mut self, _: &str) -> Result<(), FsError> {
        Err(FsError::Other("read-only".into()))
    }

    fn exists(&self, path: &str) -> bool {
        if path.starts_with("blobs/") {
            self.base.exists(path)
        } else {
            self.mem.exists(path)
        }
    }

    fn list(&self, dir: &str) -> Result<Vec<DirEntry>, FsError> {
        if dir == "blobs" {
            self.base.list(dir)
        } else {
            self.mem.list(dir)
        }
    }
}

struct Reader<'a> {
    fs: &'a dyn FileSystem,
    snapshot: Option<Snapshot>,
    known: BTreeMap<String, u64>,
    stamps: BTreeMap<String, (u64, u64)>,
    blob_lanes: BTreeMap<(Option<TrackId>, LaneId), Hash>,
    blob_sizes: Option<BTreeMap<String, u64>>,
    report: OpenReport,
}

impl Reader<'_> {
    /// A text file's bytes: the snapshot's payload when the file is as the snapshot saw
    /// it, else the file. `true` when it came from the snapshot.
    fn fetch(&mut self, path: &str, entry: &DirEntry) -> Result<(Vec<u8>, bool), FileError> {
        check_size(path, entry.size)?;
        self.stamps
            .insert(path.to_owned(), (entry.size, entry.stamp));
        if let Some(snap) = &self.snapshot
            && let Some(e) = snap.entries.get(path)
            && e.size == entry.size
            && e.stamp == entry.stamp
        {
            self.known.insert(path.to_owned(), e.hash);
            self.report.files_from_cache += 1;
            return Ok((snap.payload(e).to_vec(), true));
        }
        let bytes = self.fs.read(path).map_err(|e| fs_err(path, e))?;
        check_size(path, bytes.len() as u64)?;
        self.known.insert(path.to_owned(), fast_hash(&bytes));
        self.report.files_read += 1;
        Ok((bytes, false))
    }

    fn list(&self, dir: &str) -> Result<Vec<DirEntry>, FileError> {
        let mut v = self.fs.list(dir).map_err(|e| fs_err(dir, e))?;
        v.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(v)
    }

    fn ignore(&mut self, path: String) {
        self.report.ignored.push(path);
    }

    fn lane_blob_size(&mut self, hash: &Hash) -> Result<Option<u64>, FileError> {
        if self.blob_sizes.is_none() {
            let sizes = self
                .list("blobs")?
                .into_iter()
                .filter(|e| !e.is_dir)
                .map(|e| (e.name, e.size))
                .collect();
            self.blob_sizes = Some(sizes);
        }
        Ok(self
            .blob_sizes
            .as_ref()
            .and_then(|m| m.get(&hash.to_string()).copied()))
    }

    fn read_lane(
        &mut self,
        owner: Option<&TrackId>,
        id: &LaneId,
        path: &str,
        entry: &DirEntry,
    ) -> Result<AutomationLane, FileError> {
        let (bytes, cached) = self.fetch(path, entry)?;
        let (header, mut points) = if cached {
            parse_lane_payload(&bytes).map_err(|m| FileError::corrupt(path, m))?
        } else {
            let text = std::str::from_utf8(&bytes).map_err(|e| FileError::corrupt(path, e))?;
            text::parse_lane(path, text)?
        };
        if let Some((hash, n)) = header.blob {
            if !cached {
                let blob_path = blob_file(&hash);
                let size = self
                    .lane_blob_size(&hash)?
                    .ok_or_else(|| FileError::Missing {
                        path: blob_path.clone(),
                    })?;
                let max = super::MAX_LANE_BLOB_BYTES;
                if size > max {
                    return Err(FileError::TooLarge {
                        path: blob_path,
                        size,
                        max,
                    });
                }
                let blob = self
                    .fs
                    .read(&blob_path)
                    .map_err(|e| fs_err(&blob_path, e))?;
                if blob.len() as u64 > max {
                    return Err(FileError::TooLarge {
                        path: blob_path,
                        size: blob.len() as u64,
                        max,
                    });
                }
                if sha256(&blob) != hash {
                    return Err(FileError::corrupt(
                        &blob_path,
                        "the content is not its hash",
                    ));
                }
                points =
                    text::parse_lane_blob(&blob).map_err(|m| FileError::corrupt(&blob_path, m))?;
            }
            if points.len() != n {
                return Err(FileError::corrupt(
                    path,
                    format!("{n} points announced, {} found", points.len()),
                ));
            }
            self.blob_lanes.insert((owner.cloned(), id.clone()), hash);
        }
        Ok(AutomationLane {
            target: header.target,
            points,
        })
    }

    /// The lanes of the master (#378): `master/automation/<lane>.tsv`. A folder without
    /// them is a project without master automation (every project written before). A lane
    /// whose device the master's chain no longer holds (a Faulk older than #378 removed the
    /// effect and left the lane's file) is left out and listed with the ignored files: it
    /// drives nothing, and the project opens instead of being refused.
    fn read_master_lanes(
        &mut self,
        devices: &BTreeMap<DeviceId, Device>,
    ) -> Result<BTreeMap<LaneId, AutomationLane>, FileError> {
        let mut lanes = BTreeMap::new();
        let dir = "master/automation";
        for e in self.list(dir)? {
            let path = format!("{dir}/{}", e.name);
            let Some(id) = e
                .name
                .strip_suffix(".tsv")
                .and_then(|s| LaneId::parse(s).ok())
                .filter(|_| !e.is_dir)
            else {
                self.ignore(path);
                continue;
            };
            if lanes.len() >= MAX_LANES_PER_TRACK {
                return Err(FileError::corrupt(
                    dir,
                    "more lanes than the master can hold",
                ));
            }
            let lane = self.read_lane(None, &id, &path, &e)?;
            if matches!(&lane.target, AutomationTarget::Device { device, .. } if !devices.contains_key(device))
            {
                self.blob_lanes.remove(&(None, id));
                self.ignore(path);
                continue;
            }
            lanes.insert(id, lane);
        }
        Ok(lanes)
    }

    /// The presets saved in the project (#191): `presets/<id>.json`, one preset file each
    /// (`crate::preset`). A folder without them is a project without presets. A file whose name
    /// is not a preset id is ignored; a preset file that does not read refuses the folder, as
    /// any other file of the project does.
    fn read_presets(&mut self) -> Result<BTreeMap<PresetId, Preset>, FileError> {
        let mut presets = BTreeMap::new();
        let dir = "presets";
        for e in self.list(dir)? {
            let path = format!("{dir}/{}", e.name);
            let Some(id) = e
                .name
                .strip_suffix(".json")
                .and_then(|s| PresetId::parse(s).ok())
                .filter(|_| !e.is_dir)
            else {
                self.ignore(path);
                continue;
            };
            if presets.len() >= MAX_PROJECT_PRESETS {
                return Err(FileError::corrupt(
                    dir,
                    "more presets than a project can hold",
                ));
            }
            if e.size > MAX_PRESET_BYTES as u64 {
                return Err(FileError::corrupt(
                    &path,
                    format!("a preset file has at most {MAX_PRESET_BYTES} bytes"),
                ));
            }
            let (bytes, _) = self.fetch(&path, &e)?;
            let preset =
                Preset::parse(&bytes).map_err(|err| FileError::corrupt(&path, err.to_string()))?;
            presets.insert(id, preset);
        }
        Ok(presets)
    }

    fn read_track(&mut self, tid: &TrackId, entries: Vec<DirEntry>) -> Result<Track, FileError> {
        let base = format!("tracks/{tid}");
        let by_name: BTreeMap<&str, &DirEntry> =
            entries.iter().map(|e| (e.name.as_str(), e)).collect();
        let header_path = format!("{base}/track.json");
        let header = by_name
            .get("track.json")
            .filter(|e| !e.is_dir)
            .ok_or_else(|| FileError::Missing {
                path: header_path.clone(),
            })?;
        let (bytes, _) = self.fetch(&header_path, header)?;
        let doc: TrackDoc = json(&header_path, &bytes)?;
        let mut track = doc.into_track();
        for e in &entries {
            let known = matches!(
                (e.name.as_str(), e.is_dir),
                ("track.json" | "arrangement.tsv", false)
                    | ("devices" | "clips" | "automation", true)
            );
            if !known {
                self.ignore(format!("{base}/{}", e.name));
            }
        }

        // Devices.
        if by_name.get("devices").is_some_and(|e| e.is_dir) {
            let dir = format!("{base}/devices");
            for e in self.list(&dir)? {
                let path = format!("{dir}/{}", e.name);
                let Some(id) = e
                    .name
                    .strip_suffix(".json")
                    .and_then(|s| DeviceId::parse(s).ok())
                    .filter(|_| !e.is_dir)
                else {
                    self.ignore(path);
                    continue;
                };
                if track.devices.len() >= MAX_DEVICES_PER_TRACK {
                    return Err(FileError::corrupt(
                        &dir,
                        "more devices than a track can hold",
                    ));
                }
                let (bytes, _) = self.fetch(&path, &e)?;
                track.devices.insert(id, json(&path, &bytes)?);
            }
        }

        // Clips: a header, and for a MIDI clip its notes.
        if by_name.get("clips").is_some_and(|e| e.is_dir) {
            let dir = format!("{base}/clips");
            let mut headers: BTreeMap<ClipId, DirEntry> = BTreeMap::new();
            let mut notes: BTreeMap<ClipId, DirEntry> = BTreeMap::new();
            for e in self.list(&dir)? {
                let path = format!("{dir}/{}", e.name);
                let parsed = if e.is_dir {
                    None
                } else if let Some(id) = e.name.strip_suffix(".notes.tsv") {
                    ClipId::parse(id).ok().map(|id| (id, true))
                } else {
                    e.name
                        .strip_suffix(".json")
                        .and_then(|s| ClipId::parse(s).ok())
                        .map(|id| (id, false))
                };
                match parsed {
                    Some((id, true)) => {
                        notes.insert(id, e);
                    }
                    Some((id, false)) => {
                        headers.insert(id, e);
                    }
                    None => self.ignore(path),
                }
            }
            if headers.len() > MAX_CLIPS_PER_TRACK {
                return Err(FileError::corrupt(&dir, "more clips than a track can hold"));
            }
            for (id, e) in &headers {
                let path = format!("{dir}/{id}.json");
                let (bytes, _) = self.fetch(&path, e)?;
                let doc: ClipDoc = json(&path, &bytes)?;
                let set = if doc.is_midi() {
                    let npath = format!("{dir}/{id}.notes.tsv");
                    let ne = notes.remove(id).ok_or_else(|| FileError::Missing {
                        path: npath.clone(),
                    })?;
                    let (bytes, cached) = self.fetch(&npath, &ne)?;
                    if cached {
                        parse_notes_payload(&bytes).map_err(|m| FileError::corrupt(&npath, m))?
                    } else {
                        let text = std::str::from_utf8(&bytes)
                            .map_err(|e| FileError::corrupt(&npath, e))?;
                        text::parse_notes(&npath, text)?
                    }
                } else {
                    BTreeSet::new()
                };
                track.clips.insert(id.clone(), doc.into_clip(set));
            }
            // Notes without their clip, or of an audio clip.
            for id in notes.keys() {
                self.ignore(format!("{dir}/{id}.notes.tsv"));
            }
        }

        if let Some(e) = by_name.get("arrangement.tsv").filter(|e| !e.is_dir) {
            let path = format!("{base}/arrangement.tsv");
            let (bytes, _) = self.fetch(&path, e)?;
            let text = std::str::from_utf8(&bytes).map_err(|e| FileError::corrupt(&path, e))?;
            track.placements = text::parse_arrangement(&path, text, track.kind)?;
        }

        if by_name.get("automation").is_some_and(|e| e.is_dir) {
            let dir = format!("{base}/automation");
            for e in self.list(&dir)? {
                let path = format!("{dir}/{}", e.name);
                let Some(id) = e
                    .name
                    .strip_suffix(".tsv")
                    .and_then(|s| LaneId::parse(s).ok())
                    .filter(|_| !e.is_dir)
                else {
                    self.ignore(path);
                    continue;
                };
                if track.automation.len() >= MAX_LANES_PER_TRACK {
                    return Err(FileError::corrupt(&dir, "more lanes than a track can hold"));
                }
                let lane = self.read_lane(Some(tid), &id, &path, &e)?;
                track.automation.insert(id, lane);
            }
        }
        Ok(track)
    }
}

/// The blobs a project refers to besides the lanes' own: audio, instrument states and the
/// compiled code of modules.
pub fn referenced_blobs(project: &Project) -> BTreeSet<Hash> {
    let mut out = BTreeSet::new();
    for device in project.master_devices.values() {
        if let Some(h) = device.state {
            out.insert(h);
        }
        if let DeviceSource::Module { hash, .. } = &device.source {
            out.insert(*hash);
        }
    }
    for track in project.tracks.values() {
        for device in track.devices.values() {
            if let Some(h) = device.state {
                out.insert(h);
            }
            if let DeviceSource::Module { hash, .. } = &device.source {
                out.insert(*hash);
            }
        }
        for clip in track.clips.values() {
            if let ClipContent::Audio { media, .. } = &clip.content {
                out.insert(media.hash);
            }
        }
    }
    out
}

fn read_folder(
    fs: &dyn FileSystem,
    project_bytes: &[u8],
    options: &OpenOptions,
) -> Result<Opened, FileError> {
    let (snapshot, cache) = if options.use_cache {
        match cache::load(fs) {
            Ok(s) => (Some(s), CacheUse::Used),
            Err(reason) => (None, reason),
        }
    } else {
        (None, CacheUse::NotUsed)
    };
    let mut reader = Reader {
        fs,
        snapshot,
        known: BTreeMap::new(),
        stamps: BTreeMap::new(),
        blob_lanes: BTreeMap::new(),
        blob_sizes: None,
        report: OpenReport {
            migrated_from: None,
            ignored: Vec::new(),
            missing_blobs: Vec::new(),
            cache,
            named_clips: 0,
            colored_tracks: 0,
            files_read: 1,
            files_from_cache: 0,
        },
    };
    reader
        .known
        .insert(PROJECT_FILE.to_owned(), fast_hash(project_bytes));
    let doc: ProjectDoc = json(PROJECT_FILE, project_bytes)?;
    if doc.format != FORMAT_VERSION {
        return Err(FileError::Migration {
            from: doc.format,
            message: "the migration left project.json in another format".into(),
        });
    }
    let mut project = doc.into_project();

    let root = reader.list("tracks")?;
    let mut tracks = Vec::new();
    for e in root {
        let path = format!("tracks/{}", e.name);
        match TrackId::parse(&e.name) {
            Ok(id) if e.is_dir => tracks.push(id),
            _ => reader.ignore(path),
        }
    }
    if tracks.len() > MAX_TRACKS {
        return Err(FileError::corrupt(
            "tracks",
            "more tracks than a project can hold",
        ));
    }
    for id in tracks {
        let dir = format!("tracks/{id}");
        let entries = reader.list(&dir)?;
        let has_track_files = entries.iter().any(|e| {
            matches!(
                (e.name.as_str(), e.is_dir),
                ("track.json" | "arrangement.tsv", false)
                    | ("devices" | "clips" | "automation", true)
            )
        });
        // A folder with nothing of a track in it (what a track's removal leaves when the
        // system dropped a `.DS_Store` there) is not a track.
        if !has_track_files {
            reader.ignore(dir);
            continue;
        }
        let track = reader.read_track(&id, entries)?;
        project.tracks.insert(id, track);
    }

    project.master_automation = reader.read_master_lanes(&project.master_devices)?;
    project.presets = reader.read_presets()?;

    project
        .validate()
        .map_err(|violations| FileError::Invalid { violations })?;
    reader.report.named_clips = project.name_unnamed_clips();
    reader.report.colored_tracks = project.color_uncolored_tracks();

    // Blobs the folder lacks: not read, only counted (§ 3.3).
    let present: BTreeSet<String> = reader
        .list("blobs")?
        .into_iter()
        .filter(|e| !e.is_dir)
        .map(|e| e.name)
        .collect();
    reader.report.missing_blobs = referenced_blobs(&project)
        .into_iter()
        .filter(|h| !present.contains(&h.to_string()))
        .collect();
    reader.report.ignored.sort();

    let mut state = SaveState::new();
    state.known = reader.known;
    state.stamps = reader.stamps;
    state.blob_lanes = reader.blob_lanes;
    Ok(Opened {
        project,
        state,
        report: reader.report,
    })
}
