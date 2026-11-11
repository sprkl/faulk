//! The native audio import in Boost (#778), over a real loopback session: the page names a file of
//! the project's folder, or one the user picked in the helper's dialog (`import.start`); the helper
//! decodes it with the page's decoder, natively, and writes its blob and caches into the folder.
//! What it writes is what the page's import writes (the WASM build of `audio-import-wasm`,
//! `test_support::import_wasm::page_import`): the blob and the peaks byte for byte, the PCM cache
//! byte for byte but for MP3 and AAC (3.3e-7 at most). An agent's call crosses the link as its
//! command and its answer only, never a byte of the file. Without a folder nothing is read or
//! written; a cancel, a folder closed or replaced, a decode over the cap leave nothing, not even a
//! temporary file; every refusal has its code.

use futures_util::{SinkExt, StreamExt};
use helper::audio::{AudioCore, EngineSlot, Meter, audio_core};
use helper::disk::{FolderPicker, LINK_FILE, Picked, PickedFiles, Remembered};
use helper::live::LiveInput;
use helper::midi::{MidiManager, MidiSink};
use helper::security::Policy;
use helper::server::{Shared, serve};
use protocol::{AgentResult, ClientMessage, FolderStatus, ImportSource, ServerMessage, errors};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use test_support::import_wasm::page_import;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const ORIGIN: &str = "http://localhost:5173";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const MCP_TOKEN: &str = "fedcba9876543210fedcba9876543210";
const PROJECT: &str = "01J0000000000000000000000A";
const OTHER: &str = "01J0000000000000000000000B";
const NONCE: &str = "00112233445566778899aabbccddeeff";
const OTHER_NONCE: &str = "ffeeddccbbaa99887766554433221100";

/// The largest difference allowed between the helper's and the page's PCM of an MP3 or an AAC
/// (acceptance criterion of #778, § 5.9 of the storage architecture).
const LOSSY_PCM_TOLERANCE: f32 = 3.3e-7;
/// The fixtures whose PCM goes through `powf` and `cos`, whose last bit differs between the native
/// C library and Rust's in WebAssembly.
const LOSSY_PCM: &[&str] = &["sine.mp3", "sine.m4a"];

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A directory of its own, removed on drop.
struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The helper's dialogs, as the user answers them in a test: no folder dialog, and the files
/// given for the file dialog (none: the user closes it).
#[derive(Default)]
/// `None`: this system has no file dialog; an empty list: the user closes it.
struct Dialog(Mutex<Option<Vec<PathBuf>>>);

impl FolderPicker for Dialog {
    fn pick(&self, _title: &str) -> Picked {
        Picked::Cancelled
    }

    fn pick_files(&self, _title: &str) -> PickedFiles {
        match self.0.lock().unwrap().clone() {
            None => PickedFiles::Unavailable,
            Some(files) if files.is_empty() => PickedFiles::Cancelled,
            Some(files) => PickedFiles::Files(files),
        }
    }
}

struct Harness {
    port: u16,
    shared: Arc<Shared>,
    _core: AudioCore,
    _engine: Arc<EngineSlot>,
    temp: Temp,
    /// The project folder of `PROJECT`, with the page's marker.
    song: PathBuf,
    /// Another project's folder, with its own marker.
    other: PathBuf,
    dialog: Arc<Dialog>,
}

fn project_dir(parent: &Path, name: &str, id: &str, nonce: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(dir.join(".faulk")).unwrap();
    std::fs::create_dir_all(dir.join("imports")).unwrap();
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
    std::fs::canonicalize(dir).unwrap()
}

async fn start_with(cap: Option<u64>) -> Harness {
    let temp =
        std::env::temp_dir().join(format!("faulk-778-it-{}", helper::security::random_hex(6)));
    std::fs::create_dir_all(&temp).unwrap();
    let temp = std::fs::canonicalize(&temp).unwrap();
    let song = project_dir(&temp, "Song", PROJECT, NONCE);
    let other = project_dir(&temp, "Other", OTHER, OTHER_NONCE);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let meter = Arc::new(Meter::default());
    let (core, sender) = audio_core(48_000.0, meter.clone()).unwrap();
    let engine = Arc::new(EngineSlot::default());
    engine.install(sender, 48_000.0);
    let (midi_out, midi_in) = tokio::sync::mpsc::unbounded_channel();
    let sink = MidiSink::new(Arc::new(LiveInput::new(engine.clone())), midi_out);
    let dialog = Arc::new(Dialog::default());
    let mut shared = Shared::new(
        Policy {
            port,
            allowed_origins: vec![ORIGIN.into()],
            token: TOKEN.into(),
            mcp_token: helper::security::McpToken::Fixed(MCP_TOKEN.into()),
        },
        engine.clone(),
        MidiManager::new(sink),
        midi_in,
        meter,
        Box::new(|_| {}),
    )
    .with_folders(
        Remembered::in_memory(vec![song.clone(), other.clone()]),
        dialog.clone(),
        helper::tray_text::Lang::En,
    );
    if let Some(cap) = cap {
        shared = shared.with_import_cap(cap);
    }
    let shared = Arc::new(shared);
    tokio::spawn(serve(listener, shared.clone()));
    Harness {
        port,
        shared,
        _core: core,
        _engine: engine,
        temp: Temp(temp),
        song,
        other,
        dialog,
    }
}

async fn start() -> Harness {
    start_with(None).await
}

/// What a page saw on its session: every frame, text and binary.
#[derive(Default)]
struct Seen {
    texts: Vec<String>,
    binaries: Vec<Vec<u8>>,
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
            assert!(capabilities.import, "the hello announces the import");
        }
        other => panic!("expected hello, got {other:?}"),
    }
    ws
}

/// The next JSON message that is not about MIDI or updates, every frame kept in `seen`.
async fn next_seen(ws: &mut Ws, seen: &mut Seen) -> ServerMessage {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(120), ws.next())
            .await
            .expect("a message within 120 s")
            .expect("the stream is open")
            .expect("a valid frame");
        match frame {
            Message::Text(text) => {
                seen.texts.push(text.to_string());
                let message: ServerMessage = serde_json::from_str(&text).unwrap();
                if !matches!(
                    message,
                    ServerMessage::MidiInputs { .. } | ServerMessage::UpdateStatus { .. }
                ) {
                    return message;
                }
            }
            Message::Binary(bytes) => seen.binaries.push(bytes.to_vec()),
            _ => {}
        }
    }
}

async fn next(ws: &mut Ws) -> ServerMessage {
    next_seen(ws, &mut Seen::default()).await
}

async fn send(ws: &mut Ws, message: &ClientMessage) {
    ws.send(Message::text(serde_json::to_string(message).unwrap()))
        .await
        .unwrap();
}

/// Names `project`'s place and waits for its folder to open.
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

fn folder(path: &str) -> ImportSource {
    ImportSource::Folder { path: path.into() }
}

fn start_of(id: u32, project: &str, source: ImportSource) -> ClientMessage {
    ClientMessage::ImportStart {
        id,
        project: project.into(),
        source,
    }
}

/// What an import told the page: its progress, then its end.
struct Ended {
    progress: Vec<f64>,
    end: ServerMessage,
}

/// Waits for the end of import `id`; `on_progress` runs at the first progress message that
/// `when` takes.
async fn run_to_end<F>(
    ws: &mut Ws,
    seen: &mut Seen,
    id: u32,
    when: impl Fn(f64) -> bool,
    mut on_progress: Option<F>,
) -> Ended
where
    F: AsyncFnOnce(&mut Ws),
{
    let mut progress = Vec::new();
    loop {
        match next_seen(ws, seen).await {
            ServerMessage::ImportProgress { id: got, fraction } => {
                assert_eq!(got, id);
                assert!((0.0..=1.0).contains(&fraction), "{fraction}");
                progress.push(fraction);
                if when(fraction)
                    && let Some(f) = on_progress.take()
                {
                    f(ws).await;
                }
            }
            end @ (ServerMessage::ImportDone { .. } | ServerMessage::ImportFailed { .. }) => {
                return Ended { progress, end };
            }
            ServerMessage::FolderState { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

async fn import(ws: &mut Ws, id: u32, source: ImportSource) -> ServerMessage {
    send(ws, &start_of(id, PROJECT, source)).await;
    run_to_end::<fn(&mut Ws) -> std::future::Ready<()>>(
        ws,
        &mut Seen::default(),
        id,
        |_| false,
        None,
    )
    .await
    .end
}

fn failed_code(end: &ServerMessage) -> &str {
    match end {
        ServerMessage::ImportFailed { code, .. } => code,
        other => panic!("expected import.failed, got {other:?}"),
    }
}

/// Every file below `dir` with its bytes, relative and `/` separated.
fn listing(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(base, &path, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// The files an import may leave: none in `blobs/`, the caches or `.faulk/tmp/`.
fn nothing_imported(dir: &Path) {
    let left: Vec<String> = listing(dir)
        .into_keys()
        .filter(|f| {
            !f.ends_with('/')
                && (f.starts_with("blobs/")
                    || f.starts_with(".faulk/cache/")
                    || f.starts_with(".faulk/tmp/"))
        })
        .collect();
    assert!(left.is_empty(), "left in {}: {left:?}", dir.display());
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../audio-import/tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// A WAV of `frames` frames of noise, `bits` 8 or 16, `channels`, 48 kHz: its bytes never repeat
/// a 32-byte window by chance.
fn noise_wav(frames: u32, channels: u16, bits: u16) -> Vec<u8> {
    let bytes_per = u32::from(bits / 8);
    let data = frames * u32::from(channels) * bytes_per;
    let mut wav = Vec::with_capacity(44 + data as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&48_000u32.to_le_bytes());
    wav.extend_from_slice(&(48_000 * u32::from(channels) * bytes_per).to_le_bytes());
    wav.extend_from_slice(&(channels * bits / 8).to_le_bytes());
    wav.extend_from_slice(&bits.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    let mut x: u32 = 0x1234_5678;
    for _ in 0..(data / bytes_per) {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        if bits == 8 {
            wav.push((x >> 24) as u8);
        } else {
            wav.extend_from_slice(&(((x >> 16) as i16) / 2).to_le_bytes());
        }
    }
    wav
}

fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn sha256(bytes: &[u8]) -> String {
    let mut h = audio_import::Sha256::new();
    h.update(bytes);
    h.finish().to_string()
}

/// The decoder's hint of a file name, as the page's `extensionOf` takes it.
fn hint(name: &str) -> String {
    helper::import::extension_of(name).unwrap_or_default()
}

/// The largest difference between two PCM caches' samples (their headers equal).
fn pcm_gap(a: &[u8], b: &[u8]) -> f32 {
    assert_eq!(a.len(), b.len(), "the same length");
    assert_eq!(a[..32], b[..32], "the same header");
    a[32..]
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b[32..].as_chunks::<4>().0)
        .map(|(x, y)| (f32::from_le_bytes(*x) - f32::from_le_bytes(*y)).abs())
        .fold(0.0, f32::max)
}

#[tokio::test]
async fn every_reference_file_imported_by_the_helper_gives_the_files_the_page_writes() {
    let h = start().await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    let names = [
        "sine-s16.wav",
        "sine-s24.wav",
        "sine-f32.wav",
        "sine-u8.wav",
        "sine-s16.aiff",
        "sine-s24.aiff",
        "sine.flac",
        "sine-48k-mono.flac",
        "sine-alac.m4a",
        "sine.mp3",
        "sine.m4a",
        "sine.ogg",
        "sine.opus",
        "sine-mono.opus",
        "sine.webm",
        "recorded.webm",
        "voip-mono.opus",
        "voip.webm",
    ];
    let mut widest = 0f32;
    for (i, name) in names.iter().enumerate() {
        let bytes = fixture(name);
        std::fs::write(h.song.join("imports").join(name), &bytes).unwrap();
        let end = import(&mut ws, i as u32 + 1, folder(&format!("imports/{name}"))).await;
        let page = page_import(&bytes, &hint(name)).unwrap();
        let ServerMessage::ImportDone {
            hash,
            sample_rate,
            channels,
            frames,
            name: got_name,
            direct,
            created,
            peaks,
            ..
        } = end
        else {
            panic!("{name}: expected import.done, got {end:?}")
        };
        assert_eq!(hash, page.hash, "{name}: the hash");
        assert_eq!(hash, sha256(&bytes), "{name}: the file's SHA-256");
        assert_eq!(
            (sample_rate, channels, frames, direct),
            (page.sample_rate, page.channels, page.frames, page.direct),
            "{name}: the metadata"
        );
        assert_eq!(&got_name, name);
        assert!(created, "{name}: the blob was written");
        // The blob is the file.
        assert_eq!(
            std::fs::read(h.song.join("blobs").join(&hash)).unwrap(),
            bytes,
            "{name}: the blob"
        );
        // The PCM cache: none for a file read directly, else the page's.
        let cache = h.song.join(format!(".faulk/cache/pcm/{hash}.pcm"));
        match &page.pcm {
            None => assert!(!cache.exists(), "{name}: no cache for a direct file"),
            Some(expected) => {
                let ours = std::fs::read(&cache).unwrap();
                if LOSSY_PCM.contains(name) {
                    let gap = pcm_gap(&ours, expected);
                    widest = widest.max(gap);
                    assert!(
                        gap <= LOSSY_PCM_TOLERANCE,
                        "{name}: the PCM is {gap:e} from the page's"
                    );
                } else {
                    assert!(ours == *expected, "{name}: the PCM bit for bit");
                }
            }
        }
        // The peaks byte for byte.
        let peaks_file = h.song.join(format!(".faulk/cache/peaks/{hash}.peaks"));
        assert_eq!(peaks, page.peaks.is_some(), "{name}: peaks written");
        assert_eq!(
            std::fs::read(&peaks_file).ok(),
            page.peaks,
            "{name}: the peaks"
        );
    }
    println!("MP3 and AAC: the helper's PCM is at most {widest:e} from the page's");
    // Nothing left in the temporary folder.
    let tmp: Vec<String> = listing(&h.song)
        .into_keys()
        .filter(|f| f.starts_with(".faulk/tmp/") && !f.ends_with('/'))
        .collect();
    assert!(tmp.is_empty(), "{tmp:?}");
    // A file that is no audio: refused by the decoder's code, nothing kept.
    let text = fixture("not-audio.txt");
    std::fs::write(h.other.join("imports").join("not-audio.txt"), &text).unwrap();
    open_folder(&mut ws, OTHER, OTHER_NONCE).await;
    send(
        &mut ws,
        &start_of(99, OTHER, folder("imports/not-audio.txt")),
    )
    .await;
    let end = loop {
        match next(&mut ws).await {
            ServerMessage::ImportProgress { .. } => {}
            end => break end,
        }
    };
    assert_eq!(failed_code(&end), "import.unsupported_format");
    assert_eq!(page_import(&text, "txt").unwrap_err(), "unsupported_format");
    nothing_imported(&h.other);
}

#[tokio::test]
async fn an_imported_file_already_in_the_blobs_is_kept_and_a_blob_named_by_path_imports() {
    let h = start().await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    let bytes = fixture("sine.flac");
    std::fs::write(h.song.join("imports").join("a.flac"), &bytes).unwrap();
    let ServerMessage::ImportDone { hash, created, .. } =
        import(&mut ws, 1, folder("imports/a.flac")).await
    else {
        panic!("expected done")
    };
    assert!(created);
    // The blob itself, named by its path: the same media, the blob kept as it is.
    let modified = std::fs::metadata(h.song.join("blobs").join(&hash))
        .unwrap()
        .modified()
        .unwrap();
    let end = import(&mut ws, 2, folder(&format!("blobs/{hash}"))).await;
    let ServerMessage::ImportDone {
        hash: again,
        created,
        ..
    } = end
    else {
        panic!("expected done, got {end:?}")
    };
    assert_eq!(again, hash);
    assert!(!created, "one blob per file: the one there is kept");
    assert_eq!(
        std::fs::metadata(h.song.join("blobs").join(&hash))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    assert_eq!(
        std::fs::read(h.song.join("blobs").join(&hash)).unwrap(),
        bytes
    );
}

/// Runs `faulk-helper mcp`'s bridge on `lines` against the helper on `port`; its answers.
fn bridge(
    port: u16,
    lines: Vec<serde_json::Value>,
) -> tokio::task::JoinHandle<Vec<serde_json::Value>> {
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    tokio::task::spawn_blocking(move || {
        let out = Captured::default();
        helper::mcp_stdio::run(
            port,
            Some(MCP_TOKEN.into()),
            std::io::Cursor::new(input),
            out.clone(),
        );
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    })
}

#[tokio::test]
async fn an_agents_call_imports_a_file_of_the_folder_and_no_byte_of_it_crosses_the_link() {
    let h = start().await;
    let mut ws = session(&h).await;
    let mut seen = Seen::default();
    open_folder(&mut ws, PROJECT, NONCE).await;
    let wav = noise_wav(48_000, 2, 16);
    std::fs::write(h.song.join("imports").join("noise.wav"), &wav).unwrap();
    let lines = vec![
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"test-agent","version":"1"}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"clip_import_audio_file",
            "arguments":{"track":"t1","path":"imports/noise.wav"}}}),
    ];
    let running = bridge(h.port, lines);
    // The page gets the agent's command, and hands the file's path to the helper.
    let ServerMessage::AgentCall {
        id: call, command, ..
    } = next_seen(&mut ws, &mut seen).await
    else {
        panic!("expected agent.call")
    };
    assert_eq!(
        command,
        serde_json::json!({"type":"clip.import_audio_file","track":"t1","path":"imports/noise.wav"})
    );
    let mut sent =
        vec![serde_json::to_string(&start_of(7, PROJECT, folder("imports/noise.wav"))).unwrap()];
    send(&mut ws, &start_of(7, PROJECT, folder("imports/noise.wav"))).await;
    let ended =
        run_to_end::<fn(&mut Ws) -> std::future::Ready<()>>(&mut ws, &mut seen, 7, |_| false, None)
            .await;
    let ServerMessage::ImportDone { hash, .. } = ended.end else {
        panic!("expected done, got {:?}", ended.end)
    };
    assert_eq!(hash, sha256(&wav));
    // The page stages the media, makes the clip and answers the agent.
    let result = AgentResult::Applied {
        output: serde_json::json!({"type":"clip.import_audio_file","clip":"c1","media":hash}),
    };
    let answer = ClientMessage::AgentResult { id: call, result };
    sent.push(serde_json::to_string(&answer).unwrap());
    send(&mut ws, &answer).await;
    let answers = running.await.unwrap();
    let made = answers
        .iter()
        .find(|a| a["id"] == 2)
        .expect("the tool's answer");
    assert_eq!(made["result"]["isError"], false, "{made}");
    assert_eq!(
        made["result"]["structuredContent"],
        serde_json::json!({"clip":"c1","media":hash})
    );
    // Keep reading for a while: the engine's reports, the messages.
    let until = tokio::time::Instant::now() + Duration::from_millis(300);
    while tokio::time::Instant::now() < until {
        match tokio::time::timeout(Duration::from_millis(50), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => seen.binaries.push(b.to_vec()),
            Ok(Some(Ok(Message::Text(t)))) => seen.texts.push(t.to_string()),
            _ => {}
        }
    }
    // 32-byte windows of the file, every 997 bytes of it, header included: none in any frame,
    // either way.
    let pieces: Vec<&[u8]> = (0..wav.len() - 32)
        .step_by(997)
        .map(|at| &wav[at..at + 32])
        .collect();
    assert!(pieces.len() > 150);
    let mut total = 0usize;
    for frame in seen
        .binaries
        .iter()
        .map(Vec::as_slice)
        .chain(seen.texts.iter().chain(sent.iter()).map(|t| t.as_bytes()))
    {
        total += frame.len();
        for piece in &pieces {
            assert!(!holds(frame, piece), "a frame carries the file's bytes");
        }
    }
    let texts: usize = seen.texts.iter().chain(sent.iter()).map(String::len).sum();
    println!(
        "a {} byte file imported: {} bytes of text frames on the session ({} frames, {} bytes in all with the engine's reports)",
        wav.len(),
        texts,
        seen.texts.len() + sent.len() + seen.binaries.len(),
        total
    );
    assert!(
        texts < 8 * 1024,
        "the session's messages stay small: {texts}"
    );
}

#[tokio::test]
async fn without_the_folder_open_an_import_is_refused_and_the_disk_untouched() {
    let h = start().await;
    let mut ws = session(&h).await;
    std::fs::write(h.song.join("imports").join("a.flac"), fixture("sine.flac")).unwrap();
    let before = listing(&h.temp.0);
    // No folder named.
    let end = import(&mut ws, 1, folder("imports/a.flac")).await;
    assert_eq!(failed_code(&end), errors::IMPORT_NO_FOLDER);
    // Another project's folder open: not this project's.
    open_folder(&mut ws, OTHER, OTHER_NONCE).await;
    let end = import(&mut ws, 2, folder("imports/a.flac")).await;
    assert_eq!(failed_code(&end), errors::IMPORT_NO_FOLDER);
    // The folder closed.
    send(&mut ws, &ClientMessage::FolderClose).await;
    assert!(matches!(
        next(&mut ws).await,
        ServerMessage::FolderState { .. }
    ));
    let end = import(&mut ws, 3, folder("imports/a.flac")).await;
    assert_eq!(failed_code(&end), errors::IMPORT_NO_FOLDER);
    // The dialog too: no folder, no dialog.
    send(&mut ws, &ClientMessage::ImportChoose { id: 4 }).await;
    assert_eq!(failed_code(&next(&mut ws).await), errors::IMPORT_NO_FOLDER);
    assert_eq!(
        listing(&h.temp.0),
        before,
        "nothing read into, nothing written"
    );
    // And a project id that is none.
    send(&mut ws, &start_of(5, "nope", folder("imports/a.flac"))).await;
    assert_eq!(failed_code(&next(&mut ws).await), errors::IMPORT_INVALID);
}

#[tokio::test]
async fn every_refused_source_has_its_code_and_writes_nothing() {
    let h = start().await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    // A file outside the folder, beside it.
    std::fs::write(h.temp.0.join("outside.flac"), fixture("sine.flac")).unwrap();
    std::fs::create_dir_all(h.song.join("imports").join("dir")).unwrap();
    let before = listing(&h.temp.0);
    let cases: &[(&str, &str, &str)] = &[
        ("../outside.flac", errors::IMPORT_PATH, "path.dot"),
        ("/outside.flac", errors::IMPORT_PATH, "path.absolute"),
        ("C:/outside.flac", errors::IMPORT_PATH, "path.colon"),
        ("imports\\a.flac", errors::IMPORT_PATH, "path.backslash"),
        ("imports//a.flac", errors::IMPORT_PATH, "path.empty"),
        ("", errors::IMPORT_PATH, "path.empty"),
        ("imports/con.wav", errors::IMPORT_PATH, "path.device"),
        (".faulk/link.json", errors::IMPORT_PATH, "path.reserved"),
        (".FAULK/link.json", errors::IMPORT_PATH, "path.reserved"),
        (".git/config", errors::IMPORT_PATH, "path.reserved"),
        (
            "imports/missing.flac",
            errors::IMPORT_UNREADABLE,
            "path.not_found",
        ),
        (
            "imports/dir",
            errors::IMPORT_UNREADABLE,
            "path.is_directory",
        ),
        ("imports", errors::IMPORT_UNREADABLE, "path.is_directory"),
    ];
    for (i, (path, code, why)) in cases.iter().enumerate() {
        let end = import(&mut ws, i as u32 + 1, folder(path)).await;
        let ServerMessage::ImportFailed {
            code: got, message, ..
        } = &end
        else {
            panic!("{path:?}: expected a refusal, got {end:?}")
        };
        assert_eq!((got.as_str(), message.as_str()), (*code, *why), "{path:?}");
    }
    // An unknown token, and one that is no token.
    let token = ImportSource::Picked {
        token: "0123456789abcdef0123456789abcdef".into(),
    };
    assert_eq!(
        failed_code(&import(&mut ws, 50, token).await),
        errors::IMPORT_UNREADABLE
    );
    let token = ImportSource::Picked {
        token: "../outside.flac".into(),
    };
    assert_eq!(
        failed_code(&import(&mut ws, 51, token).await),
        errors::IMPORT_INVALID
    );
    assert_eq!(listing(&h.temp.0), before, "nothing written anywhere");
}

/// The 8-bit noise WAV of an import long enough to be stopped midway: not read directly, so it
/// gets a PCM cache.
fn long_wav() -> Vec<u8> {
    noise_wav(4 * 1024 * 1024, 2, 8)
}

#[tokio::test]
async fn a_cancel_during_the_decode_leaves_no_file_and_no_temporary_file() {
    let h = start().await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    std::fs::write(h.song.join("imports").join("long.wav"), long_wav()).unwrap();
    send(&mut ws, &start_of(1, PROJECT, folder("imports/long.wav"))).await;
    let ended = run_to_end(
        &mut ws,
        &mut Seen::default(),
        1,
        |f| f > 0.55,
        Some(async |ws: &mut Ws| send(ws, &ClientMessage::ImportCancel { id: 1 }).await),
    )
    .await;
    assert_eq!(failed_code(&ended.end), errors::IMPORT_CANCELLED);
    assert!(
        ended.progress.iter().any(|f| *f > 0.55),
        "{:?}",
        ended.progress
    );
    nothing_imported(&h.song);
    // The slot is free: the next import runs.
    std::fs::write(h.song.join("imports").join("a.flac"), fixture("sine.flac")).unwrap();
    assert!(matches!(
        import(&mut ws, 2, folder("imports/a.flac")).await,
        ServerMessage::ImportDone { .. }
    ));
}

#[tokio::test]
async fn one_import_at_a_time_and_a_page_gone_mid_import_leaves_nothing() {
    let h = start().await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    std::fs::write(h.song.join("imports").join("long.wav"), long_wav()).unwrap();
    send(&mut ws, &start_of(1, PROJECT, folder("imports/long.wav"))).await;
    send(&mut ws, &start_of(2, PROJECT, folder("imports/long.wav"))).await;
    loop {
        match next(&mut ws).await {
            ServerMessage::ImportFailed { id: 2, code, .. } => {
                assert_eq!(code, errors::IMPORT_BUSY);
                break;
            }
            ServerMessage::ImportProgress { id: 1, .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(h.shared.import_running());
    // The page goes: its import stops, nothing is kept.
    drop(ws);
    let until = std::time::Instant::now() + Duration::from_secs(60);
    while h.shared.import_running() && std::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!h.shared.import_running(), "the slot is free");
    // The thread unwinds its temporary files right after it frees the slot.
    let until = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let left = listing(&h.song).into_keys().any(|f| {
            !f.ends_with('/') && (f.starts_with(".faulk/tmp/") || f.starts_with("blobs/"))
        });
        if !left || std::time::Instant::now() > until {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    nothing_imported(&h.song);
}

#[tokio::test]
async fn a_folder_closed_or_another_project_named_mid_import_gets_nothing_in_either_folder() {
    for switch in ["close", "other"] {
        let h = start().await;
        let mut ws = session(&h).await;
        open_folder(&mut ws, PROJECT, NONCE).await;
        std::fs::write(h.song.join("imports").join("long.wav"), long_wav()).unwrap();
        send(&mut ws, &start_of(1, PROJECT, folder("imports/long.wav"))).await;
        let ended = run_to_end(
            &mut ws,
            &mut Seen::default(),
            1,
            |_| true,
            Some(async |ws: &mut Ws| {
                let message = if switch == "close" {
                    ClientMessage::FolderClose
                } else {
                    ClientMessage::FolderOpen {
                        project: OTHER.into(),
                        nonce: OTHER_NONCE.into(),
                        name: "Other".into(),
                    }
                };
                send(ws, &message).await;
            }),
        )
        .await;
        assert_eq!(
            failed_code(&ended.end),
            errors::IMPORT_FOLDER_CHANGED,
            "{switch}"
        );
        nothing_imported(&h.song);
        nothing_imported(&h.other);
    }
}

#[tokio::test]
async fn a_file_decoding_past_the_cap_is_too_large_and_leaves_nothing() {
    // The cap lowered to what one second of 48 kHz stereo decodes to, less a byte.
    let h = start_with(Some(48_000 * 2 * 4 - 1)).await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    // Over the cap, refused by the decoder's code whether cached (8-bit) or read directly
    // (16-bit); under it (half a second of the FLAC), imported.
    for (id, (name, bytes, imports)) in [
        ("long.wav", long_wav(), false),
        ("sine.flac", fixture("sine.flac"), true),
        ("two.wav", noise_wav(96_000, 2, 16), false),
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::write(h.song.join("imports").join(name), &bytes).unwrap();
        let end = import(&mut ws, id as u32 + 1, folder(&format!("imports/{name}"))).await;
        if imports {
            assert!(
                matches!(end, ServerMessage::ImportDone { .. }),
                "{name}: {end:?}"
            );
        } else {
            assert_eq!(failed_code(&end), "import.too_large", "{name}");
        }
    }
    let blobs = listing(&h.song)
        .into_keys()
        .filter(|f| f.starts_with("blobs/") && !f.ends_with('/'))
        .count();
    let tmp = listing(&h.song)
        .into_keys()
        .filter(|f| f.starts_with(".faulk/tmp/") && !f.ends_with('/'))
        .count();
    assert_eq!(tmp, 0, "no temporary file left");
    assert_eq!(blobs, 1, "only the file under the cap is kept");
}

#[tokio::test]
async fn a_file_picked_in_the_helpers_dialog_imports_by_its_token_once_and_for_its_session_only() {
    let h = start().await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    // The user closes the dialog: no file.
    *h.dialog.0.lock().unwrap() = Some(Vec::new());
    send(&mut ws, &ClientMessage::ImportChoose { id: 1 }).await;
    assert_eq!(
        next(&mut ws).await,
        ServerMessage::ImportChosen {
            id: 1,
            files: Vec::new()
        }
    );
    // The user picks two files outside the folder.
    let outside = h.temp.0.join("Downloads");
    std::fs::create_dir_all(&outside).unwrap();
    let flac = fixture("sine.flac");
    std::fs::write(outside.join("kick.flac"), &flac).unwrap();
    std::fs::write(outside.join("snare.mp3"), fixture("sine.mp3")).unwrap();
    *h.dialog.0.lock().unwrap() = Some(vec![outside.join("kick.flac"), outside.join("snare.mp3")]);
    send(&mut ws, &ClientMessage::ImportChoose { id: 2 }).await;
    let ServerMessage::ImportChosen { id: 2, files } = next(&mut ws).await else {
        panic!("expected import.chosen")
    };
    assert_eq!(
        files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["kick.flac", "snare.mp3"]
    );
    for f in &files {
        assert!(helper::disk::valid_nonce(&f.token), "{}", f.token);
        assert!(!f.token.contains("Downloads"), "no path crosses");
    }
    let kick = ImportSource::Picked {
        token: files[0].token.clone(),
    };
    let end = import(&mut ws, 3, kick.clone()).await;
    let ServerMessage::ImportDone { hash, name, .. } = &end else {
        panic!("expected done, got {end:?}")
    };
    assert_eq!(name, "kick.flac");
    assert_eq!(
        std::fs::read(h.song.join("blobs").join(hash)).unwrap(),
        flac
    );
    assert_eq!(
        std::fs::read(outside.join("kick.flac")).unwrap(),
        flac,
        "the source is left"
    );
    // A token is used once.
    assert_eq!(
        failed_code(&import(&mut ws, 4, kick).await),
        errors::IMPORT_UNREADABLE
    );
    // A new session replaces this one: the tokens of the old one are gone.
    let snare = ImportSource::Picked {
        token: files[1].token.clone(),
    };
    drop(ws);
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    assert_eq!(
        failed_code(&import(&mut ws, 5, snare).await),
        errors::IMPORT_UNREADABLE
    );
}

#[tokio::test]
async fn a_helper_without_a_file_dialog_says_so() {
    let h = start().await;
    let mut ws = session(&h).await;
    open_folder(&mut ws, PROJECT, NONCE).await;
    send(&mut ws, &ClientMessage::ImportChoose { id: 1 }).await;
    assert_eq!(
        failed_code(&next(&mut ws).await),
        errors::IMPORT_DIALOG_UNAVAILABLE
    );
    // A picker that has no file dialog of its own (the folder dialog only) says the same.
    struct FoldersOnly;
    impl FolderPicker for FoldersOnly {
        fn pick(&self, _title: &str) -> Picked {
            Picked::Unavailable
        }
    }
    assert_eq!(FoldersOnly.pick_files("x"), PickedFiles::Unavailable);
}
