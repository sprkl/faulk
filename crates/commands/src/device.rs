//! `device.*`: a track's device chain: insert, remove, move, enable, set a parameter
//! (epic 4, DV). PR-2 migrates `synth.set_param` of v1 as `device.set_param`; DV-1 adds
//! the other commands, the device descriptors, and the range and step checks.
//!
//! Also here: the parameters of the built-in synth ([`SynthParam`]), the only device
//! until DV-1, with their engine ids and defaults exported to the web app.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::macros::domain;
use engine::{Param, Synth};
use project::DeviceId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

domain! {
    domain = "device", schema = ("DeviceCommand", "DeviceOutput");

    /// Set a parameter of a device, by the parameter's id, in the unit the device
    /// declares. The parameter must be one the device has and the value must be finite;
    /// the engine clamps it to the parameter's range. The built-in synth's parameters:
    /// `waveform` (0 sine, 1 saw, 2 square, 3 triangle), `attack`, `decay` and
    /// `release` (seconds, 0.002 to 30), `sustain` (0 to 1), `gain` (0 to 1, linear).
    /// Undo: set the parameter back to its previous value.
    "device.set_param" (kind = "edit", undo = "inverse", destructive = false)
    SetParam {
        device: DeviceId,
        param: String,
        value: f64,
    } => {
        /// The value before the command.
        previous: f64,
    };
}

/// Applies a command of this domain.
pub fn apply(
    command: &Command,
    state: &mut State,
    _ctx: &mut Context,
) -> Result<Outcome<Command, Output>, CommandError> {
    match command {
        Command::SetParam {
            device,
            param,
            value,
        } => {
            if !value.is_finite() {
                return Err(CommandError::Invalid {
                    field: "value".into(),
                    message: format!("a parameter value is finite, not {value}"),
                });
            }
            let found = state
                .project
                .tracks
                .values_mut()
                .find_map(|t| t.devices.get_mut(device))
                .ok_or_else(|| CommandError::NotFound {
                    entity: "device".into(),
                    id: device.to_string(),
                })?;
            let slot = found
                .params
                .get_mut(param)
                .ok_or_else(|| CommandError::NotFound {
                    entity: "param".into(),
                    id: param.clone(),
                })?;
            let previous = *slot;
            if previous == *value {
                // Nothing changed: nothing to undo, nothing to tell the interface.
                return Ok(Outcome {
                    output: Output::SetParam { previous },
                    undo: Undo::None,
                    events: vec![],
                    control: vec![],
                });
            }
            *slot = *value;
            Ok(Outcome {
                output: Output::SetParam { previous },
                undo: Undo::Inverse(Box::new(Command::SetParam {
                    device: device.clone(),
                    param: param.clone(),
                    value: previous,
                })),
                events: vec![ProjectEvent::DeviceParamChanged {
                    device: device.clone(),
                    param: param.clone(),
                    value: *value,
                }],
                control: vec![],
            })
        }
    }
}

/// The built-in synth's type id.
pub const SYNTH_KIND: &str = "faulk.osc_synth";

/// A parameter of the built-in synth, with its unit and range. Its serde name is the
/// parameter id that `device.set_param` takes.
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

    /// The parameter id, as `device.set_param` takes it.
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

    /// The parameter with this id, if the synth has one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
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
