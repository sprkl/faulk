//! The audio export in Boost mode (#319): the helper renders the page's export job and writes
//! the WAV file into the project's folder.
//!
//! The page owns the project in both modes: it builds the job (`commands::export::job`, the
//! same streams its own export Worker renders) and sends it (`export.start`). The helper renders
//! it with `engine::offline`, the same engine code the Worker runs in WASM, on a thread of its
//! own with a fresh engine (never the live one, never an audio thread), serves the render's
//! reads of the audio clips' files from the same folder (by hash, [`ProjectFolder::read_media`]),
//! and writes the file through AU-7's confined write ([`ProjectFolder::begin`]): into
//! `.faulk/tmp/` as it is made, then moved to `exports/<name>.wav` once complete, an existing
//! file being kept ([`Collision::Unique`]). A cancel, a failure, or a change of the session's
//! folder drops the write: nothing is left, not even the temporary file.

use crate::disk::{Collision, ProjectFolder, export_path};
use base64::Engine as _;
use engine::offline::{MAX_BODY_SECS, Offline, OfflineError, Phase, SampleFormat};
use protocol::errors;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Frames per step: the page's export Worker steps by as much.
pub const STEP_FRAMES: usize = 16_384;
/// Steps in a row that may wait for the disk with no read handed over before the render counts as
/// stuck (the page's Worker fails at the first one; a few more cost nothing).
const MAX_IDLE_WAITS: u32 = 64;

/// An export the page asked for, checked (`export.start`).
#[derive(Clone, Debug, PartialEq)]
pub struct ExportRequest {
    pub id: u32,
    pub project: String,
    /// The file's path in the folder ([`export_path`]).
    pub path: String,
    pub sample_rate: u32,
    pub format: SampleFormat,
    pub body_frames: u64,
    pub setup: Vec<u8>,
    pub start: Vec<u8>,
    pub stop: Vec<u8>,
}

/// Why an export ended without a file: a stable `code` (`protocol::errors::EXPORT_*`) and a
/// message for developers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportFailure {
    pub code: &'static str,
    pub message: String,
}

impl ExportFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(errors::EXPORT_INVALID, message)
    }
}

/// The fields of `export.start`.
pub struct StartFields<'a> {
    pub id: u32,
    pub project: &'a str,
    pub name: &'a str,
    pub sample_rate: u32,
    pub bits: u32,
    pub body_frames: u64,
    pub setup: &'a str,
    pub start: &'a str,
    pub stop: &'a str,
}

/// Checks `export.start`: a project id, a file name ([`export_path`]), a rate a file can have,
/// 16, 24 or 32 bits, a content of 1 frame to 30 minutes, and three base64 streams. Nothing is
/// rendered or written for a refused one.
pub fn parse(fields: StartFields<'_>) -> Result<ExportRequest, ExportFailure> {
    project::ProjectId::parse(fields.project)
        .map_err(|_| ExportFailure::invalid("the project id is not a ULID"))?;
    let path = export_path(fields.name)
        .map_err(|e| ExportFailure::invalid(format!("the file name is refused ({})", e.code())))?;
    if !commands::export::SAMPLE_RATES.contains(&fields.sample_rate) {
        return Err(ExportFailure::invalid(format!(
            "a sample rate is one of {:?}",
            commands::export::SAMPLE_RATES
        )));
    }
    let format = SampleFormat::from_bits(fields.bits)
        .ok_or_else(|| ExportFailure::invalid("bits are 16, 24 or 32"))?;
    if fields.body_frames == 0
        || fields.body_frames as f64 > MAX_BODY_SECS * f64::from(fields.sample_rate)
    {
        return Err(ExportFailure::invalid(
            "the content lasts from one frame to 30 minutes",
        ));
    }
    // Bounded before anything is decoded: base64 holds 3 bytes per 4 characters.
    let encoded = fields.setup.len() + fields.start.len() + fields.stop.len();
    if encoded / 4 * 3 > protocol::MAX_EXPORT_STREAM_BYTES + 6 {
        return Err(ExportFailure::invalid(format!(
            "the streams hold more than {} bytes",
            protocol::MAX_EXPORT_STREAM_BYTES
        )));
    }
    let decode = |what: &str, text: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(|_| ExportFailure::invalid(format!("the {what} stream is not base64")))
    };
    Ok(ExportRequest {
        id: fields.id,
        project: fields.project.to_owned(),
        path,
        sample_rate: fields.sample_rate,
        format,
        body_frames: fields.body_frames,
        setup: decode("setup", fields.setup)?,
        start: decode("start", fields.start)?,
        stop: decode("stop", fields.stop)?,
    })
}

/// A finished export: where the file is and what the render measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Exported {
    /// The file's path in the folder, `/` separated.
    pub path: String,
    pub bytes: u64,
    pub written: u64,
    pub peak: f32,
    pub clipped: u64,
    pub reduction: f32,
    pub silenced: u64,
    pub tail_cut: bool,
    /// The media (hex SHA-256) whose file could not be read.
    pub missing: Vec<String>,
}

/// How far a render is: frames of the content rendered, of how many, and whether the tail rings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub rendered: u64,
    pub body: u64,
    pub tail: bool,
}

/// What a run checks between two steps: `Some(failure)` ends it without a file.
pub trait Watch {
    /// The export was cancelled, or the session's folder changed.
    fn stop(&self) -> Option<ExportFailure>;
    /// The export was cancelled.
    fn cancelled(&self) -> bool;
}

/// The watch of a run in the helper: its cancel flag, and the folder it began in, which must
/// stay the session's open folder. The same folder named again (the page sends `folder.open`
/// after each `hello`, so after an output change) is the same folder: compared by its canonical
/// root and project, not by the open folder's generation, which every naming moves.
pub struct RunWatch<'a> {
    pub cancel: &'a AtomicBool,
    pub open: &'a crate::disk::OpenFolder,
    pub folder: &'a ProjectFolder,
}

impl Watch for RunWatch<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    fn stop(&self) -> Option<ExportFailure> {
        if self.cancel.load(Ordering::Acquire) {
            return Some(ExportFailure::new(
                errors::EXPORT_CANCELLED,
                "the export was cancelled",
            ));
        }
        let same = self.open.get().1.is_some_and(|now| {
            now.root() == self.folder.root() && now.project() == self.folder.project()
        });
        if !same {
            return Some(ExportFailure::new(
                errors::EXPORT_FOLDER_CHANGED,
                "the session's project folder changed during the export",
            ));
        }
        None
    }
}

fn render_error(error: OfflineError) -> ExportFailure {
    ExportFailure::new(errors::EXPORT_RENDER, format!("{error:?}"))
}

fn write_error(error: crate::disk::PathError) -> ExportFailure {
    match error {
        crate::disk::PathError::FolderChanged => {
            ExportFailure::new(errors::EXPORT_FOLDER_CHANGED, error.code())
        }
        crate::disk::PathError::Io(detail) => {
            ExportFailure::new(errors::EXPORT_WRITE, format!("path.io: {detail}"))
        }
        other => ExportFailure::new(errors::EXPORT_WRITE, other.code()),
    }
}

/// Renders `request` and writes its file in `folder` (module doc). `watch` is asked between
/// steps and before the commit; `progress` is told at most every `every`. On any failure the
/// write is dropped: no file, no temporary file.
pub fn run(
    request: &ExportRequest,
    folder: &ProjectFolder,
    watch: &dyn Watch,
    every: Duration,
    mut progress: impl FnMut(Progress),
) -> Result<Exported, ExportFailure> {
    let plan = engine::offline::Plan {
        body_frames: request.body_frames,
        format: request.format,
    };
    let mut offline = Offline::new(request.sample_rate, plan).map_err(render_error)?;
    offline.set_stop(&request.stop);
    offline.set_start(&request.start);
    offline.submit(&request.setup).map_err(render_error)?;
    let mut file = folder.begin(&request.path).map_err(write_error)?;
    // The header's room: it is known at the end only.
    let header_len = offline.header_len();
    file.write(&vec![0u8; header_len]).map_err(write_error)?;
    let mut bytes = header_len as u64;
    let mut told = Instant::now();
    let mut idle_waits = 0u32;
    loop {
        if let Some(stop) = watch.stop() {
            return Err(stop);
        }
        let phase = offline.step(STEP_FRAMES).map_err(render_error)?;
        let pcm = offline.pcm();
        let pcm_len = pcm.len();
        if !pcm.is_empty() {
            file.write(pcm).map_err(write_error)?;
            bytes += pcm.len() as u64;
        }
        if phase == Phase::Done {
            break;
        }
        if told.elapsed() >= every {
            told = Instant::now();
            progress(Progress {
                rendered: offline.rendered().min(offline.body_frames()),
                body: offline.body_frames(),
                tail: phase == Phase::Tail,
            });
        }
        // The step stopped before a block whose reads are not served: served from the folder.
        let jobs = offline.take_disk_jobs();
        // A wait with no read to make would never end: a failure, not a thread spinning on a core.
        if jobs.is_empty() && pcm_len == 0 && offline.waiting() {
            idle_waits += 1;
            if idle_waits > MAX_IDLE_WAITS {
                return Err(ExportFailure::new(
                    errors::EXPORT_RENDER,
                    "the render waits for a read it never gave",
                ));
            }
        } else {
            idle_waits = 0;
        }
        for job in jobs {
            if let Some(stop) = watch.stop() {
                return Err(stop);
            }
            match folder.read_media(job.file, &job.media, job.offset, job.len) {
                Ok(read) => offline.disk_done(job.id, &read.bytes, read.file_len),
                Err(why) => offline.disk_failed(job.id, why),
            }
        }
    }
    file.write_at_start(&offline.header())
        .map_err(write_error)?;
    if let Some(stop) = watch.stop() {
        return Err(stop);
    }
    let path = file.commit(Collision::Unique).map_err(write_error)?;
    // A cancel that came during the move: the file it made goes again (it is the export's own,
    // just made), so a cancelled export leaves nothing. One later than this check finds the
    // export done; the page, which has moved on, says nothing of it.
    if watch.cancelled() {
        let _ = folder.remove_own(&path);
        return Err(ExportFailure::new(
            errors::EXPORT_CANCELLED,
            "the export was cancelled",
        ));
    }
    Ok(Exported {
        path,
        bytes,
        written: offline.written(),
        peak: offline.peak(),
        clipped: offline.clipped(),
        reduction: offline.reduction(),
        silenced: offline.silenced(),
        tail_cut: offline.tail_cut(),
        missing: offline
            .missing()
            .iter()
            .map(|m| m.iter().map(|b| format!("{b:02x}")).collect())
            .collect(),
    })
}

/// Lowers the calling thread's priority below normal (Windows): an export renders faster than
/// real time on a full core, and must leave the live output's callback (and the page) ahead of
/// it. Elsewhere the thread keeps the default priority. Not measured: no claim is made on the
/// live callback's underruns during an export.
pub fn lower_priority() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        // SAFETY: `GetCurrentThread` returns a pseudo handle valid on the calling thread, which
        // `SetThreadPriority` only reads.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
    }
}
