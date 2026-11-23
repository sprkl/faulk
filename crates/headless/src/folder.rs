//! The project folder on disk, as the headless host reads and writes it.
//!
//! - Opening: the folder is read into the frame `project_wasm::parse_folder` takes, as the page
//!   builds it from its storage (`readFolder` in `web/src/lib/storage/backend.ts`, `folderFrame`
//!   in `web/src/lib/project/core.ts`): the same files (`project.json`, `tracks/`, `master/`,
//!   `presets/`, `blobs/`, then `.faulk/session.json`, `.gitignore`, `.gitattributes`), every one
//!   whole except the blobs, which are listed by size, the blobs of dense automation lanes
//!   (`FKAUTO01`) excepted. Symbolic links are left out.
//! - Saving: the core's save frame (`crates/project-wasm/src/save.rs`) is parsed and applied
//!   with `project::file::StdFs` (temporary file then rename, paths confined to the folder).
//! - Rendering: the engine's reads of audio files (`blobs/<hash>`, `.faulk/cache/pcm/<hash>.pcm`),
//!   refused when the file's real path leaves the folder.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use engine::audio_source::{Failure, FileKind, Job};
use project::file::{FileSystem, FsError, StdFs};

/// The name the host gives the folder it stages for `project.open`.
pub const STAGED_NAME: &str = "headless";

/// The first bytes of a blob of dense automation points (`crates/project/src/file/unit.rs`).
const LANE_BLOB_MAGIC: &[u8; 8] = b"FKAUTO01";

/// What of the root belongs to the project, as the page's storage scans it
/// (`PROJECT_ROOT_FILES`, `PROJECT_ROOT_DIRS` in `web/src/lib/storage/backend.ts`): a README, a
/// mixdown, the file manager's files are the user's, never staged.
const PROJECT_ROOT_FILES: [&str; 1] = ["project.json"];
const PROJECT_ROOT_DIRS: [&str; 4] = ["tracks", "master", "presets", "blobs"];
/// Directories skipped at any depth (`SKIPPED_DIRS`).
const SKIPPED_DIRS: [&str; 1] = [".git"];
/// The suffix of a browser's swap file (`SWAP_SUFFIX`).
const SWAP_SUFFIX: &str = ".crswap";
/// The files outside the scan that opening reads too (`readFolder`).
const EXTRA_FILES: [&str; 3] = [".faulk/session.json", ".gitignore", ".gitattributes"];

/// The project's files below `root`, as the page's `walk` lists them, then the extra files that
/// exist: relative paths. Symbolic links and names that are not UTF-8 are left out.
pub fn project_files(root: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    let mut pending = vec![String::new()];
    while let Some(dir) = pending.pop() {
        let full = if dir.is_empty() {
            root.to_path_buf()
        } else {
            root.join(&dir)
        };
        let mut entries = fs::read_dir(&full)
            .and_then(|it| it.collect::<Result<Vec<_>, _>>())
            .map_err(|e| format!("{}: {e}", full.display()))?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let kind = entry.file_type().map_err(|e| format!("{name}: {e}"))?;
            let path = if dir.is_empty() {
                name.clone()
            } else {
                format!("{dir}/{name}")
            };
            if kind.is_dir() {
                let wanted = if dir.is_empty() {
                    PROJECT_ROOT_DIRS.contains(&name.as_str())
                } else {
                    !SKIPPED_DIRS.contains(&name.as_str())
                };
                if wanted {
                    pending.push(path);
                }
            } else if kind.is_file()
                && !name.ends_with(SWAP_SUFFIX)
                && (!dir.is_empty() || PROJECT_ROOT_FILES.contains(&name.as_str()))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    for extra in EXTRA_FILES {
        let path = root.join(extra);
        let is_file = fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file());
        if is_file {
            files.push(extra.to_owned());
        }
    }
    Ok(files)
}

/// The folder frame of `root` for `project_wasm::Core::stage` (layout: `parse_folder`).
pub fn folder_frame(root: &Path) -> Result<Vec<u8>, String> {
    let too_big = || {
        format!(
            "the project's files hold more than {} bytes",
            project_wasm::MAX_FOLDER_FRAME
        )
    };
    let mut frame = Vec::new();
    push_text(&mut frame, STAGED_NAME);
    for path in project_files(root)? {
        let blob = path.strip_prefix("blobs/");
        if blob.is_some_and(|name| name.contains('/')) {
            // The format keeps blobs flat: a nested file is none of its.
            continue;
        }
        let full = root.join(&path);
        let size = fs::metadata(&full)
            .map_err(|e| format!("{path}: {e}"))?
            .len();
        push_text(&mut frame, &path);
        if blob.is_some() && !is_lane_blob(&full)? {
            frame.push(1);
            frame.extend_from_slice(&size.to_le_bytes());
        } else {
            // Checked before reading: a file too big for the frame is never loaded.
            if frame.len() as u64 + size > project_wasm::MAX_FOLDER_FRAME as u64 {
                return Err(too_big());
            }
            let bytes = fs::read(&full).map_err(|e| format!("{path}: {e}"))?;
            frame.push(0);
            frame.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            frame.extend_from_slice(&bytes);
        }
        if frame.len() > project_wasm::MAX_FOLDER_FRAME {
            return Err(too_big());
        }
    }
    Ok(frame)
}

fn push_text(frame: &mut Vec<u8>, text: &str) {
    frame.extend_from_slice(&(text.len() as u32).to_le_bytes());
    frame.extend_from_slice(text.as_bytes());
}

fn is_lane_blob(path: &Path) -> Result<bool, String> {
    let mut head = [0u8; 8];
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut read = 0;
    while read < head.len() {
        match file.read(&mut head[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(read == head.len() && &head == LANE_BLOB_MAGIC)
}

/// A write or a removal of a save frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveEntry {
    Write(String, Vec<u8>),
    Remove(String),
}

/// The entries of a save frame of `project_wasm::Core::save`, or the core's failure message.
/// A truncated or malformed frame is refused whole.
pub fn parse_save_frame(bytes: &[u8]) -> Result<Vec<SaveEntry>, String> {
    fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
        if n > rest.len() {
            return Err("the save frame is truncated".to_owned());
        }
        let (head, tail) = rest.split_at(n);
        *rest = tail;
        Ok(head)
    }
    fn u32_of(rest: &mut &[u8]) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            take(rest, 4)?.try_into().map_err(|_| "u32")?,
        ))
    }
    let mut rest = bytes;
    match take(&mut rest, 1)?[0] {
        0 => {}
        1 => return Err(String::from_utf8_lossy(rest).into_owned()),
        other => return Err(format!("unknown save frame status {other}")),
    }
    let _revision = u32_of(&mut rest)?;
    let count = u32_of(&mut rest)?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let kind = take(&mut rest, 1)?[0];
        let len = u32_of(&mut rest)? as usize;
        let path = std::str::from_utf8(take(&mut rest, len)?)
            .map_err(|_| "a path that is not UTF-8")?
            .to_owned();
        match kind {
            0 => {
                let len: [u8; 8] = take(&mut rest, 8)?.try_into().map_err(|_| "u64")?;
                let len = usize::try_from(u64::from_le_bytes(len)).map_err(|_| "too long")?;
                entries.push(SaveEntry::Write(path, take(&mut rest, len)?.to_vec()));
            }
            1 => entries.push(SaveEntry::Remove(path)),
            other => return Err(format!("unknown save entry kind {other}")),
        }
    }
    if !rest.is_empty() {
        return Err("bytes after the save frame's last entry".to_owned());
    }
    Ok(entries)
}

/// Applies the entries in order (the core orders them: blobs, units with `project.json` last,
/// removals, the session). Stops at the first failure.
pub fn apply(fs: &mut StdFs, entries: &[SaveEntry]) -> Result<(), String> {
    for entry in entries {
        match entry {
            SaveEntry::Write(path, bytes) => fs
                .write(path, bytes)
                .map_err(|e| format!("{path}: {e:?}"))?,
            SaveEntry::Remove(path) => match fs.remove(path) {
                Ok(()) | Err(FsError::NotFound) => {}
                Err(e) => return Err(format!("{path}: {e:?}")),
            },
        }
    }
    Ok(())
}

/// The audio files of a folder, for the engine's reads during a render.
pub struct Media {
    /// The folder's real path.
    root: PathBuf,
}

/// The most bytes one read hands over (the helper's `MAX_READ_BYTES`).
const MAX_READ_BYTES: u32 = 4 << 20;

impl Media {
    pub fn new(root: &Path) -> Result<Self, String> {
        let root = fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
        Ok(Self { root })
    }

    /// What `job` reads: the bytes (short past the end) and the file's length.
    pub fn read(&self, job: &Job) -> Result<(Vec<u8>, u64), Failure> {
        let hex: String = job.media.iter().map(|b| format!("{b:02x}")).collect();
        let path = match job.file {
            FileKind::Blob => self.root.join("blobs").join(hex),
            FileKind::Cache => self
                .root
                .join(".faulk")
                .join("cache")
                .join("pcm")
                .join(format!("{hex}.pcm")),
        };
        let real = match fs::canonicalize(&path) {
            Ok(real) => real,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Failure::NotFound),
            Err(_) => return Err(Failure::Unreadable),
        };
        if !real.starts_with(&self.root) {
            return Err(Failure::Unreadable);
        }
        let mut file = fs::File::open(&real).map_err(|_| Failure::Unreadable)?;
        let meta = file.metadata().map_err(|_| Failure::Unreadable)?;
        if !meta.is_file() {
            return Err(Failure::Unreadable);
        }
        let file_len = meta.len();
        let want = u64::from(job.len.min(MAX_READ_BYTES)).min(file_len.saturating_sub(job.offset));
        let mut bytes = Vec::with_capacity(want as usize);
        if want > 0 {
            file.seek(SeekFrom::Start(job.offset))
                .map_err(|_| Failure::Unreadable)?;
            file.take(want)
                .read_to_end(&mut bytes)
                .map_err(|_| Failure::Unreadable)?;
        }
        Ok((bytes, file_len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_project_files_are_staged_as_the_page_scans_them() {
        let root = std::env::temp_dir().join(format!("faulk-headless-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (path, bytes) in [
            ("project.json", &b"{}"[..]),
            ("README.md", b"mine"),
            ("mixdown.wav", b"RIFF"),
            ("tracks/a/track.json", b"{}"),
            ("tracks/a/x.json.crswap", b"swap"),
            ("tracks/.git/HEAD", b"ref"),
            ("blobs/ab", b"audio"),
            ("exports/song.wav", b"RIFF"),
            ("other/file", b"x"),
            (".faulk/session.json", b"{}"),
            (".faulk/cache/pcm/ab.pcm", b"pcm"),
            (".gitignore", b"x"),
        ] {
            let full = root.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, bytes).unwrap();
        }
        assert_eq!(
            project_files(&root).unwrap(),
            [
                "blobs/ab",
                "project.json",
                "tracks/a/track.json",
                ".faulk/session.json",
                ".gitignore"
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_save_frame_round_trips_and_a_bad_one_is_refused() {
        let mut frame = vec![0];
        frame.extend_from_slice(&7u32.to_le_bytes());
        frame.extend_from_slice(&2u32.to_le_bytes());
        frame.push(0);
        frame.extend_from_slice(&3u32.to_le_bytes());
        frame.extend_from_slice(b"a/b");
        frame.extend_from_slice(&2u64.to_le_bytes());
        frame.extend_from_slice(b"hi");
        frame.push(1);
        frame.extend_from_slice(&1u32.to_le_bytes());
        frame.extend_from_slice(b"c");
        assert_eq!(
            parse_save_frame(&frame),
            Ok(vec![
                SaveEntry::Write("a/b".into(), b"hi".to_vec()),
                SaveEntry::Remove("c".into())
            ])
        );
        assert!(parse_save_frame(&frame[..frame.len() - 1]).is_err());
        let mut trailing = frame.clone();
        trailing.push(0);
        assert!(parse_save_frame(&trailing).is_err());
        assert_eq!(parse_save_frame(b"\x01boom"), Err("boom".to_owned()));
        assert!(parse_save_frame(b"\x02").is_err());
        assert!(parse_save_frame(b"").is_err());
    }
}
