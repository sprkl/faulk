//! The offline render of an export job: the same streams the page's export Worker and the
//! helper render (`commands::export::job`, through `project_wasm::Core::export_job`), run by
//! `engine::offline` on this thread, with the audio files read from the project's folder.

use commands::CommandError;
use commands::export::{ExportPlan, offline_plan};
use commands::host::ExportReport;
use engine::offline::{Offline, Phase};
use project::TrackId;
use project_wasm::Core;

use crate::folder::Media;

/// Frames per step: the helper's export steps by as much.
pub const STEP_FRAMES: usize = 16_384;

/// A finished render.
pub struct Rendered {
    /// The whole WAV file.
    pub wav: Vec<u8>,
    pub plan: ExportPlan,
    /// The tracks by engine slot of the render: what the measures are of.
    pub tracks: Vec<TrackId>,
    /// What the content measured, when the engine measured it.
    pub measure: Option<engine::offline::Measure>,
    /// The output's peak, linear.
    pub peak: f32,
    /// Frames the output protection silenced for a non-finite sample.
    pub silenced: u64,
    /// The media (SHA-256, hex) of the audio files that could not be read: their clips are
    /// silent in the file.
    pub missing: Vec<String>,
}

/// Why nothing was rendered.
#[derive(Debug)]
pub enum RenderError {
    /// The project cannot be exported as asked.
    Refused(CommandError),
    /// The render could not run.
    Failed(String),
}

/// Prepares the job of `request` (the fields of `project.export_audio`, or
/// `{"analysis": true, ...}`) and renders it.
pub fn render(core: &Core, request: &[u8], media: &Media) -> Result<Rendered, RenderError> {
    let (report, bytes) = core.export_job(request);
    let (plan, tracks, setup_len, start_len) = match report {
        ExportReport::Ready {
            plan,
            tracks,
            setup_len,
            start_len,
        } => (plan, tracks, setup_len as usize, start_len as usize),
        ExportReport::Refused { error } => return Err(RenderError::Refused(error)),
        ExportReport::Failed { stream_error } => {
            return Err(RenderError::Failed(format!(
                "the project's engine messages did not encode ({stream_error})"
            )));
        }
    };
    if setup_len + start_len > bytes.len() {
        return Err(RenderError::Failed(
            "the export job's streams are short".into(),
        ));
    }
    let (setup, rest) = bytes.split_at(setup_len);
    let (start, stop) = rest.split_at(start_len);
    let failed = |what: &str, e: engine::offline::OfflineError| {
        RenderError::Failed(format!("{what}: {e:?}"))
    };
    let mut offline = Offline::new(plan.settings.sample_rate, offline_plan(&plan))
        .map_err(|e| failed("the render is refused", e))?;
    offline.set_stop(stop);
    offline.set_start(start);
    offline
        .submit(setup)
        .map_err(|e| failed("the setup is refused", e))?;
    let mut pcm = Vec::new();
    loop {
        let phase = offline
            .step(STEP_FRAMES)
            .map_err(|e| failed("the render stopped", e))?;
        pcm.extend_from_slice(offline.pcm());
        if phase == Phase::Done {
            break;
        }
        for job in offline.take_disk_jobs() {
            match media.read(&job) {
                Ok((bytes, len)) => offline.disk_done(job.id, &bytes, len),
                Err(why) => offline.disk_failed(job.id, why),
            }
        }
    }
    let mut wav = offline.header();
    wav.extend_from_slice(&pcm);
    Ok(Rendered {
        wav,
        plan,
        tracks,
        measure: offline.measure().cloned(),
        peak: offline.peak(),
        silenced: offline.silenced(),
        missing: offline
            .missing()
            .iter()
            .map(|m| m.iter().map(|b| format!("{b:02x}")).collect())
            .collect(),
    })
}
