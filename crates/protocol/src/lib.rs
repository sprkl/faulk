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
//! `agent.left`, `control.front` and, since version 33 (#658), the helper's opening
//! `control.hello`, in Web mode as in Boost, with or without an audio output.
//! Since version 32 (#641) the helper says when an agent left (`agent.left`), so that the page
//! closes the history group that agent left open.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Version of this protocol. Bump it on any incompatible change.
pub const PROTOCOL_VERSION: u32 = 53;

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

// Version 53 (#156): the device registry gains Contour (`faulk.contour`, kind id 8, an audio
// effect: the seven-band equaliser); a helper of an earlier version would refuse its
// `DeviceInsert` and its schema bundle would not describe it. No message changes.

// Version 52 (#696): the device registry
// gains Bloom (`faulk.bloom`, kind id 14, an audio effect: chorus, ensemble and vibrato); a
// helper of an earlier version would refuse its `DeviceInsert` and its schema bundle would not
// describe it. No message changes.

// Version 51 (one batch of 2026-11-07: #689, #157, #754, #686, #749, #755, #758, #765, #761): a
// page and a helper of different versions refuse each other, because:
// - the registry gains the audio effect `faulk.hiccup` (#689, device kind 13, a stutter on the
//   transport's grid): a helper of an earlier version refuses its `DeviceInsert`;
// - the Compressor (kind 5, #157) gains seven parameters, ids 5 to 11 (`knee`, `detection`,
//   `release_mode`, `sidechain_hpf`, `sidechain_freq`, `lookahead`, `mix`): an earlier helper's
//   Compressor would ignore them silently and play another sound than the page;
// - the Filter (kind 6, #686) gains its movement parameters (a tempo LFO, an envelope follower,
//   drive and a notch mode) and the device contract the units `degrees` and `octaves`;
// - the report is `Feedback` version 10: the header grows from 100 to 108 bytes, the number of
//   device readouts at offset 100 (`FEEDBACK_READOUTS_OFFSET`, #686) and the device latency word
//   at 104 (`FEEDBACK_LATENCY_OFFSET`, #749: bits 0 to 30 the frames the graph compensates, bit 31
//   the capped flag), and a section of device readouts (track slot, device slot, parameter id,
//   value: 12 bytes each) follows the recorded notes: the live value of a parameter a device
//   moves by itself (the Filter's cutoff under its LFO and envelope);
// - the engine compensates the devices' latency (#749; the export drops it from the file's start,
//   #755; the strip, #758, and the devices' automation, #765, land on their sound behind it; a
//   device inserted or switched with a latency stays seamless, #761), and `mixer.levels` answers
//   it (`latency_frames`, `latency_capped`);
// - `device.restore`, `track.restore` and `track.restore_instrument` fill the parameters a
//   snapshot lacks (#754), so their descriptions, which the helper's MCP tools carry, change.

// Version 50 (#159): the registry gains the audio effect `faulk.ceiling` (device kind 7, the
// brick-wall limiter). No message changes shape, but a helper of an earlier version does not know
// kind 7: its engine refuses a stream that inserts it (`UnknownDevice`), so a project holding a
// Ceiling would not play in Boost.

// Version 49 (#778; 48 is #691's): the native audio import in Boost. The session gains `import.choose`,
// `import.start` and `import.cancel` (page → helper) and `import.chosen`, `import.progress`,
// `import.done` and `import.failed` (helper → page); `hello`'s capabilities gain `import`. The
// helper decodes a file of the project's folder, or one the user picked in its dialog, and writes
// its blob and caches into the folder; the page makes the clip. The commands gain
// `clip.import_audio_file`, and `clip.choose_audio`'s description changes (the helper's dialog in
// Boost); the schema bundle gains `import_progress_ms` and `import_error_codes`.

// Version 48 (#691; 45 is #693's, 46 #789's, 47 #692's): the registry gains the MIDI
// effect `faulk.scatter` (device kind 11: a note's pitch moved, at a chance, to one of up to 24 choices, in semitones or in the
// project's scale). A helper of an earlier version would not build the device: a page and a
// helper of different versions refuse each other.

// Version 47 (#692; 46 is #789's): the registry gains the MIDI effect
// `faulk.lattice` (device kind 12), which puts the notes it receives on the project's scale, a scale of
// its own or a matrix. A helper of an earlier version would not build the device: a page and a helper of
// different versions refuse each other.

// Version 46 (#319; 45 is #693's): the audio export in Boost. The session
// gains `export.start` and `export.cancel` (page → helper) and `export.progress`, `export.done`
// and `export.failed` (helper → page); `hello`'s capabilities gain `export`. The helper renders
// the page's export job and writes the file into the project's folder (`exports/`); the schema
// bundle gains `max_export_stream_bytes`, `export_name` and `export_error_codes`.

// Version 45 (#693): the device registry gains Trim (`faulk.trim`, kind
// id 10, a MIDI effect); a helper of an earlier version would refuse its `DeviceInsert` and its schema
// bundle would not describe it. No message changes.

// Version 44 (#690, #219; 43 is #783's): the engine takes `ControlMsg::SetScale` (tag 4, the project's
// scale as a set of pitch classes), the registry gains the MIDI effect `faulk.stack` (device kind 9), and
// the device commands insert, move and remove MIDI effects (role `midi_effect`). A helper of an earlier
// version would skip the message and not build the device: a page and a helper of different versions
// refuse each other.

// Version 43 (#116, AU-7): the helper reads the audio files of the project's folder. The session gains
// `folder.open`, `folder.choose` and `folder.close` (page → helper) and `folder.state` (helper → page);
// `hello`'s `disk` is `true`; the commands gain `project.choose_helper_folder`.

// Version 42 (#664): the commands gain `clip.import_audio_url` (an agent's audio file from a
// link, imported by the page once the user allowed it); the helper generates its MCP tools from
// the commands, so a helper of version 41 would not offer it to an agent. No message changes:
// the download and the import run in the page in both modes. The schema bundle gains
// `max_audio_url_bytes`.

// Version 41 (#733): the report is `Feedback` version 9: the header grows from 92 to 100 bytes with
// `host_frame` at offset 92 (the host's frame at the report's playhead), so that the page can place a
// tick on the host's clock. A page and a helper of different versions refuse each other.

// Version 40 (#682; 38 is #560's, 39 is #710's): the commands gain `project.cancel_import_pack` (stops the
// `.faulkpack` imports under way, downloads included); the helper generates its MCP tools from the
// commands, so a helper of an earlier version would not offer it to an agent. No message changes. Its output
// (`cancelled`, `too_late`, #735) was settled before any helper published version 40, so the version stays.

// Version 39 (#710): the commands gain `clip.notes.repeat` (notes repeated end to end up to a
// tick, cut at the end of the clip). The helper's MCP tools come from the commands, so a helper of
// version 38 would not offer it to an agent. No message changes.

// Version 38 (#560): the engine message `SetRecording` carries `shift_us`, how much earlier the
// take's notes are placed (the estimated output latency plus the manual offset): the engine moves
// them back on its own clock, where it knows the loop wraps, and the page writes the take with a
// `shift_ms` of 0. Its payload grows by four bytes, so a page and a helper of different versions
// refuse each other.

// Version 37 (#115, AU-6): the arrangement commands
// take audio placements (move, resize, split, duplicate, repeat, move_to_track, place_from_slot,
// place_scene, insert_time, delete_time; `arrangement.place` of an audio clip carves instead of
// refusing an overlap), `arrangement.restore` takes `audio_clips` and removes audio clips, and
// `clip.duplicate` copies an audio clip. The helper's MCP tools come from the commands, so a helper
// of another version would describe them otherwise.

// Version 36 (#695, 33 to 35 are #670's, #676's and #684's): the project's key and scale. The
// commands gain `project.set_scale`, `project.get` answers `scale_root`, `scale_mode` and
// `scale_pitch_classes`, the events `scale_changed`, and the project its `scale` (folder format
// 5). The helper's MCP tools come from the command catalogue (AG-1), so a helper of an earlier version
// would not offer the command.

// Version 35 (#617; 33 is #670's and 34 is #676's): the commands gain `project.cancel_export_pack` (stops the `.faulkpack` export under way); the helper generates its MCP tools from the commands, so a helper of version 34 would not offer it to an agent. No message changes.

// Version 34 (#667, 33 is #658's): the report is `Feedback` version 8: the header grows from 88
// to 92 bytes with `silenced_frames` at offset 88 (the stereo frames the output protection
// silenced because a sample was non-finite, cumulative since the engine was built), so that a
// NaN during playback shows in the page in both modes. `mixer.levels` answers that count too
// (`silenced_frames`), so that an agent sees it. A page and a helper of different versions
// refuse each other.

// Version 33 (#658): the control link (`/control`, #608) opens with `control.hello` (the helper's
// protocol), as the session opens with `hello`. The page refuses a helper of another version on
// that link too, with the message of the probe. A page of version 32 would take the link for
// open at once and ignore the message.

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
    /// `export.failed` (#319): an export runs already (one at a time).
    pub const EXPORT_BUSY: &str = "export.busy";
    /// `export.failed`: the helper has no folder open for the export's project.
    pub const EXPORT_NO_FOLDER: &str = "export.no_folder";
    /// `export.failed`: the request is not one the helper renders (name, format, rate, length,
    /// streams).
    pub const EXPORT_INVALID: &str = "export.invalid";
    /// `export.failed`: the engine refused the streams or the render failed.
    pub const EXPORT_RENDER: &str = "export.render";
    /// `export.failed`: the disk refused the file (`message` holds the path error's code).
    pub const EXPORT_WRITE: &str = "export.write";
    /// `export.failed`: the project's folder changed during the export (another place named, the
    /// folder closed, moved or replaced on disk).
    pub const EXPORT_FOLDER_CHANGED: &str = "export.folder_changed";
    /// `export.failed`: the page cancelled it, or its session ended.
    pub const EXPORT_CANCELLED: &str = "export.cancelled";
    /// `import.failed` (#778): an import runs already (one at a time), or the helper's dialog is
    /// open already.
    pub const IMPORT_BUSY: &str = "import.busy";
    /// `import.failed`: the helper has no folder open for the import's project, in this session.
    pub const IMPORT_NO_FOLDER: &str = "import.no_folder";
    /// `import.failed`: the request is not one the helper takes (a project id, a token).
    pub const IMPORT_INVALID: &str = "import.invalid";
    /// `import.failed`: the source's path is refused (`message` holds the path error's code:
    /// `path.escapes`, `path.reserved`, `path.dot`...).
    pub const IMPORT_PATH: &str = "import.path";
    /// `import.failed`: the source cannot be read (gone, not a file, a token unknown, used, expired
    /// or of another session).
    pub const IMPORT_UNREADABLE: &str = "import.unreadable";
    /// `import.failed`: the disk refused a file of the import (`message` holds the path error's
    /// code).
    pub const IMPORT_WRITE: &str = "import.write";
    /// `import.failed`: the project's folder changed during the import (another place named, the
    /// folder closed, moved or replaced on disk).
    pub const IMPORT_FOLDER_CHANGED: &str = "import.folder_changed";
    /// `import.failed`: the page cancelled it, or its session ended.
    pub const IMPORT_CANCELLED: &str = "import.cancelled";
    /// `import.failed` of `import.choose`: this system has no file dialog the helper can show.
    pub const IMPORT_DIALOG_UNAVAILABLE: &str = "import.dialog_unavailable";
    /// The prefix of `import.failed` for a file the decoder refuses: `import.` and
    /// `audio_import::ImportError::code` (`import.too_large`, `import.corrupt`...).
    pub const IMPORT_DECODE_PREFIX: &str = "import.";
}

/// The shortest time between two `import.progress` of one import, in milliseconds (#778).
pub const IMPORT_PROGRESS_MS: u64 = 100;

/// How long a token of `import.chosen` stays valid, in seconds (#778).
pub const IMPORT_TOKEN_SECS: u64 = 600;

/// The shortest time between two `export.progress` of one export, in milliseconds.
pub const EXPORT_PROGRESS_MS: u64 = 100;

/// The longest file name of an `export.start` (#319), in UTF-8 bytes, before `.wav`: the page
/// cuts its names to it, the helper refuses a longer one.
pub const EXPORT_NAME_MAX_BYTES: usize = 120;

/// The Windows device names (#319, AU-7): a file of that name, with any extension, is the device
/// in every folder. The helper refuses them as a path's component; the page changes an export
/// name that is one.
pub const WINDOWS_DEVICE_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "conin$", "conout$", "com1", "com2", "com3", "com4", "com5",
    "com6", "com7", "com8", "com9", "com0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7",
    "lpt8", "lpt9", "lpt0", "com¹", "com²", "com³", "lpt¹", "lpt²", "lpt³",
];

/// The most bytes the three streams of an `export.start` may hold together, before base64
/// (#319). The page renders a larger job in its own export Worker (the browser's file dialog),
/// as in Web mode: the streams travel as base64 in one JSON message, encoded on the page's main
/// thread and parsed on the session's reader, which also carries the live engine streams.
pub const MAX_EXPORT_STREAM_BYTES: usize = 64 << 20;

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
    /// The page's project lives in a real folder (AU-7, #116): `project` is its id (a ULID),
    /// `nonce` the page's link marker (32 lowercase hex digits) that the page wrote in that folder
    /// as `.faulk/link.json` before sending this, `name` the folder's name (for the dialog's
    /// title only, never a path). The helper opens the remembered folder that holds both the id
    /// and the marker, and answers `folder.state`. No message names a path: the helper learns a
    /// folder only from its own dialog (`folder.choose`). Sent after each `hello` and on each
    /// change of the project's place. Only on a session.
    #[serde(rename = "folder.open")]
    FolderOpen {
        project: String,
        nonce: String,
        name: String,
    },
    /// The user asked to show the project's folder to the helper: the helper opens its folder
    /// dialog (one at a time) and opens the folder chosen when it holds the project's id and the
    /// page's marker, then answers `folder.state`. Only on a session.
    #[serde(rename = "folder.choose")]
    FolderChoose {
        project: String,
        nonce: String,
        name: String,
    },
    /// The page's project has no real folder (kept in the browser), or none: the helper closes
    /// its folder (its audio clips go offline). Only on a session.
    #[serde(rename = "folder.close")]
    FolderClose,
    /// Render the page's export job and write the file into the project's folder (#319): the
    /// helper renders the three streams of `commands::export::job` (`setup`, `start`, `stop`, each
    /// in standard base64) with `engine::offline`, as the page's export Worker does, and writes
    /// the WAV file as `exports/<name>.wav` in the folder it has open for `project` (AU-7's
    /// confined write; an existing file is kept and the first free `<name> (2).wav`... is taken).
    /// `id` is the page's number for this run: every answer carries it (`export.progress`, then
    /// one `export.done` or `export.failed`). One export at a time. Only on a session.
    #[serde(rename = "export.start")]
    ExportStart {
        id: u32,
        project: String,
        name: String,
        sample_rate: u32,
        /// 16, 24 or 32 (float).
        bits: u32,
        body_frames: u64,
        setup: String,
        start: String,
        stop: String,
    },
    /// Stop the export `id`: nothing is written (answered `export.failed` `export.cancelled`).
    /// Only on a session.
    #[serde(rename = "export.cancel")]
    ExportCancel { id: u32 },
    /// Show the helper's own file dialog for audio files to import (#778): one dialog of the
    /// helper at a time (the folder dialog included). Answered by `import.chosen` with this `id`
    /// (no file when the user closed it), or `import.failed`. The page never learns the files'
    /// paths: each comes with a token for `import.start`. Only on a session.
    #[serde(rename = "import.choose")]
    ImportChoose { id: u32 },
    /// Import an audio file into the project's folder (#778): the helper decodes it with the
    /// page's decoder (`crates/audio-import`, natively) and writes its blob and caches through
    /// AU-7's confined write, in the folder it has open for `project`; the page then makes the
    /// clip. `id` is the page's number for this import: every answer carries it
    /// (`import.progress`, then one `import.done` or `import.failed`). One import at a time.
    /// Only on a session.
    #[serde(rename = "import.start")]
    ImportStart {
        id: u32,
        project: String,
        source: ImportSource,
    },
    /// Stop the import `id` (answered `import.failed` `import.cancelled`, or `import.done` when
    /// its blob was already in place). Only on a session.
    #[serde(rename = "import.cancel")]
    ImportCancel { id: u32 },
}

/// The file an `import.start` imports (#778).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImportSource {
    /// A file of the project's folder: its path relative to the folder's root, `/` separated
    /// (a file the user or an agent put there, `blobs/<sha256>` included). Never a path outside
    /// the folder, never `.faulk/` or `.git*`.
    Folder { path: String },
    /// A file the user picked in the helper's file dialog: the token `import.chosen` gave it
    /// (single use, this session's only, for [`IMPORT_TOKEN_SECS`]).
    Picked { token: String },
}

/// A file the user picked in the helper's dialog (`import.chosen`, #778).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PickedFile {
    /// 32 lowercase hex digits: what `import.start` names the file by.
    pub token: String,
    /// The file's own name (no folder).
    pub name: String,
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
    /// The helper reads the audio files of the project's folder for its engine (AU-7): it takes
    /// `folder.open`, `folder.choose` and `folder.close`.
    pub disk: bool,
    /// The helper renders an export and writes it into the project's folder (#319): it takes
    /// `export.start` and `export.cancel`.
    pub export: bool,
    /// The helper imports an audio file into the project's folder (#778): it takes
    /// `import.choose`, `import.start` and `import.cancel`.
    pub import: bool,
}

/// Where the helper's folder of the page's project stands (`folder.state`, AU-7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum FolderStatus {
    /// The helper reads the project's audio files from this folder (its name).
    Open { name: String },
    /// No folder: after `folder.close`, or a project the helper does not take.
    Closed,
    /// The helper knows no folder holding the project and the page's marker: the user shows it
    /// (`folder.choose`).
    Unknown,
    /// Several folders the helper knows hold them (a copy made since the marker was written):
    /// the user shows the right one.
    Ambiguous { count: u32 },
    /// The helper's folder dialog is open.
    Choosing,
    /// The folder chosen was not taken.
    Refused { reason: FolderRefusal },
}

/// Why the folder chosen in the helper's dialog was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FolderRefusal {
    /// The user closed the dialog.
    Cancelled,
    /// The folder holds no project.
    NoProject,
    /// The folder holds another project.
    OtherProject,
    /// The folder holds this project but not the page's marker: a copy, not the folder open in
    /// the page.
    NotThisFolder,
    /// The helper could not read the folder.
    Unreadable,
    /// This system has no folder dialog the helper can show.
    Unavailable,
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
    /// First message of a control link (#658): the helper's protocol, as `hello` carries it for
    /// a session. The page closes the link when it differs from its own.
    #[serde(rename = "control.hello")]
    ControlHello { protocol: u32 },
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
    /// Where the helper's folder of the page's project stands (AU-7): the answer to `folder.open`,
    /// `folder.choose` (`choosing` first, then the outcome) and `folder.close`. `project` is the
    /// project it is about (`None` after `folder.close`): the page drops a state of another one.
    #[serde(rename = "folder.state")]
    FolderState {
        project: Option<String>,
        state: FolderStatus,
    },
    /// How far the export `id` is (#319), at most every [`EXPORT_PROGRESS_MS`]: frames of the
    /// content rendered, of `body`, and whether the content is done and the effects ring out.
    #[serde(rename = "export.progress")]
    ExportProgress {
        id: u32,
        rendered: u64,
        body: u64,
        tail: bool,
    },
    /// The export `id` is written (#319): `path` is the file's path in the project's folder
    /// (`/` separated, `exports/<name>.wav` or the free name taken), `bytes` its size, `written`
    /// its frames; the figures are `engine::offline::Offline`'s, as the page's Worker reports
    /// them; `missing` the media (hex SHA-256) whose file could not be read (silent clips).
    #[serde(rename = "export.done")]
    ExportDone {
        id: u32,
        path: String,
        bytes: u64,
        written: u64,
        peak: f32,
        clipped: u64,
        reduction: f32,
        silenced: u64,
        tail_cut: bool,
        missing: Vec<String>,
    },
    /// The export `id` ended without a file (#319): `code` is one of [`errors`]' `EXPORT_*`,
    /// `message` is for developers. Nothing is left in the folder, not even a temporary file.
    #[serde(rename = "export.failed")]
    ExportFailed {
        id: u32,
        code: String,
        message: String,
    },
    /// The files the user picked in the helper's dialog for `import.choose` `id` (#778): none when
    /// the dialog was closed.
    #[serde(rename = "import.chosen")]
    ImportChosen { id: u32, files: Vec<PickedFile> },
    /// How far the import `id` is (#778), from 0 to 1, at most every [`IMPORT_PROGRESS_MS`].
    #[serde(rename = "import.progress")]
    ImportProgress { id: u32, fraction: f64 },
    /// The import `id` is in the project's folder (#778): the media as `clip.import_audio` takes it
    /// (`hash`, the file's SHA-256 and the name of its blob; the decoded `sample_rate`, `channels`,
    /// `frames`), `name` the source file's own name (the page makes the media's name of it),
    /// `direct` whether the blob is read directly (no PCM cache), `created` whether this import
    /// wrote `blobs/<hash>` (the folder may hold it already), `peaks` whether its waveform peaks
    /// were written.
    #[serde(rename = "import.done")]
    ImportDone {
        id: u32,
        hash: String,
        sample_rate: u32,
        channels: u16,
        frames: u64,
        name: String,
        direct: bool,
        created: bool,
        peaks: bool,
    },
    /// The import (or the dialog) `id` ended without a file in the folder (#778): `code` is one of
    /// [`errors`]' `IMPORT_*`, or `import.<reason>` for a file the decoder refuses
    /// (`audio_import::ImportError::code`: `import.too_large`, `import.corrupt`...); `message` is
    /// for developers. Nothing is left in the folder, not even a temporary file.
    #[serde(rename = "import.failed")]
    ImportFailed {
        id: u32,
        code: String,
        message: String,
    },
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
        "max_export_stream_bytes": MAX_EXPORT_STREAM_BYTES,
        "export_progress_ms": EXPORT_PROGRESS_MS,
        "export_name": {
            "max_bytes": EXPORT_NAME_MAX_BYTES,
            "devices": WINDOWS_DEVICE_NAMES,
        },
        "export_error_codes": {
            "busy": errors::EXPORT_BUSY,
            "no_folder": errors::EXPORT_NO_FOLDER,
            "invalid": errors::EXPORT_INVALID,
            "render": errors::EXPORT_RENDER,
            "write": errors::EXPORT_WRITE,
            "folder_changed": errors::EXPORT_FOLDER_CHANGED,
            "cancelled": errors::EXPORT_CANCELLED,
        },
        "import_progress_ms": IMPORT_PROGRESS_MS,
        "import_error_codes": {
            "busy": errors::IMPORT_BUSY,
            "no_folder": errors::IMPORT_NO_FOLDER,
            "invalid": errors::IMPORT_INVALID,
            "path": errors::IMPORT_PATH,
            "unreadable": errors::IMPORT_UNREADABLE,
            "write": errors::IMPORT_WRITE,
            "folder_changed": errors::IMPORT_FOLDER_CHANGED,
            "cancelled": errors::IMPORT_CANCELLED,
            "dialog_unavailable": errors::IMPORT_DIALOG_UNAVAILABLE,
            "decode_prefix": errors::IMPORT_DECODE_PREFIX,
        },
        "max_stream_bytes": MAX_STREAM_BYTES,
        "control_pushed_out": CONTROL_PUSHED_OUT,
        "agent_confirm_ms": AGENT_CONFIRM_MS,
        "max_audio_url_bytes": commands::clip::MAX_AUDIO_URL_BYTES,
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
        // The scales of the project (#695): each one's degrees in semitones above its root,
        // in the order the page lists them (a list: a map would sort them by name). The
        // piano roll and the scale picker read them.
        "scale_intervals": project::ScaleMode::ALL
            .iter()
            .map(|m| serde_json::json!({ "mode": m.name(), "intervals": m.intervals() }))
            .collect::<Vec<_>>(),
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
