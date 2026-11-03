//! The helper's WebSocket server over real loopback connections: handshake checks,
//! probe, pairing, session, session replacement and the release of notes on close.

use futures_util::{SinkExt, StreamExt};
use helper::audio::{AudioCore, EngineSlot, Meter, audio_core};
use helper::live::LiveInput;
use helper::midi::{MidiManager, MidiSink};
use helper::security::Policy;
use helper::server::{HelperEvent, NO_OUTPUT, Shared, serve};
use helper::updater::{Backend, UpdateStatus, UpdateWatch, Updater};
use protocol::{AgentResult, ClientMessage, HelperUpdate, PROTOCOL_VERSION, ServerMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const ORIGIN: &str = "http://localhost:5173";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const MCP_TOKEN: &str = "fedcba9876543210fedcba9876543210";
/// How long an agent's call waits for the page in these tests.
const AGENT_WAIT: Duration = Duration::from_secs(2);

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Harness {
    port: u16,
    code: Arc<Mutex<String>>,
    events: Arc<Mutex<Vec<HelperEvent>>>,
    shared: Arc<Shared>,
    core: AudioCore,
    /// The engine of the output; `remove` stands for an output that went away.
    engine: Arc<EngineSlot>,
    /// The selected MIDI input: `receive` stands for a key played on it.
    midi: MidiSink,
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
    let (midi_out, midi_in) = tokio::sync::mpsc::unbounded_channel();
    let sink = MidiSink::new(Arc::new(LiveInput::new(engine.clone())), midi_out);
    let midi = MidiManager::new(sink.clone());
    let code = Arc::new(Mutex::new(String::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let (shown, seen) = (code.clone(), events.clone());
    let shared = Shared::new(
        Policy {
            port,
            allowed_origins: vec![ORIGIN.into()],
            token: TOKEN.into(),
            mcp_token: helper::security::McpToken::Fixed(MCP_TOKEN.into()),
        },
        engine.clone(),
        midi,
        midi_in,
        meter,
        Box::new(move |event| {
            if let HelperEvent::Code(c) = &event {
                *shown.lock().unwrap() = c.clone();
            }
            seen.lock().unwrap().push(event);
        }),
    )
    .with_agent_timeout(AGENT_WAIT);
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
        midi: sink,
    }
}

/// The page's side of a session: its project core, which encodes the engine streams.
struct Page(commands::engine_msgs::Synced);

impl Page {
    fn new() -> Page {
        Page(commands::engine_msgs::Synced::single_synth(1))
    }

    /// A page whose one synth is selected: the live input plays it.
    fn selected() -> Page {
        let mut page = Page::new();
        let command = serde_json::json!({
            "type": "view.select_track",
            "track": protocol::single_synth::TRACK,
        });
        page.run(serde_json::from_value(command).unwrap());
        page
    }

    /// The snapshot that starts the helper's engine over, newer than the epoch of its hello.
    fn snapshot_after(&mut self, epoch: u16) -> Vec<u8> {
        self.0.resync_after(epoch).unwrap()
    }

    fn run(&mut self, command: commands::Command) -> Vec<u8> {
        self.0.run(&command).unwrap().1
    }

    fn note_on(&mut self, note: u8) -> Vec<u8> {
        self.run(commands::Command::Live(commands::live::Command::NoteOn {
            track: project::TrackId::parse(protocol::single_synth::TRACK).unwrap(),
            note,
            velocity: 127,
        }))
    }
}

/// Opens a session; returns it with the epoch its hello announced (the messages up to
/// `midi.inputs` are read).
async fn open_session(h: &Harness) -> (Ws, u16) {
    let mut ws = connect(h.port, &format!("/session?token={TOKEN}"), Some(ORIGIN))
        .await
        .unwrap();
    let epoch = match next(&mut ws).await {
        ServerMessage::Hello { epoch, .. } => epoch,
        other => panic!("expected hello, got {other:?}"),
    };
    assert!(matches!(
        next(&mut ws).await,
        ServerMessage::MidiInputs { .. }
    ));
    (ws, epoch)
}

async fn send_bytes(ws: &mut Ws, bytes: Vec<u8>) {
    ws.send(Message::binary(bytes)).await.unwrap();
}

/// Waits until the engine has received `n` streams in all (the session's reader submitted
/// them), or fails after 5 s.
async fn received(h: &Harness, n: u64) {
    for _ in 0..500 {
        if h.engine.with(|s| s.received()) >= Some(n) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{n} streams were not received");
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
async fn pairing_route_accepts_nothing_but_a_pairing() {
    let h = start().await;
    let mut ws = connect(h.port, "/pair", Some(ORIGIN)).await.unwrap();
    send(&mut ws, &ClientMessage::MidiListInputs).await;
    assert!(
        matches!(next(&mut ws).await, ServerMessage::Error { code, .. } if code == "protocol.invalid")
    );
}

#[tokio::test]
async fn the_hello_announces_the_engines_epoch_and_what_the_helper_can_do() {
    let h = start().await;
    let mut ws = connect(h.port, &format!("/session?token={TOKEN}"), Some(ORIGIN))
        .await
        .unwrap();
    match next(&mut ws).await {
        ServerMessage::Hello {
            protocol,
            epoch,
            capabilities,
            ..
        } => {
            assert_eq!(protocol, PROTOCOL_VERSION);
            assert_eq!(epoch, 0, "a fresh engine");
            assert!(!capabilities.disk, "the helper writes no project file yet");
            assert_eq!(capabilities.midi, helper::midi::available());
        }
        other => panic!("expected hello, got {other:?}"),
    }
}

#[tokio::test]
async fn session_plays_the_pages_streams_and_releases_notes_when_the_page_goes_away() {
    let mut h = start().await;
    let (mut ws, epoch) = open_session(&h).await;
    let mut page = Page::new();
    send_bytes(&mut ws, page.snapshot_after(epoch)).await;
    send_bytes(&mut ws, page.note_on(60)).await;
    received(&h, 2).await;
    assert_eq!(play(&mut h.core, 50), 1, "the note plays");
    // A second page connecting later gets the epoch the first one left.
    assert_eq!(h.engine.current().unwrap().1, page.0.epoch());

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

/// An output replaced within the grace plays the same engine (#577): the session stays and
/// gets a new hello at the new output's rate and latency, and no error.
#[tokio::test]
async fn a_reopened_output_says_hello_again_at_its_rate() {
    let mut h = start().await;
    let path = format!("/session?token={TOKEN}");
    let mut ws = connect(h.port, &path, Some(ORIGIN)).await.unwrap();
    match next(&mut ws).await {
        ServerMessage::Hello { sample_rate, .. } => assert_eq!(sample_rate, 48_000.0),
        other => panic!("expected hello, got {other:?}"),
    }
    // What the supervisor does: the old output forgotten (`forget_output`), the same engine at
    // the new rate, the new output's first callback, then the server is told.
    h.core.meter().forget_output();
    assert_eq!(h.core.meter().callback_frames(), None);
    h.engine.set_sample_rate(44_100.0).unwrap();
    // The new output calls back with 441-frame buffers.
    let mut out = vec![0.0; 441 * 2];
    h.core.process(&mut out, 2);
    h.shared.output_reopened();
    loop {
        match next(&mut ws).await {
            ServerMessage::Hello {
                sample_rate,
                buffer_frames,
                estimated_latency_ms,
                ..
            } => {
                assert_eq!(sample_rate, 44_100.0);
                assert_eq!(buffer_frames, Some(441));
                assert!(estimated_latency_ms.is_some());
                break;
            }
            ServerMessage::MidiInputs { .. } | ServerMessage::UpdateStatus { .. } => {}
            other => panic!("expected a second hello, got {other:?}"),
        }
    }
    assert!(h.engine.is_open());
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
    let (mut ws, epoch) = open_session(&h).await;
    // The page's project has three tracks; the last one plays a note.
    let mut page = Page::new();
    for _ in 0..2 {
        page.run(commands::Command::Track(commands::track::Command::Create {
            kind: project::TrackKind::Midi,
            instrument: None,
            name: None,
            color: None,
        }));
    }
    let last = page.0.state.project.tracks.keys().last().unwrap().clone();
    send_bytes(&mut ws, page.snapshot_after(epoch)).await;
    let note = page.run(commands::Command::Live(commands::live::Command::NoteOn {
        track: last.clone(),
        note: 69,
        velocity: 127,
    }));
    send_bytes(&mut ws, note).await;
    received(&h, 2).await;
    // A second of audio writes a report every 1/30 s.
    play(&mut h.core, 1_000);
    let bytes = next_report(&mut ws).await;
    let report = engine::host::decode_feedback(&bytes).expect("a report of this version");
    assert_eq!(report.meters.len(), 3, "the page's three tracks");
    assert!(
        report.master.peak_l > 0.05,
        "the note is on the master meter"
    );
    let slot = page.0.ctx.engine.tracks.get(&last).unwrap() as usize;
    for (i, meter) in report.meters.iter().enumerate() {
        assert_eq!(meter.peak_l > 0.05, i == slot, "track slot {i}: {meter:?}");
    }
    assert_eq!(report.voices, 1);
    assert_eq!(report.epoch, page.0.epoch(), "the page's epoch");
    assert_eq!(report.cpu_load, engine::host::NOT_MEASURED);
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

/// The next `stream_refused` of the session, skipping the other messages and the reports.
async fn next_refusal(ws: &mut Ws) -> (i32, u32, u16) {
    loop {
        if let ServerMessage::StreamRefused { code, at, epoch } = next(ws).await {
            return (code, at, epoch);
        }
    }
}

/// The next `error` code of the session, skipping the other messages and the reports.
async fn next_error(ws: &mut Ws) -> String {
    loop {
        if let ServerMessage::Error { code, .. } = next(ws).await {
            return code;
        }
    }
}

#[tokio::test]
async fn an_invalid_message_never_ends_the_session() {
    let mut h = start().await;
    let (mut ws, epoch) = open_session(&h).await;
    let mut page = Page::new();
    send_bytes(&mut ws, page.snapshot_after(epoch)).await;
    // Bytes that are no stream: refused, with the engine's epoch.
    send_bytes(&mut ws, vec![1, 2, 3]).await;
    let truncated = engine::msg::MsgError::Truncated {
        needed: 0,
        available: 0,
    }
    .code();
    assert_eq!(next_refusal(&mut ws).await, (truncated, 0, page.0.epoch()));
    // A corrupt header (reserved bits set) in a change.
    let mut corrupt = page.note_on(60);
    corrupt[1] = 0xFF;
    send_bytes(&mut ws, corrupt).await;
    let corrupt_code = engine::msg::MsgError::Corrupt.code();
    assert_eq!(next_refusal(&mut ws).await.0, corrupt_code);
    // Text that is no message, a message for another route, a message of an older protocol.
    for text in [
        "not json",
        r#"{"type":"pair","code":"12345678"}"#,
        r#"{"type":"command","command":{"type":"live.all_notes_off"}}"#,
    ] {
        ws.send(Message::text(text)).await.unwrap();
        assert_eq!(next_error(&mut ws).await, "protocol.invalid", "{text}");
    }
    // An empty frame, a ping: nothing.
    send_bytes(&mut ws, Vec::new()).await;
    ws.send(Message::Ping(vec![1].into())).await.unwrap();
    // A frame longer than the WebSocket library's default limit (16 MiB) is read whole, and
    // refused by the engine (a version this build does not read), not by the connection.
    send_bytes(&mut ws, vec![0; 17 << 20]).await;
    let version = engine::msg::MsgError::UnsupportedVersion(0).code();
    assert_eq!(next_refusal(&mut ws).await.0, version);
    // The session still plays the page's next change.
    send_bytes(&mut ws, page.note_on(69)).await;
    received(&h, 6).await;
    assert_eq!(play(&mut h.core, 50), 1, "the session goes on");
}

#[tokio::test]
async fn the_token_free_routes_keep_the_default_message_limit() {
    let h = start().await;
    let mut ws = connect(h.port, "/pair", Some(ORIGIN)).await.unwrap();
    // Over the default 16 MiB: the pairing route does not read it, the connection ends.
    let _ = ws.send(Message::binary(vec![0; 17 << 20])).await;
    let end = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(end.is_ok(), "the pairing route closed the connection");
}

#[tokio::test]
async fn a_snapshot_that_is_not_newer_than_the_engine_is_refused_and_the_next_one_taken() {
    let mut h = start().await;
    let (mut ws, epoch) = open_session(&h).await;
    let mut page = Page::new();
    send_bytes(&mut ws, page.snapshot_after(epoch)).await;
    received(&h, 1).await;
    // Another page's core started at the same epoch: its snapshot is not newer.
    let mut other = Page::new();
    send_bytes(&mut ws, other.0.setup().unwrap()).await;
    let stale = engine::msg::MsgError::StaleEpoch {
        message: 0,
        current: 0,
    }
    .code();
    let (code, at, refused_epoch) = next_refusal(&mut ws).await;
    assert_eq!((code, at, refused_epoch), (stale, 0, page.0.epoch()));
    // The answer: a snapshot newer than the refusal's epoch, then the page plays.
    send_bytes(&mut ws, other.snapshot_after(refused_epoch)).await;
    send_bytes(&mut ws, other.note_on(69)).await;
    received(&h, 4).await;
    assert_eq!(play(&mut h.core, 50), 1);
    assert_eq!(h.engine.current().unwrap().1, refused_epoch + 1);
}

#[tokio::test]
async fn a_replaced_sessions_streams_never_reach_the_engine() {
    let h = start().await;
    let (mut first, epoch) = open_session(&h).await;
    let mut a = Page::new();
    send_bytes(&mut first, a.snapshot_after(epoch)).await;
    received(&h, 1).await;
    // A second page opens a session: its hello has the engine's epoch.
    let (mut second, epoch) = open_session(&h).await;
    assert_eq!(epoch, 2);
    // The first page, not told yet, resyncs: its snapshot is dropped, not submitted.
    let _ = first.send(Message::binary(a.snapshot_after(epoch))).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.engine.with(|s| s.received()), Some(1));
    // The second page's snapshot, made for the epoch of its hello, is taken.
    let mut b = Page::new();
    send_bytes(&mut second, b.snapshot_after(epoch)).await;
    received(&h, 2).await;
    assert_eq!(h.engine.current().unwrap().1, epoch + 1);
    assert_eq!(h.engine.with(|s| s.rejected()), Some(0));
}

#[tokio::test]
async fn a_replaced_pages_project_stops_and_its_notes_end_when_the_next_page_connects() {
    let mut h = start().await;
    let (mut first, epoch) = open_session(&h).await;
    let mut a = Page::new();
    send_bytes(&mut first, a.snapshot_after(epoch)).await;
    send_bytes(&mut first, a.note_on(60)).await;
    send_bytes(
        &mut first,
        a.run(commands::Command::Transport(
            commands::transport::Command::Play {},
        )),
    )
    .await;
    received(&h, 3).await;
    assert_eq!(play(&mut h.core, 50), 1, "the first page plays");
    // A second page connects and has not sent its snapshot yet.
    let (_second, _) = open_session(&h).await;
    // The default release is 0.3 s: after 1 s the first page's note has ended.
    assert_eq!(
        play(&mut h.core, 1_000),
        0,
        "replacing the page released its notes"
    );
    let mut bytes = Vec::new();
    assert!(h.core.meter().copy_report(&mut bytes));
    let report = engine::host::decode_feedback(&bytes).unwrap();
    assert_eq!(report.transport, engine::msg::TransportState::Stopped);
}

#[tokio::test]
async fn a_session_whose_engine_was_replaced_is_told_its_output_is_gone() {
    let h = start().await;
    let (mut ws, epoch) = open_session(&h).await;
    let mut page = Page::new();
    // A new output opened meanwhile (the old one failed): a new engine, in epoch 0.
    let (_core, sender) = audio_core(48_000.0, Arc::new(Meter::default())).unwrap();
    h.engine.install(sender, 48_000.0);
    send_bytes(&mut ws, page.snapshot_after(epoch)).await;
    assert_eq!(next_error(&mut ws).await, "audio.stream");
    assert_eq!(h.engine.with(|s| s.received()), Some(0), "not submitted");
}

#[tokio::test]
async fn a_refusal_found_at_a_flush_is_told_by_the_ticker() {
    let mut h = start().await;
    let (mut ws, epoch) = open_session(&h).await;
    // A big project's snapshot waits for the engine; a change cut short waits behind it.
    let big = test_support::big::big_project(48, 64, 64);
    let mut page = Page(commands::engine_msgs::Synced::new(big.state, big.ctx, 1));
    send_bytes(&mut ws, page.snapshot_after(epoch)).await;
    let last = page.0.state.project.tracks.keys().last().unwrap().clone();
    let mut cut = page.run(commands::Command::Live(commands::live::Command::NoteOn {
        track: last,
        note: 60,
        velocity: 100,
    }));
    cut.truncate(cut.len() - 4);
    send_bytes(&mut ws, cut).await;
    received(&h, 2).await;
    // The control thread's flushes drain them: the second is refused at a flush.
    for _ in 0..10_000 {
        play(&mut h.core, 10);
        h.engine.flush();
        if h.engine.with(|s| s.waiting()) == Some(0) {
            break;
        }
    }
    let truncated = engine::msg::MsgError::Truncated {
        needed: 0,
        available: 0,
    }
    .code();
    assert_eq!(next_refusal(&mut ws).await, (truncated, 0, page.0.epoch()));
}

/// The next `midi.message`, skipping the others.
async fn next_midi(ws: &mut Ws) -> (Vec<u8>, bool) {
    loop {
        if let ServerMessage::MidiMessage { data, played } = next(ws).await {
            return (data, played);
        }
    }
}

/// Sends the page's table and waits until the session's reader took it (the reader answers
/// the messages of a session in order).
async fn route(ws: &mut Ws, epoch: u16, tracks: Vec<u32>) {
    send(ws, &ClientMessage::LiveRouting { epoch, tracks }).await;
    send(ws, &ClientMessage::MidiListInputs).await;
    loop {
        if let ServerMessage::MidiInputs { .. } = next(ws).await {
            return;
        }
    }
}

/// The page's table now.
fn routing(page: &Page) -> commands::host::LiveRouting {
    commands::host::LiveRouting::of(&page.0.state, &page.0.ctx.engine, page.0.epoch())
}

#[tokio::test]
async fn the_selected_inputs_messages_reach_the_session_as_they_come() {
    let h = start().await;
    let (mut ws, _) = open_session(&h).await;
    h.midi.receive(&[0x90, 60, 100]);
    assert_eq!(next_midi(&mut ws).await, (vec![0x90, 60, 100], false));
    // System messages are not forwarded; the next channel message is.
    h.midi.receive(&[0xF8]);
    h.midi.receive(&[0xB0, 1, 64]);
    assert_eq!(next_midi(&mut ws).await, (vec![0xB0, 1, 64], false));
    // The helper's inputs are listed and selected by messages for its devices.
    send(&mut ws, &ClientMessage::MidiListInputs).await;
    assert!(matches!(
        next(&mut ws).await,
        ServerMessage::MidiInputs { .. }
    ));
    send(&mut ws, &ClientMessage::MidiSelectInput { id: None }).await;
    assert!(matches!(
        next(&mut ws).await,
        ServerMessage::MidiInputs { selected: None, .. }
    ));
}

#[tokio::test]
async fn the_inputs_notes_play_on_the_engine_with_the_pages_table_and_still_reach_the_page() {
    let mut h = start().await;
    let (mut ws, epoch) = open_session(&h).await;
    let mut page = Page::selected();
    send_bytes(&mut ws, page.snapshot_after(epoch)).await;
    received(&h, 1).await;

    // Before any table the helper plays nothing: the page plays the key (its bytes).
    h.midi.receive(&[0x90, 60, 100]);
    assert_eq!(next_midi(&mut ws).await, (vec![0x90, 60, 100], false));
    assert_eq!(play(&mut h.core, 50), 0);
    h.midi.receive(&[0x80, 60, 0]);
    assert!(!next_midi(&mut ws).await.1);

    // With the page's table, a key sounds at once, without a stream of the page.
    let table = routing(&page);
    assert_eq!(table.epoch, page.0.epoch());
    assert_eq!(table.tracks.len(), 1);
    route(&mut ws, table.epoch, table.tracks).await;
    // The page reads nothing meanwhile (a long task): the note sounds all the same.
    h.midi.receive(&[0x90, 60, 100]);
    assert_eq!(
        play(&mut h.core, 50),
        1,
        "the note plays before the page read anything"
    );
    assert_eq!(next_midi(&mut ws).await, (vec![0x90, 60, 100], true));
    assert_eq!(h.engine.with(|s| s.received()), Some(1), "no page stream");

    // The page's table changes while the key is held (nothing selected): the release still
    // reaches the track the note went to.
    route(&mut ws, page.0.epoch(), vec![]).await;
    h.midi.receive(&[0x80, 60, 0]);
    assert_eq!(next_midi(&mut ws).await, (vec![0x80, 60, 0], true));
    assert_eq!(play(&mut h.core, 1_000), 0, "released, nothing stuck");

    // A table of another epoch is ignored: the previous one (no track) stays.
    route(&mut ws, page.0.epoch().wrapping_add(1), vec![0]).await;
    h.midi.receive(&[0x90, 62, 100]);
    assert!(next_midi(&mut ws).await.1);
    assert_eq!(play(&mut h.core, 50), 0, "the ignored table played nothing");
    h.midi.receive(&[0x80, 62, 0]);
    // A control change is the page's.
    h.midi.receive(&[0xB0, 7, 100]);
    assert_eq!(next_midi(&mut ws).await, (vec![0x80, 62, 0], true));
    assert_eq!(next_midi(&mut ws).await, (vec![0xB0, 7, 100], false));
}

#[tokio::test]
async fn a_snapshot_into_a_newer_epoch_ends_the_table_until_the_pages_next_one() {
    let mut h = start().await;
    let (mut ws, epoch) = open_session(&h).await;
    let mut page = Page::selected();
    send_bytes(&mut ws, page.snapshot_after(epoch)).await;
    received(&h, 1).await;
    let table = routing(&page);
    route(&mut ws, table.epoch, table.tracks).await;
    h.midi.receive(&[0x90, 60, 100]);
    assert_eq!(play(&mut h.core, 50), 1);
    assert!(next_midi(&mut ws).await.1);

    // The page resyncs (a refused stream, a project opened): the `Reset` ends the held note.
    send_bytes(&mut ws, page.snapshot_after(page.0.epoch())).await;
    received(&h, 2).await;
    h.midi.receive(&[0x80, 60, 0]);
    assert!(
        !next_midi(&mut ws).await.1,
        "its note went with the old epoch"
    );
    h.midi.receive(&[0x90, 60, 100]);
    assert!(!next_midi(&mut ws).await.1, "the old table plays nothing");
    h.midi.receive(&[0x80, 60, 0]);
    assert!(!next_midi(&mut ws).await.1);
    assert_eq!(play(&mut h.core, 1_000), 0);
    assert_eq!(
        h.engine.with(|s| s.rejected()),
        Some(0),
        "nothing sent in the old epoch"
    );

    let table = routing(&page);
    route(&mut ws, table.epoch, table.tracks).await;
    h.midi.receive(&[0x90, 60, 100]);
    assert!(next_midi(&mut ws).await.1);
    assert_eq!(play(&mut h.core, 50), 1);
}

#[tokio::test]
async fn a_replaced_or_ended_sessions_table_goes_with_it() {
    let mut h = start().await;
    let (mut first, epoch) = open_session(&h).await;
    let mut page = Page::selected();
    send_bytes(&mut first, page.snapshot_after(epoch)).await;
    received(&h, 1).await;
    let table = routing(&page);
    route(&mut first, table.epoch, table.tracks.clone()).await;
    h.midi.receive(&[0x90, 60, 100]);
    assert!(next_midi(&mut first).await.1);
    assert_eq!(play(&mut h.core, 50), 1);

    // Another page opens a session: the first page's notes end and its table goes.
    let (mut second, _) = open_session(&h).await;
    assert_eq!(play(&mut h.core, 1_000), 0);
    h.midi.receive(&[0x90, 60, 100]);
    assert_eq!(next_midi(&mut second).await, (vec![0x90, 60, 100], false));
    assert_eq!(
        play(&mut h.core, 50),
        0,
        "the first page's table plays nothing"
    );
    // The first page's late table is not taken either (its socket may be closed by now).
    let late = ClientMessage::LiveRouting {
        epoch: table.epoch,
        tracks: table.tracks,
    };
    let _ = first
        .send(Message::text(serde_json::to_string(&late).unwrap()))
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    h.midi.receive(&[0x80, 60, 0]);
    assert!(!next_midi(&mut second).await.1);
    h.midi.receive(&[0x90, 60, 100]);
    assert!(!next_midi(&mut second).await.1);
    assert_eq!(play(&mut h.core, 50), 0);

    // The second page's own table, then it goes away: its table goes with it.
    let mut page = Page::selected();
    let epoch = h.engine.current().unwrap().1;
    send_bytes(&mut second, page.snapshot_after(epoch)).await;
    received(&h, 2).await;
    let table = routing(&page);
    route(&mut second, table.epoch, table.tracks).await;
    h.midi.receive(&[0x80, 60, 0]);
    h.midi.receive(&[0x90, 64, 100]);
    assert!(!next_midi(&mut second).await.1);
    assert!(next_midi(&mut second).await.1);
    assert_eq!(play(&mut h.core, 50), 1);
    second.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        play(&mut h.core, 1_000),
        0,
        "the page left: its notes ended"
    );
    assert!(
        !h.midi.live().receive(&[0x90, 64, 100]),
        "and its table went"
    );
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

// The MCP server (AG-1): `/mcp` on the same port, checked before the MCP library sees a
// request, and each tool call relayed to the page's session.

/// An MCP `initialize` as an HTTP request to `path`, with `host` and the `extra` header lines.
fn mcp_request(path: &str, host: &str, extra: &str) -> String {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}"#;
    format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Sends a raw HTTP request and returns the status code of its answer.
async fn http_status(port: u16, request: String) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = vec![0u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .expect("an answer within 5 s")
        .unwrap();
    let head = String::from_utf8_lossy(&buf[..n]);
    head.split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
async fn mcp_refuses_another_host_any_origin_and_any_other_token_before_the_mcp_server() {
    let h = start().await;
    let host = format!("127.0.0.1:{}", h.port);
    let bearer = format!("Authorization: Bearer {MCP_TOKEN}\r\n");
    // The door: the helper's host, the MCP token, no Origin.
    assert_eq!(
        http_status(h.port, mcp_request("/mcp", &host, &bearer)).await,
        200
    );
    let localhost = format!("localhost:{}", h.port);
    assert_eq!(
        http_status(h.port, mcp_request("/mcp", &localhost, &bearer)).await,
        200
    );
    let attacks: Vec<(&str, String, String, u16)> = vec![
        ("no token", "/mcp".into(), String::new(), 401),
        (
            "wrong token",
            "/mcp".into(),
            "Authorization: Bearer ffffffffffffffffffffffffffffffff\r\n".into(),
            401,
        ),
        (
            "the page's token",
            "/mcp".into(),
            format!("Authorization: Bearer {TOKEN}\r\n"),
            401,
        ),
        (
            "empty bearer",
            "/mcp".into(),
            "Authorization: Bearer \r\n".into(),
            401,
        ),
        (
            "token in the query",
            format!("/mcp?token={MCP_TOKEN}"),
            String::new(),
            401,
        ),
        (
            "allowed page origin",
            "/mcp".into(),
            format!("{bearer}Origin: {ORIGIN}\r\n"),
            403,
        ),
        (
            "evil origin",
            "/mcp".into(),
            format!("{bearer}Origin: https://evil.example\r\n"),
            403,
        ),
        (
            "null origin",
            "/mcp".into(),
            format!("{bearer}Origin: null\r\n"),
            403,
        ),
    ];
    for (what, path, extra, status) in attacks {
        assert_eq!(
            http_status(h.port, mcp_request(&path, &host, &extra)).await,
            status,
            "{what}"
        );
    }
    // DNS rebinding: a name that resolves to 127.0.0.1, or another port.
    for bad in [
        format!("evil.example:{}", h.port),
        "127.0.0.1:80".into(),
        "127.0.0.1".into(),
    ] {
        assert_eq!(
            http_status(h.port, mcp_request("/mcp", &bad, &bearer)).await,
            403,
            "{bad}"
        );
    }
    // The page's routes still answer on the same port.
    let mut ws = connect(h.port, "/probe", Some(ORIGIN)).await.unwrap();
    assert!(matches!(next(&mut ws).await, ServerMessage::Probe { .. }));
}

/// Where the stdio bridge writes, read back by the test.
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

/// Runs `faulk-helper mcp`'s bridge on `lines` (each a JSON-RPC message) against the helper on
/// `port`; its answers, one JSON-RPC message per line.
fn bridge(
    port: u16,
    token: Option<&str>,
    lines: Vec<serde_json::Value>,
) -> tokio::task::JoinHandle<Vec<serde_json::Value>> {
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let token = token.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        let out = Captured::default();
        helper::mcp_stdio::run(port, token, std::io::Cursor::new(input), out.clone());
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    })
}

fn initialize() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"test-agent","version":"1"}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    ]
}

fn tool_call(id: u64, name: &str, arguments: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

fn answer(answers: &[serde_json::Value], id: u64) -> &serde_json::Value {
    answers
        .iter()
        .find(|a| a["id"] == id)
        .unwrap_or_else(|| panic!("no answer {id} in {answers:?}"))
}

/// The JSON of a tool result's text (an error's `code`, `message`...).
fn result_text(result: &serde_json::Value) -> serde_json::Value {
    let text = result["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap_or(serde_json::Value::String(text.into()))
}

#[tokio::test]
async fn the_mcp_server_lists_one_tool_per_command_of_the_catalog() {
    let h = start().await;
    let mut lines = initialize();
    lines.push(serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    let answers = bridge(h.port, Some(MCP_TOKEN), lines).await.unwrap();
    assert_eq!(answer(&answers, 1)["result"]["serverInfo"]["name"], "faulk");
    assert!(answer(&answers, 1)["result"]["capabilities"]["tools"].is_object());
    let tools = answer(&answers, 2)["result"]["tools"].as_array().unwrap();
    let catalog = commands::catalog();
    assert_eq!(tools.len(), catalog.len());
    for (tool, spec) in tools.iter().zip(&catalog) {
        assert_eq!(tool["name"], helper::mcp::tool_name(&spec.name));
        assert_eq!(tool["description"], spec.description.as_str());
        assert_eq!(tool["inputSchema"], spec.input_schema, "{}", spec.name);
        assert_eq!(tool["outputSchema"], spec.output_schema, "{}", spec.name);
        assert_eq!(tool["annotations"]["destructiveHint"], spec.destructive);
    }
}

#[tokio::test]
async fn without_a_page_every_tool_answers_page_absent_at_once() {
    let h = start().await;
    let mut lines = initialize();
    lines.push(tool_call(
        2,
        "track_create",
        serde_json::json!({"kind":"midi"}),
    ));
    let started = std::time::Instant::now();
    let answers = bridge(h.port, Some(MCP_TOKEN), lines).await.unwrap();
    assert!(
        started.elapsed() < AGENT_WAIT,
        "no wait for a page that is not there"
    );
    let call = answer(&answers, 2);
    assert_eq!(call["result"]["isError"], true);
    assert_eq!(result_text(call)["code"], "page.absent");
}

#[tokio::test]
async fn a_tool_call_runs_in_the_page_as_the_agent_and_answers_its_output() {
    let h = start().await;
    let (mut page, _) = open_session(&h).await;
    let mut lines = initialize();
    lines.push(tool_call(
        2,
        "track_rename",
        serde_json::json!({"track":"t1","name":"Lead"}),
    ));
    let running = bridge(h.port, Some(MCP_TOKEN), lines);
    let (id, agent, command) = match next(&mut page).await {
        ServerMessage::AgentCall { id, agent, command } => (id, agent, command),
        other => panic!("expected agent.call, got {other:?}"),
    };
    assert_eq!(
        agent, "test-agent",
        "the client's name, the history's author"
    );
    assert_eq!(
        command,
        serde_json::json!({"type":"track.rename","track":"t1","name":"Lead"})
    );
    // An answer for no waiting call changes nothing.
    send(
        &mut page,
        &ClientMessage::AgentResult {
            id: id + 1000,
            result: AgentResult::Invalid {
                message: "not this one".into(),
            },
        },
    )
    .await;
    send(
        &mut page,
        &ClientMessage::AgentResult {
            id,
            result: AgentResult::Applied {
                output: serde_json::json!({"type":"track.rename","previous":"Synth"}),
            },
        },
    )
    .await;
    let answers = running.await.unwrap();
    let call = answer(&answers, 2);
    assert_eq!(call["result"]["isError"], false, "{call}");
    assert_eq!(
        call["result"]["structuredContent"],
        serde_json::json!({"previous":"Synth"})
    );
}

#[tokio::test]
async fn a_refusal_of_the_page_reaches_the_agent_as_a_typed_error() {
    let h = start().await;
    let (mut page, _) = open_session(&h).await;
    let mut lines = initialize();
    lines.push(tool_call(
        2,
        "track_delete",
        serde_json::json!({"track":"nope"}),
    ));
    let running = bridge(h.port, Some(MCP_TOKEN), lines);
    let ServerMessage::AgentCall { id, .. } = next(&mut page).await else {
        panic!("expected agent.call")
    };
    let error = serde_json::json!({"code":"not_found","entity":"track","id":"nope"});
    send(
        &mut page,
        &ClientMessage::AgentResult {
            id,
            result: AgentResult::Refused {
                error: error.clone(),
            },
        },
    )
    .await;
    let answers = running.await.unwrap();
    let call = answer(&answers, 2);
    assert_eq!(call["result"]["isError"], true);
    assert_eq!(result_text(call)["code"], "command.refused");
    assert_eq!(result_text(call)["error"], error);
}

#[tokio::test]
async fn a_page_that_goes_away_before_it_answers_ends_the_call_with_page_gone() {
    let h = start().await;
    let (mut page, _) = open_session(&h).await;
    let mut lines = initialize();
    lines.push(tool_call(2, "history_undo", serde_json::json!({})));
    let running = bridge(h.port, Some(MCP_TOKEN), lines);
    assert!(matches!(
        next(&mut page).await,
        ServerMessage::AgentCall { .. }
    ));
    page.close(None).await.unwrap();
    let answers = running.await.unwrap();
    assert_eq!(result_text(answer(&answers, 2))["code"], "page.gone");
}

#[tokio::test]
async fn a_replaced_pages_call_ends_with_page_gone_and_the_new_page_cannot_answer_it() {
    let h = start().await;
    let (mut first, _) = open_session(&h).await;
    let mut lines = initialize();
    lines.push(tool_call(2, "history_undo", serde_json::json!({})));
    let running = bridge(h.port, Some(MCP_TOKEN), lines);
    let ServerMessage::AgentCall { id, .. } = next(&mut first).await else {
        panic!("expected agent.call")
    };
    let (mut second, _) = open_session(&h).await;
    // The new page answers a call that was never its own: ignored.
    send(
        &mut second,
        &ClientMessage::AgentResult {
            id,
            result: AgentResult::Applied {
                output: serde_json::json!({}),
            },
        },
    )
    .await;
    let answers = running.await.unwrap();
    assert_eq!(result_text(answer(&answers, 2))["code"], "page.gone");
}

#[tokio::test]
async fn a_page_that_never_answers_ends_the_call_with_page_timeout() {
    let h = start().await;
    let (mut page, _) = open_session(&h).await;
    let mut lines = initialize();
    lines.push(tool_call(2, "history_undo", serde_json::json!({})));
    let running = bridge(h.port, Some(MCP_TOKEN), lines);
    let ServerMessage::AgentCall { id, .. } = next(&mut page).await else {
        panic!("expected agent.call")
    };
    let answers = running.await.unwrap();
    assert_eq!(result_text(answer(&answers, 2))["code"], "page.timeout");
    // A late answer finds no call: the session goes on.
    send(
        &mut page,
        &ClientMessage::AgentResult {
            id,
            result: AgentResult::Applied {
                output: serde_json::json!({}),
            },
        },
    )
    .await;
    send(&mut page, &ClientMessage::MidiListInputs).await;
    assert!(matches!(
        next(&mut page).await,
        ServerMessage::MidiInputs { .. }
    ));
}

#[tokio::test]
async fn an_unknown_tool_or_a_type_in_the_arguments_is_an_invalid_call() {
    let h = start().await;
    let (_page, _) = open_session(&h).await;
    let mut lines = initialize();
    lines.push(tool_call(2, "track.create", serde_json::json!({})));
    lines.push(tool_call(
        3,
        "history_undo",
        serde_json::json!({"type":"project.new"}),
    ));
    let answers = bridge(h.port, Some(MCP_TOKEN), lines).await.unwrap();
    for id in [2, 3] {
        assert_eq!(answer(&answers, id)["error"]["code"], -32602, "{id}");
    }
}

#[tokio::test]
async fn the_stdio_bridge_says_what_to_do_without_a_token_or_a_helper() {
    let h = start().await;
    let answers = bridge(h.port, None, initialize()).await.unwrap();
    assert_eq!(answers.len(), 1, "the notification gets no answer");
    let message = answer(&answers, 1)["error"]["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(message.contains("faulk-helper mcp-token"), "{message}");
    let wrong = bridge(
        h.port,
        Some("ffffffffffffffffffffffffffffffff"),
        initialize(),
    )
    .await
    .unwrap();
    let message = answer(&wrong, 1)["error"]["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(message.contains("refused the MCP token"), "{message}");
    // No helper on this port.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let answers = bridge(port, Some(MCP_TOKEN), initialize()).await.unwrap();
    let message = answer(&answers, 1)["error"]["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(message.contains("not running"), "{message}");
}
