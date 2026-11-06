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
//!
//! Agents (AG-1, #608): an agent's call goes to the greeted session (Boost) if there is one,
//! else to the front control link (`/control`): a paired page's connection that carries only
//! the agents' relay, whatever plays (the page in Web mode, or the helper) and whether or not
//! the helper has an output. The front link is the newest, or the last whose page said it came
//! to the front (`control.front`). Control links never take the audio role and never replace
//! the session; at most [`MAX_CONTROLS`] are kept, a newer one closing the oldest. When an agent
//! leaves (#641, [`crate::agents`]), the greeted session and every control link are told
//! (`agent.left`), so that the page closes the history group that agent left open.

use crate::audio::{EngineSlot, Meter, Refusal};
use crate::disk::{
    DialogGate, FolderPicker, Lookup, NoPicker, OpenError, OpenFolder, Picked, ProjectFolder,
    Remembered,
};
use crate::live::LiveInput;
use crate::midi::{MidiManager, Received};
use crate::security::{Handshake, PairResult, Pairing, Policy, Route};
use crate::updater::UpdateWatch;
use futures_util::{SinkExt, StreamExt};
use protocol::{
    AgentResult, Capabilities, ClientMessage, FolderRefusal, FolderStatus, HELPER_NAME,
    MAX_STREAM_BYTES, PROTOCOL_VERSION, ServerMessage,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};

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
    /// Close the connection: a control link that a newer one pushed out.
    Close,
}

/// A page's control link (#608): the agents' relay, apart from the audio role.
struct Control {
    id: u64,
    tx: mpsc::UnboundedSender<Out>,
}

/// The most control links the helper keeps (one per open page, as a rule): a newer one closes
/// the oldest, so a page that reconnects in a loop, or many tabs, never pile up connections.
pub const MAX_CONTROLS: usize = 8;

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
    /// The pages' control links (#608), the front one last: an agent's call goes to it when no
    /// session is greeted.
    controls: Mutex<Vec<Control>>,
    /// Ids of sessions and control links alike: a waiting call names the link it went on.
    next_id: AtomicU64,
    /// The agents' calls waiting for the page's answer (AG-1), by call id.
    calls: Mutex<HashMap<u64, PendingCall>>,
    next_call: AtomicU64,
    /// How long an agent's call waits for the page's answer.
    agent_timeout: Duration,
    /// The MCP server, made once `serve` holds the `Arc` (its handler keeps a weak one).
    mcp: OnceLock<crate::mcp::McpHttp>,
    /// The agents here, by name (#641): the page is told when one leaves.
    agents: Mutex<crate::agents::Presence>,
    /// How long an agent may stay silent before it counts as gone (tests shorten it).
    agent_idle: Duration,
    /// The project folder the session's page has open (AU-7), read by the disk thread.
    open_folder: Arc<OpenFolder>,
    /// What the session's page said of its project's place: the session, the project and the
    /// page's link marker. A dialog's answer is taken only while they still hold.
    link: Mutex<Option<Link>>,
    /// Numbers the places named (`Link::seq`).
    next_place: AtomicU64,
    /// The folders the user picked in the helper's dialog.
    remembered: Mutex<Remembered>,
    picker: Arc<dyn FolderPicker>,
    /// One folder dialog at a time.
    dialog: Arc<DialogGate>,
    /// The export under way (#319): one at a time.
    export: Mutex<Option<RunningExport>>,
    /// The native import under way (#778): one at a time.
    import: Mutex<Option<RunningImport>>,
    /// The files the user picked in the helper's file dialog, by token (#778), oldest first.
    picked: Mutex<Vec<PickedEntry>>,
    /// The cap on an import's decoded size.
    import_max_bytes: u64,
    /// The language of the dialog's title.
    lang: crate::tray_text::Lang,
}

/// The project's place a session's page named (`folder.open`, `folder.choose`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Link {
    session: u64,
    project: String,
    nonce: String,
    /// Which folder message named it: a later one is a newer place.
    seq: u64,
}

/// An agent's MCP session, counted as long as it lives (#641): dropped with the session (the
/// MCP library drops the session's handler when the client closes it or it stays idle), it
/// tells the page the agent left when it was that name's last.
pub struct AgentSession {
    shared: std::sync::Weak<Shared>,
    name: String,
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            let left = shared
                .agents
                .lock()
                .is_ok_and(|mut a| a.session_closed(&self.name, Instant::now()));
            if left {
                shared.agent_left(&self.name);
            }
        }
    }
}

/// An agent's call sent to a page, waiting for its `agent.result`.
struct PendingCall {
    /// The session or control link the call went to: only it may answer.
    link: u64,
    answer: oneshot::Sender<AgentResult>,
}

/// Why an agent's call got no answer from the page (AG-1). `code` is stable for agents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayError {
    /// No page is paired and connected: no greeted session, no control link.
    Absent,
    /// The session or control link the call went on ended, or another page replaced it,
    /// before the page answered.
    Gone,
    /// The page did not answer within the helper's timeout.
    Timeout,
}

impl RelayError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Absent => "page.absent",
            Self::Gone => "page.gone",
            Self::Timeout => "page.timeout",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Absent => {
                "no Faulk page is paired with the helper: open Faulk in the browser (and pair it with the helper once)"
            }
            Self::Gone => "the Faulk page closed or was replaced before it answered",
            Self::Timeout => "the Faulk page did not answer in time",
        }
    }
}

/// How long an agent's call waits for the page by default.
pub const AGENT_TIMEOUT: Duration = Duration::from_secs(60);

/// Removes an agent's call from the waiting ones when its wait ends, whichever way (answered,
/// timed out, or abandoned by an MCP client that went away).
struct CallGuard<'a> {
    calls: &'a Mutex<HashMap<u64, PendingCall>>,
    id: u64,
}

impl Drop for CallGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.remove(&self.id);
        }
    }
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
            controls: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            calls: Mutex::new(HashMap::new()),
            next_call: AtomicU64::new(1),
            agent_timeout: AGENT_TIMEOUT,
            mcp: OnceLock::new(),
            agents: Mutex::new(crate::agents::Presence::default()),
            agent_idle: crate::agents::AGENT_IDLE,
            open_folder: Arc::new(OpenFolder::default()),
            link: Mutex::new(None),
            next_place: AtomicU64::new(1),
            remembered: Mutex::new(Remembered::default()),
            picker: Arc::new(NoPicker),
            dialog: Arc::new(DialogGate::default()),
            export: Mutex::new(None),
            import: Mutex::new(None),
            picked: Mutex::new(Vec::new()),
            import_max_bytes: audio_import::MAX_DECODED_BYTES,
            lang: crate::tray_text::Lang::En,
        }
    }

    /// A lower cap on an import's decoded size than `audio_import::MAX_DECODED_BYTES` (#778:
    /// tests, which cannot decode 16 GiB to see the refusal).
    pub fn with_import_cap(mut self, max_bytes: u64) -> Self {
        self.import_max_bytes = max_bytes.min(audio_import::MAX_DECODED_BYTES);
        self
    }

    /// The folders the helper remembers and its folder dialog (AU-7), with the language of the
    /// dialog's title. Without it the helper remembers nothing and has no dialog.
    pub fn with_folders(
        mut self,
        remembered: Remembered,
        picker: Arc<dyn FolderPicker>,
        lang: crate::tray_text::Lang,
    ) -> Self {
        self.remembered = Mutex::new(remembered);
        self.picker = picker;
        self.lang = lang;
        self
    }

    /// The project folder the session's page has open, for the disk thread (`crate::disk`).
    pub fn open_folder(&self) -> &Arc<OpenFolder> {
        &self.open_folder
    }

    /// The folders the user picked, most recent first.
    pub fn remembered_folders(&self) -> Vec<std::path::PathBuf> {
        self.remembered
            .lock()
            .map(|r| r.paths().to_vec())
            .unwrap_or_default()
    }

    /// How long an agent's call waits for the page (tests shorten it).
    pub fn with_agent_timeout(mut self, timeout: Duration) -> Self {
        self.agent_timeout = timeout;
        self
    }

    /// How long an agent may stay silent before it counts as gone (tests shorten it): the MCP
    /// sessions' idle timeout and the lease of a stateless call (#641). Set before `serve`.
    pub fn with_agent_idle(mut self, idle: Duration) -> Self {
        self.agent_idle = idle;
        self
    }

    /// How long an agent may stay silent before it counts as gone.
    pub fn agent_idle(&self) -> Duration {
        self.agent_idle
    }

    /// Counts an MCP session of the agent `name` (#641) until the guard drops.
    pub fn agent_session(self: &Arc<Self>, name: &str) -> AgentSession {
        if let Ok(mut agents) = self.agents.lock() {
            agents.session_opened(name);
        }
        AgentSession {
            shared: Arc::downgrade(self),
            name: name.to_owned(),
        }
    }

    /// The agent `name` made a call outside any MCP session (a stateless MCP request, #641):
    /// it counts as here for `agent_idle` from now.
    pub fn agent_called(&self, name: &str) {
        if let Ok(mut agents) = self.agents.lock() {
            agents.called(name, Instant::now(), self.agent_idle);
        }
    }

    /// Whether the agent `name` counts as here (#641).
    pub fn agent_here(&self, name: &str) -> bool {
        self.agents
            .lock()
            .is_ok_and(|a| a.is_here(name, Instant::now()))
    }

    /// How many MCP sessions of the agent `name` are open (#641).
    pub fn agent_sessions(&self, name: &str) -> usize {
        self.agents.lock().map_or(0, |a| a.sessions(name))
    }

    /// The agents whose stateless calls stopped for `agent_idle` left (#641): the ticker's.
    fn expire_agents(&self) {
        let left = self
            .agents
            .lock()
            .map(|mut a| a.expire(Instant::now()))
            .unwrap_or_default();
        for name in left {
            self.agent_left(&name);
        }
    }

    /// Tells the page that the agent `name` left (#641): on the greeted session and on every
    /// control link, whichever its calls went to (another page it drove may hold its group). The
    /// session gets it too, after any call it carried for that agent: the page runs that call
    /// before it learns the agent left, so a group the call opened is closed, not left open. A
    /// copy on another link may come later than a new session of the same name opened a group
    /// there: that group is then closed early, never left open (the user keeps undo).
    fn agent_left(&self, name: &str) {
        crate::log!("agent {name:?} left");
        let message = ServerMessage::AgentLeft {
            agent: name.to_owned(),
        };
        // Only a greeted session: the agents' calls go to no other, and `hello` comes first.
        if let Ok(session) = self.session.lock()
            && let Some(s) = session.as_ref().filter(|s| s.engine.is_some())
        {
            let _ = s.tx.send(Out::Msg(message.clone()));
        }
        if let Ok(controls) = self.controls.lock() {
            for c in controls.iter() {
                let _ = c.tx.send(Out::Msg(message.clone()));
            }
        }
    }

    /// Relays an agent's command to the page (AG-1, #608): sent as `agent.call` to the greeted
    /// session (Boost) if there is one, else to the front control link, and answered by the
    /// page's `agent.result` of the same id. `Absent` at once without either; `Gone` when that
    /// link ends or is replaced first; `Timeout` after the helper's timeout.
    pub async fn agent_call(
        &self,
        agent: &str,
        command: serde_json::Value,
    ) -> Result<AgentResult, RelayError> {
        let id = self.next_call.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        let call = ServerMessage::AgentCall {
            id,
            agent: agent.to_owned(),
            command,
        };
        // Registered under the lock that guards the chosen link's end, as that end: a call is
        // never registered for a link whose calls were already dropped. The session's lock is
        // let go before the control links' is taken: the two are never held together.
        let _guard = {
            let session = self.session.lock().expect("session lock");
            if let Some(s) = session.as_ref().filter(|s| s.engine.is_some()) {
                self.send_call(s.id, &s.tx, id, answer, call)?
            } else {
                drop(session);
                let controls = self.controls.lock().expect("controls lock");
                let Some(c) = controls.last() else {
                    return Err(RelayError::Absent);
                };
                self.send_call(c.id, &c.tx, id, answer, call)?
            }
        };
        match tokio::time::timeout(self.agent_timeout, answered).await {
            Ok(Ok(result)) => Ok(result),
            // The sender was dropped: the link ended or was replaced.
            Ok(Err(_)) => Err(RelayError::Gone),
            Err(_) => Err(RelayError::Timeout),
        }
    }

    /// Registers call `id` for `link` and sends it there; the guard drops it when its wait ends.
    fn send_call(
        &self,
        link: u64,
        tx: &mpsc::UnboundedSender<Out>,
        id: u64,
        answer: oneshot::Sender<AgentResult>,
        call: ServerMessage,
    ) -> Result<CallGuard<'_>, RelayError> {
        self.calls
            .lock()
            .expect("calls lock")
            .insert(id, PendingCall { link, answer });
        let guard = CallGuard {
            calls: &self.calls,
            id,
        };
        if tx.send(Out::Msg(call)).is_err() {
            return Err(RelayError::Gone);
        }
        Ok(guard)
    }

    /// Drops the waiting calls of link `id` (a session or a control link): their agents get
    /// `Gone`.
    fn drop_calls(&self, id: u64) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.retain(|_, call| call.link != id);
        }
    }

    /// Counts control link `id` (#608) as the front one. Past [`MAX_CONTROLS`] the oldest is
    /// taken out first, under the same lock (no call can reach it after), told to close, and
    /// its waiting calls end.
    fn open_control(&self, id: u64, tx: mpsc::UnboundedSender<Out>) {
        let mut controls = self.controls.lock().expect("controls lock");
        while controls.len() >= MAX_CONTROLS {
            let oldest = controls.remove(0);
            crate::log!("closed the oldest control link: {MAX_CONTROLS} are open");
            let _ = oldest.tx.send(Out::Close);
            self.drop_calls(oldest.id);
        }
        controls.push(Control { id, tx });
    }

    /// Control link `id`'s page came to the front (`control.front`): its link becomes the one
    /// agents' calls go to when no session is greeted. Nothing for a link already gone.
    fn front_control(&self, id: u64) {
        let mut controls = self.controls.lock().expect("controls lock");
        if let Some(at) = controls.iter().position(|c| c.id == id) {
            let link = controls.remove(at);
            controls.push(link);
        }
    }

    /// Control link `id` ended: no call goes to it any more, and its waiting calls end. Under
    /// the controls' lock, as a call's registration.
    fn close_control(&self, id: u64) {
        let mut controls = self.controls.lock().expect("controls lock");
        controls.retain(|c| c.id != id);
        self.drop_calls(id);
    }

    /// How many control links are open.
    pub fn control_links(&self) -> usize {
        self.controls.lock().map_or(0, |c| c.len())
    }

    /// The page's answer to call `call`, from link `link` (a session or a control link): taken
    /// only when that link is the one the call went to.
    fn answer_call(&self, link: u64, call: u64, result: AgentResult) {
        let mut calls = self.calls.lock().expect("calls lock");
        match calls.get(&call) {
            Some(pending) if pending.link == link => {
                if let Some(pending) = calls.remove(&call) {
                    let _ = pending.answer.send(result);
                }
            }
            // Another link's call: a session and a control link, or two pages, never answer
            // each other's calls.
            Some(_) => crate::log!("ignored an agent result from another link (call {call})"),
            None => crate::log!("ignored an agent result for no waiting call (call {call})"),
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
                disk: true,
                export: true,
                import: true,
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
        self: &Arc<Self>,
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
            Ok(ClientMessage::AgentResult { id: call, result }) => {
                self.answer_call(id, call, result);
                Vec::new()
            }
            Ok(ClientMessage::FolderOpen {
                project,
                nonce,
                name,
            }) => match self.checked_link(id, project, nonce, &name) {
                Ok(link) => {
                    self.folder_open(link);
                    Vec::new()
                }
                Err(error) => vec![error],
            },
            Ok(ClientMessage::FolderChoose {
                project,
                nonce,
                name,
            }) => match self.checked_link(id, project, nonce, &name) {
                Ok(link) => self.folder_choose(link, &name).into_iter().collect(),
                Err(error) => vec![error],
            },
            Ok(ClientMessage::FolderClose) => self.folder_close(id).into_iter().collect(),
            Ok(ClientMessage::ExportStart {
                id: run,
                project,
                name,
                sample_rate,
                bits,
                body_frames,
                setup,
                start,
                stop,
            }) => {
                let fields = crate::export::StartFields {
                    id: run,
                    project: &project,
                    name: &name,
                    sample_rate,
                    bits,
                    body_frames,
                    setup: &setup,
                    start: &start,
                    stop: &stop,
                };
                self.export_start(id, fields).into_iter().collect()
            }
            Ok(ClientMessage::ExportCancel { id: run }) => {
                self.export_cancel(id, run);
                Vec::new()
            }
            Ok(ClientMessage::ImportChoose { id: run }) => {
                self.import_choose(id, run).into_iter().collect()
            }
            Ok(ClientMessage::ImportStart {
                id: run,
                project,
                source,
            }) => self
                .import_start(id, run, &project, source)
                .into_iter()
                .collect(),
            Ok(ClientMessage::ImportCancel { id: run }) => {
                self.import_cancel(id, run);
                Vec::new()
            }
            Ok(ClientMessage::Pair { .. } | ClientMessage::ControlFront) | Err(_) => {
                vec![ServerMessage::Error {
                    code: "protocol.invalid".into(),
                    message: "expected a session message (the engine's streams are binary)".into(),
                }]
            }
        }
    }

    /// The place a `folder.open` or `folder.choose` names, checked: a project id, a link nonce,
    /// and a folder name of a reasonable length (shown in the dialog's title only). Otherwise the
    /// message is invalid and nothing changes. Each one gets a new sequence number: a place named
    /// since is a newer one, even with the same project and marker.
    fn checked_link(
        &self,
        session: u64,
        project: String,
        nonce: String,
        name: &str,
    ) -> Result<Link, ServerMessage> {
        let ok = project::ProjectId::parse(&project).is_ok()
            && crate::disk::valid_nonce(&nonce)
            && name.len() <= 255
            && !name.chars().any(char::is_control);
        if !ok {
            return Err(ServerMessage::Error {
                code: "protocol.invalid".into(),
                message:
                    "a folder message needs a project id, a 32-digit hex nonce and a short name"
                        .into(),
            });
        }
        Ok(Link {
            session,
            project,
            nonce,
            seq: self.next_place.fetch_add(1, Ordering::Relaxed),
        })
    }

    /// Makes `link` the place of its session, when that session is the current one: under the
    /// session lock, so it never follows the session's end (which clears its place). A place of
    /// another project or marker closes the folder open now at once: the helper never reads the
    /// folder of a place the page left, while the new one is looked up, nor after a refused or
    /// cancelled dialog.
    fn name_place(&self, link: &Link) -> bool {
        let session = self.session.lock().expect("session lock");
        if !session.as_ref().is_some_and(|s| s.id == link.session) {
            return false;
        }
        let mut current = self.link.lock().expect("link lock");
        let same = current.as_ref().is_some_and(|c| {
            c.session == link.session && c.project == link.project && c.nonce == link.nonce
        });
        if !same {
            self.open_folder.set(None);
        }
        *current = Some(link.clone());
        true
    }

    /// Commits what was found for `link`, only while it is still the place named (same session,
    /// current, and no folder message since): `folder` becomes the open folder and `state` is told.
    /// `false` otherwise: a newer message was answered, or will be. The session lock first, then
    /// the link's, as everywhere.
    fn commit(
        &self,
        link: &Link,
        folder: Option<ProjectFolder>,
        state: FolderStatus,
        opens: bool,
    ) -> bool {
        let session = self.session.lock().expect("session lock");
        let Some(s) = session.as_ref().filter(|s| s.id == link.session) else {
            return false;
        };
        let current = self.link.lock().expect("link lock");
        if current.as_ref() != Some(link) {
            return false;
        }
        if opens {
            self.open_folder.set(folder.map(Arc::new));
        }
        // Sent under the locks: the states reach the page in the order they were committed, so a
        // newer place's state is never followed by an older one's.
        let _ = s.tx.send(Out::Msg(ServerMessage::FolderState {
            project: Some(link.project.clone()),
            state,
        }));
        true
    }

    /// Looks `link` up among the remembered folders (disk work, on a blocking thread and with no
    /// lock held) and commits it: the folder that holds the project and the page's marker is
    /// opened; with none, or several, no folder is (the page asks the user to show it).
    fn resolve(&self, link: &Link) -> bool {
        let paths = self
            .remembered
            .lock()
            .map(|r| r.paths().to_vec())
            .unwrap_or_default();
        match crate::disk::lookup(&paths, &link.project, &link.nonce) {
            Lookup::Found(folder) => {
                let state = FolderStatus::Open {
                    name: folder.name().to_owned(),
                };
                self.commit(link, Some(folder), state, true)
            }
            Lookup::Unknown => self.commit(link, None, FolderStatus::Unknown, true),
            Lookup::Ambiguous(count) => {
                self.commit(link, None, FolderStatus::Ambiguous { count }, true)
            }
        }
    }

    /// `folder.open` of the current session (AU-7): the place is named at once, then looked up
    /// off the runtime, and its state told. Nothing for a session that is not the current one.
    fn folder_open(self: &Arc<Self>, link: Link) {
        if !self.name_place(&link) {
            return;
        }
        let shared = self.clone();
        tokio::task::spawn_blocking(move || {
            shared.resolve(&link);
        });
    }

    /// `folder.choose` of the current session: the helper's dialog, one at a time. Answers
    /// `choosing` at once; the outcome follows when the user closes the dialog. A request that
    /// finds a dialog open shows nothing more: when that dialog closes, the place named last is
    /// looked up again and its state told, so no page is left waiting on `choosing`.
    fn folder_choose(self: &Arc<Self>, link: Link, name: &str) -> Option<ServerMessage> {
        if !self.name_place(&link) {
            return None;
        }
        let choosing = ServerMessage::FolderState {
            project: Some(link.project.clone()),
            state: FolderStatus::Choosing,
        };
        let Some(taken) = self.dialog.take() else {
            return Some(choosing);
        };
        let shared = self.clone();
        let title = crate::tray_text::folder_dialog_title(self.lang, name);
        tokio::task::spawn_blocking(move || {
            let picked = shared.picker.pick(&title);
            drop(taken);
            if shared.take_pick(&link, picked) {
                return;
            }
            // Another place was named meanwhile (by this page or by the one that replaced it):
            // its state, the folder just picked included.
            let current = shared.link.lock().ok().and_then(|l| l.clone());
            if let Some(current) = current {
                shared.resolve(&current);
            }
        });
        Some(choosing)
    }

    /// What the dialog's answer does, for the place `link` named: a folder holding the project
    /// and the page's marker is remembered, and opened while `link` is still the place named.
    /// `false` when that place is not the one named any more.
    fn take_pick(&self, link: &Link, picked: Picked) -> bool {
        let refused = |reason| FolderStatus::Refused { reason };
        let (folder, state) = match picked {
            Picked::Cancelled => (None, refused(FolderRefusal::Cancelled)),
            Picked::Unavailable => (None, refused(FolderRefusal::Unavailable)),
            Picked::Folder(path) => match ProjectFolder::open(&path, Some(&link.project)) {
                Err(OpenError::Unreadable) => (None, refused(FolderRefusal::Unreadable)),
                Err(OpenError::NoProject) => (None, refused(FolderRefusal::NoProject)),
                Err(OpenError::OtherProject) => (None, refused(FolderRefusal::OtherProject)),
                Ok(folder) if !folder.holds_link(&link.nonce) => {
                    (None, refused(FolderRefusal::NotThisFolder))
                }
                Ok(folder) => {
                    // Kept whatever the page did since: the user picked it.
                    if let Ok(mut remembered) = self.remembered.lock() {
                        remembered.remember(folder.root());
                    }
                    let state = FolderStatus::Open {
                        name: folder.name().to_owned(),
                    };
                    (Some(folder), state)
                }
            },
        };
        let opens = folder.is_some();
        self.commit(link, folder, state, opens)
    }

    /// `folder.close` of the current session: no folder.
    fn folder_close(&self, id: u64) -> Option<ServerMessage> {
        let session = self.session.lock().expect("session lock");
        if !session.as_ref().is_some_and(|s| s.id == id) {
            return None;
        }
        self.end_link(id);
        Some(ServerMessage::FolderState {
            project: None,
            state: FolderStatus::Closed,
        })
    }

    /// Session `id`'s place is gone (closed, or the session ended or was replaced): no folder.
    /// Called under the session lock.
    fn end_link(&self, id: u64) {
        let mut link = self.link.lock().expect("link lock");
        if link.as_ref().is_none_or(|l| l.session == id) {
            *link = None;
            self.open_folder.set(None);
        }
    }
}

/// The end of an export's thread (#319): on drop, the slot is freed when it is still this run's,
/// then the page is told, `message` or, when the thread panicked before setting it,
/// `export.failed` `export.render`. Its temporary file went with the `PendingWrite`'s drop.
struct ExportEnd<'a> {
    shared: &'a Shared,
    session: u64,
    run: u32,
    message: Option<ServerMessage>,
}

impl Drop for ExportEnd<'_> {
    fn drop(&mut self) {
        {
            let mut running = self.shared.export.lock().unwrap_or_else(|p| p.into_inner());
            if running
                .as_ref()
                .is_some_and(|r| r.session == self.session && r.id == self.run)
            {
                *running = None;
            }
        }
        let message = self
            .message
            .take()
            .unwrap_or_else(|| ServerMessage::ExportFailed {
                id: self.run,
                code: protocol::errors::EXPORT_RENDER.into(),
                message: "the export's thread stopped".into(),
            });
        self.shared.to_session_id(self.session, message);
    }
}

/// The export under way (#319): the session that asked for it, its number, its cancel flag.
struct RunningExport {
    session: u64,
    id: u32,
    cancel: Arc<AtomicBool>,
}

impl Shared {
    /// Sends `message` to session `id` while it is the current one.
    fn to_session_id(&self, id: u64, message: ServerMessage) {
        if let Ok(session) = self.session.lock()
            && let Some(s) = session.as_ref().filter(|s| s.id == id)
        {
            let _ = s.tx.send(Out::Msg(message));
        }
    }

    /// `export.start` of session `id` (#319): checked, then rendered and written on a thread of
    /// its own, in the folder the session has open for the export's project (`crate::export`).
    /// The answer: `export.failed` at once for a refused one, else `export.progress` and one
    /// `export.done` or `export.failed` from the thread. Nothing for a session that is not the
    /// current one.
    fn export_start(
        self: &Arc<Self>,
        id: u64,
        fields: crate::export::StartFields<'_>,
    ) -> Option<ServerMessage> {
        let run = fields.id;
        let failed = |failure: crate::export::ExportFailure| ServerMessage::ExportFailed {
            id: run,
            code: failure.code.into(),
            message: failure.message,
        };
        // Checked and decoded first, with no lock held (the streams may hold 64 MiB).
        let request = match crate::export::parse(fields) {
            Ok(request) => request,
            Err(failure) => return Some(failed(failure)),
        };
        // The session and its place: under the session lock, then the link's, as everywhere.
        let session = self.session.lock().expect("session lock");
        if !session.as_ref().is_some_and(|s| s.id == id) {
            return None;
        }
        let place = self
            .link
            .lock()
            .expect("link lock")
            .as_ref()
            .is_some_and(|l| l.session == id && l.project == request.project);
        let (_, folder) = self.open_folder.get();
        let Some(folder) = folder.filter(|f| place && f.project() == request.project) else {
            return Some(failed(crate::export::ExportFailure {
                code: protocol::errors::EXPORT_NO_FOLDER,
                message: "the helper has no folder open for this project".into(),
            }));
        };
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut running = self.export.lock().expect("export lock");
            if running.is_some() {
                return Some(failed(crate::export::ExportFailure {
                    code: protocol::errors::EXPORT_BUSY,
                    message: "an export runs already".into(),
                }));
            }
            *running = Some(RunningExport {
                session: id,
                id: run,
                cancel: cancel.clone(),
            });
        }
        drop(session);
        let shared = self.clone();
        let spawned = std::thread::Builder::new()
            .name("faulk-export".into())
            .spawn(move || {
                // Frees the slot and tells the page however the run ends, a panic included.
                let mut end = ExportEnd {
                    shared: &shared,
                    session: id,
                    run,
                    message: None,
                };
                crate::export::lower_priority();
                let watch = crate::export::RunWatch {
                    cancel: &cancel,
                    open: &shared.open_folder,
                    folder: &folder,
                };
                let every = Duration::from_millis(protocol::EXPORT_PROGRESS_MS);
                let outcome = crate::export::run(&request, &folder, &watch, every, |p| {
                    shared.to_session_id(
                        id,
                        ServerMessage::ExportProgress {
                            id: run,
                            rendered: p.rendered,
                            body: p.body,
                            tail: p.tail,
                        },
                    );
                });
                end.message = Some(match outcome {
                    Ok(done) => ServerMessage::ExportDone {
                        id: run,
                        path: done.path,
                        bytes: done.bytes,
                        written: done.written,
                        peak: done.peak,
                        clipped: done.clipped,
                        reduction: done.reduction,
                        silenced: done.silenced,
                        tail_cut: done.tail_cut,
                        missing: done.missing,
                    },
                    Err(failure) => {
                        crate::log!(
                            "export {run} ended without a file: {} ({})",
                            failure.code,
                            failure.message
                        );
                        ServerMessage::ExportFailed {
                            id: run,
                            code: failure.code.into(),
                            message: failure.message,
                        }
                    }
                });
            });
        if spawned.is_err() {
            self.export.lock().expect("export lock").take();
            return Some(failed(crate::export::ExportFailure {
                code: protocol::errors::EXPORT_RENDER,
                message: "the export's thread did not start".into(),
            }));
        }
        None
    }

    /// `export.cancel` of session `id`: its export `run` stops (it answers `export.cancelled`).
    /// The slot is free at once: the page may start the next export while the cancelled one
    /// unwinds (it writes nothing more and removes its temporary file).
    fn export_cancel(&self, id: u64, run: u32) {
        let mut running = self.export.lock().unwrap_or_else(|p| p.into_inner());
        if running
            .as_ref()
            .is_some_and(|r| r.session == id && r.id == run)
            && let Some(r) = running.take()
        {
            r.cancel.store(true, Ordering::Release);
        }
    }

    /// Session `id` ended or was replaced: its export stops, and the slot is free.
    fn export_cancel_all(&self, id: u64) {
        let mut running = self.export.lock().unwrap_or_else(|p| p.into_inner());
        if running.as_ref().is_some_and(|r| r.session == id)
            && let Some(r) = running.take()
        {
            r.cancel.store(true, Ordering::Release);
        }
    }

    /// Whether an export runs now.
    pub fn export_running(&self) -> bool {
        self.export.lock().is_ok_and(|r| r.is_some())
    }

    /// The folder the place session `id` named has open, for `project` when given (AU-7); `None`
    /// without one. Called under the session lock.
    fn session_folder(&self, id: u64, project: Option<&str>) -> Option<Arc<ProjectFolder>> {
        let place = self
            .link
            .lock()
            .expect("link lock")
            .as_ref()
            .is_some_and(|l| l.session == id && project.is_none_or(|p| l.project == p));
        let (_, folder) = self.open_folder.get();
        folder.filter(|f| place && project.is_none_or(|p| f.project() == p))
    }

    /// `import.choose` of session `id` (#778): the helper's file dialog, one dialog of the helper
    /// at a time, for the folder the session has open. Answered `import.chosen` once the user
    /// closed it (a token per file), or `import.failed` at once. Nothing for a session that is not
    /// the current one.
    fn import_choose(self: &Arc<Self>, id: u64, run: u32) -> Option<ServerMessage> {
        let failed = move |code: &str, message: &str| ServerMessage::ImportFailed {
            id: run,
            code: code.into(),
            message: message.into(),
        };
        let session = self.session.lock().expect("session lock");
        if !session.as_ref().is_some_and(|s| s.id == id) {
            return None;
        }
        let Some(folder) = self.session_folder(id, None) else {
            return Some(failed(
                protocol::errors::IMPORT_NO_FOLDER,
                "the helper has no folder open for this session",
            ));
        };
        let Some(taken) = self.dialog.take() else {
            return Some(failed(
                protocol::errors::IMPORT_BUSY,
                "a dialog of the helper is open already",
            ));
        };
        drop(session);
        let shared = self.clone();
        let title = crate::tray_text::import_dialog_title(self.lang, folder.name());
        tokio::task::spawn_blocking(move || {
            let picked = shared.picker.pick_files(&title);
            drop(taken);
            let message = match picked {
                crate::disk::PickedFiles::Unavailable => failed(
                    protocol::errors::IMPORT_DIALOG_UNAVAILABLE,
                    "this system has no file dialog the helper can show",
                ),
                crate::disk::PickedFiles::Cancelled => ServerMessage::ImportChosen {
                    id: run,
                    files: Vec::new(),
                },
                crate::disk::PickedFiles::Files(paths) => ServerMessage::ImportChosen {
                    id: run,
                    files: shared.keep_picks(id, paths),
                },
            };
            shared.to_session_id(id, message);
        });
        None
    }

    /// Keeps the files picked for session `id` and gives each its token: at most
    /// [`MAX_PICKS`] per dialog, and [`MAX_PICKS`] held in all (the oldest dropped).
    fn keep_picks(&self, id: u64, paths: Vec<std::path::PathBuf>) -> Vec<protocol::PickedFile> {
        let mut picked = self.picked.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        picked.retain(|p| now.duration_since(p.at) < IMPORT_TOKEN_TTL);
        let mut files = Vec::new();
        for path in paths.into_iter().take(MAX_PICKS) {
            let token = crate::security::random_hex(16);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            picked.push(PickedEntry {
                token: token.clone(),
                session: id,
                path,
                at: now,
            });
            files.push(protocol::PickedFile { token, name });
        }
        let excess = picked.len().saturating_sub(MAX_PICKS);
        picked.drain(..excess);
        files
    }

    /// Takes the file of `token`, picked for session `id` and not expired: a token is used once.
    fn take_picked(&self, id: u64, token: &str) -> Option<std::path::PathBuf> {
        let mut picked = self.picked.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        picked.retain(|p| now.duration_since(p.at) < IMPORT_TOKEN_TTL);
        let at = picked.iter().position(|p| {
            p.session == id
                && crate::security::constant_time_eq(p.token.as_bytes(), token.as_bytes())
        })?;
        Some(picked.remove(at).path)
    }

    /// `import.start` of session `id` (#778): checked, then run on a thread of its own in the
    /// folder the session has open for the import's project (`crate::import`). The answer:
    /// `import.failed` at once for a refused one, else `import.progress` every
    /// `IMPORT_PROGRESS_MS` while it runs and one `import.done` or `import.failed` from the
    /// thread. Nothing for a session that is not the current one.
    fn import_start(
        self: &Arc<Self>,
        id: u64,
        run: u32,
        project: &str,
        source: protocol::ImportSource,
    ) -> Option<ServerMessage> {
        let failed = |code: &str, message: String| ServerMessage::ImportFailed {
            id: run,
            code: code.into(),
            message,
        };
        if project::ProjectId::parse(project).is_err() {
            return Some(failed(
                protocol::errors::IMPORT_INVALID,
                "the project id is not a ULID".into(),
            ));
        }
        let session = self.session.lock().expect("session lock");
        if !session.as_ref().is_some_and(|s| s.id == id) {
            return None;
        }
        let Some(folder) = self.session_folder(id, Some(project)) else {
            return Some(failed(
                protocol::errors::IMPORT_NO_FOLDER,
                "the helper has no folder open for this project".into(),
            ));
        };
        let source = match source {
            protocol::ImportSource::Folder { path } => ImportFrom::Folder(path),
            protocol::ImportSource::Picked { token } => {
                if !crate::disk::valid_nonce(&token) {
                    return Some(failed(
                        protocol::errors::IMPORT_INVALID,
                        "a token is 32 lowercase hex digits".into(),
                    ));
                }
                match self.take_picked(id, &token) {
                    Some(path) => ImportFrom::Picked(path),
                    None => {
                        return Some(failed(
                            protocol::errors::IMPORT_UNREADABLE,
                            "no file of this session has this token (used, expired or unknown)"
                                .into(),
                        ));
                    }
                }
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut running = self.import.lock().expect("import lock");
            if running.is_some() {
                return Some(failed(
                    protocol::errors::IMPORT_BUSY,
                    "an import runs already".into(),
                ));
            }
            *running = Some(RunningImport {
                session: id,
                id: run,
                cancel: cancel.clone(),
            });
        }
        drop(session);
        let shared = self.clone();
        let spawned = std::thread::Builder::new()
            .name("faulk-import".into())
            .spawn(move || {
                // Frees the slot and tells the page however the run ends, a panic included.
                let mut end = ImportEnd {
                    shared: &shared,
                    session: id,
                    run,
                    message: None,
                };
                crate::import::lower_priority();
                let outcome = shared.run_import(id, run, &folder, source, &cancel);
                end.message = Some(match outcome {
                    Ok(done) => ServerMessage::ImportDone {
                        id: run,
                        hash: done.hash.to_string(),
                        sample_rate: done.info.sample_rate,
                        channels: done.info.channels,
                        frames: done.info.frames,
                        name: done.name,
                        direct: done.direct,
                        created: done.created,
                        peaks: done.peaks,
                    },
                    Err(failure) => {
                        crate::log!(
                            "import {run} ended without a file: {} ({})",
                            failure.code,
                            failure.message
                        );
                        ServerMessage::ImportFailed {
                            id: run,
                            code: failure.code,
                            message: failure.message,
                        }
                    }
                });
            });
        if spawned.is_err() {
            self.import.lock().expect("import lock").take();
            return Some(failed(
                protocol::errors::IMPORT_WRITE,
                "the import's thread did not start".into(),
            ));
        }
        None
    }

    /// Runs an import (on its own thread), telling session `id` how far it is every
    /// `IMPORT_PROGRESS_MS` until it ends, whatever step it is in (a long copy, a slow commit):
    /// the page never takes a busy helper for a silent one.
    fn run_import(
        &self,
        id: u64,
        run: u32,
        folder: &ProjectFolder,
        from: ImportFrom,
        cancel: &AtomicBool,
    ) -> Result<crate::import::Imported, crate::import::ImportFailure> {
        let fraction = AtomicU64::new(0f64.to_bits());
        let every = Duration::from_millis(protocol::IMPORT_PROGRESS_MS);
        std::thread::scope(|scope| {
            let work = scope.spawn(|| {
                // The thread that reads and decodes: below the live output's callback.
                crate::import::lower_priority();
                let source = match &from {
                    ImportFrom::Folder(path) => folder
                        .open_source(path)
                        .map_err(crate::import::source_failure)?,
                    ImportFrom::Picked(path) => crate::import::open_picked(path)?,
                };
                let watch = crate::import::RunWatch {
                    cancel,
                    open: &self.open_folder,
                    folder,
                };
                crate::import::run(source, folder, &watch, self.import_max_bytes, |f| {
                    fraction.store(f.to_bits(), Ordering::Relaxed);
                })
            });
            let mut told = Instant::now();
            while !work.is_finished() {
                std::thread::sleep(Duration::from_millis(10));
                if told.elapsed() >= every {
                    told = Instant::now();
                    self.to_session_id(
                        id,
                        ServerMessage::ImportProgress {
                            id: run,
                            fraction: f64::from_bits(fraction.load(Ordering::Relaxed)),
                        },
                    );
                }
            }
            work.join().unwrap_or_else(|_| {
                Err(crate::import::ImportFailure::new(
                    protocol::errors::IMPORT_WRITE,
                    "the import panicked",
                ))
            })
        })
    }

    /// `import.cancel` of session `id`: its import `run` stops (it answers `import.cancelled`, or
    /// `import.done` when its blob was in place already). The slot is free at once.
    fn import_cancel(&self, id: u64, run: u32) {
        let mut running = self.import.lock().unwrap_or_else(|p| p.into_inner());
        if running
            .as_ref()
            .is_some_and(|r| r.session == id && r.id == run)
            && let Some(r) = running.take()
        {
            r.cancel.store(true, Ordering::Release);
        }
    }

    /// Session `id` ended or was replaced: its import stops, the slot is free, and the files it
    /// picked are forgotten.
    fn import_end_session(&self, id: u64) {
        {
            let mut running = self.import.lock().unwrap_or_else(|p| p.into_inner());
            if running.as_ref().is_some_and(|r| r.session == id)
                && let Some(r) = running.take()
            {
                r.cancel.store(true, Ordering::Release);
            }
        }
        self.picked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|p| p.session != id);
    }

    /// Whether an import runs now.
    pub fn import_running(&self) -> bool {
        self.import.lock().is_ok_and(|r| r.is_some())
    }
}

/// The most files one dialog hands over, and the most tokens held (#778).
pub const MAX_PICKS: usize = 64;
/// How long a picked file's token is valid.
const IMPORT_TOKEN_TTL: Duration = Duration::from_secs(protocol::IMPORT_TOKEN_SECS);

/// A file the user picked in the helper's dialog (#778), until `import.start` takes its token.
struct PickedEntry {
    token: String,
    session: u64,
    path: std::path::PathBuf,
    at: Instant,
}

/// What an import reads.
enum ImportFrom {
    /// A path relative to the project's folder.
    Folder(String),
    /// A file the user picked in the helper's dialog.
    Picked(std::path::PathBuf),
}

/// The import under way (#778): the session that asked for it, its number, its cancel flag.
struct RunningImport {
    session: u64,
    id: u32,
    cancel: Arc<AtomicBool>,
}

/// The end of an import's thread (#778): on drop, the slot is freed when it is still this run's,
/// then the page is told, `message` or, when the thread panicked before setting it,
/// `import.failed` `import.write`. Its temporary files went with the `PendingWrite`s' drops.
struct ImportEnd<'a> {
    shared: &'a Shared,
    session: u64,
    run: u32,
    message: Option<ServerMessage>,
}

impl Drop for ImportEnd<'_> {
    fn drop(&mut self) {
        {
            let mut running = self.shared.import.lock().unwrap_or_else(|p| p.into_inner());
            if running
                .as_ref()
                .is_some_and(|r| r.session == self.session && r.id == self.run)
            {
                *running = None;
            }
        }
        let message = self
            .message
            .take()
            .unwrap_or_else(|| ServerMessage::ImportFailed {
                id: self.run,
                code: protocol::errors::IMPORT_WRITE.into(),
                message: "the import's thread stopped".into(),
            });
        self.shared.to_session_id(self.session, message);
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
    let _ = shared.mcp.set(crate::mcp::McpHttp::new(&shared));
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
            // An agent whose stateless calls stopped left (#641).
            shared.expire_agents();
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

/// What the request a connection starts with asks for, read without consuming it (the
/// handshake, or the MCP server, reads it again and checks it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Peeked {
    /// The page's session: its limits are chosen before the handshake, where the route is
    /// only known after.
    Session,
    /// The MCP server (AG-1): plain HTTP, not a WebSocket.
    Mcp,
    /// Anything else, or a request line that did not arrive in time: the WebSocket handshake
    /// decides as usual.
    Other,
}

async fn peek_route(stream: &TcpStream) -> Peeked {
    let mut buf = [0u8; 64];
    let read = tokio::time::timeout(REQUEST_LINE_WAIT, async {
        loop {
            let n = stream.peek(&mut buf).await.unwrap_or(0);
            // Enough to tell (the longest method, the path and the byte after it), or the
            // peer is done.
            if n == 0 || n > PEEK_ENOUGH {
                return n;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or(0);
    route_of(&buf[..read])
}

/// Bytes enough to tell the route: `GET /session?` and `DELETE /mcp?` are shorter.
const PEEK_ENOUGH: usize = 4 + protocol::paths::SESSION.len();

/// The route the start of an HTTP request asks for: `GET /session?...` or `GET /session ` is the
/// session; `/mcp` with any method is the MCP server.
fn route_of(start: &[u8]) -> Peeked {
    let names = |path: &[u8], method_end: usize| {
        let after = start
            .get(method_end + 1..)
            .and_then(|rest| rest.strip_prefix(path));
        matches!(after.and_then(|a| a.first()), Some(b'?' | b' '))
    };
    let Some(method_end) = start.iter().position(|b| *b == b' ') else {
        return Peeked::Other;
    };
    if names(crate::mcp::PATH.as_bytes(), method_end) {
        return Peeked::Mcp;
    }
    if &start[..method_end] == b"GET" && names(protocol::paths::SESSION.as_bytes(), method_end) {
        return Peeked::Session;
    }
    Peeked::Other
}

/// Header of every refused handshake, valued [`HELPER_NAME`].
pub const HELPER_HEADER: &str = "x-faulk-helper";

// The error type of the handshake callback is fixed by tungstenite (an HTTP response).
#[allow(clippy::result_large_err)]
async fn connection(stream: TcpStream, shared: Arc<Shared>) {
    let peeked = peek_route(&stream).await;
    if peeked == Peeked::Mcp {
        if let Some(mcp) = shared.mcp.get() {
            crate::mcp::serve_connection(stream, shared.clone(), mcp).await;
        }
        return;
    }
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
    let config = (peeked == Peeked::Session).then(session_config);
    let Ok(ws) = tokio_tungstenite::accept_hdr_async_with_config(stream, callback, config).await
    else {
        return;
    };
    let route = route.lock().expect("route lock").take();
    match route {
        Some(Route::Probe) => probe(ws, &shared).await,
        Some(Route::Pair) => pair(ws, &shared).await,
        Some(Route::Session) => session(ws, &shared).await,
        Some(Route::Control) => control(ws, &shared).await,
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
            // Its agents' calls end unanswered: the new page never answers them.
            shared.drop_calls(previous.id);
            // Its project's folder closes with it: the next page names its own (AU-7).
            shared.end_link(previous.id);
            // Its export stops: nothing is written for a page that is gone (#319).
            shared.export_cancel_all(previous.id);
            // So does its import, and its picked files are forgotten (#778).
            shared.import_end_session(previous.id);
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
                Out::Close => break,
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
        shared.drop_calls(id);
        // The page and its project are gone: so is its folder (AU-7).
        shared.end_link(id);
        shared.export_cancel_all(id);
        shared.import_end_session(id);
        if let Ok(mut midi) = shared.midi.lock() {
            midi.disconnect();
        }
        // Under the lock, so a newer session's start always comes after it.
        (shared.notify)(HelperEvent::Session(false));
    }
    drop(current);
    writer.abort();
}

/// A page's control link (#608): `agent.call` out; `agent.result` and `control.front` in,
/// nothing else. It opens whatever plays and whether or not the helper has an output, and
/// leaves the session alone.
async fn control(ws: Ws, shared: &Arc<Shared>) {
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
    // First message, before the link counts: the page checks the version as for a session's `hello` (#658).
    let _ = tx.send(Out::Msg(ServerMessage::ControlHello {
        protocol: PROTOCOL_VERSION,
    }));
    shared.open_control(id, tx.clone());
    let mut writer = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            let frame = match out {
                Out::Msg(message) => text(&message),
                Out::Report(_) => continue,
                Out::Close => {
                    // Pushed out by newer links: the page is told so, and does not come back on its own.
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code: CloseCode::Library(protocol::CONTROL_PUSHED_OUT),
                            reason: "pushed out by newer control links".into(),
                        })))
                        .await;
                    return;
                }
            };
            if sink.send(frame).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    loop {
        tokio::select! {
            message = stream.next() => {
                let reply = match message {
                    Some(Ok(Message::Text(body))) => {
                        match serde_json::from_str::<ClientMessage>(&body) {
                            Ok(ClientMessage::AgentResult { id: call, result }) => {
                                shared.answer_call(id, call, result);
                                None
                            }
                            Ok(ClientMessage::ControlFront) => {
                                shared.front_control(id);
                                None
                            }
                            _ => Some(control_invalid()),
                        }
                    }
                    Some(Ok(Message::Binary(_))) => Some(control_invalid()),
                    Some(Ok(_)) => None,
                    Some(Err(_)) | None => break,
                };
                if let Some(reply) = reply {
                    let _ = tx.send(Out::Msg(reply));
                }
            }
            // Closed by the helper (a newer link pushed it out), or the page stopped reading.
            _ = &mut writer => break,
        }
    }
    shared.close_control(id);
    writer.abort();
}

/// The answer to anything else than the control link's messages; the link goes on.
fn control_invalid() -> ServerMessage {
    ServerMessage::Error {
        code: "protocol.invalid".into(),
        message: "a control link takes agent.result and control.front only".into(),
    }
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
        assert_eq!(
            route_of(b"GET /session?token=abc HTTP/1.1\r\n"),
            Peeked::Session
        );
        assert_eq!(route_of(b"GET /session HTTP/1.1\r\n"), Peeked::Session);
        for start in [
            &b"GET /pair HTTP/1.1\r\n"[..],
            b"GET /probe HTTP/1.1\r\n",
            b"GET /sessions HTTP/1.1\r\n",
            b"GET /session",
            b"POST /session?token=a HTTP/1.1\r\n",
            b"GET /mcpx HTTP/1.1\r\n",
            b"GET",
            b"",
        ] {
            assert_eq!(route_of(start), Peeked::Other, "{start:?}");
        }
    }

    #[test]
    fn the_mcp_path_goes_to_the_mcp_server_whatever_the_method() {
        for start in [
            &b"POST /mcp HTTP/1.1\r\n"[..],
            b"GET /mcp HTTP/1.1\r\n",
            b"DELETE /mcp HTTP/1.1\r\n",
            b"POST /mcp?x=1 HTTP/1.1\r\n",
        ] {
            assert_eq!(route_of(start), Peeked::Mcp, "{start:?}");
        }
        assert!(b"DELETE /mcp?".len() <= PEEK_ENOUGH);
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
