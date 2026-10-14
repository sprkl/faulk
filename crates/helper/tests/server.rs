//! The helper's WebSocket server over real loopback connections: handshake checks,
//! probe, pairing, session, session replacement and the release of notes on close.

use futures_util::{SinkExt, StreamExt};
use helper::audio::{AudioCore, Meter, audio_core};
use helper::midi::MidiManager;
use helper::security::Policy;
use helper::server::{HelperEvent, Shared, serve};
use protocol::{ClientMessage, PROTOCOL_VERSION, ServerMessage};
use std::sync::atomic::AtomicBool;
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
    audio_error: Arc<AtomicBool>,
    core: AudioCore,
}

async fn start() -> Harness {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let meter = Arc::new(Meter::default());
    let (core, sender) = audio_core(48_000.0, meter.clone());
    let sender = Arc::new(Mutex::new(sender));
    let (activity_tx, activity_rx) = mpsc::channel();
    let midi = MidiManager::new(sender.clone(), activity_tx);
    let code = Arc::new(Mutex::new(String::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let audio_error = Arc::new(AtomicBool::new(false));
    let (shown, seen) = (code.clone(), events.clone());
    let shared = Arc::new(Shared::new(
        Policy {
            port,
            allowed_origins: vec![ORIGIN.into()],
            token: TOKEN.into(),
        },
        sender,
        midi,
        activity_rx,
        meter,
        48_000.0,
        audio_error.clone(),
        Box::new(move |event| {
            if let HelperEvent::Code(c) = &event {
                *shown.lock().unwrap() = c.clone();
            }
            seen.lock().unwrap().push(event);
        }),
    ));
    tokio::spawn(serve(listener, shared));
    Harness {
        port,
        code,
        events,
        audio_error,
        core,
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

/// Next server message that is not a periodic meter.
async fn next(ws: &mut Ws) -> ServerMessage {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("a message within 5 s")
            .expect("the stream is open")
            .expect("a valid frame");
        if let Message::Text(text) = frame {
            let message: ServerMessage = serde_json::from_str(&text).unwrap();
            if !matches!(message, ServerMessage::Meter { .. }) {
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
            protocol: PROTOCOL_VERSION
        }
    );
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
            command: commands::Command::NoteAllOff,
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
            command: commands::Command::NoteOn {
                note: 60,
                velocity: 100,
            },
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
    h.audio_error
        .store(true, std::sync::atomic::Ordering::Relaxed);
    wait_events(&h, &[HelperEvent::AudioError]).await;
}
