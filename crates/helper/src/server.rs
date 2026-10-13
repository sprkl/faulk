//! WebSocket server of the local bridge: probe, pairing and the session.
//!
//! One session at a time: a new session replaces the previous one (its page is told).
//! When a session ends, every note is released so nothing keeps sounding.

use crate::audio::{EngineEvent, EventSender, Meter, engine_event};
use crate::midi::MidiManager;
use crate::security::{Handshake, PairResult, Pairing, Policy, Route};
use commands::Command;
use futures_util::{SinkExt, StreamExt};
use protocol::{ClientMessage, HELPER_NAME, PROTOCOL_VERSION, ServerMessage};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

struct Session {
    id: u64,
    tx: mpsc::UnboundedSender<ServerMessage>,
}

pub struct Shared {
    pub policy: Policy,
    pub pairing: Mutex<Pairing>,
    pub sender: Arc<Mutex<EventSender>>,
    pub midi: Mutex<MidiManager>,
    pub activity: Mutex<std_mpsc::Receiver<(u8, u8)>>,
    pub meter: Arc<Meter>,
    pub sample_rate: f32,
    pub audio_error: Arc<AtomicBool>,
    /// Called with the new pairing code whenever it changes.
    pub show_code: Box<dyn Fn(&str) + Send + Sync>,
    session: Mutex<Option<Session>>,
    next_id: AtomicU64,
}

impl Shared {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        policy: Policy,
        sender: Arc<Mutex<EventSender>>,
        midi: MidiManager,
        activity: std_mpsc::Receiver<(u8, u8)>,
        meter: Arc<Meter>,
        sample_rate: f32,
        audio_error: Arc<AtomicBool>,
        show_code: Box<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        let pairing = Pairing::new(Instant::now());
        show_code(pairing.code());
        Self {
            policy,
            pairing: Mutex::new(pairing),
            sender,
            midi: Mutex::new(midi),
            activity: Mutex::new(activity),
            meter,
            sample_rate,
            audio_error,
            show_code,
            session: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    fn send_event(&self, event: EngineEvent) {
        if let Ok(mut sender) = self.sender.lock() {
            sender.send(event);
        }
    }

    fn to_session(&self, message: ServerMessage) {
        if let Ok(session) = self.session.lock()
            && let Some(s) = session.as_ref()
        {
            let _ = s.tx.send(message);
        }
    }

    fn midi_inputs(&self) -> ServerMessage {
        let midi = self.midi.lock().expect("MIDI lock poisoned");
        ServerMessage::MidiInputs {
            inputs: midi.inputs().to_vec(),
            selected: midi.selected().map(str::to_owned),
        }
    }

    fn hello(&self) -> ServerMessage {
        ServerMessage::Hello {
            protocol: PROTOCOL_VERSION,
            sample_rate: self.sample_rate,
            buffer_frames: self.meter.callback_frames(),
            estimated_latency_ms: self.meter.estimated_latency_ms(self.sample_rate),
        }
    }

    /// Runs a command from the session. Returns the messages to send back.
    fn run(&self, command: &Command) -> Vec<ServerMessage> {
        match command {
            Command::MidiListInputs => {
                self.midi.lock().expect("MIDI lock poisoned").refresh();
                vec![self.midi_inputs()]
            }
            Command::MidiSelectInput { id } => {
                let result = self
                    .midi
                    .lock()
                    .expect("MIDI lock poisoned")
                    .select(id.as_deref());
                let mut out = vec![self.midi_inputs()];
                if let Err(message) = result {
                    out.push(ServerMessage::Error {
                        code: "midi.connect".into(),
                        message,
                    });
                }
                out
            }
            other => {
                if let Some(event) = engine_event(other) {
                    self.send_event(event);
                }
                Vec::new()
            }
        }
    }
}

/// Accepts connections until the listener fails.
pub async fn serve(listener: TcpListener, shared: Arc<Shared>) {
    tokio::spawn(ticker(shared.clone()));
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(connection(stream, shared.clone()));
    }
}

/// Periodic work: meter, MIDI activity, hot-plug polling, audio errors.
async fn ticker(shared: Arc<Shared>) {
    let mut interval = tokio::time::interval(Duration::from_millis(33));
    let mut ticks: u64 = 0;
    loop {
        interval.tick().await;
        ticks += 1;
        let (peak, voices) = shared.meter.take();
        shared.to_session(ServerMessage::Meter { peak, voices });
        let lights: Vec<_> = shared
            .activity
            .lock()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for (note, velocity) in lights {
            shared.to_session(ServerMessage::MidiActivity { note, velocity });
        }
        if ticks.is_multiple_of(30) {
            let changed = shared.midi.lock().is_ok_and(|mut m| m.refresh());
            if changed {
                shared.to_session(shared.midi_inputs());
            }
        }
        if shared.audio_error.swap(false, Ordering::Relaxed) {
            shared.to_session(ServerMessage::Error {
                code: "audio.stream".into(),
                message: "the audio output stream reported an error".into(),
            });
        }
    }
}

// The error type of the handshake callback is fixed by tungstenite (an HTTP response).
#[allow(clippy::result_large_err)]
async fn connection(stream: TcpStream, shared: Arc<Shared>) {
    let route: Mutex<Option<Route>> = Mutex::new(None);
    let callback = |req: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok());
        let handshake = Handshake {
            path: req.uri().path(),
            query: req.uri().query(),
            host: header("host"),
            origin: header("origin"),
        };
        match shared.policy.check(&handshake) {
            Ok(r) => {
                *route.lock().expect("route lock") = Some(r);
                Ok(response)
            }
            Err(refusal) => {
                eprintln!("refused a connection: {refusal:?}");
                let mut refused = ErrorResponse::new(None);
                *refused.status_mut() = StatusCode::FORBIDDEN;
                Err(refused)
            }
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    let route = route.lock().expect("route lock").take();
    match route {
        Some(Route::Probe) => probe(ws).await,
        Some(Route::Pair) => pair(ws, &shared).await,
        Some(Route::Session) => session(ws, &shared).await,
        None => {}
    }
}

type Ws = tokio_tungstenite::WebSocketStream<TcpStream>;

fn text(message: &ServerMessage) -> Message {
    Message::text(serde_json::to_string(message).expect("messages always serialize"))
}

async fn probe(mut ws: Ws) {
    let answer = ServerMessage::Probe {
        helper: HELPER_NAME.into(),
        protocol: PROTOCOL_VERSION,
    };
    let _ = ws.send(text(&answer)).await;
    let _ = ws.close(None).await;
}

async fn pair(mut ws: Ws, shared: &Shared) {
    while let Some(Ok(message)) = ws.next().await {
        let Message::Text(body) = message else {
            continue;
        };
        let reply = match serde_json::from_str::<ClientMessage>(&body) {
            Ok(ClientMessage::Pair { code }) => {
                let result = shared
                    .pairing
                    .lock()
                    .expect("pairing lock")
                    .attempt(&code, Instant::now());
                match result {
                    PairResult::Accepted => {
                        (shared.show_code)(shared.pairing.lock().expect("pairing lock").code());
                        ServerMessage::Paired {
                            token: shared.policy.token.clone(),
                        }
                    }
                    PairResult::Refused {
                        attempts_left,
                        new_code,
                    } => {
                        if new_code {
                            (shared.show_code)(shared.pairing.lock().expect("pairing lock").code());
                        }
                        ServerMessage::PairingRefused { attempts_left }
                    }
                }
            }
            _ => ServerMessage::Error {
                code: "protocol.invalid".into(),
                message: "only a pair message is accepted here".into(),
            },
        };
        let done =
            !matches!(reply, ServerMessage::PairingRefused { attempts_left } if attempts_left > 0);
        let _ = ws.send(text(&reply)).await;
        if done {
            break;
        }
    }
    let _ = ws.close(None).await;
}

async fn session(ws: Ws, shared: &Arc<Shared>) {
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
    {
        let mut current = shared.session.lock().expect("session lock");
        if let Some(previous) = current.take() {
            let _ = previous.tx.send(ServerMessage::Error {
                code: "session.replaced".into(),
                message: "another page opened a session".into(),
            });
        }
        *current = Some(Session { id, tx: tx.clone() });
    }
    // A new session starts from silence.
    shared.send_event(EngineEvent::AllNotesOff);
    let _ = tx.send(shared.hello());
    shared.midi.lock().expect("MIDI lock poisoned").refresh();
    let _ = tx.send(shared.midi_inputs());

    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let replaced =
                matches!(&message, ServerMessage::Error { code, .. } if code == "session.replaced");
            if sink.send(text(&message)).await.is_err() || replaced {
                break;
            }
        }
        let _ = sink.close().await;
    });

    while let Some(Ok(message)) = stream.next().await {
        if writer.is_finished() {
            break;
        }
        let Message::Text(body) = message else {
            continue;
        };
        let replies = match serde_json::from_str::<ClientMessage>(&body) {
            Ok(ClientMessage::Command { command }) => shared.run(&command),
            Ok(ClientMessage::Pair { .. }) | Err(_) => vec![ServerMessage::Error {
                code: "protocol.invalid".into(),
                message: "expected a command message".into(),
            }],
        };
        for reply in replies {
            let _ = tx.send(reply);
        }
    }

    let mut current = shared.session.lock().expect("session lock");
    if current.as_ref().is_some_and(|s| s.id == id) {
        *current = None;
        // The page is gone: release every note so nothing hangs, and let go of the MIDI
        // input so the page (Web mode) can open it again: on Windows an input can be
        // open in one application only.
        shared.send_event(EngineEvent::AllNotesOff);
        if let Ok(mut midi) = shared.midi.lock() {
            midi.disconnect();
        }
    }
    drop(current);
    writer.abort();
}
