//! `faulk-helper mcp`: the helper's MCP server over stdio (AG-1), for an agent that launches
//! its MCP servers as processes.
//!
//! A bridge, not a second server: it relays each JSON-RPC message read on stdin to the running
//! helper's MCP server over HTTP (`http://127.0.0.1:<port>/mcp`, [`crate::mcp`]), with the MCP
//! token of the helper's config folder, and writes each JSON-RPC message of the answers on
//! stdout, one per line. It keeps the MCP session (`Mcp-Session-Id`) and the protocol version
//! the server answered `initialize` with, and ends the session when stdin closes.
//!
//! Order: every message is relayed in the order it was read, except a `tools/call`, which runs
//! on its own thread once sent, so that a call waiting for the page never holds up a ping or a
//! cancellation. Answers are written whole, one line at a time.
//!
//! A session the helper no longer knows (HTTP 404: it expired after [`crate::agents::AGENT_IDLE`]
//! without a request, or the helper restarted) is opened again by the bridge itself (#657): it
//! replays the `initialize` request it kept (same `clientInfo`, so the same agent name) and the
//! `notifications/initialized` after it, then replays the message that met the 404, once. The
//! agent is not told. For the helper's presence by name ([`crate::agents`]) the new session is
//! the same agent coming back. What the agent does lose is what the page did when the old session
//! ended: it was told `agent.left` (or lost its link to the helper) and closed the history group
//! that agent had left open (`history_begin_group` without `history_end_group`) and ended its
//! turn, so the agent's later edits run outside any group, and its `history_end_group` has no group
//! left to close, with no word to the agent. A second 404 on the replay is answered with an error, as
//! is a 404 before any `initialize` went through the bridge.
//!
//! Without a running helper, or without the token, each request is answered with a JSON-RPC
//! error that says what to do; notifications are dropped.

use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The JSON-RPC error code of a request the bridge could not relay.
pub const BRIDGE_ERROR: i64 = -32000;

/// The largest answer the bridge reads (the tool list is about 1.4 MB).
const MAX_ANSWER: u64 = 64 << 20;

/// What the agent is told when the helper no longer knows its session and the bridge cannot open
/// it again.
const RESTART: &str =
    "the helper no longer knows this MCP session (it restarted): restart the agent";

/// What the bridge keeps between messages.
struct Link {
    url: String,
    port: u16,
    token: Option<String>,
    agent: ureq::Agent,
    session: Mutex<Option<String>>,
    version: Mutex<Option<String>>,
    /// The agent's `initialize` request and the `notifications/initialized` that followed, kept
    /// to open the session again (#657).
    opening: Mutex<Opening>,
    /// Held while the session is opened again: one thread does it, the others replay on its result.
    reopening: Mutex<()>,
}

#[derive(Default)]
struct Opening {
    initialize: Option<Value>,
    initialized: Option<Value>,
}

/// Why a message could not be relayed.
enum Failure {
    /// HTTP 404 on a message that carried this session id: the helper no longer knows it.
    SessionGone(String),
    /// HTTP 404 on a message with no session: nothing to open again.
    Unknown,
    /// Anything else: what to tell the agent.
    Text(String),
}

/// Where the bridge writes: one whole line per message.
type Out = Arc<Mutex<Box<dyn Write + Send>>>;

fn write_line(out: &Out, message: &Value) {
    let mut out = out.lock().unwrap_or_else(|p| p.into_inner());
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

impl Link {
    fn new(port: u16, token: Option<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // A tool call waits for the page; the helper gives up first (60 s).
            .timeout_global(Some(Duration::from_secs(120)))
            .build()
            .into();
        Self {
            url: format!("http://127.0.0.1:{port}{}", crate::mcp::PATH),
            port,
            token,
            agent,
            session: Mutex::new(None),
            version: Mutex::new(None),
            opening: Mutex::new(Opening::default()),
            reopening: Mutex::new(()),
        }
    }

    /// Relays one message; returns the messages to write (answers, or the error of a request
    /// that could not be relayed). A session the helper forgot is opened again and the message
    /// replayed once (#657).
    fn relay(&self, message: &Value) -> Vec<Value> {
        let id = message.get("id").cloned();
        let is_request = id.is_some() && message.get("method").is_some();
        let fail = |text: String| -> Vec<Value> {
            eprintln!("faulk-helper mcp: {text}");
            match (&id, is_request) {
                (Some(id), true) => vec![json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": BRIDGE_ERROR, "message": text },
                })],
                _ => Vec::new(),
            }
        };
        let method = message.get("method").and_then(Value::as_str);
        {
            let mut opening = self.opening.lock().expect("opening lock");
            match method {
                Some("initialize") if is_request => opening.initialize = Some(message.clone()),
                Some("notifications/initialized") => opening.initialized = Some(message.clone()),
                _ => {}
            }
        }
        let outcome = match self.post(message, true) {
            Err(Failure::SessionGone(stale)) => {
                if method == Some("initialize") {
                    // The agent starts over: no session to keep, nothing stored to replay.
                    *self.session.lock().expect("session lock") = None;
                    *self.version.lock().expect("version lock") = None;
                    self.post(message, true)
                } else if method == Some("notifications/initialized") {
                    // `reopen` sends it again, on the new session.
                    self.reopen(&stale).map(|()| Vec::new())
                } else {
                    self.reopen(&stale).and_then(|()| self.post(message, true))
                }
            }
            other => other,
        };
        match outcome {
            Ok(messages) => messages,
            Err(Failure::Text(text)) => fail(text),
            Err(Failure::SessionGone(_) | Failure::Unknown) => fail(RESTART.to_owned()),
        }
    }

    /// Opens the MCP session again after the helper answered 404 to `stale` (#657): the kept
    /// `initialize`, then `notifications/initialized`. Another thread may have done it already
    /// (the session is no longer `stale`): then there is nothing to do.
    fn reopen(&self, stale: &str) -> Result<(), Failure> {
        let _one = self.reopening.lock().unwrap_or_else(|p| p.into_inner());
        if self.session.lock().expect("session lock").as_deref() != Some(stale) {
            return Ok(());
        }
        let (initialize, initialized) = {
            let opening = self.opening.lock().expect("opening lock");
            (opening.initialize.clone(), opening.initialized.clone())
        };
        // Never initialized through this bridge: nothing to open again.
        let Some(mut initialize) = initialize else {
            return Err(Failure::Unknown);
        };
        initialize["id"] = json!("faulk-bridge-reopen");
        eprintln!("faulk-helper mcp: the helper no longer knows the MCP session: opening it again");
        // The old session id stays in place until the new one replaces it, so that a call on
        // another thread meanwhile meets the same 404 and waits for this one.
        let answers = self.post(&initialize, false)?;
        if !answers.iter().any(|a| a.get("result").is_some()) {
            return Err(Failure::Text(
                "the helper refused to open the MCP session again: restart the agent".to_owned(),
            ));
        }
        if let Some(initialized) = initialized {
            self.post(&initialized, true)?;
        }
        Ok(())
    }

    /// Sends one message to the helper; `with_session` is false for an `initialize` that opens a
    /// new session.
    fn post(&self, message: &Value, with_session: bool) -> Result<Vec<Value>, Failure> {
        let Some(token) = &self.token else {
            return Err(Failure::Text(
                "no MCP token yet: run `faulk-helper mcp-token` once, then restart the agent"
                    .to_owned(),
            ));
        };
        let mut request = self
            .agent
            .post(&self.url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        let mut sent_with = None;
        if with_session {
            if let Some(session) = self.session.lock().expect("session lock").clone() {
                request = request.header("Mcp-Session-Id", &session);
                sent_with = Some(session);
            }
            if let Some(version) = self.version.lock().expect("version lock").clone() {
                request = request.header("MCP-Protocol-Version", &version);
            }
        }
        let mut response = match request.send(message.to_string()) {
            Ok(response) => response,
            Err(e) => {
                return Err(Failure::Text(format!(
                    "the Faulk helper is not running on 127.0.0.1:{} ({e}): start it, then retry",
                    self.port
                )));
            }
        };
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(match status {
                401 => Failure::Text("the helper refused the MCP token: run `faulk-helper mcp-token`, then restart the agent".to_owned()),
                404 => sent_with.map_or(Failure::Unknown, Failure::SessionGone),
                _ => Failure::Text(format!("the helper answered HTTP {status}")),
            });
        }
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *self.session.lock().expect("session lock") = Some(session.to_owned());
        }
        let is_sse = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let body = match response
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_string()
        {
            Ok(body) => body,
            Err(e) => {
                return Err(Failure::Text(format!(
                    "the helper's answer could not be read: {e}"
                )));
            }
        };
        let messages = if is_sse {
            sse_messages(&body)
        } else {
            serde_json::from_str::<Value>(&body).into_iter().collect()
        };
        for answer in &messages {
            if let Some(version) = answer
                .pointer("/result/protocolVersion")
                .and_then(Value::as_str)
            {
                *self.version.lock().expect("version lock") = Some(version.to_owned());
            }
        }
        Ok(messages)
    }

    /// Ends the MCP session on the helper (stdin closed).
    fn close(&self) {
        let (Some(token), Some(session)) = (
            &self.token,
            self.session.lock().expect("session lock").clone(),
        ) else {
            return;
        };
        let _ = self
            .agent
            .delete(&self.url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Mcp-Session-Id", &session)
            .call();
    }
}

/// The JSON-RPC messages of a server-sent event stream: each event's `data` lines, joined;
/// events without data (priming, keep-alive) are skipped.
pub fn sse_messages(body: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut data = String::new();
    let mut flush = |data: &mut String| {
        if !data.is_empty() {
            if let Ok(message) = serde_json::from_str(data) {
                messages.push(message);
            }
            data.clear();
        }
    };
    for line in body.lines() {
        if line.is_empty() {
            flush(&mut data);
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    flush(&mut data);
    messages
}

/// Runs the bridge until `input` ends: relays to the helper on `port`, with `token` (`None`:
/// the user has not created it), and writes to `output`.
pub fn run(
    port: u16,
    token: Option<String>,
    input: impl BufRead,
    output: impl Write + Send + 'static,
) {
    let link = Arc::new(Link::new(port, token));
    let out: Out = Arc::new(Mutex::new(Box::new(output)));
    let mut calls = Vec::new();
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(e) => {
                write_line(
                    &out,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": { "code": -32700, "message": format!("parse error: {e}") },
                    }),
                );
                continue;
            }
        };
        if message.get("method").and_then(Value::as_str) == Some("tools/call") {
            let (link, out) = (link.clone(), out.clone());
            calls.push(std::thread::spawn(move || {
                for answer in link.relay(&message) {
                    write_line(&out, &answer);
                }
            }));
            calls.retain(|c| !c.is_finished());
        } else {
            for answer in link.relay(&message) {
                write_line(&out, &answer);
            }
        }
    }
    for call in calls {
        let _ = call.join();
    }
    link.close();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn server_sent_events_give_their_data_and_skip_the_rest() {
        let body = "id: 0\nretry: 3000\ndata:\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n: keep-alive\n\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"id\":2,\"result\":{}}\n";
        assert_eq!(
            sse_messages(body),
            vec![
                json!({"jsonrpc":"2.0","id":1,"result":{}}),
                json!({"jsonrpc":"2.0","id":2,"result":{}}),
            ]
        );
        assert!(sse_messages("").is_empty());
    }

    /// A message the stand-in received: the session header it carried and its JSON.
    type Seen = (Option<String>, Value);

    /// A stand-in for the helper's MCP endpoint: `initialize` opens a session, any other message
    /// needs a session it knows, or meets a 404. `restart` forgets every session.
    struct Fake {
        port: u16,
        sessions: Arc<Mutex<Vec<String>>>,
        /// Every message received: the session header it carried and its JSON.
        seen: Arc<Mutex<Vec<Seen>>>,
        /// An `initialize` answers with a session id the server does not keep.
        forgetful: Arc<AtomicBool>,
        /// A `tools/call` meets a 404 even on a session the server keeps.
        deaf_to_calls: Arc<AtomicBool>,
    }

    impl Fake {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let fake = Self {
                port,
                sessions: Arc::default(),
                seen: Arc::default(),
                forgetful: Arc::default(),
                deaf_to_calls: Arc::default(),
            };
            let (sessions, seen, forgetful, deaf) = (
                fake.sessions.clone(),
                fake.seen.clone(),
                fake.forgetful.clone(),
                fake.deaf_to_calls.clone(),
            );
            std::thread::spawn(move || {
                let mut opened = 0;
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let (head, body) = read_request(&mut stream);
                    let session = head.lines().find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("mcp-session-id:")
                            .map(|v| v.trim().to_owned())
                    });
                    let reply = if head.starts_with("DELETE") {
                        "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    } else {
                        let message: Value = serde_json::from_str(&body).unwrap();
                        seen.lock()
                            .unwrap()
                            .push((session.clone(), message.clone()));
                        let id = message.get("id").cloned();
                        let known = session
                            .as_ref()
                            .is_some_and(|s| sessions.lock().unwrap().contains(s));
                        if message["method"] == "initialize" {
                            opened += 1;
                            let new = format!("s{opened}");
                            if !forgetful.load(Ordering::SeqCst) {
                                sessions.lock().unwrap().push(new.clone());
                            }
                            let body = json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-06-18"}}).to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: {new}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                        } else if !known
                            || (message["method"] == "tools/call" && deaf.load(Ordering::SeqCst))
                        {
                            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                        } else if id.is_none() {
                            "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                        } else {
                            let body =
                                json!({"jsonrpc":"2.0","id":id,"result":{"ok":true}}).to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                        }
                    };
                    let _ = stream.write_all(reply.as_bytes());
                }
            });
            fake
        }

        /// The helper restarted (or the session expired): it knows no session.
        fn restart(&self) {
            self.sessions.lock().unwrap().clear();
        }

        fn methods(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|(_, m)| m["method"].as_str().unwrap().to_owned())
                .collect()
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> (String, String) {
        let mut data = Vec::new();
        let mut byte = [0u8; 1];
        while !data.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            data.push(byte[0]);
        }
        let head = String::from_utf8(data).unwrap();
        let length = head
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).unwrap();
        (head, String::from_utf8(body).unwrap())
    }

    fn initialize() -> Value {
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"test-agent","version":"1"}}})
    }

    fn initialized() -> Value {
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    }

    fn call(id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"history_undo","arguments":{}}})
    }

    fn opened_link(fake: &Fake) -> Link {
        let link = Link::new(fake.port, Some("t".into()));
        assert!(link.relay(&initialize())[0].get("result").is_some());
        assert!(link.relay(&initialized()).is_empty());
        link
    }

    #[test]
    fn a_session_the_helper_forgot_is_opened_again_and_the_call_replayed_once() {
        let fake = Fake::start();
        let link = opened_link(&fake);
        assert_eq!(
            link.relay(&call(2)),
            vec![json!({"jsonrpc":"2.0","id":2,"result":{"ok":true}})]
        );
        fake.restart();
        // The agent's second call succeeds, with the answer to its own id and nothing else.
        assert_eq!(
            link.relay(&call(3)),
            vec![json!({"jsonrpc":"2.0","id":3,"result":{"ok":true}})]
        );
        assert_eq!(
            fake.methods(),
            [
                "initialize",
                "notifications/initialized",
                "tools/call",
                // The 404, then the session opened again with the same initialize and the
                // `initialized` that followed it, then the replay.
                "tools/call",
                "initialize",
                "notifications/initialized",
                "tools/call",
            ]
        );
        let seen = fake.seen.lock().unwrap();
        assert_eq!(
            seen[4].1["params"],
            initialize()["params"],
            "same agent: same clientInfo"
        );
        assert_eq!(seen[4].0, None, "the new initialize carries no session");
        assert_eq!(
            seen[6].0.as_deref(),
            Some("s2"),
            "the replay is on the new session"
        );
        drop(seen);
        // The next call goes on the new session with no further initialize.
        assert_eq!(link.relay(&call(4)).len(), 1);
        assert_eq!(
            fake.methods().iter().filter(|m| *m == "initialize").count(),
            2
        );
    }

    #[test]
    fn a_session_that_cannot_be_opened_again_is_an_error_without_a_loop() {
        let fake = Fake::start();
        let link = opened_link(&fake);
        fake.restart();
        // From now on the helper hands out sessions it does not keep: opening it again fails.
        fake.forgetful.store(true, Ordering::SeqCst);
        let answers = link.relay(&call(2));
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0]["id"], 2);
        assert_eq!(answers[0]["error"]["code"], BRIDGE_ERROR);
        assert!(
            answers[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("restart the agent")
        );
        let calls = fake.methods().iter().filter(|m| *m == "tools/call").count();
        assert_eq!(
            calls, 1,
            "the call alone: the session could not be opened again, so nothing is replayed"
        );
        let methods = fake.methods();
        assert_eq!(
            methods.iter().filter(|m| *m == "initialize").count(),
            2,
            "opened again once, no loop"
        );
    }

    #[test]
    fn a_replay_that_meets_a_404_too_is_an_error_and_not_sent_again() {
        let fake = Fake::start();
        let link = opened_link(&fake);
        fake.deaf_to_calls.store(true, Ordering::SeqCst);
        let answers = link.relay(&call(2));
        assert_eq!(answers.len(), 1);
        assert!(
            answers[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("restart the agent")
        );
        // The call, the session opened again, the call once more: no third.
        assert_eq!(
            fake.methods(),
            [
                "initialize",
                "notifications/initialized",
                "tools/call",
                "initialize",
                "notifications/initialized",
                "tools/call",
            ]
        );
    }

    #[test]
    fn an_initialized_notification_that_meets_a_404_is_sent_once_on_the_new_session() {
        let fake = Fake::start();
        let link = opened_link(&fake);
        fake.restart();
        assert!(link.relay(&initialized()).is_empty());
        assert_eq!(
            fake.methods(),
            [
                "initialize",
                "notifications/initialized",
                "notifications/initialized",
                "initialize",
                "notifications/initialized",
            ]
        );
    }

    #[test]
    fn without_an_initialize_to_replay_the_404_is_an_error() {
        let fake = Fake::start();
        let link = Link::new(fake.port, Some("t".into()));
        // A session id the bridge has no initialize for (never seen through it).
        *link.session.lock().unwrap() = Some("old".into());
        let answers = link.relay(&call(2));
        assert!(
            answers[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("restart the agent")
        );
        assert_eq!(
            fake.methods(),
            ["tools/call"],
            "nothing is initialized behind the agent's back"
        );
        // A notification that met the 404 is dropped without an answer.
        assert!(
            link.relay(&json!({"jsonrpc":"2.0","method":"notifications/cancelled"}))
                .is_empty()
        );
    }

    #[test]
    fn calls_on_several_threads_share_one_new_session() {
        let fake = Fake::start();
        let link = Arc::new(opened_link(&fake));
        fake.restart();
        let threads: Vec<_> = (2..6)
            .map(|id| {
                let link = link.clone();
                std::thread::spawn(move || link.relay(&call(id)))
            })
            .collect();
        for (id, thread) in (2..6).zip(threads) {
            assert_eq!(
                thread.join().unwrap(),
                vec![json!({"jsonrpc":"2.0","id":id,"result":{"ok":true}})]
            );
        }
        assert_eq!(
            fake.methods().iter().filter(|m| *m == "initialize").count(),
            2,
            "the session was opened again once, not once per call"
        );
    }
}
