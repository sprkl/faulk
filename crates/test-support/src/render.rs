//! The native render, and the native/WASM comparison.

use crate::scenario::{Scenario, Step};
use crate::signal::Render;
use crate::wasm::render_wasm;

pub const SAMPLE_RATE: f32 = 48_000.0;
/// Frames per block: the AudioWorklet's render quantum.
pub const QUANTUM: u32 = 128;
/// libm differs slightly between the native targets and wasm32 (sin, exp2, tanh): the
/// largest sample difference the two builds may show on the same scenario.
pub const PARITY_TOLERANCE: f32 = 1e-5;

/// Renders the scenario through `engine::host`, the native target, behind the helper's queue
/// of streams (`StreamQueue`, #245: a stream longer than the engine takes at once waits and
/// goes in as it consumes), flushing it before each block as the real hosts do. Panics when
/// the engine rejected or skipped a message, or when one is still pending or waiting at the
/// end.
pub fn render_native(scenario: &Scenario) -> Render {
    render_native_with_disk(scenario, &Disk::default())
}

/// The files of a project folder the engine's disk planner reads (AU-3): by media (the
/// blob's SHA-256) and kind (`engine::audio_source::FileKind`: the blob, or its PCM cache).
#[derive(Clone, Debug, Default)]
pub struct Disk {
    pub files: std::collections::HashMap<([u8; 32], u8), Vec<u8>>,
}

impl Disk {
    /// Puts `bytes` as the blob of `media`.
    pub fn blob(&mut self, media: [u8; 32], bytes: Vec<u8>) -> &mut Self {
        self.files
            .insert((media, engine::audio_source::FileKind::Blob as u8), bytes);
        self
    }

    /// What `job` reads: its bytes and the file's length, or why it cannot be read.
    pub fn read(
        &self,
        job: &engine::audio_source::Job,
    ) -> Result<(&[u8], u64), engine::audio_source::Failure> {
        match self.files.get(&(job.media, job.file as u8)) {
            Some(file) => {
                let from = (job.offset as usize).min(file.len());
                let to = from.saturating_add(job.len as usize).min(file.len());
                Ok((&file[from..to], file.len() as u64))
            }
            None => Err(engine::audio_source::Failure::NotFound),
        }
    }

    /// Serves the planner's jobs at `now_ms`: each read at once, from the files.
    fn serve(&self, prepare: &mut engine::host::Prepare, now_ms: u64) {
        let jobs = prepare.disk_poll(now_ms).to_vec();
        for job in jobs {
            match self.read(&job) {
                Ok((bytes, len)) => prepare.disk_done(job.id, bytes, len),
                Err(why) => prepare.disk_failed(job.id, why),
            }
        }
    }
}

/// [`render_native`] with a project folder's audio files (AU-4): after each block the disk
/// planner's reads are served from `disk` (its clock is the rendered time), as the page's
/// executor serves them from the folder.
pub fn render_native_with_disk(scenario: &Scenario, disk: &Disk) -> Render {
    let (prepare, mut host) = engine::host::channel(SAMPLE_RATE).expect("the channel opens");
    let mut queue = engine::host::StreamQueue::new(prepare);
    let mut frames = 0u64;
    let mut left = Vec::new();
    let mut right = Vec::new();
    for step in &scenario.steps {
        match step {
            Step::Push(bytes) => queue.send(bytes).expect("the stream is accepted"),
            Step::Render(blocks) => {
                for _ in 0..*blocks {
                    // The control thread's regular flush: what waited behind a full ring.
                    queue.flush().expect("the stream is accepted");
                    let n = host.render(QUANTUM as usize);
                    left.extend_from_slice(&host.left()[..n]);
                    right.extend_from_slice(&host.right()[..n]);
                    frames += n as u64;
                    let now_ms = (frames as f64 * 1000.0 / f64::from(SAMPLE_RATE)) as u64;
                    disk.serve(queue.prepare_mut(), now_ms);
                }
            }
        }
    }
    queue.flush().expect("the stream is accepted");
    let prepare = queue.prepare().preparer();
    assert_eq!(prepare.rejected(), 0, "messages were rejected");
    assert_eq!(
        prepare.pending(),
        0,
        "messages are still pending after the last block"
    );
    assert_eq!(
        queue.waiting(),
        0,
        "streams still wait for the engine after the last block"
    );
    assert_eq!(host.engine().skipped(), 0, "messages were skipped");
    // The output protection (#402) is the last stage, so `assert_finite`/`assert_clean` on the
    // returned render pass by construction (a NaN is silenced, a runaway clamped). This catches a
    // blow-up on any path the engine ran — the master strip, the master chain, automation, the
    // summing, a device — whose output assertion can no longer see it.
    assert_eq!(
        host.engine().limiter_silenced(),
        0,
        "the output protection silenced non-finite samples: a device or a master-path produced NaN or infinity"
    );
    Render::new(SAMPLE_RATE, left, right)
}

/// Renders the scenario natively and in WASM, asserts the two agree within
/// [`PARITY_TOLERANCE`], and returns the native render.
pub fn parity(scenario: &Scenario) -> Render {
    let native = render_native(scenario);
    let wasm = render_wasm(scenario);
    native.assert_close(&wasm, PARITY_TOLERANCE);
    native
}
