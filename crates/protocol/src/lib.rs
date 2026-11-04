//! Messages between the Faulk web UI and the native helper.
//!
//! The page sends [`ClientMessage`]s and receives [`ServerMessage`]s, as JSON text
//! frames over a local WebSocket. The probe and the session's first message carry
//! [`PROTOCOL_VERSION`]; the page refuses a helper of another version at both points.
//!
//! Binary frames carry the engine (BO-1): from the page, each one is a `ControlMsg` byte
//! stream the page's project core encoded (`engine::msg`), the same bytes the page's own
//! engine takes, which the helper submits unchanged (at most [`MAX_STREAM_BYTES`]); from the
//! helper, each one is a report (`engine::msg::Feedback`, version 3 of this protocol). The page
//! owns the project in both modes: no command crosses the link on the page's initiative.
//!
//! Agents (AG-1, version 26): the helper serves MCP and relays each tool call to the page as
//! `agent.call` (a command, with an `id`); the page runs it through its own dispatcher, as a
//! user's action, and answers `agent.result` with the same `id`. Since version 29 (#608) a
//! paired page also holds a control link (`/control`) that carries only these two messages,
//! `agent.left` and `control.front`, in Web mode as in Boost, with or without an audio output.
//! Since version 32 (#641) the helper says when an agent left (`agent.left`), so that the page
//! closes the history group that agent left open.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Version of this protocol. Bump it on any incompatible change.
pub const PROTOCOL_VERSION: u32 = 35;

/// The largest engine stream the page sends in one binary frame, and the largest message the
/// helper reads on a session: above every snapshot a project within the project's limits
/// encodes (16 Mi notes at about 21 bytes each are 352 MB). The page never sends a longer
/// one; the helper's other routes keep the WebSocket library's default limits.
pub const MAX_STREAM_BYTES: usize = 1 << 30;

/// The WebSocket close code of a control link the helper closed because newer ones pushed it
/// out (#608, more than its limit of links open): the page does not try it again on its own,
/// so that many pages never push each other out in a loop.
pub const CONTROL_PUSHED_OUT: u16 = 4001;

/// How long the page waits for the user to answer an agent's destructive call (AG-4, #100), in
/// milliseconds, before it answers the agent `refused` with the code `confirmation.timeout`.
/// Well under the helper's own wait for the page (`AGENT_TIMEOUT` in `crates/helper`, tested):
/// for a question nobody answers, the agent learns the page's answer, not a bare `page.timeout`.
/// The bound covers the question only: an allowed action that takes long can still end in
/// `page.timeout` although it ran.
pub const AGENT_CONFIRM_MS: u32 = 45_000;

// Version 35 (#617; 33 is #670's and 34 is #676's): the commands gain `project.cancel_export_pack` (stops the `.faulkpack` export under way); the helper generates its MCP tools from the commands, so a helper of version 34 would not offer it to an agent. No message changes.

// Version 32 (#641): the helper tells the page that an agent left (`agent.left`, its name): its
// last MCP session ended (the client closed it, the stdio bridge's input ended, or it stayed idle
// for `AGENT_IDLE`), or its stateless calls stopped for as long. The page closes the history group
// that agent left open and ends its turn, so that the user can undo again. Sent on the session and
// on every control link. A page of version 31 would not know the message.

// Version 31 (#112, AU-3): audio clips reach the engine. `AudioSourceDeclare` carries the
// blob's SHA-256 and `ClipAudioSet` the clip's gain and loop (their payloads change), and the
// report is `Feedback` version 7 (offline sources, the PCM budget flag and the PCM in memory
// after the header, which grows from 76 to 88 bytes). A page and a helper of different
// versions refuse each other.

// Version 30 (#114, 29 is #619's): audio clip commands (AU-5). `clip.import_audio` takes an
// optional target `at` and answers its `placement`; new `clip.audio.set_range`,
// `clip.audio.set_gain`, `clip.audio.set_loop`; `clip.move` takes audio clips; clip summaries
// gain `media_missing`; the clip event part `gain`. A helper of version 29 would not offer the
// new tools.

// Version 29 (#608; 28 is #191's): agents reach a paired page in Web mode and with a helper that has no audio output. A new token route, `/control`, is the page's control link: it carries only `agent.call`, `agent.result` and `control.front` (the page came to the front), never takes the audio role, never replaces the session and opens whatever the output. The helper sends an agent's call to the greeted session (Boost) first, else to the front control link (the newest, or the last that sent `control.front`); `page.absent` only when neither exists. At most 8 links: a newer one closes the oldest with close code `CONTROL_PUSHED_OUT` (4001). A page and a helper of different versions refuse each other.

// Version 28 (#191, 27 is #616's): device presets. The commands gain the `preset` domain (`preset.save`, `preset.load`, `preset.delete`, `preset.rename`, `preset.copy`, `preset.list`, `preset.restore`, `preset.restore_device`), the events `preset_changed` and `device_preset_changed`, a device an optional `preset`, the project its `presets` (folder format 4). The helper's MCP tools come from the command catalogue (AG-1), so a helper of version 27 would not offer them.

// Version 27 (#110, AU-1): the commands gain `clip.import_audio`, `clip.restore_audio`, `clip.choose_audio` and `clip.cancel_audio_import`, and `clip.delete` takes an audio clip; the helper generates its MCP tools from the commands, so a helper of version 26 would not offer them to an agent. No message changes: the import runs in the page in both modes, and nothing of an audio file crosses the link.

// Version 26 (#97, AG-1; 25 is #594's): agents. The helper serves MCP (`/mcp` on the same port) and relays each tool call to the session as `agent.call` (`id`, the agent's name, the command as JSON); the page runs it through its dispatcher like a user's action (history author `agent:<name>`, undo, refusals) and answers `agent.result` with the same `id` and an `AgentResult`. A page and a helper of different versions refuse each other.

// Version 25 (#486): the feedback layout is version 6: a 76-byte header whose new word at offset 72 is the CPU load's peak (`cpu_peak`, f32, the largest load of one peak window of the period, -1 until the host timed a block), and the helper now times its render, so `cpu_load` is measured in Boost. A page and a helper of different versions refuse each other.

// Version 24 (#598): the notes of the helper's MIDI input play on the helper's engine at once, without the page's main thread. The page sends `live.routing` (`epoch`, the slots of the tracks the live input reaches) after each snapshot and whenever it changes; the helper encodes each note on in that epoch for those tracks and each note off for the tracks its note went to, and ignores a table of another epoch than its engine's. `midi.message` still carries every message to the page and gains `played`: the page runs the message in its project and sends its engine bytes only when the helper did not play it (control changes, a note before any table). A page of version 23 would double the notes, and one of version 24 would leave them unplayed with a helper of version 23.

// Version 23 (#102, BO-1): the helper holds no project. The page sends the engine's `ControlMsg` byte streams it encodes in binary frames (a snapshot newer than the epoch `hello` announces, then each change), the helper submits them unchanged and answers a stream its engine stops with `stream_refused` (code, offset, the engine's epoch), to which the page answers with a snapshot. `hello` gains `epoch` and `capabilities` (`midi`, `disk`). The `command` message is gone: the helper's own MIDI devices take `midi.list_inputs` and `midi.select_input`, and the selected input's messages go to the page as `midi.message` (raw bytes), which plays them through its project; `midi.activity` is gone.

// Version 22 (#525): the helper starts without an audio output. The probe answers `audio_output` (an output is open now), and a session opened without an output gets `error` `audio.no_output` and is closed, without `hello`. A page of an earlier version would take that refusal for a revoked token, so the versions refuse each other.

// Version 21 (#555): the new command `arrangement.repeat` (linked copies of a placement, end to end, up to a tick); a helper of version 20 would refuse it.

// Version 20 (#490): a track keeps the height of its row in the arrangement. The commands gain `track.set_height`, the track summary and the project's tracks an optional `height` (not written when unset), the event `track_changed` a part `height`. The project file format is 3 (was 2): `track.json` gets an optional `height`; format 2 folders migrate by the number alone, and a build of format 2 reports a format 3 folder as made by a newer build.

// Version 19 (#491): the new command `arrangement.move_to_track` (placements to other MIDI tracks); a helper of version 18 would refuse it.

// Version 18 (#500): MIDI recording replace mode and quantization on input. `record.set_settings` takes optional `offset_ms`, `replace`, `quantize_on_input` and `quantize_strength` (it required `offset_ms`) and answers with the four previous values; `record.commit_take` takes optional `replace`, `quantize_on_input` and `quantize_strength` and answers with `replaced`; the event `record_settings_changed` carries the four settings; the session's `record` gains them. A helper of version 17 would refuse the new fields.

// Version 17 (#499): `live.input_note_on` and `live.input_note_off` take an optional `frame`, the host frame the key was played at, so that a recording places the note where it was played; a helper of version 16 would refuse the field (commands refuse unknown fields).

// Version 16 (#473, markers): the arrangement gains named section markers (`arrangement.marker.add`, `.rename`, `.move`, `.delete`, `.restore`), the project its `markers`, `arrangement.restore_time` an optional `markers`, the events `markers_changed`.

// #515, without a new version: the helper reports its self-update to the session (`update.status`: installed version, end of the last check, the updater's state) after `midi.inputs` and on every change. Compatible both ways: a page that does not know the message ignores it (it switches on `type` and drops the others), and a page that knows it keeps `null` with a helper that never sends it.

// Version 15 (#473, clip pool): the arrangement commands' outputs lose `clips_removed` (a clip that loses its last placement stays in its track), `clip.move` takes a clip out of the grid into the pool, and a placement longer than a clip without a loop repeats the whole clip.

// Version 14 (#473): commands gain the `setting` kind (`transport.toggle_loop` and `transport.set_metronome` leave the undo history for its activity log), `mixer.set_solo` becomes an undoable edit, `transport.set_loop` an optional `enabled`, `project.new` an optional `name`; the history view carries each entry's subject and the activity log.

// Version 13 (#470): MIDI recording. The commands gain `record.set_settings`, `record.start`, `record.stop`, `record.discard_take`, `record.commit_take` and `record.restore`; the events gain `recording_changed` and `record_settings_changed`; the session gains `record` (the manual latency offset). The feedback layout is version 5: a 72-byte header with the take's number, its count of notes, the notes lost and whether the engine captures, then the take's last notes (at most 256) after the active clips. A page and a helper of different versions refuse each other.

// Version 12 (#469): the feedback layout is version 4: each meter carries the largest gain reduction, in decibels, of the devices of its strip (the Compressor), 20 bytes instead of 16. A page and a helper of different versions refuse each other.

// Version 10 (#378): the master has automation lanes. `automation.lane.create` and `automation.lane.restore` take `track` or `master: true` (exactly one), `automation.lane.delete` answers `track: null` for a master lane, the events `automation_lane_created`, `automation_lane_deleted` and `automation_points_changed` carry `track: null` for the master, and the project holds `master_automation`. The engine addresses the master's lanes with the track slot `u32::MAX` in `AutomationLaneUpload`. The project file format is unchanged (still 1): the master's lanes are files `master/automation/<lane>.tsv`.

// Version 9 (#118): the commands gain `automation.lane.create`, `automation.lane.delete`, `automation.lane.restore`, `automation.points.add`, `automation.points.remove`, `automation.points.update`, `automation.points.restore` and `automation.lane.clear`; `device.restore` takes an optional `lanes` (the automation lanes `device.remove` took with the device); the events gain `automation_lane_created`, `automation_lane_deleted` and `automation_points_changed`. The project file format is unchanged (still 1).

// Version 8 (#364): the master has a chain of effects. `device.insert` and `device.restore` take `track` or `master: true`, `device.remove` answers `track: null` for a master device, the event `device_chain_changed` carries `track: null` for the master, and the project holds `master_devices`. The engine addresses that chain with the track slot `u32::MAX` in the four device messages. The helper does not run device chains yet (BO-1, #102).

// Version 7 (#83): the commands gain `clip.notes.quantize`, `clip.notes.transpose` and `clip.notes.duplicate`.

// Version 6 (#331): the commands gain `live.all_sound_off`, and the engine's control stream gains
// the `AllSoundOff` message (tag 93).

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

/// Stable `code`s of [`ServerMessage::Error`] the page acts on.
pub mod errors {
    /// A session opened while the helper has no audio output (#525): sent instead of `hello`,
    /// then the session is closed. The page keeps playing itself and its pairing.
    pub const NO_OUTPUT: &str = "audio.no_output";
    /// The audio output failed and its engine is gone: the page plays itself again. Before
    /// `hello`, the page takes it as [`NO_OUTPUT`].
    pub const AUDIO_STREAM: &str = "audio.stream";
}

/// Name the helper announces in its probe answer.
pub const HELPER_NAME: &str = "faulk-helper";

/// WebSocket paths served by the helper.
pub mod paths {
    /// No token: answers one [`super::ServerMessage::Probe`] and closes.
    pub const PROBE: &str = "/probe";
    /// No token: accepts [`super::ClientMessage::Pair`] only.
    pub const PAIR: &str = "/pair";
    /// Requires `?token=`: the session, which takes the page's engine streams (binary frames)
    /// and the messages for the helper's MIDI inputs.
    pub const SESSION: &str = "/session";
    /// Requires `?token=`: the page's control link (#608), which carries only the agents'
    /// relay (`agent.call`, `agent.result`, `agent.left`). It never takes the audio role, never replaces the
    /// session, and opens whether or not the helper has an audio output.
    pub const CONTROL: &str = "/control";
}

/// The ids of the page's default project (`commands::engine_msgs::single_synth`, the project
/// a new page starts with): `live.*` commands play `TRACK`, `device.set_param` sets `DEVICE`.
pub mod single_synth {
    pub use commands::engine_msgs::single_synth::{DEVICE, TRACK};
}

/// A JSON message from the page to the helper (the engine streams are binary frames).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ClientMessage {
    /// Pair with the helper using the one-time code it displays. Only on `/pair`.
    #[serde(rename = "pair")]
    Pair { code: String },
    /// List the helper's MIDI inputs again: answered by `midi.inputs`. The page sends it
    /// after its own `midi.list_inputs` command. Only on a session.
    #[serde(rename = "midi.list_inputs")]
    MidiListInputs,
    /// Open the helper's MIDI input `id`, or close it (`None`): answered by `midi.inputs`, and
    /// `error` `midi.connect` when it cannot open. The page sends it after its own
    /// `midi.select_input` command. Only on a session.
    #[serde(rename = "midi.select_input")]
    MidiSelectInput { id: Option<String> },
    /// Where the helper's MIDI input plays its notes (#598): the engine slots of the tracks
    /// the live input reaches now (`commands::live::input_targets`: the selected track and the
    /// armed MIDI tracks), in the engine `epoch` whose slots they are
    /// (`commands::host::LiveRouting`). The page sends it after each snapshot it sends and
    /// whenever the table changes. The helper plays each note of its input on these tracks at
    /// once, in this epoch, and its note off on the tracks the note went to; a table of
    /// another epoch than its engine's is ignored, and so is one longer than the engine's
    /// track limit. Only on a session.
    #[serde(rename = "live.routing")]
    LiveRouting { epoch: u16, tracks: Vec<u32> },
    /// The page's answer to the helper's `agent.call` of the same `id` (AG-1): what running
    /// the agent's command did. The helper takes it only from the link (session or control
    /// link) the call went to. On a session or a control link.
    #[serde(rename = "agent.result")]
    AgentResult { id: u64, result: AgentResult },
    /// The page came to the front (shown, or focused) (#608): its control link becomes the one
    /// the agents' calls go to while no session is greeted, so with several pages open an agent
    /// drives the one the user last looked at. Only on a control link.
    #[serde(rename = "control.front")]
    ControlFront,
}

/// What running an agent's command in the page did (`agent.result`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentResult {
    /// The command applied. `output` is its typed output (the helper drops its `type` tag: the
    /// tool's name carries it). For the commands the page answers once its own work is done, the
    /// output it builds then: `project.list_stored` its lists, `project.new` the new project's
    /// `name`; `null` when the report of an applied command was lost.
    Applied { output: serde_json::Value },
    /// The project refused it (a `CommandError`, `code` first), or the page did (a recording in
    /// progress: `{ "code": <the page's notice> }`): nothing changed.
    Refused { error: serde_json::Value },
    /// The JSON is not a command this page knows: nothing changed.
    Invalid { message: String },
    /// The page cannot run it now (no project yet, no engine, a session it no longer plays, or a
    /// command that opens a picker or a permission prompt, which needs a click in the page):
    /// nothing changed.
    Unavailable { reason: String },
}

/// What the helper can do, announced in `hello` (BO-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    /// The helper can open MIDI inputs (its system has a MIDI backend).
    pub midi: bool,
    /// The helper reads and writes the project's folder for the page. Not yet: always `false`.
    pub disk: bool,
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
    Probe {
        helper: String,
        protocol: u32,
        /// Whether the helper has an audio output open now (#525). Without one it serves
        /// anyway, and answers a session with `error` `audio.no_output` instead of `hello`.
        audio_output: bool,
    },
    /// Pairing accepted: keep this token and pass it as `?token=` to `/session`.
    #[serde(rename = "paired")]
    Paired { token: String },
    /// Pairing refused (wrong or expired code). At 0 attempts left the helper shows a new code.
    #[serde(rename = "pairing_refused")]
    PairingRefused { attempts_left: u32 },
    /// First message of a session; a helper without an audio output sends `error`
    /// `audio.no_output` instead and closes the session. Sent again when the output was
    /// replaced and the session plays on (#577): the new output's rate, buffer and latency.
    #[serde(rename = "hello")]
    Hello {
        protocol: u32,
        sample_rate: f32,
        /// Frames per audio callback, once the device reported it.
        buffer_frames: Option<u32>,
        /// Output latency estimated from one callback buffer plus the driver's
        /// reported output delay; not measured.
        estimated_latency_ms: Option<f32>,
        /// The epoch of the helper's engine (its last `Reset`): the page's first stream is a
        /// `Reset` and a snapshot in a newer epoch (the engine outlives a page).
        epoch: u16,
        capabilities: Capabilities,
    },
    /// A stream of the page that the helper's engine stopped (corrupt, out of memory, past
    /// the waiting bound) or refused whole (not of the engine's epoch): `code` is
    /// `engine::msg::MsgError::code`, `at` the byte of the message that stopped it in the piece the engine was given (for a stream sent in pieces, from the start of the rest it was retrying; informative only), `epoch`
    /// the engine's. The engine is behind the page's project: the page sends a snapshot in an
    /// epoch newer than `epoch`.
    #[serde(rename = "stream_refused")]
    StreamRefused { code: i32, at: u32, epoch: u16 },
    /// A message of the selected MIDI input, as received (a channel message: note, control
    /// change...). The page runs it in its project as it runs a Web MIDI message (the keys
    /// shown, a recording, its held notes), and sends the engine bytes it encodes unless
    /// `played`.
    #[serde(rename = "midi.message")]
    MidiMessage {
        data: Vec<u8>,
        /// The helper already played this note on its engine, with the session's
        /// `live.routing` (#598): a note on on the table's tracks, a note off on the tracks its
        /// note went to. `false` for anything else: a control change (120 and 123 included,
        /// which the page turns into `live.all_notes_off`), a note before any table or with a
        /// table of another epoch than the engine's, a note off of a note the helper did not
        /// play.
        played: bool,
    },
    /// The MIDI inputs available, and the one selected.
    #[serde(rename = "midi.inputs")]
    MidiInputs {
        inputs: Vec<MidiInput>,
        selected: Option<String>,
    },
    /// The helper's self-update: sent after `midi.inputs` at the start of a session, then on
    /// every change.
    #[serde(rename = "update.status")]
    UpdateStatus {
        /// The installed version; `None` when the helper is not installed (run from `cargo`).
        current: Option<String>,
        /// End of the last check, in seconds since the Unix epoch; `None` before the first.
        checked_at: Option<u64>,
        status: HelperUpdate,
    },
    /// An agent's tool call (AG-1): the command to run, as JSON (`{"type": "<domain.verb>",
    /// ...}`), and the agent's name, the history's author `agent:<name>`. The page runs it
    /// through its dispatcher like a user's action and answers `agent.result` with this `id`;
    /// the helper gives up on it when the link it went on ends or after its timeout. Sent on
    /// the greeted session (Boost) if there is one, else on the newest control link (#608).
    #[serde(rename = "agent.call")]
    AgentCall {
        id: u64,
        agent: String,
        command: serde_json::Value,
    },
    /// An agent left (#641): no MCP session of that name is open any more and it made no
    /// stateless call for `AGENT_IDLE` (`crates/helper`). Its last session ended when the client
    /// closed it, when the stdio bridge's input ended, or after `AGENT_IDLE` without a request.
    /// Two sessions of one name are one author: the name leaves with its last one. The page
    /// closes the history group that agent (`agent:<agent>`) left open, ends its turn and
    /// withdraws its questions to the user. Sent on the session and on every control link.
    #[serde(rename = "agent.left")]
    AgentLeft { agent: String },
    /// The helper could not do something; `code` is stable, `message` is for developers.
    #[serde(rename = "error")]
    Error { code: String, message: String },
}

/// What the helper's updater is doing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum HelperUpdate {
    /// Updates are off (not installed, or no feed); `reason` is for developers.
    Disabled {
        reason: String,
    },
    /// Started, first check not done yet.
    Idle,
    Checking,
    /// The installed version is the newest of its channel.
    UpToDate {
        version: String,
    },
    Downloading {
        version: String,
        percent: u8,
    },
    /// Downloaded; applied at the helper's next start or by "Restart to update".
    Ready {
        version: String,
    },
    /// The last check or download failed; the next check retries. `error` is for developers.
    Failed {
        error: String,
    },
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
        "error_codes": { "no_output": errors::NO_OUTPUT, "audio_stream": errors::AUDIO_STREAM },
        "max_stream_bytes": MAX_STREAM_BYTES,
        "control_pushed_out": CONTROL_PUSHED_OUT,
        "agent_confirm_ms": AGENT_CONFIRM_MS,
        // The commands whose call by an agent waits for the user's confirmation in the page
        // (`x-destructive`, AG-4): the page reads the list from here, never from the schema.
        "destructive_commands": commands::catalog()
            .into_iter()
            .filter(|spec| spec.destructive)
            .map(|spec| spec.name)
            .collect::<Vec<_>>(),
        "param_defaults": param_defaults,
        // The fader's law (#389): `[position, dB]` points, from the bottom (silence) to the top.
        // The page's fader, the axis of a volume lane and the engine's reading of a lane all
        // follow it; the page builds its own from this table.
        "fader_taper": engine::fader::TAPER,
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
            "history_view": schemars::schema_for!(commands::host::HistoryView),
            "live_routing": schemars::schema_for!(commands::host::LiveRouting),
        },
        // The device contract (CT-5): the descriptor of every built-in device, from which
        // the UI generates the panels and agents learn the parameters, and its schema.
        "devices": engine::device::builtin_descriptors().collect::<Vec<_>>(),
        "device_descriptor": schemars::schema_for!(engine::device::DeviceDescriptor),
    })
}
