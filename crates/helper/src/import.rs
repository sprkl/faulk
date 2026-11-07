//! The native audio import in Boost (#778): the helper imports an audio file into the project's
//! folder, with the page's own decoder (`crates/audio-import`, natively).
//!
//! The page owns the project in both modes: it names the file (`import.start`) and, once the
//! helper says it is in, makes the clip with its commands (`pw_stage_media`, then
//! `clip.import_audio`). Nothing of the file crosses the link. The source is a file of the
//! project's folder ([`ProjectFolder::open_source`]: a path relative to it, checked as AU-7's
//! paths are), or a file the user picked in the helper's own file dialog (`import.choose`), the one
//! read outside the folder, by the user's choice.
//!
//! The steps are the page's (`web/src/lib/audio/import/pipeline.ts`), so the folder ends up with
//! the same files, byte for byte (the PCM of MP3 and AAC aside, see § 5.9 of the storage
//! architecture): (1) the file is copied into `.faulk/tmp/` while its SHA-256 is computed; (2) the
//! copy is decoded, every packet (`audio_import::Decoding`, capped at `MAX_DECODED_BYTES` like the
//! page's, #780), the PCM cache written into another temporary file unless the file is read
//! directly, and its waveform peaks computed; (3) only then the cache is moved to
//! `.faulk/cache/pcm/<sha256>.pcm`, the copy to `blobs/<sha256>` (kept when the folder has that
//! blob already) and the peaks to `.faulk/cache/peaks/<sha256>.peaks` (a failure there does not fail
//! the import, as in the page). Every place is built from the hash ([`PendingWrite::commit_media`]).
//! The copy is what is decoded: a source changed while it is read gives a blob whose bytes are the
//! ones hashed and decoded. A cancel, a failure or a change of the session's folder before (3)
//! leaves nothing, not even a temporary file.

use crate::disk::{MediaFile, OpenFolder, PathError, PendingWrite, ProjectFolder, SourceFile};
use audio_import::{Decoding, ImportError, MediaInfo, PcmLayout, Sha256, Source, cache, peaks};
use protocol::errors;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The bytes copied, or of PCM cache written, at once.
pub const SLICE: usize = 1 << 20;

/// Why an import ended without its file in the folder: a stable `code` (`protocol::errors::IMPORT_*`,
/// or `import.<reason>` of the decoder) and a message for developers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportFailure {
    pub code: String,
    pub message: String,
}

impl ImportFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// A file the decoder refuses: `import.<reason>`, the codes an agent gets from the page's import.
pub fn decode_failure(error: ImportError) -> ImportFailure {
    ImportFailure::new(
        format!("{}{}", errors::IMPORT_DECODE_PREFIX, error.code()),
        error.to_string(),
    )
}

/// A write of the import refused by the folder.
pub fn write_failure(error: PathError) -> ImportFailure {
    match error {
        PathError::FolderChanged => ImportFailure::new(errors::IMPORT_FOLDER_CHANGED, error.code()),
        PathError::Io(detail) => {
            ImportFailure::new(errors::IMPORT_WRITE, format!("path.io: {detail}"))
        }
        other => ImportFailure::new(errors::IMPORT_WRITE, other.code()),
    }
}

/// A source the folder refuses: its path (`import.path`, with the path's code), or a file that
/// cannot be read (`import.unreadable`).
pub fn source_failure(error: PathError) -> ImportFailure {
    match error {
        PathError::FolderChanged => ImportFailure::new(errors::IMPORT_FOLDER_CHANGED, error.code()),
        PathError::NotFound | PathError::IsDirectory => {
            ImportFailure::new(errors::IMPORT_UNREADABLE, error.code())
        }
        PathError::Io(detail) => {
            ImportFailure::new(errors::IMPORT_UNREADABLE, format!("path.io: {detail}"))
        }
        other => ImportFailure::new(errors::IMPORT_PATH, other.code()),
    }
}

fn unreadable(error: std::io::Error) -> ImportFailure {
    ImportFailure::new(
        errors::IMPORT_UNREADABLE,
        format!("the file could not be read: {error}"),
    )
}

/// Opens a file the user picked in the helper's dialog: a regular file only.
pub fn open_picked(path: &Path) -> Result<SourceFile, ImportFailure> {
    let meta = std::fs::metadata(path).map_err(unreadable)?;
    if !meta.is_file() {
        return Err(ImportFailure::new(errors::IMPORT_UNREADABLE, "not a file"));
    }
    let file = File::open(path).map_err(unreadable)?;
    let len = file.metadata().map_err(unreadable)?.len();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(SourceFile { file, len, name })
}

/// The decoder's hint, as the page takes it from a file's name (`extensionOf`): the extension in
/// lower case, 1 to 8 ASCII letters or digits; none otherwise.
pub fn extension_of(name: &str) -> Option<String> {
    let at = name.rfind('.').filter(|at| *at > 0)?;
    let ext = name[at + 1..].to_lowercase();
    ((1..=8).contains(&ext.len())
        && ext
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
    .then_some(ext)
}

/// A finished import: the media, as `clip.import_audio` takes it, and what was written.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub hash: project::Hash,
    pub info: MediaInfo,
    /// The source file's own name.
    pub name: String,
    /// Read directly from the blob (no PCM cache).
    pub direct: bool,
    /// This import wrote `blobs/<hash>` (the folder did not hold it).
    pub created: bool,
    /// Its waveform peaks were written.
    pub peaks: bool,
}

/// What a run checks between two steps: `Some(failure)` ends it without a file.
pub trait Watch {
    fn stop(&self) -> Option<ImportFailure>;
}

/// The watch of a run in the helper: its cancel flag, and the folder it began in, which must stay
/// the session's open folder (the same canonical root and project: the same folder named again
/// after a `hello` is the same folder).
pub struct RunWatch<'a> {
    pub cancel: &'a AtomicBool,
    pub open: &'a OpenFolder,
    pub folder: &'a ProjectFolder,
}

impl Watch for RunWatch<'_> {
    fn stop(&self) -> Option<ImportFailure> {
        if self.cancel.load(Ordering::Acquire) {
            return Some(ImportFailure::new(
                errors::IMPORT_CANCELLED,
                "the import was cancelled",
            ));
        }
        let same = self.open.get().1.is_some_and(|now| {
            now.root() == self.folder.root() && now.project() == self.folder.project()
        });
        if !same {
            return Some(ImportFailure::new(
                errors::IMPORT_FOLDER_CHANGED,
                "the session's project folder changed during the import",
            ));
        }
        None
    }
}

/// A temporary copy read by the decoder, of known length; counts how far it was read.
struct Copy {
    file: File,
    len: u64,
    position: Arc<AtomicU64>,
}

impl Read for Copy {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read(buf)?;
        let at = self.file.stream_position()?;
        self.position.fetch_max(at, Ordering::Relaxed);
        Ok(n)
    }
}

impl Seek for Copy {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl Source for Copy {
    fn byte_len(&self) -> u64 {
        self.len
    }
}

/// What a decode gave: the metadata and the waveform peaks (`None` when memory was refused).
struct Decoded {
    info: MediaInfo,
    peaks: Option<Vec<u8>>,
}

/// Decodes `file` (`len` bytes), every packet, writing the PCM cache into `cache` when given (its
/// header at the end, once the frames are known). Progress goes from 0 to 1 as the decoder reads.
fn decode(
    file: File,
    len: u64,
    hint: Option<&str>,
    mut cache: Option<&mut PendingWrite<'_>>,
    watch: &dyn Watch,
    max_bytes: u64,
    mut progress: impl FnMut(f64),
) -> Result<Decoded, ImportFailure> {
    let position = Arc::new(AtomicU64::new(0));
    let source = Copy {
        file,
        len,
        position: position.clone(),
    };
    let mut decoding =
        Decoding::open_capped(Box::new(source), hint, max_bytes).map_err(decode_failure)?;
    // Opening may read up to the end of the file (as the page's module counts it): the progress
    // counts the reads of the decoding.
    position.store(0, Ordering::Relaxed);
    let mut out = Vec::with_capacity(SLICE);
    if let Some(cache) = cache.as_mut() {
        // The header's room: it is known at the end only.
        cache
            .write(&[0u8; cache::HEADER_BYTES])
            .map_err(write_failure)?;
    }
    let mut block = Vec::new();
    // Made at the first block: some headers give no channel count, the first block does.
    let mut builder: Option<peaks::PeakBuilder> = None;
    while decoding.next(&mut block).map_err(decode_failure)? {
        if let Some(stop) = watch.stop() {
            return Err(stop);
        }
        builder
            .get_or_insert_with(|| peaks::PeakBuilder::new(decoding.channels()))
            .push(&block);
        if let Some(cache) = cache.as_mut() {
            cache::encode(&block, &mut out);
            if out.len() >= SLICE {
                cache.write(&out).map_err(write_failure)?;
                out.clear();
            }
        }
        if len > 0 {
            progress(position.load(Ordering::Relaxed) as f64 / len as f64);
        }
    }
    let info = decoding.finish().map_err(decode_failure)?;
    if let Some(cache) = cache {
        cache.write(&out).map_err(write_failure)?;
        cache
            .write_at_start(&cache::header(&info))
            .map_err(write_failure)?;
    }
    progress(1.0);
    Ok(Decoded {
        info,
        peaks: builder.and_then(|p| p.finish(&info)),
    })
}

/// The first bytes of `file` (`audio_import::layout::HEAD_BYTES` at most).
fn head(mut file: File) -> Result<Vec<u8>, ImportFailure> {
    let mut head = Vec::with_capacity(audio_import::layout::HEAD_BYTES);
    (&mut file)
        .take(audio_import::layout::HEAD_BYTES as u64)
        .read_to_end(&mut head)
        .map_err(unreadable)?;
    Ok(head)
}

/// Imports `source` into `folder` (module doc). `watch` is asked between slices and blocks and
/// before anything is put in place; `progress` is told the fraction done (0 to 1) as often as it
/// moves (the caller paces what it sends). `max_bytes` caps the decoded size
/// (`audio_import::MAX_DECODED_BYTES` in the helper; a test lowers it).
pub fn run(
    source: SourceFile,
    folder: &ProjectFolder,
    watch: &dyn Watch,
    max_bytes: u64,
    mut progress: impl FnMut(f64),
) -> Result<Imported, ImportFailure> {
    let SourceFile {
        mut file,
        len,
        name,
    } = source;
    // (1) The hash, and a copy of the bytes it was computed on.
    let mut copy = folder.begin_media().map_err(write_failure)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; SLICE];
    let mut size = 0u64;
    loop {
        if let Some(stop) = watch.stop() {
            return Err(stop);
        }
        let n = match file.read(&mut buffer) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(unreadable(e)),
        };
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        copy.write(&buffer[..n]).map_err(write_failure)?;
        size += n as u64;
        progress(0.5 * (size as f64 / len.max(size).max(1) as f64));
    }
    drop(file);
    copy.finish().map_err(write_failure)?;
    let hash = hasher.finish();
    // (2) Decoding, from the copy.
    let hint = extension_of(&name);
    let start = head(copy.reopen().map_err(write_failure)?)?;
    // A file that looks read directly is decoded without a cache, and checked afterwards; in the
    // rare case the check fails (the headers and the decoder disagree), it is decoded again into one.
    let maybe_direct = PcmLayout::parse(&start, size).is_some();
    let mut pcm = if maybe_direct {
        None
    } else {
        Some(folder.begin_media().map_err(write_failure)?)
    };
    let decoded = decode(
        copy.reopen().map_err(write_failure)?,
        size,
        hint.as_deref(),
        pcm.as_mut(),
        watch,
        max_bytes,
        |f| progress(0.5 + 0.5 * f),
    )?;
    let info = decoded.info;
    let direct = maybe_direct
        && PcmLayout::parse(&start, size)
            .is_some_and(|l| l.matches(info.sample_rate, info.channels, info.frames));
    if maybe_direct && !direct {
        let mut again = folder.begin_media().map_err(write_failure)?;
        decode(
            copy.reopen().map_err(write_failure)?,
            size,
            hint.as_deref(),
            Some(&mut again),
            watch,
            max_bytes,
            |_| {},
        )?;
        pcm = Some(again);
    }
    if let Some(stop) = watch.stop() {
        return Err(stop);
    }
    // (3) Into place: the cache first, the blob last. A failure between them leaves a cache, which
    // nothing reads without its blob (and which a later import makes again), never a blob without
    // its cache.
    if let Some(pcm) = pcm {
        pcm.commit_media(MediaFile::Pcm, &hash)
            .map_err(write_failure)?;
    }
    let created = copy
        .commit_media(MediaFile::Blob, &hash)
        .map_err(write_failure)?;
    // The peaks last: a cache, made again by the page at the next opening when this write fails.
    let peaks = decoded.peaks.is_some_and(|bytes| {
        folder
            .begin_media()
            .and_then(|mut w| {
                w.write(&bytes)?;
                w.commit_media(MediaFile::Peaks, &hash)
            })
            .is_ok()
    });
    progress(1.0);
    Ok(Imported {
        hash,
        info,
        name,
        direct,
        created,
        peaks,
    })
}

/// Lowers the calling thread's priority below normal (Windows), as the export does: a decode runs
/// on a full core and must leave the live output's callback ahead of it.
pub fn lower_priority() {
    crate::export::lower_priority();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Stops the run at its `at`-th look (never when 0), counting the looks.
    struct StopAt {
        at: u64,
        looks: Cell<u64>,
    }

    impl Watch for StopAt {
        fn stop(&self) -> Option<ImportFailure> {
            let n = self.looks.get() + 1;
            self.looks.set(n);
            (n == self.at).then(|| ImportFailure::new(errors::IMPORT_CANCELLED, "stopped"))
        }
    }

    /// An 8-bit WAV of `frames` frames of noise, stereo, 48 kHz: not read directly (a cache).
    fn wav_u8(frames: u32) -> Vec<u8> {
        let data = frames * 2;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 2] {
            wav.extend_from_slice(&v.to_le_bytes());
        }
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&96_000u32.to_le_bytes());
        for v in [2u16, 8] {
            wav.extend_from_slice(&v.to_le_bytes());
        }
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data.to_le_bytes());
        let mut x: u32 = 7;
        for _ in 0..data {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            wav.push((x >> 24) as u8);
        }
        wav
    }

    struct Dir(std::path::PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Every file below `dir` but `project.json` and the source, relative.
    fn written(dir: &Path) -> Vec<String> {
        fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    out.push(
                        path.strip_prefix(base)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.retain(|f| f != "project.json" && f != "src.wav");
        out.sort();
        out
    }

    #[test]
    fn a_stop_at_any_step_before_the_files_are_put_in_place_leaves_nothing() {
        let dir = std::env::temp_dir().join(format!(
            "faulk-778-steps-{}",
            crate::security::random_hex(6)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = Dir(std::fs::canonicalize(&dir).unwrap());
        std::fs::write(
            dir.0.join("project.json"),
            "{\"id\": \"01J0000000000000000000000A\"}",
        )
        .unwrap();
        // Three slices of copy, then a few hundred blocks of decode.
        let wav = wav_u8((SLICE as u32) + 200_000);
        std::fs::write(dir.0.join("src.wav"), &wav).unwrap();
        let folder = ProjectFolder::open(&dir.0, None).unwrap();
        let open = || folder.open_source("src.wav").unwrap();
        // A run that never stops: how many looks it takes, and what it writes.
        let all = StopAt {
            at: 0,
            looks: Cell::new(0),
        };
        let done = run(
            open(),
            &folder,
            &all,
            audio_import::MAX_DECODED_BYTES,
            |_| {},
        )
        .unwrap();
        assert!(!done.direct && done.created);
        let total = all.looks.get();
        let files = written(&dir.0);
        assert_eq!(files.len(), 3, "{files:?}");
        for f in &files {
            std::fs::remove_file(dir.0.join(f)).unwrap();
        }
        // Stopped at the first, second and third slice of the copy, the first, middle and last
        // block of the decode, and the look before the files are put in place.
        for at in [1, 2, 3, 4, 5, total / 2, total - 1, total] {
            let watch = StopAt {
                at,
                looks: Cell::new(0),
            };
            let end = run(
                open(),
                &folder,
                &watch,
                audio_import::MAX_DECODED_BYTES,
                |_| {},
            );
            assert_eq!(end.unwrap_err().code, errors::IMPORT_CANCELLED, "at {at}");
            assert!(written(&dir.0).is_empty(), "at {at}: {:?}", written(&dir.0));
        }
    }

    #[test]
    fn the_decoders_hint_is_the_pages() {
        for (name, hint) in [
            ("a.WAV", Some("wav")),
            ("kick.flac", Some("flac")),
            ("a.b.Mp3", Some("mp3")),
            (".wav", None),
            ("noext", None),
            ("a.", None),
            ("a.toolongext", None),
            ("a.wa v", None),
            ("a.wäv", None),
        ] {
            assert_eq!(extension_of(name).as_deref(), hint, "{name}");
        }
    }
}
