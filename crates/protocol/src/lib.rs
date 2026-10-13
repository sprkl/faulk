//! Messages between the Faulk web UI and the native helper.
//!
//! The page sends [`ClientMessage`]s and receives [`ServerMessage`]s, as JSON text
//! frames over a local WebSocket. Every exchange starts with a message carrying
//! [`PROTOCOL_VERSION`]; a page and a helper of different versions refuse each other.

use commands::Command;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Version of this protocol. Bump it on any incompatible change.
pub const PROTOCOL_VERSION: u32 = 1;

/// Name the helper announces in its probe answer.
pub const HELPER_NAME: &str = "faulk-helper";

/// WebSocket paths served by the helper.
pub mod paths {
    /// No token: answers one [`super::ServerMessage::Probe`] and closes.
    pub const PROBE: &str = "/probe";
    /// No token: accepts [`super::ClientMessage::Pair`] only.
    pub const PAIR: &str = "/pair";
    /// Requires `?token=`: the session, which accepts [`super::ClientMessage::Command`].
    pub const SESSION: &str = "/session";
}

/// A message from the page to the helper.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ClientMessage {
    /// Pair with the helper using the one-time code it displays. Only on `/pair`.
    #[serde(rename = "pair")]
    Pair { code: String },
    /// Run a command on the helper's engine. Only on an authenticated `/session`.
    #[serde(rename = "command")]
    Command { command: Command },
}

/// A MIDI input as the helper sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MidiInput {
    /// Stable for as long as the device stays plugged in.
    pub id: String,
    pub name: String,
}

/// A message from the helper to the page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ServerMessage {
    /// Answer to a probe: a Faulk helper listens here.
    #[serde(rename = "probe")]
    Probe { helper: String, protocol: u32 },
    /// Pairing accepted: keep this token and pass it as `?token=` to `/session`.
    #[serde(rename = "paired")]
    Paired { token: String },
    /// Pairing refused (wrong or expired code). At 0 attempts left the helper shows a new code.
    #[serde(rename = "pairing_refused")]
    PairingRefused { attempts_left: u32 },
    /// First message of a session.
    #[serde(rename = "hello")]
    Hello {
        protocol: u32,
        sample_rate: f32,
        /// Frames per audio callback, once the device reported it.
        buffer_frames: Option<u32>,
        /// Output latency estimated from the buffer size, not measured.
        estimated_latency_ms: Option<f32>,
    },
    /// The MIDI inputs available, and the one selected.
    #[serde(rename = "midi.inputs")]
    MidiInputs {
        inputs: Vec<MidiInput>,
        selected: Option<String>,
    },
    /// A note received from the selected MIDI input (for the activity light).
    #[serde(rename = "midi.activity")]
    MidiActivity { note: u8, velocity: u8 },
    /// Output level and voice count, about 30 times per second.
    #[serde(rename = "meter")]
    Meter { peak: f32, voices: u32 },
    /// The helper could not do something; `code` is stable, `message` is for developers.
    #[serde(rename = "error")]
    Error { code: String, message: String },
}

/// Every schema the web app needs, in one document.
pub fn schema_bundle() -> serde_json::Value {
    let param_ids: serde_json::Map<String, serde_json::Value> = commands::param_ids()
        .into_iter()
        .map(|(name, id)| (name.to_owned(), id.into()))
        .collect();
    let param_defaults: serde_json::Map<String, serde_json::Value> = commands::param_defaults()
        .into_iter()
        // Rounded so that f32 values print as typed (0.005, not 0.004999999888).
        .map(|(name, value)| {
            (
                name.to_owned(),
                ((f64::from(value) * 1e6).round() / 1e6).into(),
            )
        })
        .collect();
    serde_json::json!({
        "protocol_version": PROTOCOL_VERSION,
        "param_ids": param_ids,
        "param_defaults": param_defaults,
        "command": commands::schema(),
        "client_message": schemars::schema_for!(ClientMessage),
        "server_message": schemars::schema_for!(ServerMessage),
    })
}
