//! One helper per port. The listening socket is the lock: a second helper cannot bind
//! the port. It then asks whoever holds the port whether it is a Faulk helper.

use crate::server::HELPER_HEADER;
use futures_util::StreamExt;
use protocol::{HELPER_NAME, paths};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Error;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// What a new helper does about the port.
#[derive(Debug)]
pub enum Startup {
    /// The port was free: this helper serves.
    Serve(std::net::TcpListener),
    /// A Faulk helper already serves the port (any protocol version).
    AlreadyRunning,
    /// Something else holds the port, or binding failed for another reason.
    Blocked(String),
}

/// How long a second helper keeps asking: the first one may still be opening its audio
/// output before it answers (its port is bound first).
pub const PROBE_PATIENCE: Duration = Duration::from_secs(8);

/// Binds `127.0.0.1:port`, or finds out who holds it. `origins` are the origins the
/// probe presents, one after the other (the running helper may allow other ones).
pub fn claim(port: u16, origins: &[String], patience: Duration) -> Startup {
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => Startup::Serve(listener),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return Startup::Blocked(format!("port {port} is in use"));
            };
            if rt.block_on(faulk_helper_answers(port, origins, patience)) {
                Startup::AlreadyRunning
            } else {
                Startup::Blocked(format!("port {port} is used by another program"))
            }
        }
        Err(e) => Startup::Blocked(format!("cannot listen on 127.0.0.1:{port}: {e}")),
    }
}

/// Whether a Faulk helper answers the probe on `port` before `patience` runs out.
pub async fn faulk_helper_answers(port: u16, origins: &[String], patience: Duration) -> bool {
    let deadline = Instant::now() + patience;
    loop {
        for origin in origins {
            let left = deadline.saturating_duration_since(Instant::now());
            let attempt =
                tokio::time::timeout(left.min(Duration::from_secs(2)), probe(port, origin));
            if let Ok(Some(true)) = attempt.await {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// One probe: `Some(true)` for a Faulk helper, `None` when nothing usable came back.
async fn probe(port: u16, origin: &str) -> Option<bool> {
    let mut request = format!("ws://127.0.0.1:{port}{}", paths::PROBE)
        .into_client_request()
        .ok()?;
    request.headers_mut().insert("origin", origin.parse().ok()?);
    let mut ws = match tokio_tungstenite::connect_async(request).await {
        Ok((ws, _)) => ws,
        // A helper that does not allow this origin still says who it is.
        Err(Error::Http(response)) => {
            let name = response.headers().get(HELPER_HEADER)?.to_str().ok()?;
            return Some(name == HELPER_NAME);
        }
        Err(_) => return None,
    };
    let text = ws.next().await?.ok()?.into_text().ok()?;
    Some(is_helper_probe(&text))
}

/// Whether a probe answer comes from a Faulk helper, of any protocol version: read loosely, so
/// that a helper whose probe has other fields (#525 added `audio_output`) is still recognised.
fn is_helper_probe(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    value.get("type").and_then(|t| t.as_str()) == Some("probe")
        && value.get("helper").and_then(|h| h.as_str()) == Some(HELPER_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_of_any_version_is_recognised() {
        let current = serde_json::to_string(&protocol::ServerMessage::Probe {
            helper: HELPER_NAME.into(),
            protocol: protocol::PROTOCOL_VERSION,
            audio_output: false,
        })
        .unwrap();
        assert!(is_helper_probe(&current));
        // Version 20, before `audio_output`.
        assert!(is_helper_probe(
            r#"{"type":"probe","helper":"faulk-helper","protocol":20}"#
        ));
        assert!(!is_helper_probe(
            r#"{"type":"probe","helper":"another","protocol":21}"#
        ));
        assert!(!is_helper_probe(
            r#"{"type":"hello","helper":"faulk-helper"}"#
        ));
        assert!(!is_helper_probe("not json"));
    }
}
