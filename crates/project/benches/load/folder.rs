//! The prototype folder format of the load bench (QA-7).
//!
//! This is NOT the product's writer. It follows the layout of `docs/architecture/02-modele-projet.md`
//! § 2.2 to § 2.4 (JSON skeletons, TSV for notes, points and placements, packed binary
//! blobs, a binary cache snapshot) closely enough to measure what the format costs. SV-1
//! (#93) wrote the product's writer and reader, `project::file`, with its own tests and
//! its own cost measure (`tests/folder_load.rs`); this prototype stays as the instrument
//! the QA-7 numbers were taken with, so that they can be reproduced. It differs from the
//! product on purpose: its lane header holds JSON, its blob has no magic, it reads on
//! several threads. Pointing the bench at `project::file` is a possible follow-up.
//!
//! The reader works through the [`Files`] trait: the folder on disk ([`FsFiles`]) or a map
//! of bytes in memory ([`MemFiles`], what the browser build feeds from OPFS).

use project::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::Hasher;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// `std::time::Instant` panics on `wasm32-unknown-unknown`: there the stats read zero.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
struct Instant;

#[cfg(target_arch = "wasm32")]
impl Instant {
    fn now() -> Self {
        Self
    }

    fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

/// Points above which a lane is written as a blob; `usize::MAX` keeps every lane in text.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub blob_above: usize,
}

impl Layout {
    pub const TEXT: Layout = Layout {
        blob_above: usize::MAX,
    };

    pub fn blob_above(points: usize) -> Layout {
        Layout { blob_above: points }
    }
}

/// A file the writer produces: its path in the folder (forward slashes), its bytes, and
/// whether it is a content-addressed blob (written once, never modified).
pub struct Out {
    pub path: String,
    pub bytes: Vec<u8>,
    pub blob: bool,
}

// ------------------------------------------------------------------- text encoders

fn json(value: &impl serde::Serialize) -> Vec<u8> {
    // `Value` keeps the keys sorted: the canonical order of § 2.4.
    let mut v =
        serde_json::to_vec_pretty(&serde_json::to_value(value).expect("serialize")).expect("json");
    v.push(b'\n');
    v
}

fn skeleton_track(t: &Track) -> Track {
    Track {
        name: t.name.clone(),
        color: t.color,
        order: t.order.clone(),
        kind: t.kind,
        mixer: t.mixer,
        output: t.output,
        devices: BTreeMap::new(),
        clips: BTreeMap::new(),
        placements: BTreeMap::new(),
        automation: BTreeMap::new(),
    }
}

fn skeleton_clip(c: &Clip) -> Clip {
    Clip {
        name: c.name.clone(),
        color: c.color,
        slot: c.slot.clone(),
        launch_quantize: c.launch_quantize,
        content: match &c.content {
            ClipContent::Midi {
                length, looping, ..
            } => ClipContent::Midi {
                length: *length,
                looping: *looping,
                notes: BTreeSet::new(),
            },
            audio => audio.clone(),
        },
    }
}

/// The whole project with no notes and no points: the part of it that is small.
pub fn skeleton(project: &Project) -> Project {
    let mut p = project.clone_without_tracks();
    for (id, track) in &project.tracks {
        let mut t = track.clone_light();
        for (cid, clip) in &track.clips {
            t.clips.insert(cid.clone(), skeleton_clip(clip));
        }
        for (lid, lane) in &track.automation {
            t.automation.insert(
                lid.clone(),
                AutomationLane {
                    target: lane.target.clone(),
                    points: BTreeMap::new(),
                },
            );
        }
        p.tracks.insert(id.clone(), t);
    }
    p
}

/// What `skeleton` needs from the model without cloning the notes and points.
trait Light {
    fn clone_light(&self) -> Self;
}

impl Light for Track {
    fn clone_light(&self) -> Track {
        Track {
            devices: self.devices.clone(),
            placements: self.placements.clone(),
            ..skeleton_track(self)
        }
    }
}

trait WithoutTracks {
    fn clone_without_tracks(&self) -> Project;
}

impl WithoutTracks for Project {
    fn clone_without_tracks(&self) -> Project {
        Project {
            format: self.format,
            id: self.id.clone(),
            name: self.name.clone(),
            tempo: self.tempo.clone(),
            meter: self.meter.clone(),
            transport: self.transport,
            scenes: self.scenes.clone(),
            master: self.master,
            tracks: BTreeMap::new(),
        }
    }
}

pub fn notes_tsv(notes: &BTreeSet<Note>) -> Vec<u8> {
    let cents = notes.iter().any(|n| n.cents != 0);
    let chance = notes.iter().any(|n| n.chance != Note::DEFAULT_CHANCE);
    let repeat = notes.iter().any(|n| n.repeat != Note::DEFAULT_REPEAT);
    let mut s = String::with_capacity(32 + notes.len() * 24);
    s.push_str("# faulk-notes 1\tstart\tlength\tpitch\tvelocity");
    if cents {
        s.push_str("\tcents");
    }
    if chance {
        s.push_str("\tchance");
    }
    if repeat {
        s.push_str("\trepeat");
    }
    s.push('\n');
    for n in notes {
        let _ = write!(
            s,
            "{}\t{}\t{}\t{}",
            n.start.get(),
            n.length.get(),
            n.pitch,
            n.velocity
        );
        if cents {
            let _ = write!(s, "\t{}", n.cents);
        }
        if chance {
            let _ = write!(s, "\t{}", n.chance);
        }
        if repeat {
            let _ = write!(s, "\t{}", n.repeat);
        }
        s.push('\n');
    }
    s.into_bytes()
}

fn value_text(v: f64) -> f64 {
    // Never `-0` (§ 2.5).
    if v == 0.0 { 0.0 } else { v }
}

pub fn points_tsv(target: &AutomationTarget, points: &BTreeMap<Tick, AutomationPoint>) -> Vec<u8> {
    let mut s = String::with_capacity(64 + points.len() * 20);
    let _ = writeln!(
        s,
        "# faulk-automation 1\ttarget={}",
        serde_json::to_string(target).expect("target")
    );
    for (tick, p) in points {
        let shape = match p.shape {
            Shape::Linear => "linear",
            Shape::Step => "step",
        };
        let _ = writeln!(s, "{}\t{}\t{shape}", tick.get(), value_text(p.value));
    }
    s.into_bytes()
}

/// The packed form of a lane: 17 bytes a point, tick (i64), value (f64), shape (u8), all
/// little-endian.
pub fn points_blob(points: &BTreeMap<Tick, AutomationPoint>) -> Vec<u8> {
    let mut v = Vec::with_capacity(points.len() * 17);
    for (tick, p) in points {
        v.extend_from_slice(&tick.get().to_le_bytes());
        v.extend_from_slice(&value_text(p.value).to_le_bytes());
        v.push(u8::from(p.shape == Shape::Step));
    }
    v
}

fn unpack_points(bytes: &[u8]) -> Result<BTreeMap<Tick, AutomationPoint>, String> {
    if !bytes.len().is_multiple_of(17) {
        return Err("a lane blob is not a whole number of points".into());
    }
    let v: Vec<(Tick, AutomationPoint)> = bytes
        .as_chunks::<17>()
        .0
        .iter()
        .map(|r| {
            (
                Tick(i64::from_le_bytes(r[0..8].try_into().unwrap())),
                AutomationPoint {
                    value: f64::from_le_bytes(r[8..16].try_into().unwrap()),
                    shape: if r[16] == 1 {
                        Shape::Step
                    } else {
                        Shape::Linear
                    },
                },
            )
        })
        .collect();
    Ok(v.into_iter().collect())
}

fn sha_hex(bytes: &[u8]) -> String {
    Hash(Sha256::digest(bytes).into()).to_string()
}

pub fn arrangement_tsv(track: &Track) -> Vec<u8> {
    let mut rows: Vec<(&PlacementId, &Placement)> = track.placements.iter().collect();
    rows.sort_by(|a, b| (a.1.start, a.0).cmp(&(b.1.start, b.0)));
    let mut s = String::from("# faulk-arrangement 1\tid\tstart\tlength\tclip\toffset\n");
    for (id, p) in rows {
        let (length, offset) = match p.span {
            Span::Ticks { length, offset } => (length.get(), offset.get()),
            Span::Frames { length, offset } => (length.get() as i64, offset.get() as i64),
        };
        let _ = writeln!(s, "{id}\t{}\t{length}\t{}\t{offset}", p.start.get(), p.clip);
    }
    s.into_bytes()
}

// ------------------------------------------------------------------- emit

pub fn emit_project_json(project: &Project) -> Out {
    Out {
        path: "project.json".into(),
        bytes: json(&project.clone_without_tracks()),
        blob: false,
    }
}

pub fn emit_clip_notes(tid: &TrackId, cid: &ClipId, notes: &BTreeSet<Note>) -> Out {
    Out {
        path: format!("tracks/{tid}/clips/{cid}.notes.tsv"),
        bytes: notes_tsv(notes),
        blob: false,
    }
}

/// Every file of one track, one at a time.
pub fn emit_track(tid: &TrackId, track: &Track, layout: Layout, sink: &mut dyn FnMut(Out)) {
    let base = format!("tracks/{tid}");
    sink(Out {
        path: format!("{base}/track.json"),
        bytes: json(&skeleton_track(track)),
        blob: false,
    });
    for (id, device) in &track.devices {
        sink(Out {
            path: format!("{base}/devices/{id}.json"),
            bytes: json(device),
            blob: false,
        });
    }
    for (id, clip) in &track.clips {
        sink(Out {
            path: format!("{base}/clips/{id}.json"),
            bytes: json(&skeleton_clip(clip)),
            blob: false,
        });
        if let ClipContent::Midi { notes, .. } = &clip.content {
            sink(emit_clip_notes(tid, id, notes));
        }
    }
    if !track.placements.is_empty() {
        sink(Out {
            path: format!("{base}/arrangement.tsv"),
            bytes: arrangement_tsv(track),
            blob: false,
        });
    }
    for (id, lane) in &track.automation {
        emit_lane(tid, id, lane, layout, sink);
    }
}

/// The file of one lane, and its blob when it is above the layout's threshold.
pub fn emit_lane(
    tid: &TrackId,
    id: &LaneId,
    lane: &AutomationLane,
    layout: Layout,
    sink: &mut dyn FnMut(Out),
) {
    let base = format!("tracks/{tid}");
    if lane.points.len() > layout.blob_above {
        let blob = points_blob(&lane.points);
        let hex = sha_hex(&blob);
        let header = format!(
            "# faulk-automation 1\ttarget={}\tblob={hex}\tpoints={}\n",
            serde_json::to_string(&lane.target).expect("target"),
            lane.points.len()
        );
        sink(Out {
            path: format!("{base}/automation/{id}.tsv"),
            bytes: header.into_bytes(),
            blob: false,
        });
        sink(Out {
            path: format!("blobs/{hex}"),
            bytes: blob,
            blob: true,
        });
    } else {
        sink(Out {
            path: format!("{base}/automation/{id}.tsv"),
            bytes: points_tsv(&lane.target, &lane.points),
            blob: false,
        });
    }
}

/// Dirty-file save of one lane: its file, and its blob if it has one.
pub fn save_lane(
    project: &Project,
    root: &Path,
    tid: &TrackId,
    lid: &LaneId,
    layout: Layout,
    durable: bool,
) -> io::Result<WriteStats> {
    let lane = project
        .tracks
        .get(tid)
        .and_then(|t| t.automation.get(lid))
        .ok_or_else(|| io::Error::other("no such lane"))?;
    let mut w = Writer::new(root, durable, None);
    let mut error = None;
    emit_lane(tid, lid, lane, layout, &mut |out| {
        if error.is_none()
            && let Err(e) = w.write(out)
        {
            error = Some(e);
        }
    });
    match error {
        Some(e) => Err(e),
        None => Ok(w.stats),
    }
}

pub fn emit_all(project: &Project, layout: Layout, sink: &mut dyn FnMut(Out)) {
    sink(emit_project_json(project));
    for (id, track) in &project.tracks {
        emit_track(id, track, layout, sink);
    }
}

// ------------------------------------------------------------------- write

#[derive(Clone, Copy, Debug, Default)]
pub struct WriteStats {
    pub written: u64,
    pub skipped: u64,
    pub bytes: u64,
}

impl WriteStats {
    pub fn add(&mut self, o: WriteStats) {
        self.written += o.written;
        self.skipped += o.skipped;
        self.bytes += o.bytes;
    }
}

/// What the lazy writer remembers of the last save: a hash of each file's bytes. The writer
/// skips a file whose hash is unchanged. It cannot see an edit made outside it (documented
/// limit, tested).
#[derive(Default)]
pub struct Known(pub HashMap<String, u64>);

fn fast_hash(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(bytes);
    h.finish()
}

/// Writes through a temporary file then a rename, so that a crash leaves the old file or the
/// new one, never half of one. `durable` adds an `fsync` of the file before the rename.
pub fn atomic_write(path: &Path, bytes: &[u8], durable: bool) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        if durable {
            f.sync_all()?;
        }
    }
    std::fs::rename(&tmp, path)
}

pub struct Writer<'a> {
    root: PathBuf,
    durable: bool,
    known: Option<&'a mut Known>,
    dirs: HashSet<PathBuf>,
    pub stats: WriteStats,
}

impl<'a> Writer<'a> {
    pub fn new(root: &Path, durable: bool, known: Option<&'a mut Known>) -> Self {
        Self {
            root: root.to_owned(),
            durable,
            known,
            dirs: HashSet::new(),
            stats: WriteStats::default(),
        }
    }

    pub fn write(&mut self, out: Out) -> io::Result<()> {
        let path = self.root.join(&out.path);
        if out.blob {
            // Content-addressed: the name says what is inside, so an existing file is final.
            if path.exists() {
                self.stats.skipped += 1;
                return Ok(());
            }
        } else if let Some(known) = self.known.as_deref_mut() {
            let h = fast_hash(&out.bytes);
            if known.0.get(&out.path) == Some(&h) {
                self.stats.skipped += 1;
                return Ok(());
            }
            known.0.insert(out.path.clone(), h);
        }
        if let Some(dir) = path.parent()
            && !self.dirs.contains(dir)
        {
            std::fs::create_dir_all(dir)?;
            self.dirs.insert(dir.to_owned());
        }
        atomic_write(&path, &out.bytes, self.durable)?;
        self.stats.written += 1;
        self.stats.bytes += out.bytes.len() as u64;
        Ok(())
    }
}

/// Saves the whole project. With `known`, files whose bytes did not change are skipped (the
/// lazy write of § 2.4); with `threads` above 1 the tracks are written in parallel (no lazy
/// write then).
pub fn write_all(
    project: &Project,
    root: &Path,
    layout: Layout,
    durable: bool,
    mut known: Option<&mut Known>,
    threads: usize,
) -> io::Result<WriteStats> {
    std::fs::create_dir_all(root)?;
    let mut total = WriteStats::default();
    {
        let mut w = Writer::new(root, durable, known.as_deref_mut());
        w.write(emit_project_json(project))?;
        total.add(w.stats);
    }
    let tracks: Vec<(&TrackId, &Track)> = project.tracks.iter().collect();
    if threads <= 1 || known.is_some() {
        let mut w = Writer::new(root, durable, known);
        let mut error = None;
        for (id, track) in tracks {
            emit_track(id, track, layout, &mut |out| {
                if error.is_none()
                    && let Err(e) = w.write(out)
                {
                    error = Some(e);
                }
            });
        }
        if let Some(e) = error {
            return Err(e);
        }
        total.add(w.stats);
        return Ok(total);
    }
    let results: Vec<io::Result<WriteStats>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|k| {
                let mine: Vec<_> = tracks.iter().skip(k).step_by(threads).copied().collect();
                s.spawn(move || {
                    let mut w = Writer::new(root, durable, None);
                    let mut error = None;
                    for (id, track) in mine {
                        emit_track(id, track, layout, &mut |out| {
                            if error.is_none()
                                && let Err(e) = w.write(out)
                            {
                                error = Some(e);
                            }
                        });
                    }
                    match error {
                        Some(e) => Err(e),
                        None => Ok(w.stats),
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("writer thread"))
            .collect()
    });
    for r in results {
        total.add(r?);
    }
    Ok(total)
}

/// Dirty-file save of one clip's notes: the one file the edit touched.
pub fn save_clip_notes(
    project: &Project,
    root: &Path,
    tid: &TrackId,
    cid: &ClipId,
    durable: bool,
) -> io::Result<WriteStats> {
    let Some(ClipContent::Midi { notes, .. }) = project
        .tracks
        .get(tid)
        .and_then(|t| t.clips.get(cid))
        .map(|c| &c.content)
    else {
        return Err(io::Error::other("not a MIDI clip"));
    };
    let mut w = Writer::new(root, durable, None);
    w.write(emit_clip_notes(tid, cid, notes))?;
    Ok(w.stats)
}

/// Writes the audio files into `blobs/` and returns their hashes.
pub fn write_audio(
    files: &[crate::synth::AudioFile],
    root: &Path,
) -> io::Result<Vec<(crate::synth::AudioFile, Hash)>> {
    let dir = root.join("blobs");
    std::fs::create_dir_all(&dir)?;
    let mut out = Vec::new();
    for f in files {
        let tmp = dir.join(format!("{}.tmp", f.name));
        let hash = {
            let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
            let h = f.write_to(&mut w)?;
            w.flush()?;
            h
        };
        std::fs::rename(&tmp, dir.join(hash.to_string()))?;
        out.push((f.clone(), hash));
    }
    Ok(out)
}

/// The first MIDI clip of the project, by id order.
pub fn first_midi_clip(project: &Project) -> Option<(TrackId, ClipId)> {
    project.tracks.iter().find_map(|(t, tr)| {
        tr.clips
            .iter()
            .find(|(_, c)| c.content.is_midi())
            .map(|(c, _)| (t.clone(), c.clone()))
    })
}

/// Changes the velocity of the first note of a clip (a note is a value: remove, insert).
pub fn change_note(project: &mut Project, t: &TrackId, c: &ClipId, velocity: u8) {
    let ClipContent::Midi { notes, .. } = &mut project
        .tracks
        .get_mut(t)
        .and_then(|t| t.clips.get_mut(c))
        .expect("clip")
        .content
    else {
        panic!("not a MIDI clip");
    };
    let old = *notes.iter().next().expect("a note");
    notes.remove(&old);
    notes.insert(Note {
        velocity: velocity.clamp(1, 127),
        ..old
    });
}

// ------------------------------------------------------------------- read

/// A folder, as the reader sees it.
pub trait Files: Sync {
    fn read(&self, rel: &str) -> io::Result<Vec<u8>>;
    /// The names of the files directly under `rel`; an absent directory has none.
    fn files(&self, rel: &str) -> io::Result<Vec<String>>;
    /// The names of the directories directly under `rel`.
    fn dirs(&self, rel: &str) -> io::Result<Vec<String>>;
}

pub struct FsFiles(pub PathBuf);

impl Files for FsFiles {
    fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.0.join(rel))
    }

    fn files(&self, rel: &str) -> io::Result<Vec<String>> {
        list(&self.0.join(rel), false)
    }

    fn dirs(&self, rel: &str) -> io::Result<Vec<String>> {
        list(&self.0.join(rel), true)
    }
}

fn list(dir: &Path, want_dirs: bool) -> io::Result<Vec<String>> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in rd {
        let entry = entry?;
        if entry.file_type()?.is_dir() == want_dirs {
            out.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// A folder held in memory: path to bytes.
#[derive(Default)]
pub struct MemFiles(pub BTreeMap<String, Vec<u8>>);

impl MemFiles {
    pub fn from_project(project: &Project, layout: Layout) -> Self {
        let mut m = MemFiles::default();
        emit_all(project, layout, &mut |o| {
            m.0.insert(o.path, o.bytes);
        });
        m
    }

    fn children(&self, rel: &str, want_dirs: bool) -> Vec<String> {
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        let mut seen = BTreeSet::new();
        for path in self.0.range(prefix.clone()..) {
            let Some(rest) = path.0.strip_prefix(&prefix) else {
                break;
            };
            match rest.split_once('/') {
                Some((dir, _)) if want_dirs => {
                    seen.insert(dir.to_owned());
                }
                None if !want_dirs => {
                    seen.insert(rest.to_owned());
                }
                _ => {}
            }
        }
        seen.into_iter().collect()
    }
}

impl Files for MemFiles {
    fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
        self.0
            .get(rel)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, rel.to_owned()))
    }

    fn files(&self, rel: &str) -> io::Result<Vec<String>> {
        Ok(self.children(rel, false))
    }

    fn dirs(&self, rel: &str) -> io::Result<Vec<String>> {
        Ok(self.children(rel, true))
    }
}

/// What a read cost, summed over threads: time in the file system, time parsing.
#[derive(Default)]
pub struct ReadStats {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    pub io_ns: AtomicU64,
    pub parse_ns: AtomicU64,
}

impl ReadStats {
    fn read(&self, files: &dyn Files, rel: &str) -> Result<Vec<u8>, String> {
        let t = Instant::now();
        let bytes = files.read(rel).map_err(|e| format!("{rel}: {e}"))?;
        self.io_ns.fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
        self.files.fetch_add(1, Relaxed);
        self.bytes.fetch_add(bytes.len() as u64, Relaxed);
        Ok(bytes)
    }

    fn parse<T>(&self, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let t = Instant::now();
        let out = f();
        self.parse_ns
            .fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
        out
    }
}

fn utf8(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes).map_err(|e| e.to_string())
}

fn stem<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    name.strip_suffix(suffix)
}

pub fn parse_notes(bytes: &[u8]) -> Result<BTreeSet<Note>, String> {
    let text = utf8(bytes)?;
    let mut lines = text.lines();
    let header = lines.next().ok_or("empty notes file")?;
    let cols: Vec<&str> = header
        .strip_prefix("# faulk-notes 1")
        .ok_or("bad notes header")?
        .split('\t')
        .skip(1)
        .collect();
    if cols.len() < 4 || cols[..4] != ["start", "length", "pitch", "velocity"] {
        return Err("bad notes columns".into());
    }
    let mut v = Vec::with_capacity(text.len() / 16);
    for line in lines {
        let mut it = line.split('\t');
        let mut next = || it.next().ok_or_else(|| format!("short line {line:?}"));
        let start: i64 = next()?.parse().map_err(|_| "bad start")?;
        let length: i64 = next()?.parse().map_err(|_| "bad length")?;
        let pitch: u8 = next()?.parse().map_err(|_| "bad pitch")?;
        let velocity: u8 = next()?.parse().map_err(|_| "bad velocity")?;
        let mut note = Note::new(Tick(start), pitch, Tick(length), velocity);
        for col in &cols[4..] {
            let field = next()?;
            match *col {
                "cents" => note.cents = field.parse().map_err(|_| "bad cents")?,
                "chance" => note.chance = field.parse().map_err(|_| "bad chance")?,
                "repeat" => note.repeat = field.parse().map_err(|_| "bad repeat")?,
                other => return Err(format!("unknown column {other}")),
            }
        }
        v.push(note);
    }
    Ok(v.into_iter().collect())
}

/// A lane file: its target and either its points or the blob that holds them.
enum LaneFile {
    Text(AutomationTarget, BTreeMap<Tick, AutomationPoint>),
    Blob(AutomationTarget, String, usize),
}

fn parse_lane(bytes: &[u8]) -> Result<LaneFile, String> {
    let text = utf8(bytes)?;
    let mut lines = text.lines();
    let header = lines.next().ok_or("empty lane file")?;
    let mut fields = header
        .strip_prefix("# faulk-automation 1\t")
        .ok_or("bad lane header")?
        .split('\t');
    let target: AutomationTarget = serde_json::from_str(
        fields
            .next()
            .and_then(|f| f.strip_prefix("target="))
            .ok_or("no target")?,
    )
    .map_err(|e| e.to_string())?;
    if let Some(blob) = fields.next().and_then(|f| f.strip_prefix("blob=")) {
        let points = fields
            .next()
            .and_then(|f| f.strip_prefix("points="))
            .and_then(|n| n.parse().ok())
            .ok_or("no point count")?;
        return Ok(LaneFile::Blob(target, blob.to_owned(), points));
    }
    let mut v = Vec::with_capacity(text.len() / 12);
    for line in lines {
        let mut it = line.split('\t');
        let tick: i64 = it
            .next()
            .ok_or("short point")?
            .parse()
            .map_err(|_| "bad tick")?;
        let value: f64 = it
            .next()
            .ok_or("short point")?
            .parse()
            .map_err(|_| "bad value")?;
        let shape = match it.next() {
            Some("linear") => Shape::Linear,
            Some("step") => Shape::Step,
            _ => return Err("bad shape".into()),
        };
        v.push((Tick(tick), AutomationPoint { value, shape }));
    }
    Ok(LaneFile::Text(target, v.into_iter().collect()))
}

/// The number of points a lane's text file holds (the parse is what is timed).
pub fn parse_lane_text(bytes: &[u8]) -> usize {
    match parse_lane(bytes) {
        Ok(LaneFile::Text(_, points)) => points.len(),
        _ => 0,
    }
}

/// The number of points a lane's blob holds.
pub fn parse_lane_blob(bytes: &[u8]) -> usize {
    unpack_points(bytes).map(|p| p.len()).unwrap_or(0)
}

fn parse_arrangement(
    bytes: &[u8],
    kind: TrackKind,
) -> Result<BTreeMap<PlacementId, Placement>, String> {
    let text = utf8(bytes)?;
    let mut lines = text.lines();
    lines.next().ok_or("empty arrangement")?;
    let mut out = BTreeMap::new();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 5 {
            return Err(format!("bad placement {line:?}"));
        }
        let start = Tick(f[1].parse().map_err(|_| "bad start")?);
        let length: i64 = f[2].parse().map_err(|_| "bad length")?;
        let offset: i64 = f[4].parse().map_err(|_| "bad offset")?;
        let span = match kind {
            TrackKind::Midi => Span::Ticks {
                length: Tick(length),
                offset: Tick(offset),
            },
            TrackKind::Audio => Span::Frames {
                length: Frame(length as u64),
                offset: Frame(offset as u64),
            },
        };
        out.insert(
            PlacementId::parse(f[0]).map_err(|e| e.to_string())?,
            Placement {
                clip: ClipId::parse(f[3]).map_err(|e| e.to_string())?,
                start,
                span,
            },
        );
    }
    Ok(out)
}

fn read_track(files: &dyn Files, tid: &str, stats: &ReadStats) -> Result<(TrackId, Track), String> {
    let base = format!("tracks/{tid}");
    let bytes = stats.read(files, &format!("{base}/track.json"))?;
    let mut track: Track =
        stats.parse(|| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))?;
    let list = |rel: &str| files.files(rel).map_err(|e| e.to_string());
    for name in list(&format!("{base}/devices"))? {
        let id = stem(&name, ".json").ok_or("stray file in devices")?;
        let bytes = stats.read(files, &format!("{base}/devices/{name}"))?;
        let device: Device =
            stats.parse(|| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))?;
        track
            .devices
            .insert(DeviceId::parse(id).map_err(|e| e.to_string())?, device);
    }
    for name in list(&format!("{base}/clips"))? {
        let Some(id) = stem(&name, ".json") else {
            continue;
        };
        let bytes = stats.read(files, &format!("{base}/clips/{name}"))?;
        let mut clip: Clip =
            stats.parse(|| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))?;
        if let ClipContent::Midi { notes, .. } = &mut clip.content {
            let bytes = stats.read(files, &format!("{base}/clips/{id}.notes.tsv"))?;
            *notes = stats.parse(|| parse_notes(&bytes))?;
        }
        track
            .clips
            .insert(ClipId::parse(id).map_err(|e| e.to_string())?, clip);
    }
    if let Ok(bytes) = stats.read(files, &format!("{base}/arrangement.tsv")) {
        track.placements = stats.parse(|| parse_arrangement(&bytes, track.kind))?;
    }
    for name in list(&format!("{base}/automation"))? {
        let id = stem(&name, ".tsv").ok_or("stray file in automation")?;
        let bytes = stats.read(files, &format!("{base}/automation/{name}"))?;
        let lane = match stats.parse(|| parse_lane(&bytes))? {
            LaneFile::Text(target, points) => AutomationLane { target, points },
            LaneFile::Blob(target, hex, n) => {
                let blob = stats.read(files, &format!("blobs/{hex}"))?;
                let points = stats.parse(|| unpack_points(&blob))?;
                if points.len() != n {
                    return Err(format!(
                        "lane {id}: {n} points announced, {} found",
                        points.len()
                    ));
                }
                AutomationLane { target, points }
            }
        };
        track
            .automation
            .insert(LaneId::parse(id).map_err(|e| e.to_string())?, lane);
    }
    Ok((TrackId::parse(tid).map_err(|e| e.to_string())?, track))
}

/// Reads a folder into a project; `threads` tracks are read at once.
pub fn read_project(files: &dyn Files, threads: usize) -> Result<(Project, ReadStats), String> {
    let stats = ReadStats::default();
    let bytes = stats.read(files, "project.json")?;
    let mut project: Project =
        stats.parse(|| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))?;
    let tracks = files.dirs("tracks").map_err(|e| e.to_string())?;
    let threads = threads.max(1);
    let parts: Vec<Result<Vec<(TrackId, Track)>, String>> = if threads == 1 {
        vec![
            tracks
                .iter()
                .map(|t| read_track(files, t, &stats))
                .collect(),
        ]
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|k| {
                    let mine: Vec<&String> = tracks.iter().skip(k).step_by(threads).collect();
                    let stats = &stats;
                    s.spawn(move || {
                        mine.into_iter()
                            .map(|t| read_track(files, t, stats))
                            .collect::<Result<Vec<_>, String>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("reader thread"))
                .collect()
        })
    };
    for part in parts {
        for (id, track) in part? {
            project.tracks.insert(id, track);
        }
    }
    Ok((project, stats))
}

// ------------------------------------------------------------------- cache snapshot

const SNAP_MAGIC: &[u8; 8] = b"FKSNAP01";

/// One source file as the snapshot saw it: path, size, modification time (ns).
pub type Manifest = Vec<(String, u64, u64)>;

/// Walks the folder (not `.faulk`, `.git`, nor `blobs`: blobs are named by their content) and
/// stats each file: what the snapshot is checked against at open.
pub fn scan_manifest(root: &Path) -> io::Result<Manifest> {
    fn walk(dir: &Path, rel: &str, out: &mut Manifest) -> io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{rel}{name}");
            let ty = entry.file_type()?;
            if ty.is_dir() {
                if matches!(name.as_str(), ".faulk" | ".git" | "blobs") {
                    continue;
                }
                walk(&entry.path(), &format!("{path}/"), out)?;
            } else if !name.ends_with(".tmp") {
                let meta = entry.metadata()?;
                let mtime = meta
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                out.push((path, meta.len(), mtime));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, "", &mut out)?;
    out.sort();
    Ok(out)
}

pub fn snapshot_path(root: &Path) -> PathBuf {
    root.join(".faulk").join("cache").join("project.snap")
}

/// Writes the snapshot of `project`, bound to the manifest of the text on disk now. Written
/// after the text and through a rename: a crash between the two leaves a snapshot older than
/// the text, which the manifest rejects.
pub fn write_snapshot(project: &Project, root: &Path) -> io::Result<u64> {
    let manifest = scan_manifest(root)?;
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(SNAP_MAGIC);
    out.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
    for (path, size, mtime) in &manifest {
        out.extend_from_slice(&(path.len() as u16).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&mtime.to_le_bytes());
    }
    let skel = serde_json::to_vec(&skeleton(project)).expect("skeleton");
    out.extend_from_slice(&(skel.len() as u64).to_le_bytes());
    out.extend_from_slice(&skel);
    for track in project.tracks.values() {
        for clip in track.clips.values() {
            if let ClipContent::Midi { notes, .. } = &clip.content {
                out.extend_from_slice(&(notes.len() as u32).to_le_bytes());
                for n in notes {
                    out.extend_from_slice(&n.start.get().to_le_bytes());
                    out.extend_from_slice(&n.length.get().to_le_bytes());
                    out.extend_from_slice(&[
                        n.pitch,
                        n.velocity,
                        n.cents as u8,
                        n.chance,
                        n.repeat,
                    ]);
                }
            }
        }
        for lane in track.automation.values() {
            out.extend_from_slice(&(lane.points.len() as u32).to_le_bytes());
            out.extend_from_slice(&points_blob(&lane.points));
        }
    }
    let path = snapshot_path(root);
    std::fs::create_dir_all(path.parent().expect("parent"))?;
    atomic_write(&path, &out, false)?;
    Ok(out.len() as u64)
}

/// Why a snapshot was not used.
#[derive(Debug, PartialEq, Eq)]
pub enum Stale {
    Missing,
    Corrupt(String),
    /// A source file was added, removed or changed since (path of the first difference).
    Changed(String),
}

struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Stale> {
        if self.0.len() < n {
            return Err(Stale::Corrupt("truncated".into()));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }

    fn u32(&mut self) -> Result<u32, Stale> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, Stale> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

/// Loads the snapshot if it is current: the manifest it holds equals the folder's now.
pub fn read_snapshot(root: &Path) -> Result<Project, Stale> {
    let bytes = match std::fs::read(snapshot_path(root)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Stale::Missing),
        Err(e) => return Err(Stale::Corrupt(e.to_string())),
    };
    let (saved, c) = snapshot_header(&bytes)?;
    let now = scan_manifest(root).map_err(|e| Stale::Corrupt(e.to_string()))?;
    if saved != now {
        let first = saved
            .iter()
            .zip(&now)
            .find(|(a, b)| a != b)
            .map(|(a, _)| a.0.clone())
            .or_else(|| {
                saved
                    .get(now.len())
                    .or(now.get(saved.len()))
                    .map(|m| m.0.clone())
            })
            .unwrap_or_default();
        return Err(Stale::Changed(first));
    }
    decode_snapshot_body(c)
}

fn snapshot_header(bytes: &[u8]) -> Result<(Manifest, Cursor<'_>), Stale> {
    let mut c = Cursor(bytes);
    if c.take(8)? != SNAP_MAGIC {
        return Err(Stale::Corrupt("magic".into()));
    }
    let count = c.u32()? as usize;
    let mut saved: Manifest = Vec::with_capacity(count.min(1 << 20));
    for _ in 0..count {
        let len = u16::from_le_bytes(c.take(2)?.try_into().unwrap()) as usize;
        let path =
            String::from_utf8(c.take(len)?.to_vec()).map_err(|e| Stale::Corrupt(e.to_string()))?;
        saved.push((path, c.u64()?, c.u64()?));
    }
    Ok((saved, c))
}

/// Decodes a snapshot's bytes without checking its manifest against a folder (the browser
/// build checks nothing of the file system: it measures the decode).
pub fn decode_snapshot(bytes: &[u8]) -> Result<Project, Stale> {
    let (_, c) = snapshot_header(bytes)?;
    decode_snapshot_body(c)
}

fn decode_snapshot_body(mut c: Cursor<'_>) -> Result<Project, Stale> {
    let skel_len = c.u64()? as usize;
    let mut project: Project =
        serde_json::from_slice(c.take(skel_len)?).map_err(|e| Stale::Corrupt(e.to_string()))?;
    for track in project.tracks.values_mut() {
        for clip in track.clips.values_mut() {
            if let ClipContent::Midi { notes, .. } = &mut clip.content {
                let n = c.u32()? as usize;
                let rec = c.take(
                    n.checked_mul(21)
                        .ok_or_else(|| Stale::Corrupt("count".into()))?,
                )?;
                let v: Vec<Note> = rec
                    .as_chunks::<21>()
                    .0
                    .iter()
                    .map(|r| Note {
                        start: Tick(i64::from_le_bytes(r[0..8].try_into().unwrap())),
                        length: Tick(i64::from_le_bytes(r[8..16].try_into().unwrap())),
                        pitch: r[16],
                        velocity: r[17],
                        cents: r[18] as i8,
                        chance: r[19],
                        repeat: r[20],
                    })
                    .collect();
                *notes = v.into_iter().collect();
            }
        }
        for lane in track.automation.values_mut() {
            let n = c.u32()? as usize;
            lane.points = unpack_points(
                c.take(
                    n.checked_mul(17)
                        .ok_or_else(|| Stale::Corrupt("count".into()))?,
                )?,
            )
            .map_err(Stale::Corrupt)?;
        }
    }
    Ok(project)
}

/// A digest of the folder (paths and bytes, in path order), to compare two writes.
pub fn folder_digest(root: &Path, include_blobs: bool) -> io::Result<String> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>, blobs: bool) -> io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{rel}{name}");
            if entry.file_type()?.is_dir() {
                if matches!(name.as_str(), ".faulk" | ".git") || (name == "blobs" && !blobs) {
                    continue;
                }
                walk(&entry.path(), &format!("{path}/"), out, blobs)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    walk(root, "", &mut paths, include_blobs)?;
    paths.sort();
    let mut h = Sha256::new();
    for p in &paths {
        h.update(p.as_bytes());
        h.update([0]);
        h.update(std::fs::read(root.join(p))?);
        h.update([0]);
    }
    Ok(Hash(h.finalize().into()).to_string())
}
