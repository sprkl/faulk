//! WebSocket server of the local bridge: probe, pairing and the session.
//!
//! One session at a time: a new session replaces the previous one (its page is told).
//! When a session ends, every note is released so nothing keeps sounding.
//!
//! Without an audio output (#525) the helper still serves: the probe says so
//! (`audio_output: false`) and a session is answered `error audio.no_output` and closed, so the
//! page keeps playing itself until an output opens.

use crate::audio::{EngineSlot, Meter};
use crate::midi::MidiManager;
use crate::midi::all_notes_off;
use crate::security::{Handshake, PairResult, Pairing, Policy, Route};
use crate::updater::UpdateWatch;
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

/// What the helper reports to its host (the tray, or the console).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelperEvent {
    /// The pairing code changed (also sent once at start).
    Code(String),
    /// A page opened a session (`true`) or the last session ended (`false`).
    Session(bool),
    /// The audio output stream reported an error (the output is gone).
    AudioError,
    /// An audio output opened (`true`) or there is none (`false`), from the output supervisor.
    Output(bool),
}

/// The error code of a session opened while the helper has no audio output.
pub use protocol::errors::NO_OUTPUT;

/// What the session's writer sends: a JSON message, or the bytes of an engine report (a
/// binary frame, `protocol` module doc).
enum Out {
    Msg(ServerMessage),
    Report(Vec<u8>),
}

struct Session {
    id: u64,
    tx: mpsc::UnboundedSender<Out>,
    /// A report frame waits in the queue: the next one is not sent until the writer took it
    /// (a stalled page never makes the queue grow by 30 reports per second).
    report_pending: Arc<ReportGate>,
}

pub struct Shared {
    pub policy: Policy,
    pub pairing: Mutex<Pairing>,
    /// The engine of the open audio output; empty while there is none.
    pub engine: Arc<EngineSlot>,
    pub midi: Mutex<MidiManager>,
    pub activity: Mutex<std_mpsc::Receiver<(u8, u8)>>,
    pub meter: Arc<Meter>,
    /// Told about new pairing codes, sessions and audio errors.
    pub notify: Box<dyn Fn(HelperEvent) + Send + Sync>,
    /// The helper's updater, reported to the session (`update.status`); `None` in tests.
    updates: Option<UpdateWatch>,
    session: Mutex<Option<Session>>,
    next_id: AtomicU64,
}

impl Shared {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        policy: Policy,
        engine: Arc<EngineSlot>,
        midi: MidiManager,
        activity: std_mpsc::Receiver<(u8, u8)>,
        meter: Arc<Meter>,
        notify: Box<dyn Fn(HelperEvent) + Send + Sync>,
    ) -> Self {
        let pairing = Pairing::new(Instant::now());
        notify(HelperEvent::Code(pairing.code().to_owned()));
        Self {
            policy,
            pairing: Mutex::new(pairing),
            engine,
            midi: Mutex::new(midi),
            activity: Mutex::new(activity),
            meter,
            notify,
            updates: None,
            session: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    /// The open output failed and its engine is gone (the output supervisor calls this once it
    /// emptied the slot): the page is told at once, before any new output can open, so the news
    /// never reaches a session opened on the next output.
    pub fn output_lost(&self) {
        (self.notify)(HelperEvent::AudioError);
        self.to_session(audio_stream_error());
    }

    /// An output opened with the engine of the one that failed (#577): the session plays on, and
    /// a new `hello` tells the page the output's rate, buffer and latency.
    pub fn output_reopened(&self) {
        if let Some(hello) = self.hello() {
            self.to_session(hello);
        }
    }

    /// Reports `updates` to every session: after `midi.inputs` at its start, then on change.
    pub fn with_updates(mut self, updates: UpdateWatch) -> Self {
        self.updates = Some(updates);
        self
    }

    fn update_status(&self) -> Option<ServerMessage> {
        self.updates.as_ref().map(UpdateWatch::message)
    }

    fn notify_code(&self) {
        let code = self.pairing.lock().expect("pairing lock").code().to_owned();
        (self.notify)(HelperEvent::Code(code));
    }

    fn send_command(&self, command: &commands::Command) {
        self.engine.send(command);
    }

    fn to_session(&self, message: ServerMessage) {
        if let Ok(session) = self.session.lock()
            && let Some(s) = session.as_ref()
        {
            let _ = s.tx.send(Out::Msg(message));
        }
    }

    /// Sends a report frame unless the previous one still waits in the session's queue.
    fn report_to_session(&self, bytes: Vec<u8>) {
        if let Ok(session) = self.session.lock()
            && let Some(s) = session.as_ref()
            && s.report_pending.claim()
        {
            let _ = s.tx.send(Out::Report(bytes));
        }
    }

    fn midi_inputs(&self) -> ServerMessage {
        let midi = self.midi.lock().expect("MIDI lock poisoned");
        ServerMessage::MidiInputs {
            inputs: midi.inputs().to_vec(),
            selected: midi.selected().map(str::to_owned),
        }
    }

    /// The session's first message, at the rate of the open output; `None` without one.
    fn hello(&self) -> Option<ServerMessage> {
        let sample_rate = self.engine.sample_rate()?;
        Some(ServerMessage::Hello {
            protocol: PROTOCOL_VERSION,
            sample_rate,
            buffer_frames: self.meter.callback_frames(),
            estimated_latency_ms: self.meter.estimated_latency_ms(sample_rate),
        })
    }

    /// Runs a command from the session. Returns the messages to send back.
    fn run(&self, command: &Command) -> Vec<ServerMessage> {
        match command {
            Command::Midi(commands::midi::Command::ListInputs {}) => {
                self.midi.lock().expect("MIDI lock poisoned").refresh();
                vec![self.midi_inputs()]
            }
            Command::Midi(commands::midi::Command::SelectInput { id }) => {
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
                // The engine takes what the command layer maps (live notes, parameters).
                self.send_command(other);
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

/// Periodic work: meter, MIDI activity, hot-plug polling, update status.
async fn ticker(shared: Arc<Shared>) {
    let mut interval = tokio::time::interval(Duration::from_millis(33));
    let mut ticks: u64 = 0;
    let mut report = Vec::new();
    let mut last_report = None;
    let mut last_update = shared.update_status();
    loop {
        interval.tick().await;
        ticks += 1;
        if shared.meter.copy_report(&mut report) {
            let seq = report
                .get(4..8)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            if seq != last_report {
                last_report = seq;
                shared.report_to_session(core::mem::take(&mut report));
            }
        }
        let lights: Vec<_> = shared
            .activity
            .lock()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for (note, velocity) in lights {
            shared.to_session(ServerMessage::MidiActivity { note, velocity });
        }
        if ticks.is_multiple_of(30) {
            let renewed = shared
                .pairing
                .lock()
                .is_ok_and(|mut p| p.renew_if_expired(Instant::now()));
            if renewed {
                shared.notify_code();
            }
            let changed = shared.midi.lock().is_ok_and(|mut m| m.refresh());
            if changed {
                shared.to_session(shared.midi_inputs());
            }
            // About once a second: a download's progress is not sent at every step.
            let update = shared.update_status();
            if update != last_update {
                if let Some(message) = &update {
                    shared.to_session(message.clone());
                }
                last_update = update;
            }
        }
    }
}

/// The output stream failed and the engine is gone: the page plays itself again.
fn audio_stream_error() -> ServerMessage {
    ServerMessage::Error {
        code: protocol::errors::AUDIO_STREAM.into(),
        message: "the audio output stream reported an error".into(),
    }
}

/// Header of every refused handshake, valued [`HELPER_NAME`].
pub const HELPER_HEADER: &str = "x-faulk-helper";

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
                crate::log!("refused a connection: {refusal:?}");
                let mut refused = ErrorResponse::new(None);
                *refused.status_mut() = StatusCode::FORBIDDEN;
                // Lets a second helper recognise this one even when it presents an
                // origin this helper does not allow. A page cannot read it.
                refused.headers_mut().insert(
                    HELPER_HEADER,
                    tokio_tungstenite::tungstenite::http::HeaderValue::from_static(HELPER_NAME),
                );
                Err(refused)
            }
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    let route = route.lock().expect("route lock").take();
    match route {
        Some(Route::Probe) => probe(ws, &shared).await,
        Some(Route::Pair) => pair(ws, &shared).await,
        Some(Route::Session) => session(ws, &shared).await,
        None => {}
    }
}

type Ws = tokio_tungstenite::WebSocketStream<TcpStream>;

fn text(message: &ServerMessage) -> Message {
    Message::text(serde_json::to_string(message).expect("messages always serialize"))
}

async fn probe(mut ws: Ws, shared: &Shared) {
    let answer = ServerMessage::Probe {
        helper: HELPER_NAME.into(),
        protocol: PROTOCOL_VERSION,
        audio_output: shared.engine.is_open(),
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
                        shared.notify_code();
                        ServerMessage::Paired {
                            token: shared.policy.token.clone(),
                        }
                    }
                    PairResult::Refused {
                        attempts_left,
                        new_code,
                    } => {
                        if new_code {
                            shared.notify_code();
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

/// How long a session waits for a new output's first callback before its `hello`.
const FIRST_CALLBACK_TRIES: u32 = 25;
const FIRST_CALLBACK_STEP: Duration = Duration::from_millis(10);

async fn session(mut ws: Ws, shared: &Arc<Shared>) {
    // Without an output there is no engine to play: the page is told and keeps its own (#525).
    // Checked before the session is counted, so a refused page never replaces the current one.
    if !shared.engine.is_open() {
        let refusal = ServerMessage::Error {
            code: protocol::errors::NO_OUTPUT.into(),
            message: "the helper has no audio output".into(),
        };
        let _ = ws.send(text(&refusal)).await;
        let _ = ws.close(None).await;
        return;
    };
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let pending = Arc::new(ReportGate::default());
    let taken = pending.clone();
    let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
    {
        let mut current = shared.session.lock().expect("session lock");
        if let Some(previous) = current.take() {
            let _ = previous.tx.send(Out::Msg(ServerMessage::Error {
                code: "session.replaced".into(),
                message: "another page opened a session".into(),
            }));
        }
        *current = Some(Session {
            id,
            tx: tx.clone(),
            report_pending: pending.clone(),
        });
        // Under the lock, so it can never be overtaken by an older session's end.
        (shared.notify)(HelperEvent::Session(true));
    }
    // An output that has just opened has not called back yet: wait for its first callback (a
    // moment), so that `hello` carries its buffer size and latency.
    for _ in 0..FIRST_CALLBACK_TRIES {
        if !shared.engine.is_open() || shared.meter.callback_frames().is_some() {
            break;
        }
        tokio::time::sleep(FIRST_CALLBACK_STEP).await;
    }
    // A new session starts from silence.
    shared.send_command(&all_notes_off());
    // Read now, at the rate of the output open now. An output lost since the check above was
    // told to this session already (`output_lost`, before the hello): the page takes it as no
    // output. One lost and not replaced is said again here.
    match shared.hello() {
        Some(hello) => {
            let _ = tx.send(Out::Msg(hello));
        }
        None => {
            let _ = tx.send(Out::Msg(audio_stream_error()));
        }
    }
    shared.midi.lock().expect("MIDI lock poisoned").refresh();
    let _ = tx.send(Out::Msg(shared.midi_inputs()));
    // A page that just connected may be waiting on a release: look for it now (rate-limited).
    if let Some(updates) = &shared.updates {
        updates.request_check();
    }
    if let Some(update) = shared.update_status() {
        let _ = tx.send(Out::Msg(update));
    }

    let writer = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            let is_report = matches!(out, Out::Report(_));
            let (frame, replaced) = match out {
                Out::Msg(message) => {
                    let replaced = matches!(&message, ServerMessage::Error { code, .. } if code == "session.replaced");
                    (text(&message), replaced)
                }
                Out::Report(bytes) => (Message::binary(bytes), false),
            };
            let sent = sink.send(frame).await;
            // A report left the queue: the next one may follow (a JSON frame says nothing
            // about the reports waiting).
            if is_report {
                taken.release();
            }
            if sent.is_err() || replaced {
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
            let _ = tx.send(Out::Msg(reply));
        }
    }

    let mut current = shared.session.lock().expect("session lock");
    if current.as_ref().is_some_and(|s| s.id == id) {
        *current = None;
        // The page is gone: release every note so nothing hangs, and let go of the MIDI
        // input so the page (Web mode) can open it again: on Windows an input can be
        // open in one application only.
        shared.send_command(&all_notes_off());
        if let Ok(mut midi) = shared.midi.lock() {
            midi.disconnect();
        }
        // Under the lock, so a newer session's start always comes after it.
        (shared.notify)(HelperEvent::Session(false));
    }
    drop(current);
    writer.abort();
}

/// At most one report waits in a session's queue: a stalled page never makes the queue grow
/// by 30 reports per second. The ticker claims the gate before it queues a report, the writer
/// releases it once that report left the queue (and only then: a JSON frame says nothing
/// about the reports waiting).
#[derive(Debug, Default)]
struct ReportGate(AtomicBool);

impl ReportGate {
    /// Takes the gate: `true` when no report waits.
    fn claim(&self) -> bool {
        !self.0.swap(true, Ordering::Relaxed)
    }

    fn release(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_lets_one_report_wait_until_it_is_released() {
        let gate = ReportGate::default();
        assert!(gate.claim(), "the first report is queued");
        for _ in 0..100 {
            assert!(!gate.claim(), "no second report while one waits");
        }
        gate.release();
        assert!(gate.claim(), "the next one follows the first");
        assert!(!gate.claim());
    }
}
