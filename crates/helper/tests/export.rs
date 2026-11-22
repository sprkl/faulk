//! The audio export in Boost mode (#319), over a real loopback session: the page sends its
//! export job (`export.start`), the helper renders it natively and writes the WAV file into
//! the project's folder, never anywhere else; the file is the one the page's export Worker
//! makes (the WASM build, `test_support::export::export_job_wasm`) within `PARITY_TOLERANCE`;
//! a cancel, a folder switched or a page gone mid-render leaves nothing, not even a temporary
//! file; every refusal has its code and writes nothing.

use base64::Engine as _;
use commands::export::{ExportBitDepth, ExportJob, ExportRegion, ExportSettings};
use futures_util::{SinkExt, StreamExt};
use helper::audio::{AudioCore, EngineSlot, Meter, audio_core};
use helper::disk::{LINK_FILE, NoPicker, Remembered};
use helper::live::LiveInput;
use helper::midi::{MidiManager, MidiSink};
use helper::security::Policy;
use helper::server::{Shared, serve};
use protocol::{ClientMessage, FolderStatus, ServerMessage, errors};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use test_support::Disk;
use test_support::PARITY_TOLERANCE;
use test_support::export::{Exported, demo_song, export_job_wasm, job_of};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const ORIGIN: &str = "http://localhost:5173";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const PROJECT: &str = "01J0000000000000000000000A";
const OTHER: &str = "01J0000000000000000000000B";
const NONCE: &str = "00112233445566778899aabbccddeeff";
const OTHER_NONCE: &str = "ffeeddccbbaa99887766554433221100";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A directory of its own, removed on drop.
struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    port: u16,
    shared: Arc<Shared>,
    _core: AudioCore,
    _engine: Arc<EngineSlot>,
    _temp: Temp,
    /// The project folder of `PROJECT`, with the page's marker.
    song: PathBuf,
    /// Another project's folder, with its own marker.
    other: PathBuf,
}

/// A project folder `name` of project `id` below `parent`, with the page's marker `nonce` and
/// the files of `disk` (blobs and caches, by hash).
fn project_dir(parent: &Path, name: &str, id: &str, nonce: &str, disk: &Disk) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    std::fs::create_dir_all(dir.join(".faulk").join("cache").join("pcm")).unwrap();
    std::fs::write(
        dir.join("project.json"),
        format!("{{\"id\": \"{id}\", \"name\": \"{name}\"}}"),
    )
    .unwrap();
    std::fs::write(
        dir.join(".faulk").join(LINK_FILE),
        format!("{{\"nonce\": \"{nonce}\"}}"),
    )
    .unwrap();
    for ((media, kind), bytes) in &disk.files {
        let hex: String = media.iter().map(|b| format!("{b:02x}")).collect();
        let path = if *kind == engine::audio_source::FileKind::Blob as u8 {
            dir.join("blobs").join(hex)
        } else {
            dir.join(".faulk")
                .join("cache")
                .join("pcm")
                .join(format!("{hex}.pcm"))
        };
        std::fs::write(path, bytes).unwrap();
    }
    std::fs::canonicalize(dir).unwrap()
}

async fn start(disk: &Disk) -> Harness {
    let temp =
        std::env::temp_dir().join(format!("faulk-319-it-{}", helper::security::random_hex(6)));
    std::fs::create_dir_all(&temp).unwrap();
    let temp = std::fs::canonicalize(&temp).unwrap();
    let song = project_dir(&temp, "Song", PROJECT, NONCE, disk);
    let other = project_dir(&temp, "Other", OTHER, OTHER_NONCE, disk);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let meter = Arc::new(Meter::default());
    let (core, sender) = audio_core(48_000.0, meter.clone()).unwrap();
    let engine = Arc::new(EngineSlot::default());
    engine.install(sender, 48_000.0);
    let (midi_out, midi_in) = tokio::sync::mpsc::unbounded_channel();
    let sink = MidiSink::new(Arc::new(LiveInput::new(engine.clone())), midi_out);
    let shared = Arc::new(
        Shared::new(
            Policy {
                port,
                allowed_origins: vec![ORIGIN.into()],
                token: TOKEN.into(),
                mcp_token: helper::security::McpToken::Fixed("m".repeat(32)),
            },
            engine.clone(),
            MidiManager::new(sink),
            midi_in,
            meter,
            Box::new(|_| {}),
        )
        .with_folders(
            Remembered::in_memory(vec![song.clone(), other.clone()]),
            Arc::new(NoPicker),
            helper::tray_text::Lang::En,
        ),
    );
    tokio::spawn(serve(listener, shared.clone()));
    Harness {
        port,
        shared,
        _core: core,
        _engine: engine,
        _temp: Temp(temp),
        song,
        other,
    }
}

async fn session(h: &Harness) -> Ws {
    let mut request = format!("ws://127.0.0.1:{}/session?token={TOKEN}", h.port)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", ORIGIN.parse().unwrap());
    let (mut ws, _) = connect_async(request).await.unwrap();
    match next(&mut ws).await {
        ServerMessage::Hello { capabilities, .. } => {
            assert!(capabilities.export, "the hello announces the export");
        }
        other => panic!("expected hello, got {other:?}"),
    }
    ws
}

/// The next JSON message that is not about MIDI or updates (reports are skipped).
async fn next(ws: &mut Ws) -> ServerMessage {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(60), ws.next())
            .await
            .expect("a message within 60 s")
            .expect("the stream is open")
            .expect("a valid frame");
        if let Message::Text(text) = frame {
            let message: ServerMessage = serde_json::from_str(&text).unwrap();
            if !matches!(
                message,
                ServerMessage::MidiInputs { .. } | ServerMessage::UpdateStatus { .. }
            ) {
                return message;
            }
        }
    }
}

async fn send(ws: &mut Ws, message: &ClientMessage) {
    ws.send(Message::text(serde_json::to_string(message).unwrap()))
        .await
        .unwrap();
}

/// Names `PROJECT`'s place (or `project`'s) and waits for its folder to open.
async fn open_folder(ws: &mut Ws, project: &str, nonce: &str) {
    send(
        ws,
        &ClientMessage::FolderOpen {
            project: project.into(),
            nonce: nonce.into(),
            name: "Song".into(),
        },
    )
    .await;
    match next(ws).await {
        ServerMessage::FolderState {
            state: FolderStatus::Open { .. },
            ..
        } => {}
        other => panic!("expected the folder open, got {other:?}"),
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The `export.start` of `job`, as the page sends it.
fn start_of(id: u32, project: &str, name: &str, job: &ExportJob) -> ClientMessage {
    ClientMessage::ExportStart {
        id,
        project: project.into(),
        name: name.into(),
        sample_rate: job.plan.settings.sample_rate,
        bits: job.plan.settings.bit_depth.format().bits(),
        body_frames: job.plan.body_frames,
        setup: b64(&job.setup),
        start: b64(&job.start),
        stop: b64(&job.stop),
    }
}

/// What an export's run told the page: its progress messages, then its end.
struct Ended {
    progress: Vec<(u64, u64, bool)>,
    end: ServerMessage,
    /// From the start to the end.
    took: Duration,
}

/// Waits for the end of export `id`; `on_progress` runs at its first progress message.
async fn run_to_end<F>(ws: &mut Ws, id: u32, mut on_progress: Option<F>) -> Ended
where
    F: AsyncFnOnce(&mut Ws),
{
    let started = Instant::now();
    let mut progress = Vec::new();
    loop {
        match next(ws).await {
            ServerMessage::ExportProgress {
                id: got,
                rendered,
                body,
                tail,
            } => {
                assert_eq!(got, id);
                progress.push((rendered, body, tail));
                if let Some(f) = on_progress.take() {
                    f(ws).await;
                }
            }
            end @ (ServerMessage::ExportDone { .. } | ServerMessage::ExportFailed { .. }) => {
                return Ended {
                    progress,
                    end,
                    took: started.elapsed(),
                };
            }
            ServerMessage::FolderState { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

fn failed_code(end: &ServerMessage) -> &str {
    match end {
        ServerMessage::ExportFailed { code, .. } => code,
        other => panic!("expected export.failed, got {other:?}"),
    }
}

/// Every file below `dir`, relative, `/` separated.
fn files(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
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
    out.sort();
    out
}

/// Nothing in `exports/` nor `.faulk/tmp/` of `dir`.
fn nothing_written(dir: &Path) {
    let left: Vec<String> = files(dir)
        .into_iter()
        .filter(|f| f.starts_with("exports/") || f.starts_with(".faulk/tmp/"))
        .collect();
    assert!(left.is_empty(), "left in {}: {left:?}", dir.display());
}

/// Exports `job` through the helper and returns the file it wrote, with its path.
async fn export_through_the_helper(h: &Harness, job: &ExportJob, name: &str) -> (String, Vec<u8>) {
    let mut ws = session(h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    send(&mut ws, &start_of(1, PROJECT, name, job)).await;
    let ended = run_to_end(&mut ws, 1, None::<fn(&mut Ws) -> std::future::Ready<()>>).await;
    let ServerMessage::ExportDone {
        path,
        bytes,
        written,
        tail_cut,
        missing,
        ..
    } = ended.end
    else {
        panic!("expected export.done, got {:?}", ended.end);
    };
    assert!(!tail_cut);
    assert!(missing.is_empty(), "missing media: {missing:?}");
    let wav = std::fs::read(h.song.join(&path)).unwrap();
    assert_eq!(wav.len() as u64, bytes, "the size told is the file's");
    let frame = job.plan.settings.bit_depth.format().frame_bytes() as u64;
    let header = job.plan.settings.bit_depth.format().header_bytes() as u64;
    assert_eq!(bytes, header + written * frame);
    // Progress messages are spaced by at least `EXPORT_PROGRESS_MS`.
    let most = ended.took.as_millis() as usize / protocol::EXPORT_PROGRESS_MS as usize + 1;
    assert!(
        ended.progress.len() <= most,
        "{} progress messages in {:?}",
        ended.progress.len(),
        ended.took
    );
    assert!(
        files(&h.song.join(".faulk").join("tmp")).is_empty(),
        "no temporary file left"
    );
    (path, wav)
}

/// The helper's file against the page's Worker's (the WASM build): the header and the length
/// exact, every sample within `PARITY_TOLERANCE`.
fn same_file(helper: &[u8], worker: &Exported) {
    let header = worker.settings.bit_depth.format().header_bytes();
    assert_eq!(helper.len(), worker.wav.len(), "the files differ in length");
    assert_eq!(helper[..header], worker.wav[..header], "the headers differ");
    let helper = Exported {
        wav: helper.to_vec(),
        ..worker.clone()
    };
    let (h, w) = (helper.render(), worker.render());
    assert!(w.peak() > 0.05, "the render is not silent");
    h.assert_close(&w, PARITY_TOLERANCE);
    let worst = h
        .left
        .iter()
        .zip(&w.left)
        .chain(h.right.iter().zip(&w.right))
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    eprintln!(
        "parity: {} frames, worst sample difference {worst:e} (tolerance {PARITY_TOLERANCE:e})",
        h.left.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_song_with_an_echo_exported_by_the_helper_is_the_workers_file() {
    let state = demo_song(1, Some(0.3));
    let settings = ExportSettings {
        region: ExportRegion::Song,
        bit_depth: ExportBitDepth::Float32,
        sample_rate: 48_000,
        ..ExportSettings::default()
    };
    let job = job_of(&state, settings);
    let h = start(&Disk::default()).await;
    let (path, wav) = export_through_the_helper(&h, &job, "Song").await;
    assert_eq!(path, "exports/Song.wav");
    let worker = export_job_wasm(&job, &Disk::default());
    same_file(&wav, &worker);
    // The tail is there: the file is longer than the content.
    assert!(worker.written > worker.body_frames + 48_000);
    // Nothing of the project was touched, nothing written anywhere else.
    assert_eq!(
        files(&h.song)
            .into_iter()
            .filter(|f| !f.starts_with("blobs/") && !f.starts_with(".faulk/"))
            .collect::<Vec<_>>(),
        vec!["exports/Song.wav".to_owned(), "project.json".to_owned()]
    );
    nothing_written(&h.other);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_audio_clip_read_from_the_folder_exports_alike_by_the_helper_and_the_worker() {
    use test_support::clip_start::{impulse_disk, impulse_job};
    let rate = 44_100;
    let disk = impulse_disk(rate);
    let mut job = impulse_job(rate, &[]);
    job.plan.settings.bit_depth = ExportBitDepth::Pcm24;
    let h = start(&disk).await;
    let (_, wav) = export_through_the_helper(&h, &job, "Clip at 44.1").await;
    let worker = export_job_wasm(&job, &disk);
    assert!(worker.missing.is_empty());
    same_file(&wav, &worker);
}

/// A long render: 40 bars, 80 s of content.
fn long_job() -> ExportJob {
    job_of(&demo_song(40, None), ExportSettings::default())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_mid_render_leaves_no_file_and_no_temporary_file() {
    let h = start(&Disk::default()).await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    send(&mut ws, &start_of(4, PROJECT, "Song", &long_job())).await;
    let ended = run_to_end(
        &mut ws,
        4,
        Some(async |ws: &mut Ws| {
            // A cancel of another run is ignored; this run's stops it.
            send(ws, &ClientMessage::ExportCancel { id: 99 }).await;
            send(ws, &ClientMessage::ExportCancel { id: 4 }).await;
        }),
    )
    .await;
    assert_eq!(failed_code(&ended.end), errors::EXPORT_CANCELLED);
    nothing_written(&h.song);
    assert!(!h.shared.export_running());
    assert_eq!(h.shared.export_threads(), 0, "a run told its end is over");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_closed_mid_render_gets_nothing_in_either_folder() {
    let h = start(&Disk::default()).await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    send(&mut ws, &start_of(5, PROJECT, "Song", &long_job())).await;
    let ended = run_to_end(
        &mut ws,
        5,
        Some(async |ws: &mut Ws| {
            // The page moved to another project: its folder opens in place of the first.
            send(
                ws,
                &ClientMessage::FolderOpen {
                    project: OTHER.into(),
                    nonce: OTHER_NONCE.into(),
                    name: "Other".into(),
                },
            )
            .await;
        }),
    )
    .await;
    assert_eq!(failed_code(&ended.end), errors::EXPORT_FOLDER_CHANGED);
    nothing_written(&h.song);
    nothing_written(&h.other);

    // `folder.close` alike.
    open_folder(&mut ws, PROJECT, NONCE).await;
    send(&mut ws, &start_of(6, PROJECT, "Song", &long_job())).await;
    let ended = run_to_end(
        &mut ws,
        6,
        Some(async |ws: &mut Ws| {
            send(ws, &ClientMessage::FolderClose).await;
        }),
    )
    .await;
    assert_eq!(failed_code(&ended.end), errors::EXPORT_FOLDER_CHANGED);
    nothing_written(&h.song);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_folder_named_again_mid_render_is_the_same_folder() {
    // The page names its place after each `hello` (an output replaced mid-export): the export
    // goes on into the same folder.
    let h = start(&Disk::default()).await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    let job = job_of(&demo_song(8, None), ExportSettings::default());
    send(&mut ws, &start_of(7, PROJECT, "Song", &job)).await;
    let ended = run_to_end(
        &mut ws,
        7,
        Some(async |ws: &mut Ws| {
            send(
                ws,
                &ClientMessage::FolderOpen {
                    project: PROJECT.into(),
                    nonce: NONCE.into(),
                    name: "Song".into(),
                },
            )
            .await;
        }),
    )
    .await;
    assert!(
        matches!(ended.end, ServerMessage::ExportDone { ref path, .. } if path == "exports/Song.wav"),
        "{:?}",
        ended.end
    );
    assert!(h.song.join("exports").join("Song.wav").is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_gone_mid_render_leaves_no_file_and_frees_the_helper() {
    let h = start(&Disk::default()).await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    send(&mut ws, &start_of(8, PROJECT, "Song", &long_job())).await;
    // Its first progress, then the page goes.
    loop {
        if matches!(next(&mut ws).await, ServerMessage::ExportProgress { .. }) {
            break;
        }
    }
    assert!(h.shared.export_running());
    assert_eq!(h.shared.export_threads(), 1, "the run's thread is counted");
    drop(ws);
    // The slot is freed as soon as the helper sees the page gone, while the run's thread still
    // unwinds: the folder is checked once that thread is over too, its write dropped (#792).
    let deadline = Instant::now() + Duration::from_secs(30);
    while h.shared.export_running() || h.shared.export_threads() > 0 {
        assert!(Instant::now() < deadline, "the export never stopped");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    nothing_written(&h.song);
    // A new page exports at once.
    let job = job_of(&demo_song(1, None), ExportSettings::default());
    let (path, _) = export_through_the_helper(&h, &job, "Song").await;
    assert_eq!(path, "exports/Song.wav");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_export_is_kept_and_the_next_free_name_taken() {
    let h = start(&Disk::default()).await;
    let job = job_of(&demo_song(1, None), ExportSettings::default());
    let (first, wav) = export_through_the_helper(&h, &job, "Song").await;
    let (second, _) = export_through_the_helper(&h, &job, "Song").await;
    assert_eq!(first, "exports/Song.wav");
    assert_eq!(second, "exports/Song (2).wav");
    assert_eq!(std::fs::read(h.song.join(&first)).unwrap(), wav);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_refused_export_has_its_code_and_writes_nothing() {
    let h = start(&Disk::default()).await;
    let mut ws = session(&h).await;
    let job = job_of(&demo_song(1, None), ExportSettings::default());
    let refused = async |ws: &mut Ws, message: ClientMessage| -> String {
        send(ws, &message).await;
        match next(ws).await {
            ServerMessage::ExportFailed { code, .. } => code,
            other => panic!("expected export.failed, got {other:?}"),
        }
    };
    // No folder open yet.
    assert_eq!(
        refused(&mut ws, start_of(1, PROJECT, "Song", &job)).await,
        errors::EXPORT_NO_FOLDER
    );
    open_folder(&mut ws, PROJECT, NONCE).await;
    // Another project than the folder's.
    assert_eq!(
        refused(&mut ws, start_of(2, OTHER, "Song", &job)).await,
        errors::EXPORT_NO_FOLDER
    );
    // Names that would leave `exports/` or are no file name.
    for name in [
        "../escape",
        "..",
        r"..\escape",
        "C:escape",
        "a/b",
        "CON",
        "x*",
        "",
    ] {
        assert_eq!(
            refused(&mut ws, start_of(3, PROJECT, name, &job)).await,
            errors::EXPORT_INVALID,
            "{name:?}"
        );
    }
    let with = |f: &dyn Fn(&mut ClientMessage)| {
        let mut m = start_of(4, PROJECT, "Song", &job);
        f(&mut m);
        m
    };
    let edits: [&dyn Fn(&mut ClientMessage); 6] = [
        &|m| {
            if let ClientMessage::ExportStart { setup, .. } = m {
                *setup = "not base64!".into();
            }
        },
        &|m| {
            if let ClientMessage::ExportStart { bits, .. } = m {
                *bits = 8;
            }
        },
        &|m| {
            if let ClientMessage::ExportStart { sample_rate, .. } = m {
                *sample_rate = 22_050;
            }
        },
        &|m| {
            if let ClientMessage::ExportStart { body_frames, .. } = m {
                *body_frames = 0;
            }
        },
        &|m| {
            if let ClientMessage::ExportStart { body_frames, .. } = m {
                *body_frames = 48_000 * 1801;
            }
        },
        &|m| {
            if let ClientMessage::ExportStart { project, .. } = m {
                *project = "not-a-ulid".into();
            }
        },
    ];
    for edit in edits {
        assert_eq!(refused(&mut ws, with(edit)).await, errors::EXPORT_INVALID);
    }
    // Streams the engine refuses: rendered nothing, written nothing.
    let garbage = with(&|m| {
        if let ClientMessage::ExportStart { setup, .. } = m {
            *setup = b64(&[0xff; 64]);
        }
    });
    send(&mut ws, &garbage).await;
    let ended = run_to_end(&mut ws, 4, None::<fn(&mut Ws) -> std::future::Ready<()>>).await;
    assert_eq!(failed_code(&ended.end), errors::EXPORT_RENDER);
    nothing_written(&h.song);

    // One at a time.
    send(&mut ws, &start_of(10, PROJECT, "Song", &long_job())).await;
    assert_eq!(
        refused(&mut ws, start_of(11, PROJECT, "Other", &job)).await,
        errors::EXPORT_BUSY
    );
    send(&mut ws, &ClientMessage::ExportCancel { id: 10 }).await;
    let ended = run_to_end(&mut ws, 10, None::<fn(&mut Ws) -> std::future::Ready<()>>).await;
    assert_eq!(failed_code(&ended.end), errors::EXPORT_CANCELLED);
    nothing_written(&h.song);
    nothing_written(&h.other);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_export_started_right_after_a_cancel_is_not_refused_busy() {
    // The page takes a cancel as done at once: the helper frees its slot at once too, while the
    // cancelled run unwinds.
    let h = start(&Disk::default()).await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    send(&mut ws, &start_of(20, PROJECT, "Long", &long_job())).await;
    loop {
        if matches!(next(&mut ws).await, ServerMessage::ExportProgress { .. }) {
            break;
        }
    }
    send(&mut ws, &ClientMessage::ExportCancel { id: 20 }).await;
    let short = job_of(&demo_song(1, None), ExportSettings::default());
    send(&mut ws, &start_of(21, PROJECT, "Short", &short)).await;
    let (mut first, mut second) = (None, None);
    while first.is_none() || second.is_none() {
        match next(&mut ws).await {
            ServerMessage::ExportFailed { id: 20, code, .. } => first = Some(code),
            ServerMessage::ExportDone { id: 21, path, .. } => second = Some(path),
            ServerMessage::ExportFailed { id: 21, code, .. } => {
                panic!("the second export failed: {code}")
            }
            _ => {}
        }
    }
    assert_eq!(first.as_deref(), Some(errors::EXPORT_CANCELLED));
    assert_eq!(second.as_deref(), Some("exports/Short.wav"));
    assert!(!h.song.join("exports").join("Long.wav").exists());
    assert!(files(&h.song.join(".faulk").join("tmp")).is_empty());
    assert_eq!(h.shared.export_threads(), 0, "both runs are over");
}

/// A watch whose export is cancelled once its render is over: the cancel reaches the run between
/// the commit and its last check.
struct CancelledAtTheCommit;

impl helper::export::Watch for CancelledAtTheCommit {
    fn stop(&self) -> Option<helper::export::ExportFailure> {
        None
    }

    fn cancelled(&self) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_that_reaches_the_commit_removes_the_file_just_made() {
    let h = start(&Disk::default()).await;
    let folder = helper::disk::ProjectFolder::open(&h.song, Some(PROJECT)).unwrap();
    let job = job_of(&demo_song(1, None), ExportSettings::default());
    let ClientMessage::ExportStart {
        id,
        project,
        name,
        sample_rate,
        bits,
        body_frames,
        setup,
        start,
        stop,
    } = start_of(30, PROJECT, "Song", &job)
    else {
        unreachable!()
    };
    let request = helper::export::parse(helper::export::StartFields {
        id,
        project: &project,
        name: &name,
        sample_rate,
        bits,
        body_frames,
        setup: &setup,
        start: &start,
        stop: &stop,
    })
    .unwrap();
    let mut told = 0;
    let outcome = helper::export::run(
        &request,
        &folder,
        &CancelledAtTheCommit,
        Duration::ZERO,
        |_| told += 1,
    );
    assert_eq!(outcome.unwrap_err().code, errors::EXPORT_CANCELLED);
    assert!(told > 0);
    nothing_written(&h.song);
}
