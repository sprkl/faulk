//! The helper's WebSocket server over real loopback connections: handshake checks,
//! probe, pairing, session, session replacement and the release of notes on close.

use futures_util::{SinkExt, StreamExt};
use helper::audio::{AudioCore, EngineSlot, Meter, audio_core};
use helper::midi::MidiManager;
use helper::security::Policy;
use helper::server::{HelperEvent, NO_OUTPUT, Shared, serve};
use helper::updater::{Backend, UpdateStatus, UpdateWatch, Updater};
use protocol::{ClientMessage, HelperUpdate, PROTOCOL_VERSION, ServerMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const ORIGIN: &str = "http://localhost:5173";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Harness {
    port: u16,
    code: Arc<Mutex<String>>,
    events: Arc<Mutex<Vec<HelperEvent>>>,
    shared: Arc<Shared>,
    core: AudioCore,
    /// The engine of the output; `remove` stands for an output that went away.
    engine: Arc<EngineSlot>,
}

async fn start() -> Harness {
    start_with_updates(None).await
}

async fn start_with_updates(updates: Option<UpdateWatch>) -> Harness {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let meter = Arc::new(Meter::default());
    let (core, sender) = audio_core(48_000.0, meter.clone()).unwrap();
    let engine = Arc::new(EngineSlot::default());
    engine.install(sender, 48_000.0);
    let (activity_tx, activity_rx) = mpsc::channel();
    let midi = MidiManager::new(engine.clone(), activity_tx);
    let code = Arc::new(Mutex::new(String::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let (shown, seen) = (code.clone(), events.clone());
    let shared = Shared::new(
        Policy {
            port,
            allowed_origins: vec![ORIGIN.into()],
            token: TOKEN.into(),
        },
        engine.clone(),
        midi,
        activity_rx,
        meter,
        Box::new(move |event| {
            if let HelperEvent::Code(c) = &event {
                *shown.lock().unwrap() = c.clone();
            }
            seen.lock().unwrap().push(event);
        }),
    );
    let shared = Arc::new(match updates {
        Some(watch) => shared.with_updates(watch),
        None => shared,
    });
    tokio::spawn(serve(listener, shared.clone()));
    Harness {
        port,
        code,
        events,
        shared,
        core,
        engine,
    }
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

fn refused(result: Result<Ws, Error>) -> bool {
    matches!(result, Err(Error::Http(response)) if response.status() == 403)
}

/// Next server message (text frames; the engine report frames are binary and skipped).
async fn next(ws: &mut Ws) -> ServerMessage {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("a message within 5 s")
            .expect("the stream is open")
            .expect("a valid frame");
        if let Message::Text(text) = frame {
            let message: ServerMessage = serde_json::from_str(&text).unwrap();
            return message;
        }
    }
}

async fn send(ws: &mut Ws, message: &ClientMessage) {
    ws.send(Message::text(serde_json::to_string(message).unwrap()))
        .await
        .unwrap();
}

/// Runs the audio core for `ms` of audio and returns the voices sounding at the end.
fn play(core: &mut AudioCore, ms: usize) -> u32 {
    let mut out = vec![0.0; 480 * 2];
    for _ in 0..ms / 10 {
        core.process(&mut out, 2);
    }
    core.meter().take().1
}

#[tokio::test]
async fn refuses_handshakes_without_origin_token_or_known_path() {
    let h = start().await;
    assert!(refused(connect(h.port, "/probe", None).await));
    assert!(refused(
        connect(h.port, "/probe", Some("https://evil.example")).await
    ));
    assert!(refused(connect(h.port, "/session", Some(ORIGIN)).await));
    assert!(refused(
        connect(
            h.port,
            "/session?token=ffffffffffffffffffffffffffffffff",
            Some(ORIGIN)
        )
        .await
    ));
    assert!(refused(connect(h.port, "/admin", Some(ORIGIN)).await));
}

#[tokio::test]
async fn probe_answers_with_the_protocol_version() {
    let h = start().await;
    let mut ws = connect(h.port, "/probe", Some(ORIGIN)).await.unwrap();
    assert_eq!(
        next(&mut ws).await,
        ServerMessage::Probe {
            helper: "faulk-helper".into(),
            protocol: PROTOCOL_VERSION,
            audio_output: true,
        }
    );
}

/// #525: without an output the helper still serves. The probe says so, and a session is
/// refused with `audio.no_output` (no hello, not counted as a page) until an output opens.
#[tokio::test]
async fn without_an_output_the_probe_says_so_and_a_session_is_refused() {
    let h = start().await;
    h.engine.remove();
    let mut probe = connect(h.port, "/probe", Some(ORIGIN)).await.unwrap();
    assert!(matches!(
        next(&mut probe).await,
        ServerMessage::Probe {
            audio_output: false,
            ..
        }
    ));

    let mut ws = connect(h.port, &format!("/session?token={TOKEN}"), Some(ORIGIN))
        .await
        .unwrap();
    match next(&mut ws).await {
        ServerMessage::Error { code, .. } => assert_eq!(code, NO_OUTPUT),
        other => panic!("expected the no-output refusal, got {other:?}"),
    }
    // Then the helper closes the session.
    let end = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("the session is closed");
    assert!(
        matches!(end, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
        "{end:?}"
    );
    assert!(
        !h.events
            .lock()
            .unwrap()
            .contains(&HelperEvent::Session(true)),
        "a refused page is not a session"
    );

    // An output opens: the next session gets its hello at that output's rate.
    let meter = Arc::new(Meter::default());
    let (_core, sender) = audio_core(44_100.0, meter).unwrap();
    h.engine.install(sender, 44_100.0);
    let mut probe = connect(h.port, "/probe", Some(ORIGIN)).await.unwrap();
    assert!(matches!(
        next(&mut probe).await,
        ServerMessage::Probe {
            audio_output: true,
            ..
        }
    ));
    let mut ws = connect(h.port, &format!("/session?token={TOKEN}"), Some(ORIGIN))
        .await
        .unwrap();
    match next(&mut ws).await {
        ServerMessage::Hello { sample_rate, .. } => assert_eq!(sample_rate, 44_100.0),
        other => panic!("expected hello, got {other:?}"),
    }
}

#[tokio::test]
async fn pairing_refuses_a_wrong_code_then_gives_the_token() {
    let h = start().await;
    let mut ws = connect(h.port, "/pair", Some(ORIGIN)).await.unwrap();
    send(
        &mut ws,
        &ClientMessage::Pair {
            code: "not-it".into(),
        },
    )
    .await;
    assert_eq!(
        next(&mut ws).await,
        ServerMessage::PairingRefused { attempts_left: 4 }
    );
    let code = h.code.lock().unwrap().clone();
    send(&mut ws, &ClientMessage::Pair { code }).await;
    assert_eq!(
        next(&mut ws).await,
        ServerMessage::Paired {
            token: TOKEN.into()
        }
    );
}

#[tokio::test]
async fn pairing_route_accepts_no_command() {
    let h = start().await;
    let mut ws = connect(h.port, "/pair", Some(ORIGIN)).await.unwrap();
    send(
        &mut ws,
        &ClientMessage::Command {
            command: commands::Command::Live(commands::live::Command::AllNotesOff {}),
        },
    )
    .await;
    assert!(
        matches!(next(&mut ws).await, ServerMessage::Error { code, .. } if code == "protocol.invalid")
    );
}

#[tokio::test]
async fn session_plays_commands_and_releases_notes_when_the_page_goes_away() {
    let mut h = start().await;
    let path = format!("/session?token={TOKEN}");
    let mut ws = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    assert!(
        matches!(next(&mut ws).await, ServerMessage::Hello { protocol, .. } if protocol == PROTOCOL_VERSION)
    );
    assert!(matches!(
        next(&mut ws).await,
        ServerMessage::MidiInputs { .. }
    ));

    send(
        &mut ws,
        &ClientMessage::Command {
            command: commands::Command::Live(commands::live::Command::NoteOn {
                track: project::TrackId::parse(protocol::single_synth::TRACK).unwrap(),
                note: 60,
                velocity: 100,
            }),
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(play(&mut h.core, 50), 1, "the note plays");

    ws.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    // The default release is 0.3 s: after 1 s every note has ended.
    assert_eq!(
        play(&mut h.core, 1_000),
        0,
        "closing the page released the note"
    );
}

#[tokio::test]
async fn a_new_session_replaces_the_previous_one() {
    let h = start().await;
    let path = format!("/session?token={TOKEN}");
    let mut first = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    assert!(matches!(
        next(&mut first).await,
        ServerMessage::Hello { .. }
    ));
    let mut second = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    assert!(matches!(
        next(&mut second).await,
        ServerMessage::Hello { .. }
    ));
    loop {
        match next(&mut first).await {
            ServerMessage::Error { code, .. } => {
                assert_eq!(code, "session.replaced");
                break;
            }
            ServerMessage::MidiInputs { .. } => continue,
            other => panic!("unexpected {other:?}"),
        }
    }
}

/// Waits until the events seen so far end with `expected`, or fails after 2 s.
async fn wait_events(h: &Harness, expected: &[HelperEvent]) {
    for _ in 0..200 {
        if h.events.lock().unwrap().ends_with(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "expected the events to end with {expected:?}, got {:?}",
        h.events.lock().unwrap()
    );
}

#[tokio::test]
async fn reports_the_first_code_and_each_new_one() {
    let h = start().await;
    let first = h.code.lock().unwrap().clone();
    assert_eq!(
        h.events.lock().unwrap().as_slice(),
        [HelperEvent::Code(first.clone())]
    );
    let mut ws = connect(h.port, "/pair", Some(ORIGIN)).await.unwrap();
    send(
        &mut ws,
        &ClientMessage::Pair {
            code: first.clone(),
        },
    )
    .await;
    assert!(matches!(next(&mut ws).await, ServerMessage::Paired { .. }));
    let second = h.code.lock().unwrap().clone();
    assert_ne!(second, first, "a used code is replaced");
    wait_events(&h, &[HelperEvent::Code(second)]).await;

    // Five wrong codes also replace it.
    let mut ws = connect(h.port, "/pair", Some(ORIGIN)).await.unwrap();
    for _ in 0..5 {
        send(&mut ws, &ClientMessage::Pair { code: "x".into() }).await;
        let _ = next(&mut ws).await;
    }
    let third = h.code.lock().unwrap().clone();
    wait_events(&h, &[HelperEvent::Code(third)]).await;
}

#[tokio::test]
async fn reports_sessions_and_ignores_the_end_of_a_replaced_one() {
    let h = start().await;
    let path = format!("/session?token={TOKEN}");
    let mut first = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    assert!(matches!(
        next(&mut first).await,
        ServerMessage::Hello { .. }
    ));
    wait_events(&h, &[HelperEvent::Session(true)]).await;

    let mut second = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    assert!(matches!(
        next(&mut second).await,
        ServerMessage::Hello { .. }
    ));
    first.close(None).await.ok();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        h.events.lock().unwrap().last(),
        Some(&HelperEvent::Session(true)),
        "the replaced session ending does not mean no page is connected"
    );

    second.close(None).await.unwrap();
    wait_events(&h, &[HelperEvent::Session(false)]).await;
}

#[tokio::test]
async fn reports_an_audio_error() {
    let h = start().await;
    h.shared.output_lost();
    wait_events(&h, &[HelperEvent::AudioError]).await;
}

/// A session opened on an output that has not called back yet waits for its first callback,
/// so its hello carries the buffer size and the latency (#525).
#[tokio::test]
async fn the_hello_of_a_new_output_waits_for_its_first_callback() {
    let mut h = start().await;
    let path = format!("/session?token={TOKEN}");
    let mut ws = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    // The output calls back 50 ms after the page connected.
    tokio::time::sleep(Duration::from_millis(50)).await;
    play(&mut h.core, 10);
    match next(&mut ws).await {
        ServerMessage::Hello {
            buffer_frames,
            estimated_latency_ms,
            ..
        } => {
            assert!(buffer_frames.is_some(), "the buffer size is known");
            assert!(estimated_latency_ms.is_some(), "the latency is known");
        }
        other => panic!("expected hello, got {other:?}"),
    }
}

/// An output that goes away while a session waits for its first callback: the page gets
/// `audio.stream` instead of a hello, and takes it as no output (#525).
#[tokio::test]
async fn an_output_lost_before_the_hello_is_said_instead_of_it() {
    let h = start().await;
    let path = format!("/session?token={TOKEN}");
    let mut ws = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    // No callback yet: the session waits; the output goes away meanwhile (no `output_lost`,
    // as if the supervisor had not told the server yet).
    tokio::time::sleep(Duration::from_millis(50)).await;
    h.engine.remove();
    match next(&mut ws).await {
        ServerMessage::Error { code, .. } => assert_eq!(code, "audio.stream"),
        other => panic!("expected audio.stream instead of hello, got {other:?}"),
    }
}

/// The page in Boost hears of a lost output at once; a page that connects to the next output
/// gets its hello and never the news of the old one (#525).
#[tokio::test]
async fn a_lost_output_is_told_to_its_session_only() {
    let h = start().await;
    let path = format!("/session?token={TOKEN}");
    let mut ws = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    assert!(matches!(next(&mut ws).await, ServerMessage::Hello { .. }));
    // What the supervisor does when the stream fails: empty the slot, then tell the server.
    h.engine.remove();
    h.shared.output_lost();
    loop {
        match next(&mut ws).await {
            ServerMessage::Error { code, .. } => {
                assert_eq!(code, "audio.stream");
                break;
            }
            ServerMessage::MidiInputs { .. } | ServerMessage::UpdateStatus { .. } => {}
            other => panic!("expected audio.stream, got {other:?}"),
        }
    }
    // The next output opens; the page that had Boost closes its session and comes back.
    drop(ws);
    let (_core, sender) = audio_core(48_000.0, Arc::new(Meter::default())).unwrap();
    h.engine.install(sender, 48_000.0);
    let mut ws = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    assert!(matches!(next(&mut ws).await, ServerMessage::Hello { .. }));
    // Nothing about the old output follows within a while (the ticker runs every 33 ms).
    let quiet = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let ServerMessage::Error { code, .. } = next(&mut ws).await {
                return code;
            }
        }
    })
    .await;
    assert!(quiet.is_err(), "no stale error: {quiet:?}");
}

/// The next binary frame of the session within 5 s (text frames are skipped).
async fn next_report(ws: &mut Ws) -> Vec<u8> {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("a report within 5 s")
            .expect("the stream is open")
            .expect("a valid frame");
        if let Message::Binary(bytes) = frame {
            return bytes.to_vec();
        }
    }
}

#[tokio::test]
async fn the_session_receives_the_engine_report_as_binary_frames() {
    let mut h = start().await;
    let path = format!("/session?token={TOKEN}");
    let mut ws = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    assert!(matches!(next(&mut ws).await, ServerMessage::Hello { .. }));
    send(
        &mut ws,
        &ClientMessage::Command {
            command: commands::Command::Live(commands::live::Command::NoteOn {
                track: project::TrackId::parse(protocol::single_synth::TRACK).unwrap(),
                note: 69,
                velocity: 127,
            }),
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    // A second of audio writes a report every 1/30 s.
    play(&mut h.core, 1_000);
    let bytes = next_report(&mut ws).await;
    let report = engine::host::decode_feedback(&bytes).expect("a report of this version");
    assert_eq!(report.meters.len(), 1, "the helper's one track");
    assert!(
        report.master.peak_l > 0.05,
        "the note is on the master meter"
    );
    assert!(report.meters[0].peak_l > 0.05);
    assert_eq!(report.voices, 1);
    // The helper times its render (#486): a load in 0..1, its peak at least as high.
    assert!(
        (0.0..=1.0).contains(&report.cpu_load),
        "{}",
        report.cpu_load
    );
    assert!(report.cpu_peak >= report.cpu_load && report.cpu_peak <= 1.0);
    // Reports keep coming only when the engine wrote a new one: another second of audio,
    // another frame, with a newer sequence number.
    play(&mut h.core, 1_000);
    let mut newer = None;
    for _ in 0..5 {
        let next = engine::host::decode_feedback(&next_report(&mut ws).await).unwrap();
        if next.seq != report.seq {
            newer = Some(next);
            break;
        }
    }
    assert!(newer.is_some(), "a newer report arrived");
}

#[tokio::test]
async fn an_unchanged_report_is_not_sent_again() {
    let mut h = start().await;
    let path = format!("/session?token={TOKEN}");
    let mut ws = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    assert!(matches!(next(&mut ws).await, ServerMessage::Hello { .. }));
    play(&mut h.core, 200);
    // Drain what the ticker sent while the audio ran, then the audio stops: no new report is
    // written, so none is sent.
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Ok(Some(Ok(_))) = tokio::time::timeout(Duration::from_millis(150), ws.next()).await {}
    let mut frames = 0;
    while let Ok(Some(Ok(frame))) =
        tokio::time::timeout(Duration::from_millis(400), ws.next()).await
    {
        if matches!(frame, Message::Binary(_)) {
            frames += 1;
        }
    }
    assert_eq!(frames, 0, "the same report was sent again");
}

/// An update found at start whose download waits for the test.
struct GatedUpdate {
    gate: Mutex<Option<mpsc::Receiver<()>>>,
    downloaded: AtomicBool,
}

impl Backend for GatedUpdate {
    fn current_version(&self) -> String {
        "0.1.11".into()
    }

    fn check(&self) -> Result<Option<velopack::UpdateInfo>, velopack::Error> {
        if self.downloaded.load(Ordering::SeqCst) {
            return Ok(None);
        }
        Ok(Some(velopack::UpdateInfo {
            TargetFullRelease: velopack::VelopackAsset {
                Version: "0.1.12".into(),
                ..Default::default()
            },
            ..Default::default()
        }))
    }

    fn download(
        &self,
        _update: &velopack::UpdateInfo,
        progress: mpsc::Sender<i16>,
    ) -> Result<(), velopack::Error> {
        let _ = progress.send(40);
        if let Some(gate) = self.gate.lock().unwrap().take() {
            let _ = gate.recv();
        }
        self.downloaded.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn pending(&self) -> Option<velopack::VelopackAsset> {
        self.downloaded
            .load(Ordering::SeqCst)
            .then(|| velopack::VelopackAsset {
                Version: "0.1.12".into(),
                ..Default::default()
            })
    }

    fn apply_and_restart(
        &self,
        _asset: &velopack::VelopackAsset,
        _args: &[String],
    ) -> Result<(), velopack::Error> {
        Ok(())
    }
}

/// Next `update.status` message, skipping the others.
async fn next_update(ws: &mut Ws) -> (Option<String>, Option<u64>, HelperUpdate) {
    loop {
        if let ServerMessage::UpdateStatus {
            current,
            checked_at,
            status,
        } = next(ws).await
        {
            return (current, checked_at, status);
        }
    }
}

#[tokio::test]
async fn the_session_is_told_the_update_status_at_start_and_on_change() {
    let (open, gate) = mpsc::channel();
    let backend = GatedUpdate {
        gate: Mutex::new(Some(gate)),
        downloaded: AtomicBool::new(false),
    };
    let updater = Updater::start_with(backend, Duration::from_secs(3600));
    // The download has started (and waits) before the page connects.
    for _ in 0..500 {
        if matches!(
            updater.status(),
            UpdateStatus::Downloading { percent: 40, .. }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let h = start_with_updates(Some(updater.watch())).await;
    let mut ws = connect(h.port, &format!("/session?token={TOKEN}"), Some(ORIGIN))
        .await
        .unwrap();
    assert!(matches!(next(&mut ws).await, ServerMessage::Hello { .. }));
    assert!(matches!(
        next(&mut ws).await,
        ServerMessage::MidiInputs { .. }
    ));
    // Third message of the session: the update in progress.
    assert_eq!(
        next(&mut ws).await,
        ServerMessage::UpdateStatus {
            current: Some("0.1.11".into()),
            checked_at: None,
            status: HelperUpdate::Downloading {
                version: "0.1.12".into(),
                percent: 40
            },
        }
    );
    // The download ends: the change reaches the page within the ticker's second.
    open.send(()).unwrap();
    let (current, mut checked_at, status) = next_update(&mut ws).await;
    assert_eq!(current.as_deref(), Some("0.1.11"));
    assert_eq!(
        status,
        HelperUpdate::Ready {
            version: "0.1.12".into()
        }
    );
    // The end of the check may come with the same message or the next one.
    if checked_at.is_none() {
        checked_at = next_update(&mut ws).await.1;
    }
    assert!(checked_at.is_some());
    updater.shutdown();
}

/// Counts its checks; finds nothing.
struct CountingUpdate(Arc<std::sync::atomic::AtomicUsize>);

impl Backend for CountingUpdate {
    fn current_version(&self) -> String {
        "0.1.11".into()
    }

    fn check(&self) -> Result<Option<velopack::UpdateInfo>, velopack::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }

    fn download(
        &self,
        _update: &velopack::UpdateInfo,
        _progress: mpsc::Sender<i16>,
    ) -> Result<(), velopack::Error> {
        Ok(())
    }

    fn pending(&self) -> Option<velopack::VelopackAsset> {
        None
    }

    fn apply_and_restart(
        &self,
        _asset: &velopack::VelopackAsset,
        _args: &[String],
    ) -> Result<(), velopack::Error> {
        Ok(())
    }
}

#[tokio::test]
async fn a_page_connecting_asks_for_a_check_within_the_rate_limit() {
    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // Gap of zero: every connection after the first check is due.
    let updater = Updater::start_with(CountingUpdate(checks.clone()), Duration::from_secs(3600))
        .with_page_gap(Duration::ZERO);
    let watch = updater.watch();
    for _ in 0..500 {
        if watch.checked_at().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    let h = start_with_updates(Some(watch)).await;
    let mut ws = connect(h.port, &format!("/session?token={TOKEN}"), Some(ORIGIN))
        .await
        .unwrap();
    assert!(matches!(next(&mut ws).await, ServerMessage::Hello { .. }));
    for _ in 0..500 {
        if checks.load(Ordering::SeqCst) >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    updater.shutdown();
}

#[tokio::test]
async fn a_page_connecting_inside_the_gap_does_not_check_again() {
    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let updater = Updater::start_with(CountingUpdate(checks.clone()), Duration::from_secs(3600));
    let watch = updater.watch();
    for _ in 0..500 {
        if watch.checked_at().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let h = start_with_updates(Some(watch)).await;
    let mut ws = connect(h.port, &format!("/session?token={TOKEN}"), Some(ORIGIN))
        .await
        .unwrap();
    assert!(matches!(next(&mut ws).await, ServerMessage::Hello { .. }));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    updater.shutdown();
}
