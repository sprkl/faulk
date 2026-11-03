//! Device presets (#191): a device's parameter values kept under a name, and the one file
//! format that holds them in the three places a preset lives: the project folder
//! (`presets/<id>.json`, it travels with the project), the user's library (the page's origin
//! storage, one file per preset) and the factory presets built into Faulk ([`factory`]).
//!
//! The file is canonical JSON, as the project's files (project model § 2.4): keys sorted,
//! one value per line, LF and a final newline, the shortest decimal that reads back the same
//! value, never `-0`:
//!
//! ```json
//! {
//!   "category": "Bass",
//!   "device": "faulk.osc_synth",
//!   "format": 1,
//!   "name": "Round Bass",
//!   "params": {
//!     "attack": 0.004
//!   },
//!   "version": 1
//! }
//! ```
//!
//! `format` is the version of this file format ([`PRESET_FORMAT`]); `device` and `version`
//! are the device kind and the version of the kind the values were made for; `params` holds
//! the values by the parameter's stable string id, in the unit the device declares;
//! `category` is absent when there is none. Writing what was read gives the same bytes.
//!
//! A preset is applied to a device through [`resolve`]: a preset made for a newer version of
//! the device is refused, an older one is migrated ([`MIGRATIONS`]) and the parameters it
//! lacks take their defaults, an unknown parameter is ignored and a value out of range is
//! clamped. Nothing here panics on any input.

pub mod factory;

use crate::file::canon;
use crate::ids::{Id, IdError, IdGenerator};
use crate::limits::MAX_NAME_CHARS;
use crate::serde_util::finite_map;
use engine::device::DeviceDescriptor;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The version of the preset file format this build writes. A file of a higher format is
/// refused as made by a newer Faulk.
pub const PRESET_FORMAT: u32 = 1;
/// The most bytes a preset file may have.
pub const MAX_PRESET_BYTES: usize = 64 << 10;
/// The most parameter values a preset holds.
pub const MAX_PRESET_PARAMS: usize = 256;
/// The longest parameter id a preset holds, in bytes.
pub const MAX_PARAM_KEY_LEN: usize = 64;
/// The longest category, in characters.
pub const MAX_CATEGORY_CHARS: usize = 64;
/// The most presets a project holds.
pub const MAX_PROJECT_PRESETS: usize = 1024;
/// The most presets the user's library holds.
pub const MAX_LIBRARY_PRESETS: usize = 4096;
/// The longest device kind a preset names, in bytes.
pub const MAX_KIND_LEN: usize = 64;

/// Identifier of a preset of the project or of the user's library: a ULID like every other
/// identifier, generated when the preset is saved. Factory presets are named otherwise
/// ([`factory::Factory::id`]).
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct PresetId(pub Id);

impl PresetId {
    pub fn parse(s: &str) -> Result<Self, IdError> {
        Id::parse(s).map(Self)
    }

    /// A fresh identifier from the generator.
    pub fn generate(ids: &mut IdGenerator) -> Self {
        Self(ids.next_id())
    }
}

impl fmt::Display for PresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// A preset: the values of a device's parameters under a name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    /// The device kind the values are for, such as `faulk.osc_synth`.
    pub device: String,
    /// The version of that device kind the values were made with.
    pub version: u32,
    /// Its name: trimmed, not empty, at most 200 characters.
    pub name: String,
    /// A category to group it by (such as `Bass`), at most 64 characters; absent when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Parameter id -> value, in the unit the device declares.
    #[schemars(with = "BTreeMap<String, f64>")]
    #[serde(with = "finite_map")]
    pub params: BTreeMap<String, f64>,
}

/// Where a preset lives.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PresetLocation {
    /// Built into Faulk, read-only.
    Factory,
    /// In the project's folder: it travels with the project.
    Project,
    /// In the user's library: available in every project.
    Library,
}

impl PresetLocation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Factory => "factory",
            Self::Project => "project",
            Self::Library => "library",
        }
    }
}

/// The preset a device was last set from, kept on the device so that its panel names it
/// after a reload and on another computer (where a library preset may be absent).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DevicePreset {
    /// Where the preset lives: `factory`, `project` or `library`.
    pub location: PresetLocation,
    /// The preset's id in its location: a factory id (`faulk.osc_synth/round-bass`) or a
    /// ULID.
    pub id: String,
    /// The preset's name when the device was set from it.
    pub name: String,
}

impl DevicePreset {
    /// Whether the reference is well formed: an id of its location (a factory id
    /// `faulk.<kind>/<slug>`, else a preset id) and a name as [`check_name`] takes it.
    pub fn is_valid(&self) -> bool {
        let id_ok = match self.location {
            PresetLocation::Factory => is_factory_id(&self.id),
            PresetLocation::Project | PresetLocation::Library => PresetId::parse(&self.id).is_ok(),
        };
        id_ok && check_name(&self.name).is_ok_and(|n| n == self.name)
    }
}

/// Whether `id` has the shape of a factory preset's id: `<kind>/<slug>`, the kind a device
/// kind (`faulk.` then lower-case letters, digits or `_`), the slug lower-case letters, digits
/// and `-`, at most 128 bytes in all.
pub fn is_factory_id(id: &str) -> bool {
    let Some((kind, slug)) = id.split_once('/') else {
        return false;
    };
    let kind_ok = kind.strip_prefix("faulk.").is_some_and(|k| {
        !k.is_empty()
            && k.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    });
    let slug_ok = !slug.is_empty()
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    id.len() <= 128 && kind_ok && slug_ok
}

/// Why a preset file, or a preset, is not one Faulk reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetError {
    /// The file has more than [`MAX_PRESET_BYTES`].
    TooLarge { bytes: usize },
    /// The file is not a preset (not JSON, a field missing or unknown, a number JSON cannot
    /// hold, a value of the wrong type).
    Corrupt { message: String },
    /// The file format is newer than this build's.
    NewerFormat { format: u32 },
    /// A field breaks a rule of the format (name, category, parameters).
    Invalid {
        field: &'static str,
        message: String,
    },
    /// The preset is for another device kind.
    OtherDevice { preset: String, device: String },
    /// The preset was made by a newer version of the device than this build has.
    NewerVersion {
        device: String,
        preset: u32,
        current: u32,
    },
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "a preset file has at most {MAX_PRESET_BYTES} bytes, this one {bytes}"
            ),
            Self::Corrupt { message } => write!(f, "not a preset file: {message}"),
            Self::NewerFormat { format } => write!(
                f,
                "the preset file is in format {format}, made by a newer Faulk (this one reads up to {PRESET_FORMAT})"
            ),
            Self::Invalid { field, message } => write!(f, "{field}: {message}"),
            Self::OtherDevice { preset, device } => {
                write!(f, "the preset is for {preset}, not for {device}")
            }
            Self::NewerVersion {
                device,
                preset,
                current,
            } => write!(
                f,
                "the preset was made with version {preset} of {device}, newer than this Faulk's version {current}: update Faulk to load it"
            ),
        }
    }
}

impl std::error::Error for PresetError {}

/// A preset name as the format takes it: trimmed, not empty, at most 200 characters.
pub fn check_name(name: &str) -> Result<&str, PresetError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(PresetError::Invalid {
            field: "name",
            message: "a preset name is not empty".into(),
        });
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(PresetError::Invalid {
            field: "name",
            message: format!("a preset name has at most {MAX_NAME_CHARS} characters"),
        });
    }
    Ok(name)
}

/// A category as the format takes it: trimmed, not empty, at most 64 characters.
pub fn check_category(category: &str) -> Result<&str, PresetError> {
    let category = category.trim();
    if category.is_empty() {
        return Err(PresetError::Invalid {
            field: "category",
            message: "a category is not empty (leave it out for none)".into(),
        });
    }
    if category.chars().count() > MAX_CATEGORY_CHARS {
        return Err(PresetError::Invalid {
            field: "category",
            message: format!("a category has at most {MAX_CATEGORY_CHARS} characters"),
        });
    }
    Ok(category)
}

impl Preset {
    /// Checks the rules of the format: a device kind, a name and a category as
    /// [`check_name`] and [`check_category`] take them (already trimmed), at most
    /// [`MAX_PRESET_PARAMS`] finite values under ids of at most [`MAX_PARAM_KEY_LEN`] bytes.
    pub fn check(&self) -> Result<(), PresetError> {
        let invalid = |field, message: String| PresetError::Invalid { field, message };
        if self.device.is_empty() || self.device.len() > MAX_KIND_LEN {
            return Err(invalid(
                "device",
                format!("a device kind has 1 to {MAX_KIND_LEN} bytes"),
            ));
        }
        if check_name(&self.name)? != self.name {
            return Err(invalid(
                "name",
                "a preset name has no space around it".into(),
            ));
        }
        if let Some(category) = &self.category
            && check_category(category)? != category
        {
            return Err(invalid(
                "category",
                "a category has no space around it".into(),
            ));
        }
        if self.params.len() > MAX_PRESET_PARAMS {
            return Err(invalid(
                "params",
                format!("a preset holds at most {MAX_PRESET_PARAMS} values"),
            ));
        }
        for (key, value) in &self.params {
            if key.is_empty() || key.len() > MAX_PARAM_KEY_LEN {
                return Err(invalid(
                    "params",
                    format!("a parameter id has 1 to {MAX_PARAM_KEY_LEN} bytes"),
                ));
            }
            if !value.is_finite() {
                return Err(invalid(
                    "params",
                    format!("the value of {key} is not a finite number"),
                ));
            }
        }
        Ok(())
    }

    /// The preset's file: canonical JSON (see the module docs). Fails only for a preset that
    /// [`Preset::check`] refuses.
    pub fn to_file(&self) -> Result<Vec<u8>, PresetError> {
        self.check()?;
        canon::json(&PresetFile::of(self)).map_err(|message| PresetError::Invalid {
            field: "params",
            message,
        })
    }

    /// Reads a preset file: at most [`MAX_PRESET_BYTES`], of a format this build reads, every
    /// rule of [`Preset::check`] kept. Never panics.
    pub fn parse(bytes: &[u8]) -> Result<Preset, PresetError> {
        if bytes.len() > MAX_PRESET_BYTES {
            return Err(PresetError::TooLarge { bytes: bytes.len() });
        }
        // The format first, so that a newer file is told apart from a corrupt one even when
        // its other fields changed.
        #[derive(Deserialize)]
        struct Probe {
            format: u32,
        }
        let corrupt = |e: serde_json::Error| PresetError::Corrupt {
            message: e.to_string(),
        };
        let probe: Probe = serde_json::from_slice(bytes).map_err(corrupt)?;
        if probe.format > PRESET_FORMAT {
            return Err(PresetError::NewerFormat {
                format: probe.format,
            });
        }
        if probe.format == 0 {
            return Err(PresetError::Corrupt {
                message: "format 0 does not exist".into(),
            });
        }
        let preset = serde_json::from_slice::<PresetFile>(bytes)
            .map_err(corrupt)?
            .into_preset();
        preset.check()?;
        Ok(preset)
    }
}

/// The file's shape: the preset's fields and the format number, written flat (serde's
/// `flatten` would not refuse an unknown field).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetFile {
    format: u32,
    device: String,
    version: u32,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(with = "finite_map")]
    params: BTreeMap<String, f64>,
}

impl PresetFile {
    fn of(preset: &Preset) -> Self {
        Self {
            format: PRESET_FORMAT,
            device: preset.device.clone(),
            version: preset.version,
            name: preset.name.clone(),
            category: preset.category.clone(),
            params: preset.params.clone(),
        }
    }

    fn into_preset(self) -> Preset {
        Preset {
            device: self.device,
            version: self.version,
            name: self.name,
            category: self.category,
            params: self.params,
        }
    }
}

/// One step of a device's preset migrations: the values of version `from` become those of
/// `from + 1`. A device raises its version when its parameters change meaning, and adds the
/// step here at the same time.
#[derive(Clone, Copy)]
pub struct Migration {
    pub device: &'static str,
    pub from: u32,
    pub apply: fn(&mut BTreeMap<String, f64>),
}

/// The migrations of this build. Empty: every built-in device is at its first version.
pub const MIGRATIONS: &[Migration] = &[];

/// What applying a preset to a device gives.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// A value for every parameter the device declares, clamped and rounded as the device
    /// takes it.
    pub params: BTreeMap<String, f64>,
    /// The values the device has no parameter for: ignored.
    pub ignored: Vec<String>,
    /// The values that were out of range or between steps: clamped or rounded.
    pub clamped: Vec<String>,
    /// The parameters the preset had no value for: at their default.
    pub defaulted: Vec<String>,
    /// The device version the preset was migrated from, when it was older.
    pub migrated_from: Option<u32>,
}

/// The values `preset` gives a device of `descriptor`, with [`MIGRATIONS`].
pub fn resolve(preset: &Preset, descriptor: &DeviceDescriptor) -> Result<Resolved, PresetError> {
    resolve_with(preset, descriptor, MIGRATIONS)
}

/// [`resolve`] with a given table of migrations (the tests use their own).
pub fn resolve_with(
    preset: &Preset,
    descriptor: &DeviceDescriptor,
    migrations: &[Migration],
) -> Result<Resolved, PresetError> {
    if preset.device != descriptor.kind {
        return Err(PresetError::OtherDevice {
            preset: preset.device.clone(),
            device: descriptor.kind.to_owned(),
        });
    }
    if preset.version > descriptor.version {
        return Err(PresetError::NewerVersion {
            device: descriptor.name.to_owned(),
            preset: preset.version,
            current: descriptor.version,
        });
    }
    let mut values = preset.params.clone();
    let mut version = preset.version;
    while version < descriptor.version {
        // A version with no step keeps its values: the parameters it lacks take their
        // defaults below.
        if let Some(step) = migrations
            .iter()
            .find(|m| m.device == descriptor.kind && m.from == version)
        {
            (step.apply)(&mut values);
        }
        version += 1;
    }
    let mut resolved = Resolved {
        params: BTreeMap::new(),
        ignored: Vec::new(),
        clamped: Vec::new(),
        defaulted: Vec::new(),
        migrated_from: (preset.version < descriptor.version).then_some(preset.version),
    };
    for param in descriptor.params {
        let value = match values.get(param.key) {
            Some(&value) => match param.sanitize_f64(value) {
                Some(clean) => {
                    if clean != value {
                        resolved.clamped.push(param.key.to_owned());
                    }
                    clean
                }
                // A migration that made a value NaN: the default, never a panic.
                None => {
                    resolved.clamped.push(param.key.to_owned());
                    param.default
                }
            },
            None => {
                resolved.defaulted.push(param.key.to_owned());
                param.default
            }
        };
        resolved.params.insert(param.key.to_owned(), value);
    }
    resolved.ignored = values
        .keys()
        .filter(|k| !descriptor.params.iter().any(|p| p.key == k.as_str()))
        .cloned()
        .collect();
    Ok(resolved)
}

#[cfg(test)]
mod tests;
