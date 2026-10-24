//! Messages between the Faulk web UI and the native helper.
//!
//! The page sends [`ClientMessage`]s and receives [`ServerMessage`]s, as JSON text
//! frames over a local WebSocket. The probe and the session's first message carry
//! [`PROTOCOL_VERSION`]; the page refuses a helper of another version at both points.

use commands::Command;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Version of this protocol. Bump it on any incompatible change.
pub const PROTOCOL_VERSION: u32 = 5;

// Version 5 (#314): the feedback reports a track the launch grid holds without a clip as
// `ClipState::Held` (wire value 5), and the commands gain `launch.back_track_to_arrangement`.

// Version 4 (#293): the feedback layout is version 3: each active clip carries the tick of its
// position 0 (`anchor`, 16 bytes per cell instead of 8), so that the page places a playhead
// in a clip launched from the grid. A page and a helper of different versions refuse each other.

// Version 3 (MO-5): the helper reports the engine's state as binary WebSocket frames, about
// 30 per second: the bytes of an `engine::msg::Feedback` (little-endian layout of that
// type, version `engine::msg::FEEDBACK_VERSION`), the same bytes the web build's engine
// writes into its shared memory. Every other message is a JSON text frame. The JSON
// `meter` message of version 2 is gone.

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

/// The ids of the page's one-synth project, until the project runs in the page (PR-4,
/// PR-5): `live.*` commands play `TRACK`, `device.set_param` sets `DEVICE`. The project
/// itself is `commands::engine_msgs::single_synth`, which the page's module and the helper
/// both run the commands on.
pub mod single_synth {
    pub use commands::engine_msgs::single_synth::{DEVICE, TRACK};
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
        /// Output latency estimated from one callback buffer plus the driver's
        /// reported output delay; not measured.
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
    /// The helper could not do something; `code` is stable, `message` is for developers.
    #[serde(rename = "error")]
    Error { code: String, message: String },
}

/// Every schema the web app needs, in one document.
pub fn schema_bundle() -> serde_json::Value {
    let param_ids: serde_json::Map<String, serde_json::Value> = commands::device::param_ids()
        .into_iter()
        .map(|(name, id)| (name.to_owned(), id.into()))
        .collect();
    let param_defaults: serde_json::Map<String, serde_json::Value> =
        commands::device::param_defaults()
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
        "single_synth": {
            "track": single_synth::TRACK,
            "device": single_synth::DEVICE,
            // The kind of the one synth: the page reads its panel from that descriptor.
            "kind": engine::OSC_SYNTH.kind,
        },
        // The command contract: input and output schemas and the MCP tool catalog.
        "command": {
            "input": commands::schema(),
            "output": commands::output_schema(),
            "catalog": commands::catalog(),
        },
        "client_message": schemars::schema_for!(ClientMessage),
        "server_message": schemars::schema_for!(ServerMessage),
        // The page's project core (PR-4, `crates/project-wasm`): what its calls return.
        "project_core": {
            "run_report": schemars::schema_for!(commands::host::RunReport),
            "snapshot_report": schemars::schema_for!(commands::host::SnapshotReport),
            "project_view": schemars::schema_for!(commands::host::ProjectView),
            "export_report": schemars::schema_for!(commands::host::ExportReport),
        },
        // The device contract (CT-5): the descriptor of every built-in device, from which
        // the UI generates the panels and agents learn the parameters, and its schema.
        "devices": engine::device::builtin_descriptors().collect::<Vec<_>>(),
        "device_descriptor": schemars::schema_for!(engine::device::DeviceDescriptor),
    })
}
