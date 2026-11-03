//! WebSocket server of the local bridge: probe, pairing and the session.
//!
//! One session at a time: a new session replaces the previous one (its page is told).
//! When a session ends, the transport stops and every note is released so nothing keeps
//! sounding.
//!
//! The session relays the page's engine (BO-1): each binary frame is a stream of the page's
//! project core, submitted unchanged to the engine the session's `hello` described, and only
//! while the session is the current one (the check and the submit hold the session lock, so
//! a replaced page's stream never lands after the next page's `hello`). A stream the engine
//! stops is answered `stream_refused`; the session goes on whatever a page sends.
//!
//! Without an audio output (#525) the helper still serves: the probe says so
//! (`audio_output: false`) and a session is answered `error audio.no_output` and closed, so the
//! page keeps playing itself until an output opens.

use crate::audio::{EngineSlot, Meter, Refusal};
use crate::live::LiveInput;
use crate::midi::{MidiManager, Received};
use crate::security::{Handshake, PairResult, Pairing, Policy, Route};
use crate::updater::UpdateWatch;
use futures_util::{SinkExt, StreamExt};
use protocol::{
    Capabilities, ClientMessage, HELPER_NAME, MAX_STREAM_BYTES, PROTOCOL_VERSION, ServerMessage,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

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
    /// The generation of the engine this session's `hello` described; `None` before it.
    engine: Option<u64>,
}

pub struct Shared {
    pub policy: Policy,
    pub pairing: Mutex<Pairing>,
    /// The engine of the open audio output; empty while there is none.
    pub engine: Arc<EngineSlot>,
    pub midi: Mutex<MidiManager>,
    /// The notes of the MIDI input, played on the engine with the session's table (#598).
    live: Arc<LiveInput>,
    /// The selected MIDI input's messages, forwarded to the session as they come (`serve`).
    midi_in: Mutex<Option<mpsc::UnboundedReceiver<Received>>>,
    /// Whether this system has a MIDI backend (`hello`'s capabilities).
    midi_available: bool,
    pub meter: Arc<Meter>,
    /// Told about new pairing codes, sessions and audio errors.
    pub notify: Box<dyn Fn(HelperEvent) + Send + Sync>,
    /// The helper's updater, reported to the session (`update.status`); `None` in tests.
    updates: Option<UpdateWatch>,
    session: Mutex<Option<Session>>,
    next_id: AtomicU64,
}

impl Shared {
    /// `midi_in` receives what `midi` forwards (the `MidiSink` it was made with, whose live
    /// input plays on `engine`).
    pub fn new(
        policy: Policy,
        engine: Arc<EngineSlot>,
        midi: MidiManager,
        midi_in: mpsc::UnboundedReceiver<Received>,
        meter: Arc<Meter>,
        notify: Box<dyn Fn(HelperEvent) + Send + Sync>,
    ) -> Self {
        let pairing = Pairing::new(Instant::now());
        notify(HelperEvent::Code(pairing.code().to_owned()));
        Self {
            policy,
            pairing: Mutex::new(pairing),
            engine,
            live: midi.live(),
            midi: Mutex::new(midi),
            midi_in: Mutex::new(Some(midi_in)),
            midi_available: crate::midi::available(),
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
        // The same engine (same generation, same epoch): the session's streams go on to it.
        if let Some((hello, _)) = self.hello() {
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

    /// The session's first message, at the rate of the open output and in its engine's
    /// epoch, and the generation of that engine; `None` without an output.
    fn hello(&self) -> Option<(ServerMessage, u64)> {
        let sample_rate = self.engine.sample_rate()?;
        let (generation, epoch) = self.engine.current()?;
        let hello = ServerMessage::Hello {
            protocol: PROTOCOL_VERSION,
            sample_rate,
            buffer_frames: self.meter.callback_frames(),
            estimated_latency_ms: self.meter.estimated_latency_ms(sample_rate),
            epoch,
            capabilities: Capabilities {
                midi: self.midi_available,
                disk: false,
            },
        };
        Some((hello, generation))
    }

    /// Opens session `id` with its hello, under the session lock: no stream of a replaced
    /// session can be submitted between the epoch read here and this page's first stream.
    /// `false` when the session was replaced meanwhile (nothing sent).
    fn greet(&self, id: u64) -> bool {
        let mut current = self.session.lock().expect("session lock");
        let Some(session) = current.as_mut().filter(|s| s.id == id) else {
            return false;
        };
        // An output lost since the session began was told already (`output_lost`): the page
        // takes it as no output. One lost and not replaced is said again here.
        match self.hello() {
            Some((hello, generation)) => {
                session.engine = Some(generation);
                let _ = session.tx.send(Out::Msg(hello));
            }
            None => {
                let _ = session.tx.send(Out::Msg(audio_stream_error()));
            }
        }
        true
    }

    /// Submits a stream of session `id` to the engine its hello described. Nothing happens
    /// for a session that is not the current one (replaced: its page is told) or before its
    /// hello. The reply: `stream_refused` for a stream the engine stopped, `audio.stream` when
    /// the session's engine is gone (a new output opened since: the page plays itself again).
    fn submit(&self, id: u64, bytes: &[u8]) -> Option<ServerMessage> {
        let current = self.session.lock().expect("session lock");
        let generation = current.as_ref().filter(|s| s.id == id)?.engine?;
        match self.engine.submit(generation, bytes) {
            None => Some(audio_stream_error()),
            Some(Ok(())) => None,
            Some(Err(refusal)) => Some(refused(refusal)),
        }
    }

    /// Answers a JSON message of session `id`: the helper's MIDI inputs, and the table its
    /// notes play with.
    fn answer(
        &self,
        id: u64,
        message: Result<ClientMessage, serde_json::Error>,
    ) -> Vec<ServerMessage> {
        match message {
            Ok(ClientMessage::LiveRouting { epoch, tracks }) => {
                // Only the current session's, for the engine its hello described: under the
                // session lock, so a replaced page's table never follows the next page's.
                let current = self.session.lock().expect("session lock");
                if let Some(generation) = current
                    .as_ref()
                    .filter(|s| s.id == id)
                    .and_then(|s| s.engine)
                {
                    self.live.set_routing(generation, epoch, tracks);
                }
                Vec::new()
            }
            Ok(ClientMessage::MidiListInputs) => {
                self.midi.lock().expect("MIDI lock poisoned").refresh();
                vec![self.midi_inputs()]
            }
            Ok(ClientMessage::MidiSelectInput { id }) => {
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
            Ok(ClientMessage::Pair { .. }) | Err(_) => vec![ServerMessage::Error {
                code: "protocol.invalid".into(),
                message: "expected a MIDI input message (the engine's streams are binary)".into(),
            }],
        }
    }
}

/// The page's answer to a refused stream is a snapshot newer than the refusal's epoch.
fn refused(refusal: Refusal) -> ServerMessage {
    ServerMessage::StreamRefused {
        code: refusal.code,
        at: refusal.at,
        epoch: refusal.epoch,
    }
}

/// Accepts connections until the listener fails.
pub async fn serve(listener: TcpListener, shared: Arc<Shared>) {
    tokio::spawn(ticker(shared.clone()));
    let midi_in = shared.midi_in.lock().expect("MIDI lock").take();
    if let Some(mut midi_in) = midi_in {
        let forward = shared.clone();
        // Each message goes to the session as it comes, not at the ticker's pace.
        tokio::spawn(async move {
            while let Some(Received { data, played }) = midi_in.recv().await {
                forward.to_session(ServerMessage::MidiMessage { data, played });
            }
        });
    }
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(connection(stream, shared.clone()));
    }
}

/// Periodic work: meter, refusals found at a flush, hot-plug polling, update status.
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
        // A stream that waited and that the engine stopped at a flush (control thread).
        if let Some(refusal) = shared.engine.with(|s| s.take_refusal()).flatten() {
            shared.to_session(refused(refusal));
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

/// The WebSocket limits of a session: one message, or one frame, of a whole engine stream.
fn session_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_STREAM_BYTES))
        .max_frame_size(Some(MAX_STREAM_BYTES))
}

/// How long the request line of a connection may take to arrive before its route is guessed.
const REQUEST_LINE_WAIT: Duration = Duration::from_secs(2);

/// Whether the request a connection starts with asks for the session's path, read without
/// consuming it (the handshake reads it again and checks it): the session's limits are chosen
/// before the handshake, where the route is only known after. `false` when the request line
/// does not arrive in time or names another path: the handshake then decides as usual.
async fn is_session(stream: &TcpStream) -> bool {
    let mut buf = [0u8; 64];
    let read = tokio::time::timeout(REQUEST_LINE_WAIT, async {
        loop {
            let n = stream.peek(&mut buf).await.unwrap_or(0);
            // Enough to tell (`GET `, the path and the byte after it), or the peer is done.
            if n == 0 || n > 4 + SESSION_PATH_LEN {
                return n;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or(0);
    requests_session(&buf[..read])
}

const SESSION_PATH_LEN: usize = protocol::paths::SESSION.len();

/// Whether the start of an HTTP request asks for the session's path (`GET /session?...` or
/// `GET /session `).
fn requests_session(start: &[u8]) -> bool {
    let Some(rest) = start.strip_prefix(b"GET ") else {
        return false;
    };
    let Some(after) = rest.strip_prefix(protocol::paths::SESSION.as_bytes()) else {
        return false;
    };
    matches!(after.first(), Some(b'?' | b' '))
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
    // A session carries the page's engine streams, a big project's snapshot among them: only
    // it reads messages up to `MAX_STREAM_BYTES`; the token-free routes keep the defaults.
    let config = is_session(&stream).await.then(session_config);
    let Ok(ws) = tokio_tungstenite::accept_hdr_async_with_config(stream, callback, config).await
    else {
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
            // The replaced page's project stops and its notes end now, not only once the new
            // page's snapshot arrives (its streams are dropped from here on). Its table goes
            // too, first: a key struck meanwhile is either played before the release, which
            // ends it, or not played by the helper at all (#598).
            shared.live.clear();
            shared.engine.release_all();
        }
        *current = Some(Session {
            id,
            tx: tx.clone(),
            report_pending: pending.clone(),
            engine: None,
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
    // Read now, at the rate of the output open now and in its engine's epoch: the page starts
    // the engine over from its snapshot, in a newer epoch (what sounded fades out with the
    // `Reset`). A session replaced meanwhile says nothing more.
    if !shared.greet(id) {
        return;
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
        let replies = match message {
            // The page's engine stream; whatever it holds, the session goes on.
            Message::Binary(bytes) => shared.submit(id, &bytes).into_iter().collect(),
            Message::Text(body) => shared.answer(id, serde_json::from_str::<ClientMessage>(&body)),
            _ => Vec::new(),
        };
        for reply in replies {
            let _ = tx.send(Out::Msg(reply));
        }
    }

    let mut current = shared.session.lock().expect("session lock");
    if current.as_ref().is_some_and(|s| s.id == id) {
        *current = None;
        // The page is gone: stop its project and release every note so nothing hangs, and
        // let go of the MIDI input so the page (Web mode) can open it again: on Windows an
        // input can be open in one application only. Its table goes with it, and the notes
        // the helper played are among those released: the table goes first, so that no key
        // struck meanwhile is played after the release (#598).
        shared.live.clear();
        shared.engine.release_all();
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
    fn only_the_sessions_request_gets_the_sessions_limits() {
        assert!(requests_session(b"GET /session?token=abc HTTP/1.1\r\n"));
        assert!(requests_session(b"GET /session HTTP/1.1\r\n"));
        for start in [
            &b"GET /pair HTTP/1.1\r\n"[..],
            b"GET /probe HTTP/1.1\r\n",
            b"GET /sessions HTTP/1.1\r\n",
            b"GET /session",
            b"POST /session?token=a HTTP/1.1\r\n",
            b"",
        ] {
            assert!(!requests_session(start), "{start:?}");
        }
    }

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
