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

/// What the bridge keeps between messages.
struct Link {
    url: String,
    port: u16,
    token: Option<String>,
    agent: ureq::Agent,
    session: Mutex<Option<String>>,
    version: Mutex<Option<String>>,
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
        }
    }

    /// Relays one message; returns the messages to write (answers, or the error of a request
    /// that could not be relayed).
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
        let Some(token) = &self.token else {
            return fail(
                "no MCP token yet: run `faulk-helper mcp-token` once, then restart the agent"
                    .to_owned(),
            );
        };
        let mut request = self
            .agent
            .post(&self.url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(session) = self.session.lock().expect("session lock").clone() {
            request = request.header("Mcp-Session-Id", &session);
        }
        if let Some(version) = self.version.lock().expect("version lock").clone() {
            request = request.header("MCP-Protocol-Version", &version);
        }
        let mut response = match request.send(message.to_string()) {
            Ok(response) => response,
            Err(e) => {
                return fail(format!(
                    "the Faulk helper is not running on 127.0.0.1:{} ({e}): start it, then retry",
                    self.port
                ));
            }
        };
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return fail(match status {
                401 => "the helper refused the MCP token: run `faulk-helper mcp-token`, then restart the agent".to_owned(),
                404 => "the helper no longer knows this MCP session (it restarted): restart the agent".to_owned(),
                _ => format!("the helper answered HTTP {status}"),
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
            Err(e) => return fail(format!("the helper's answer could not be read: {e}")),
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
        messages
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
}
