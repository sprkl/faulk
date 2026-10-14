//! Every Faulk user action as a typed command.
//!
//! The UI, the MCP tools and the ACP agents all act through [`Command`]: the UI has
//! no private path to the engine. Each variant's doc comment is its description for
//! agents, exported in the JSON Schema (see the `export-schema` binary).
//!
//! This root is the v1 contract, applied straight to the synth; the helper, the
//! worklet and the web app still use it. The v2 contract (CT-4), applied to the
//! project, lives in [`v2`]; PR-2 promotes it here and migrates these commands.

pub mod v2;

use engine::{Param, Synth};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A Faulk command, tagged by its `type` field (e.g. `{"type": "note.on", ...}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Command {
    /// List the MIDI inputs available to the engine. The answer is a `midi.inputs` event.
    #[serde(rename = "midi.list_inputs")]
    #[schemars(extend("x-undo" = "none"))]
    MidiListInputs,

    /// Choose the MIDI input that plays the synth. `id` is an id from `midi.inputs`;
    /// `null` disconnects MIDI input.
    #[serde(rename = "midi.select_input")]
    #[schemars(extend("x-undo" = "host"))]
    MidiSelectInput { id: Option<String> },

    /// Set a synth parameter. Values outside the parameter's range are clamped.
    /// Undo: set the parameter back to its previous value.
    #[serde(rename = "synth.set_param")]
    #[schemars(extend("x-undo" = "inverse"))]
    SynthSetParam { param: SynthParam, value: f32 },

    /// Start a note on the synth. `note` is a MIDI note number (60 = middle C),
    /// `velocity` 1 to 127 (0 acts as a note off). Not undoable: it changes no project data.
    #[serde(rename = "note.on")]
    #[schemars(extend("x-undo" = "none"))]
    NoteOn { note: u8, velocity: u8 },

    /// Release a note started with `note.on`. Not undoable: it changes no project data.
    #[serde(rename = "note.off")]
    #[schemars(extend("x-undo" = "none"))]
    NoteOff { note: u8 },

    /// Release every playing note (panic button). Not undoable: it changes no project data.
    #[serde(rename = "note.all_off")]
    #[schemars(extend("x-undo" = "none"))]
    NoteAllOff,
}

/// A synth parameter, with its unit and range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SynthParam {
    /// Oscillator waveform: 0 sine, 1 saw, 2 square, 3 triangle.
    Waveform,
    /// Attack time in seconds, 0.002 to 30.
    Attack,
    /// Decay time in seconds, 0.002 to 30.
    Decay,
    /// Sustain level, 0 to 1.
    Sustain,
    /// Release time in seconds, 0.002 to 30.
    Release,
    /// Master gain, 0 to 1 (linear).
    Gain,
}

impl SynthParam {
    /// Every parameter, in a stable order.
    pub const ALL: [SynthParam; 6] = [
        Self::Waveform,
        Self::Attack,
        Self::Decay,
        Self::Sustain,
        Self::Release,
        Self::Gain,
    ];

    pub fn to_engine(self) -> Param {
        match self {
            Self::Waveform => Param::Waveform,
            Self::Attack => Param::Attack,
            Self::Decay => Param::Decay,
            Self::Sustain => Param::Sustain,
            Self::Release => Param::Release,
            Self::Gain => Param::Gain,
        }
    }
}

/// What applying a command to a synth did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Applied {
    /// The synth changed; `undo` restores it, if the command is undoable.
    Done { undo: Option<UndoAction> },
    /// The command does not act on the synth (MIDI device management): the host handles it.
    NotForSynth,
}

/// The inverse of an applied undoable command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UndoAction {
    SetParam { param: SynthParam, value: f32 },
}

impl UndoAction {
    pub fn into_command(self) -> Command {
        match self {
            Self::SetParam { param, value } => Command::SynthSetParam { param, value },
        }
    }
}

impl Command {
    /// The command's name, as in its `type` field.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MidiListInputs => "midi.list_inputs",
            Self::MidiSelectInput { .. } => "midi.select_input",
            Self::SynthSetParam { .. } => "synth.set_param",
            Self::NoteOn { .. } => "note.on",
            Self::NoteOff { .. } => "note.off",
            Self::NoteAllOff => "note.all_off",
        }
    }

    /// Applies the command to a synth. Allocation-free: safe on the audio thread.
    pub fn apply(&self, synth: &mut Synth) -> Applied {
        match *self {
            Self::SynthSetParam { param, value } => {
                let previous = synth.param(param.to_engine());
                synth.set_param(param.to_engine(), value);
                Applied::Done {
                    undo: Some(UndoAction::SetParam {
                        param,
                        value: previous,
                    }),
                }
            }
            Self::NoteOn { note, velocity } => {
                synth.note_on(note, velocity);
                Applied::Done { undo: None }
            }
            Self::NoteOff { note } => {
                synth.note_off(note);
                Applied::Done { undo: None }
            }
            Self::NoteAllOff => {
                synth.all_notes_off();
                Applied::Done { undo: None }
            }
            Self::MidiListInputs | Self::MidiSelectInput { .. } => Applied::NotForSynth,
        }
    }
}

/// How a command is undone, as declared in its schema (`x-undo`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoKind {
    /// [`Command::apply`] returns the inverse command.
    Inverse,
    /// The host undoes it (e.g. reselecting the previous MIDI input).
    Host,
    /// Not undoable: the command changes no project data.
    None,
}

impl Command {
    pub fn undo_kind(&self) -> UndoKind {
        match self {
            Self::SynthSetParam { .. } => UndoKind::Inverse,
            Self::MidiSelectInput { .. } => UndoKind::Host,
            Self::MidiListInputs
            | Self::NoteOn { .. }
            | Self::NoteOff { .. }
            | Self::NoteAllOff => UndoKind::None,
        }
    }
}

/// Stable numeric id of each synth parameter across the WebAssembly boundary,
/// exported so the web app never hard-codes them.
pub fn param_ids() -> Vec<(&'static str, u32)> {
    SynthParam::ALL
        .iter()
        .map(|p| (p.name(), p.to_engine().id()))
        .collect()
}

/// Default value of each synth parameter, read from a fresh engine, exported so the
/// web app starts from the engine's values.
pub fn param_defaults() -> Vec<(&'static str, f32)> {
    let synth = Synth::new(48_000.0);
    SynthParam::ALL
        .iter()
        .map(|p| (p.name(), synth.param(p.to_engine())))
        .collect()
}

impl SynthParam {
    pub fn name(self) -> &'static str {
        match self {
            Self::Waveform => "waveform",
            Self::Attack => "attack",
            Self::Decay => "decay",
            Self::Sustain => "sustain",
            Self::Release => "release",
            Self::Gain => "gain",
        }
    }
}

/// The JSON Schema of [`Command`], as exported to the web app and to agents.
pub fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Command)).unwrap_or_default()
}
