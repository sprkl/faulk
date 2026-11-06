//! The project's folder in Boost mode (AU-7, #116), over real loopback connections: the helper
//! plays the audio clips of the folder the page has open and no audio crosses the link; the
//! folder is reached through the helper's own dialog and the page's marker, never a path; two
//! folders of one project are a question; a dialog's answer after the page moved on opens
//! nothing; folder messages need a session, with its token and origin.

use futures_util::{SinkExt, StreamExt};
use helper::audio::{AudioCore, EngineSlot, Meter, audio_core};
use helper::disk::{FolderPicker, LINK_FILE, Picked, Remembered, spawn_disk};
use helper::live::LiveInput;
use helper::midi::{MidiManager, MidiSink};
use helper::security::Policy;
use helper::server::{Shared, serve};
use protocol::{ClientMessage, FolderRefusal, FolderStatus, ServerMessage, paths};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const ORIGIN: &str = "http://localhost:5173";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const PROJECT: &str = "01J0000000000000000000000A";
const OTHER: &str = "01J0000000000000000000000B";
const NONCE: &str = "00112233445566778899aabbccddeeff";
const RATE: u32 = 48_000;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The helper's dialog, as the user: each `pick` waits for the test's answer.
struct Dialog {
    answers: Mutex<mpsc::Receiver<Picked>>,
    calls: AtomicUsize,
}

impl FolderPicker for Dialog {
    fn pick(&self, _title: &str) -> Picked {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answers
            .lock()
            .unwrap()
            .recv()
            .unwrap_or(Picked::Cancelled)
    }
}

struct Harness {
    port: u16,
    shared: Arc<Shared>,
    core: AudioCore,
    dialog: Arc<Dialog>,
    /// What the user picks in the dialog.
    user: mpsc::Sender<Picked>,
    _engine: Arc<EngineSlot>,
    _temp: Temp,
    temp: PathBuf,
}

/// A directory of its own, removed on drop.
struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn start(remembered: impl FnOnce(&Path) -> Vec<PathBuf>) -> Harness {
    let temp =
        std::env::temp_dir().join(format!("faulk-116-it-{}", helper::security::random_hex(6)));
    std::fs::create_dir_all(&temp).unwrap();
    let temp = std::fs::canonicalize(&temp).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let meter = Arc::new(Meter::default());
    let (core, sender) = audio_core(RATE as f32, meter.clone()).unwrap();
    let engine = Arc::new(EngineSlot::default());
    engine.install(sender, RATE as f32);
    let (midi_out, midi_in) = tokio::sync::mpsc::unbounded_channel();
    let sink = MidiSink::new(Arc::new(LiveInput::new(engine.clone())), midi_out);
    let (user, answers) = mpsc::channel();
    let dialog = Arc::new(Dialog {
        answers: Mutex::new(answers),
        calls: AtomicUsize::new(0),
    });
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
            Remembered::in_memory(remembered(&temp)),
            dialog.clone(),
            helper::tray_text::Lang::En,
        ),
    );
    let _ = spawn_disk(&engine, shared.open_folder());
    tokio::spawn(serve(listener, shared.clone()));
    Harness {
        port,
        shared,
        core,
        dialog,
        user,
        _engine: engine,
        _temp: Temp(temp.clone()),
        temp,
    }
}

/// A project folder `name` below `parent`, of project `id`, holding the impulses' blob; with
/// the page's marker when `nonce`.
fn project_dir(parent: &Path, name: &str, id: &str, nonce: Option<&str>) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    std::fs::write(
        dir.join("project.json"),
        format!("{{\"id\": \"{id}\", \"name\": \"{name}\"}}"),
    )
    .unwrap();
    let hex: String = test_support::clip_start::IMPULSES
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(dir.join("blobs").join(hex), noise_wav()).unwrap();
    if let Some(nonce) = nonce {
        mark(&dir, nonce);
    }
    dir
}

/// The blob of the impulses' media (`test_support::clip_start`: mono, 16 bits, half a second),
/// filled with noise instead of impulses: any 32 bytes of it are its own, so finding them in a
/// frame means the blob crossed the link.
fn noise_wav() -> Vec<u8> {
    let mut wav = test_support::clip_start::impulse_wav(RATE);
    let mut x: u32 = 0x1234_5678;
    for sample in wav[44..].as_chunks_mut::<2>().0 {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let v = ((x >> 16) as i16) / 2;
        sample.copy_from_slice(&v.to_le_bytes());
    }
    wav
}

/// The page's link marker in `dir`, as the page writes it.
fn mark(dir: &Path, nonce: &str) {
    std::fs::create_dir_all(dir.join(".faulk")).unwrap();
    std::fs::write(
        dir.join(".faulk").join(LINK_FILE),
        format!("{{\"nonce\": \"{nonce}\"}}"),
    )
    .unwrap();
}

async fn connect(port: u16, path: &str, origin: Option<&str>) -> Result<Ws, Error> {
    let mut request = format!("ws://127.0.0.1:{port}{path}")
        .into_client_request()
        .unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
    }
    connect_async(request).await.map(|(ws, _)| ws)
}

/// Every frame the page received, in order.
#[derive(Default)]
struct Received {
    texts: Vec<String>,
    binaries: Vec<Vec<u8>>,
}

/// Next JSON message, keeping every frame received on the way in `seen`.
async fn next_in(ws: &mut Ws, seen: &mut Received) -> ServerMessage {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("a message within 5 s")
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
    next_in(ws, &mut Received::default()).await
}

async fn send(ws: &mut Ws, message: &ClientMessage) {
    ws.send(Message::text(serde_json::to_string(message).unwrap()))
        .await
        .unwrap();
}

async fn session(h: &Harness) -> Ws {
    let mut ws = connect(h.port, &format!("/session?token={TOKEN}"), Some(ORIGIN))
        .await
        .unwrap();
    assert!(matches!(next(&mut ws).await, ServerMessage::Hello { .. }));
    ws
}

fn open(project: &str, nonce: &str) -> ClientMessage {
    ClientMessage::FolderOpen {
        project: project.into(),
        nonce: nonce.into(),
        name: "Song".into(),
    }
}

fn choose(project: &str, nonce: &str) -> ClientMessage {
    ClientMessage::FolderChoose {
        project: project.into(),
        nonce: nonce.into(),
        name: "Song".into(),
    }
}

fn state(message: ServerMessage) -> (Option<String>, FolderStatus) {
    match message {
        ServerMessage::FolderState { project, state } => (project, state),
        other => panic!("expected folder.state, got {other:?}"),
    }
}

/// Renders `ms` of audio; returns the largest sample.
fn render(core: &mut AudioCore, ms: usize) -> f32 {
    let mut out = vec![0.0; 480 * 2];
    let mut peak = 0.0f32;
    for _ in 0..ms / 10 {
        core.process(&mut out, 2);
        peak = out.iter().fold(peak, |p, s| p.max(s.abs()));
    }
    peak
}

/// The engine's report now (its offline sources).
fn offline_sources(h: &Harness) -> Option<u32> {
    let mut bytes = Vec::new();
    h.core
        .meter()
        .copy_report(&mut bytes)
        .then(|| engine::host::decode_feedback(&bytes))
        .flatten()
        .map(|f| f.offline_sources)
}

/// Renders until the engine counts no source offline (the disk thread served its probes and
/// its segments arrived), or fails after 8 s. A source found offline is probed again 2 s later
/// (`REPROBE_MS`, on the disk thread's clock).
fn render_until_online(h: &mut Harness) {
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        render(&mut h.core, 10);
        if offline_sources(h) == Some(0) {
            // A few blocks more for the segments the probes asked for.
            render(&mut h.core, 200);
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the source never came online: {:?}", offline_sources(h));
}

/// Whether `haystack` holds `needle` anywhere.
fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_helper_plays_audio_clips_from_the_folder_and_no_audio_crosses_the_link() {
    let mut h = start(|temp| vec![project_dir(temp, "Song", PROJECT, Some(NONCE))]).await;
    let mut ws = session(&h).await;
    let mut seen = Received::default();
    let (setup, play, stop) = test_support::clip_start::impulse_streams(RATE, &[]);
    let mut sent: Vec<Vec<u8>> = vec![setup.clone()];
    ws.send(Message::binary(setup)).await.unwrap();

    // Before the folder: the clip's source stays offline and the clip silent.
    tokio::time::sleep(Duration::from_millis(100)).await;
    render(&mut h.core, 300);
    assert!(
        offline_sources(&h).unwrap_or(0) > 0,
        "offline without a folder"
    );
    sent.push(play.clone());
    ws.send(Message::binary(play)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(render(&mut h.core, 500), 0.0, "silent without a folder");
    sent.push(stop.clone());
    ws.send(Message::binary(stop)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    render(&mut h.core, 50);

    // The page names its project and marker: the remembered folder holding them opens.
    send(&mut ws, &open(PROJECT, NONCE)).await;
    let (project, status) = state(next_in(&mut ws, &mut seen).await);
    assert_eq!(project.as_deref(), Some(PROJECT));
    assert_eq!(
        status,
        FolderStatus::Open {
            name: "Song".into()
        }
    );
    render_until_online(&mut h);
    let (_, play, _) = test_support::clip_start::impulse_streams(RATE, &[]);
    sent.push(play.clone());
    ws.send(Message::binary(play)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let peak = render(&mut h.core, 600);
    assert!(peak > 0.1, "the impulses play from the folder: peak {peak}");

    // Keep reading what the helper sends for a while: reports and messages only.
    let until = tokio::time::Instant::now() + Duration::from_millis(400);
    while tokio::time::Instant::now() < until {
        match tokio::time::timeout(Duration::from_millis(50), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => seen.binaries.push(b.to_vec()),
            Ok(Some(Ok(Message::Text(t)))) => seen.texts.push(t.to_string()),
            _ => {}
        }
        render(&mut h.core, 10);
    }
    assert!(!seen.binaries.is_empty(), "the engine's reports came");
    // The blob's samples: 32-byte windows of its noise, every 1 000 bytes of its data.
    let wav = noise_wav();
    let pieces: Vec<&[u8]> = (44..wav.len() - 32)
        .step_by(1000)
        .map(|at| &wav[at..at + 32])
        .collect();
    assert!(pieces.len() > 40);
    for frame in &seen.binaries {
        assert!(
            engine::host::decode_feedback(frame).is_some(),
            "a binary frame from the helper is an engine report ({} bytes)",
            frame.len()
        );
        for piece in &pieces {
            assert!(!holds(frame, piece), "a report carries the blob's samples");
        }
    }
    for text in &seen.texts {
        for piece in &pieces {
            assert!(!holds(text.as_bytes(), piece));
        }
    }
    // And the page's frames: engine streams, without the samples either.
    for stream in &sent {
        for piece in &pieces {
            assert!(!holds(stream, piece), "a page stream carries the samples");
        }
    }
}

#[tokio::test]
async fn two_folders_of_one_project_are_a_question_and_the_folder_shown_opens() {
    let h = start(|temp| {
        std::fs::create_dir_all(temp.join("backup")).unwrap();
        vec![
            project_dir(temp, "Song", PROJECT, Some(NONCE)),
            project_dir(&temp.join("backup"), "Song", PROJECT, Some(NONCE)),
        ]
    })
    .await;
    let mut ws = session(&h).await;
    send(&mut ws, &open(PROJECT, NONCE)).await;
    assert_eq!(
        state(next(&mut ws).await).1,
        FolderStatus::Ambiguous { count: 2 }
    );
    assert!(h.shared.open_folder().get().1.is_none(), "no silent choice");
    // The user shows the copy in the backup.
    send(&mut ws, &choose(PROJECT, NONCE)).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Choosing);
    h.user
        .send(Picked::Folder(h.temp.join("backup").join("Song")))
        .unwrap();
    assert_eq!(
        state(next(&mut ws).await).1,
        FolderStatus::Open {
            name: "Song".into()
        }
    );
    let open_now = h.shared.open_folder().get().1.unwrap();
    assert_eq!(
        open_now.root(),
        std::fs::canonicalize(h.temp.join("backup").join("Song")).unwrap()
    );
}

#[tokio::test]
async fn a_folder_never_shown_is_asked_for_and_only_the_one_open_in_the_page_is_taken() {
    let h = start(|_| Vec::new()).await;
    let song = project_dir(&h.temp, "Song", PROJECT, Some(NONCE));
    let copy = project_dir(&h.temp, "Song copy", PROJECT, None);
    let other = project_dir(&h.temp, "Other", OTHER, Some(NONCE));
    let empty = h.temp.join("Empty");
    std::fs::create_dir_all(&empty).unwrap();
    let mut ws = session(&h).await;
    send(&mut ws, &open(PROJECT, NONCE)).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Unknown);
    for (picked, reason) in [
        (Picked::Cancelled, FolderRefusal::Cancelled),
        (Picked::Unavailable, FolderRefusal::Unavailable),
        (Picked::Folder(empty.clone()), FolderRefusal::NoProject),
        (Picked::Folder(other.clone()), FolderRefusal::OtherProject),
        (Picked::Folder(copy.clone()), FolderRefusal::NotThisFolder),
        (
            Picked::Folder(h.temp.join("missing")),
            FolderRefusal::Unreadable,
        ),
    ] {
        send(&mut ws, &choose(PROJECT, NONCE)).await;
        assert_eq!(state(next(&mut ws).await).1, FolderStatus::Choosing);
        h.user.send(picked).unwrap();
        assert_eq!(
            state(next(&mut ws).await).1,
            FolderStatus::Refused { reason }
        );
        assert!(h.shared.open_folder().get().1.is_none());
    }
    assert!(
        h.shared.remembered_folders().is_empty(),
        "nothing refused is kept"
    );
    send(&mut ws, &choose(PROJECT, NONCE)).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Choosing);
    h.user.send(Picked::Folder(song.clone())).unwrap();
    assert!(matches!(
        state(next(&mut ws).await).1,
        FolderStatus::Open { .. }
    ));
    assert_eq!(
        h.shared.remembered_folders(),
        vec![std::fs::canonicalize(&song).unwrap()]
    );
    // Remembered: the next session opens it by itself.
    drop(ws);
    let mut ws = session(&h).await;
    send(&mut ws, &open(PROJECT, NONCE)).await;
    assert!(matches!(
        state(next(&mut ws).await).1,
        FolderStatus::Open { .. }
    ));
}

#[tokio::test]
async fn one_dialog_at_a_time_and_an_answer_after_the_page_moved_on_opens_nothing() {
    let h = start(|_| Vec::new()).await;
    let song = project_dir(&h.temp, "Song", PROJECT, Some(NONCE));
    let mut ws = session(&h).await;
    send(&mut ws, &choose(PROJECT, NONCE)).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Choosing);
    // A second request while the dialog is open shows nothing more.
    send(&mut ws, &choose(PROJECT, NONCE)).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Choosing);
    // The page opens another project meanwhile.
    send(&mut ws, &open(OTHER, NONCE)).await;
    assert_eq!(
        state(next(&mut ws).await),
        (Some(OTHER.into()), FolderStatus::Unknown)
    );
    h.user.send(Picked::Folder(song.clone())).unwrap();
    // The answer is remembered (the user picked it) but opens nothing: the page hears the state
    // of the place it names now.
    assert_eq!(
        state(next(&mut ws).await),
        (Some(OTHER.into()), FolderStatus::Unknown)
    );
    assert_eq!(h.dialog.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        h.shared.remembered_folders(),
        vec![std::fs::canonicalize(&song).unwrap()]
    );
    assert!(h.shared.open_folder().get().1.is_none());
    send(&mut ws, &ClientMessage::FolderClose).await;
    assert_eq!(state(next(&mut ws).await), (None, FolderStatus::Closed));
}

#[tokio::test]
async fn a_request_that_found_the_dialog_open_for_another_project_is_answered_when_it_closes() {
    let h = start(|_| Vec::new()).await;
    let other = project_dir(&h.temp, "Other", OTHER, Some(NONCE));
    let mut ws = session(&h).await;
    send(&mut ws, &choose(PROJECT, NONCE)).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Choosing);
    // The page moved to another project and asks again while the first dialog is open.
    send(&mut ws, &choose(OTHER, NONCE)).await;
    assert_eq!(
        state(next(&mut ws).await),
        (Some(OTHER.into()), FolderStatus::Choosing)
    );
    // The user shows the other project's folder in the open dialog (asked for the first): not
    // the first project's, so not taken, but the page waiting on `choosing` is told where it
    // stands; never left waiting.
    h.user.send(Picked::Folder(other.clone())).unwrap();
    assert_eq!(
        state(next(&mut ws).await),
        (Some(OTHER.into()), FolderStatus::Unknown)
    );
    // Its own dialog now: the folder opens.
    send(&mut ws, &choose(OTHER, NONCE)).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Choosing);
    h.user.send(Picked::Folder(other.clone())).unwrap();
    assert!(matches!(
        state(next(&mut ws).await),
        (Some(p), FolderStatus::Open { .. }) if p == OTHER
    ));
    assert_eq!(h.dialog.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_page_that_replaced_another_during_its_dialog_is_answered_when_the_dialog_closes() {
    let h = start(|_| Vec::new()).await;
    let song = project_dir(&h.temp, "Song", PROJECT, Some(NONCE));
    let mut old = session(&h).await;
    send(&mut old, &choose(PROJECT, NONCE)).await;
    assert_eq!(state(next(&mut old).await).1, FolderStatus::Choosing);
    // The tab reloads: a new page, a new marker, while the first dialog is still open.
    let mut ws = session(&h).await;
    const RELOADED: &str = "ffeeddccbbaa99887766554433221100";
    mark(&song, RELOADED);
    send(&mut ws, &choose(PROJECT, RELOADED)).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Choosing);
    // The user picks the folder in the open dialog (the old page's): it holds the new marker, not
    // the old one, so it is not taken for the old page; the new page hears its own place's state.
    h.user.send(Picked::Folder(song.clone())).unwrap();
    assert_eq!(
        state(next(&mut ws).await),
        (Some(PROJECT.into()), FolderStatus::Unknown)
    );
    assert!(h.shared.open_folder().get().1.is_none());
    // Its own dialog opens the folder.
    send(&mut ws, &choose(PROJECT, RELOADED)).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Choosing);
    h.user.send(Picked::Folder(song.clone())).unwrap();
    assert!(matches!(
        state(next(&mut ws).await).1,
        FolderStatus::Open { .. }
    ));
    assert_eq!(h.dialog.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_folder_closes_with_the_session_and_on_folder_close() {
    let h = start(|temp| vec![project_dir(temp, "Song", PROJECT, Some(NONCE))]).await;
    let mut ws = session(&h).await;
    send(&mut ws, &open(PROJECT, NONCE)).await;
    assert!(matches!(
        state(next(&mut ws).await).1,
        FolderStatus::Open { .. }
    ));
    send(&mut ws, &ClientMessage::FolderClose).await;
    assert_eq!(state(next(&mut ws).await).1, FolderStatus::Closed);
    assert!(h.shared.open_folder().get().1.is_none());
    send(&mut ws, &open(PROJECT, NONCE)).await;
    assert!(matches!(
        state(next(&mut ws).await).1,
        FolderStatus::Open { .. }
    ));
    // Another page replaces this one: its folder closes.
    let _other = session(&h).await;
    for _ in 0..100 {
        if h.shared.open_folder().get().1.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(h.shared.open_folder().get().1.is_none());
}

#[tokio::test]
async fn folder_messages_need_a_session_its_token_its_origin_and_well_formed_names() {
    let h = start(|_| Vec::new()).await;
    let song = project_dir(&h.temp, "Song", PROJECT, Some(NONCE));
    let before = std::fs::read_dir(&song).unwrap().count();
    // Without the token, with a wrong one, from another origin or with none: refused at the
    // handshake, before any message.
    for (path, origin) in [
        ("/session".to_owned(), Some(ORIGIN)),
        ("/session?token=wrong".to_owned(), Some(ORIGIN)),
        (
            format!("/session?token={TOKEN}"),
            Some("https://evil.example"),
        ),
        (format!("/session?token={TOKEN}"), None),
    ] {
        let refused = connect(h.port, &path, origin).await;
        assert!(
            matches!(&refused, Err(Error::Http(r)) if r.status() == 403),
            "{path} from {origin:?}"
        );
    }
    // On the control link (agents only): invalid, no dialog.
    let mut control = connect(
        h.port,
        &format!("{}?token={TOKEN}", paths::CONTROL),
        Some(ORIGIN),
    )
    .await
    .unwrap();
    assert!(matches!(
        next(&mut control).await,
        ServerMessage::ControlHello { .. }
    ));
    send(&mut control, &choose(PROJECT, NONCE)).await;
    assert!(matches!(
        next(&mut control).await,
        ServerMessage::Error { code, .. } if code == "protocol.invalid"
    ));
    // On the session, a message that names no project id or nonce, or a path for a name.
    let mut ws = session(&h).await;
    for message in [
        open("not a ulid", NONCE),
        open(PROJECT, "short"),
        open(PROJECT, &NONCE.to_uppercase()),
        ClientMessage::FolderChoose {
            project: PROJECT.into(),
            nonce: NONCE.into(),
            name: "x".repeat(300),
        },
        ClientMessage::FolderChoose {
            project: PROJECT.into(),
            nonce: NONCE.into(),
            name: "a\nb".into(),
        },
    ] {
        send(&mut ws, &message).await;
        assert!(matches!(
            next(&mut ws).await,
            ServerMessage::Error { code, .. } if code == "protocol.invalid"
        ));
    }
    assert_eq!(h.dialog.calls.load(Ordering::SeqCst), 0, "no dialog shown");
    assert!(h.shared.open_folder().get().1.is_none());
    assert_eq!(std::fs::read_dir(&song).unwrap().count(), before);
}
