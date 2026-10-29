//! The cache snapshot (`.faulk/cache/project.snap`, § 7): what the project was when it was
//! last saved, in one file, so that a browser opens a folder of thousands of small files
//! with one read and re-reads only the files that changed since.
//!
//! One entry per source file: its path, size and stamp as the folder listed them when the
//! snapshot was written, the hash of its text, and a payload (the text itself, or the packed
//! binary of a file of notes or points). At open, a file whose size and stamp equal its
//! entry's is decoded from the payload; any other file, new files included, is read as text;
//! an entry whose file is gone is ignored. The snapshot is regenerable: missing, corrupt or
//! of another version, it is ignored and the folder is read in full, never an error.
//!
//! Layout, little endian: magic `FKSNAP02`, format `u32`, entry count `u32`, the entries,
//! then a `u64` checksum of everything before it.

use super::canon::fast_hash;
use super::error::FileError;
use super::fs::FileSystem;
use super::read::project_files;
use super::unit::{EmitCtx, SNAPSHOT_FILE, Unit, emit};
use super::write::{SaveOptions, SaveState};
use crate::model::{FORMAT_VERSION, Project};
use std::collections::{BTreeMap, BTreeSet};

const MAGIC: &[u8; 8] = b"FKSNAP02";

/// Why a snapshot was not used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheUse {
    /// The caller did not ask for it.
    NotUsed,
    /// There is none.
    Missing,
    /// There is one, and it is not usable (corrupt, other format).
    Unusable(String),
    /// Used: the files it covered were not read.
    Used,
}

/// One entry of a loaded snapshot.
pub(crate) struct Entry {
    pub size: u64,
    pub stamp: u64,
    pub hash: u64,
    start: usize,
    end: usize,
}

pub(crate) struct Snapshot {
    bytes: Vec<u8>,
    pub entries: BTreeMap<String, Entry>,
}

impl Snapshot {
    pub fn payload(&self, e: &Entry) -> &[u8] {
        self.bytes.get(e.start..e.end).unwrap_or(&[])
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<(usize, usize), String> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.bytes.len())
            .ok_or("truncated")?;
        let start = self.at;
        self.at = end;
        Ok((start, end))
    }

    fn u16(&mut self) -> Result<u16, String> {
        let (s, _) = self.take(2)?;
        Ok(u16::from_le_bytes([self.bytes[s], self.bytes[s + 1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let (s, _) = self.take(4)?;
        Ok(u32::from_le_bytes([
            self.bytes[s],
            self.bytes[s + 1],
            self.bytes[s + 2],
            self.bytes[s + 3],
        ]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let (s, _) = self.take(8)?;
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.bytes[s..s + 8]);
        Ok(u64::from_le_bytes(b))
    }
}

fn decode(bytes: Vec<u8>) -> Result<Snapshot, String> {
    let body_len = bytes.len().checked_sub(8).ok_or("truncated")?;
    let mut tail = [0u8; 8];
    tail.copy_from_slice(&bytes[body_len..]);
    if fast_hash(&bytes[..body_len]) != u64::from_le_bytes(tail) {
        return Err("checksum".into());
    }
    let mut entries = BTreeMap::new();
    {
        let mut c = Cursor {
            bytes: &bytes[..body_len],
            at: 0,
        };
        let (s, e) = c.take(8)?;
        if &bytes[s..e] != MAGIC {
            return Err("magic".into());
        }
        if c.u32()? != FORMAT_VERSION {
            return Err("another format".into());
        }
        let count = c.u32()? as usize;
        for _ in 0..count {
            let len = c.u16()? as usize;
            let (s, e) = c.take(len)?;
            let path = String::from_utf8(bytes[s..e].to_vec()).map_err(|e| e.to_string())?;
            let size = c.u64()?;
            let stamp = c.u64()?;
            let hash = c.u64()?;
            let plen = usize::try_from(c.u64()?).map_err(|e| e.to_string())?;
            let (start, end) = c.take(plen)?;
            entries.insert(
                path,
                Entry {
                    size,
                    stamp,
                    hash,
                    start,
                    end,
                },
            );
        }
        if c.at != body_len {
            return Err("trailing bytes".into());
        }
    }
    Ok(Snapshot { bytes, entries })
}

/// Loads the snapshot of a folder, if there is a usable one.
pub(crate) fn load(fs: &dyn FileSystem) -> Result<Snapshot, CacheUse> {
    match fs.read(SNAPSHOT_FILE) {
        Err(super::fs::FsError::NotFound) => Err(CacheUse::Missing),
        Err(e) => Err(CacheUse::Unusable(e.to_string())),
        Ok(bytes) => decode(bytes).map_err(CacheUse::Unusable),
    }
}

/// Writes the snapshot of the folder as `save` just left it. `fresh` holds the files this
/// save emitted; the others are taken from the previous snapshot when it has them, and when
/// it does not the whole project is emitted once more to build them.
pub(crate) fn write(
    fs: &mut dyn FileSystem,
    project: &Project,
    state: &mut SaveState,
    options: &SaveOptions,
    fresh: BTreeMap<String, (u64, Vec<u8>)>,
    written: &[String],
) -> Result<bool, FileError> {
    let mut entries = fresh;
    let old = load(fs).ok();
    let mut complete = true;
    for (path, hash) in &state.known {
        if entries.get(path).is_some_and(|(h, _)| h == hash) {
            continue;
        }
        let reused = old.as_ref().and_then(|o| {
            o.entries
                .get(path)
                .filter(|e| e.hash == *hash)
                .map(|e| o.payload(e))
        });
        match reused {
            Some(payload) => {
                entries.insert(path.clone(), (*hash, payload.to_vec()));
            }
            None => complete = false,
        }
    }
    if !complete {
        let mut lanes = state.blob_lanes.clone();
        let mut ctx = EmitCtx {
            layout: options.layout,
            blob_lanes: &mut lanes,
            with_payload: true,
        };
        let mut all = BTreeMap::new();
        let mut sink = |out: super::unit::Out| -> Result<(), FileError> {
            if !out.blob {
                let hash = fast_hash(&out.bytes);
                all.insert(out.path, (hash, out.payload.unwrap_or(out.bytes)));
            }
            Ok(())
        };
        for t in project.tracks.keys() {
            emit(project, &Unit::Track(t.clone()), &mut ctx, &mut sink)?;
        }
        for l in project.master_automation.keys() {
            emit(project, &Unit::MasterLane(l.clone()), &mut ctx, &mut sink)?;
        }
        emit(project, &Unit::Project, &mut ctx, &mut sink)?;
        entries = all;
    }
    // Only what the folder holds and the state knows. A file this save did not write whose
    // stamp is not the one the state saw (a `git pull`, another editor) may no longer be what
    // the state believes: it is checked against its hash, and left out when it differs, so
    // that the next open reads it from the text. A folder that cannot be listed gives no
    // snapshot, not a failed save: the files are written already.
    let Ok(listing) = project_files(fs) else {
        return Ok(false);
    };
    let on_disk: BTreeMap<&str, (u64, u64)> = listing
        .iter()
        .map(|(p, e)| (p.as_str(), (e.size, e.stamp)))
        .collect();
    let written: BTreeSet<&str> = written.iter().map(String::as_str).collect();
    let mut vouched: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for path in entries.keys() {
        let Some(&now) = on_disk.get(path.as_str()) else {
            continue;
        };
        let seen = state.stamps.get(path) == Some(&now);
        let own = written.contains(path.as_str());
        let current = own
            || seen
            || fs
                .read(path)
                .is_ok_and(|b| Some(&fast_hash(&b)) == state.known.get(path));
        if current {
            vouched.insert(path.clone(), now);
        }
    }
    let keep: BTreeSet<&String> = state.known.keys().collect();
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    let count_at = out.len();
    out.extend_from_slice(&0u32.to_le_bytes());
    let mut count = 0u32;
    for (path, (hash, payload)) in &entries {
        let (Some(&(size, stamp)), true, Ok(len)) = (
            vouched.get(path.as_str()),
            keep.contains(path),
            u16::try_from(path.len()),
        ) else {
            continue;
        };
        if state.known.get(path) != Some(hash) {
            continue;
        }
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&stamp.to_le_bytes());
        out.extend_from_slice(&hash.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        out.extend_from_slice(payload);
        count += 1;
    }
    out[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());
    let checksum = fast_hash(&out);
    out.extend_from_slice(&checksum.to_le_bytes());
    fs.write(SNAPSHOT_FILE, &out)
        .map_err(|e| FileError::io(SNAPSHOT_FILE, e))?;
    state.stamps.extend(vouched);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_refuses_garbage_without_panicking() {
        assert!(decode(Vec::new()).is_err());
        assert!(decode(vec![0; 7]).is_err());
        assert!(decode(vec![0; 64]).is_err());
        // A correct checksum over a lying body.
        let mut body = MAGIC.to_vec();
        body.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        body.extend_from_slice(&u32::MAX.to_le_bytes());
        let sum = fast_hash(&body);
        body.extend_from_slice(&sum.to_le_bytes());
        assert!(decode(body).is_err());
    }
}
