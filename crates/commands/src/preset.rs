//! `preset.*`: device presets (#191): keep a device's settings under a name, apply them again
//! in one gesture, and the factory presets built into Faulk.
//!
//! The contract, written once here and repeated in the descriptions for agents:
//!
//! - A preset lives in one of three places, its `location`: `factory` (built into Faulk,
//!   read-only), `project` (saved in the project's folder, it travels with the project) or
//!   `library` (the user's own library, available in every project). A preset is named by a
//!   [`PresetRef`]: its location and its `id` there. A factory id is `<kind>/<slug>`
//!   (`faulk.osc_synth/round-bass`); a project or library id is a ULID made when the preset is
//!   saved. `preset.list` gives them.
//! - A preset holds the values of a built-in device's parameters, with the device kind and
//!   the version of the kind they were made for. Loading one applies every value at once; the
//!   parameters it has no value for take their defaults, a value the device has no parameter
//!   for is ignored and one out of range is clamped. A preset made for a newer version of the
//!   device than this Faulk has is refused; an older one is migrated.
//! - A device remembers the preset it was last set from (by a load or a save), with the
//!   preset's name then: the panel shows it, and the list says whether the device's values
//!   still match it (`modified`). Renaming or deleting a preset leaves the devices as they are.
//! - Within a location, two presets of the same device kind never share a name (compared
//!   without case or surrounding spaces): `preset.save`, `preset.copy` and `preset.rename`
//!   refuse a name in use.
//! - The library is the page's: the page gives the core what its storage holds and writes the
//!   files the commands change; the project's presets are files of the project's folder,
//!   written with the rest of the project.
//! - Every edit has an inverse. `preset.restore` and `preset.restore_device` exist as the
//!   inverses of the others, and are commands like the rest.

use crate::contract::{CommandError, Context, Outcome, ProjectEvent, State, Undo};
use crate::device::{descriptor_of, registry};
use crate::macros::domain;
use engine::device::DeviceDescriptor;
use engine::devices::Builtin;
use project::preset::{
    self, DevicePreset, MAX_LIBRARY_PRESETS, MAX_PROJECT_PRESETS, Preset, PresetError, PresetId,
    PresetLocation, factory,
};
use project::{Device, DeviceId, DeviceSource};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A preset, by where it lives and its id there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresetRef {
    /// Where it lives: `factory`, `project` or `library`.
    pub location: PresetLocation,
    /// A factory id (`faulk.osc_synth/round-bass`) or, in the project or the library, the
    /// preset's ULID.
    #[schemars(length(min = 1, max = 128))]
    pub id: String,
}

/// One preset as `preset.list` gives it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresetEntry {
    /// What `preset.load`, `preset.copy`, `preset.rename` and `preset.delete` take.
    pub preset: PresetRef,
    /// Its name.
    pub name: String,
    /// Its category, `null` when it has none.
    pub category: Option<String>,
    /// The device kind it is for.
    pub kind: String,
    /// The version of the device kind it was made with.
    pub version: u32,
    /// A factory preset: it cannot be renamed nor deleted.
    pub read_only: bool,
    /// Made with a newer version of the device than this Faulk has: it cannot be loaded.
    pub newer: bool,
    /// The values a load would give the device, by parameter key: every parameter of the
    /// device, migrated, clamped and completed with defaults. For a preset that cannot be
    /// loaded (`newer`, or a device kind this Faulk does not have), the values it holds.
    pub params: BTreeMap<String, f64>,
}

/// The preset a device shows, as `preset.restore` puts it back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceRestore {
    pub device: DeviceId,
    /// The preset the device shows; absent or `null` for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<Box<DevicePreset>>,
}

domain! {
    domain = "preset", schema = ("PresetCommand", "PresetOutput");

    /// Save a device's current settings as a new preset named `name` (trimmed, 1 to 200
    /// characters), with an optional `category` (1 to 64 characters), in the `project` (saved
    /// in the project's folder, it travels with the project) or in the user's `library`
    /// (available in every project). The device then shows this preset, not modified. Only a
    /// built-in device with parameters has presets (not the clip player, not a module).
    /// Refused with `invalid` for the `factory` location (read-only), a name already used by
    /// a preset of that device kind in that location, an empty or too long name or category,
    /// and with `limit` when the location is full (1 024 presets in a project, 4 096 in the
    /// library). Undo: `preset.restore` takes the preset out and gives the device its
    /// previous preset back.
    "preset.save" (kind = "edit", undo = "inverse", destructive = false)
    Save {
        device: DeviceId,
        name: String,
        /// A category to group it by, such as `Bass`; leave out for none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<String>,
        /// `project` or `library`.
        location: PresetLocation,
    } => {
        /// The new preset.
        preset: PresetRef,
    };

    /// Load a preset into a device: every parameter of the device takes the preset's value
    /// at once (a parameter the preset has no value for takes its default, a value the device
    /// has no parameter for is ignored, a value out of range is clamped), and the device
    /// shows the preset. One undo takes the whole load back. Refused with `not_found` for an
    /// unknown device or preset, with `invalid` for a preset of another device kind, one made
    /// with a newer version of the device than this Faulk has (the message names both
    /// versions), and a device without presets (the clip player, a module). An older preset
    /// is migrated (`migrated_from`). Undo: `preset.restore_device` puts every previous value
    /// and the previous preset back.
    "preset.load" (kind = "edit", undo = "inverse", destructive = false)
    Load { device: DeviceId, preset: PresetRef } => {
        /// The values the device has no parameter for: ignored.
        ignored: Vec<String>,
        /// The values that were out of range or between steps: clamped or rounded.
        clamped: Vec<String>,
        /// The parameters the preset had no value for: set to their default.
        defaulted: Vec<String>,
        /// The device version the preset was made with, when older than this Faulk's.
        migrated_from: Option<u32>,
    };

    /// Delete a preset of the project or of the library. A factory preset cannot be deleted
    /// (`invalid`); an unknown one is `not_found`. The devices that show it keep its name.
    /// Undo: `preset.restore` brings it back under the same id.
    "preset.delete" (kind = "edit", undo = "inverse", destructive = false)
    Delete { preset: PresetRef } => {};

    /// Rename a preset of the project or of the library (trimmed, 1 to 200 characters). The
    /// devices that show it keep the name they had until it is loaded again. Refused with
    /// `invalid` for a factory preset or a name used by another preset of that device kind in
    /// that location, `not_found` for an unknown one. Undo: `preset.restore` puts the preset
    /// back as it was.
    "preset.rename" (kind = "edit", undo = "inverse", destructive = false)
    Rename { preset: PresetRef, name: String } => {
        /// The name before the command.
        previous: String,
    };

    /// Copy a preset (from any location, factory included) into the `project` or the
    /// `library`, as a new preset with the same values, under its own name or `name` when
    /// given. The devices are left as they are: this is how a preset of a project goes into
    /// the library for every project. Refused like `preset.save` (factory location, a name in
    /// use there, a full location), and with `not_found` for an unknown preset. Undo:
    /// `preset.restore` takes the copy out.
    "preset.copy" (kind = "edit", undo = "inverse", destructive = false)
    Copy {
        preset: PresetRef,
        /// `project` or `library`.
        location: PresetLocation,
        /// The copy's name; leave out to keep the preset's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    } => {
        /// The copy.
        preset: PresetRef,
    };

    /// List presets: those of the device kind of `device` (a device's id) or of `kind` (a
    /// built-in kind such as `faulk.filter`), or every preset when both are left out (giving
    /// both is `invalid`). Factory presets come first, then the project's, then the
    /// library's, each by name. Each entry has the preset's reference, name, category, device
    /// kind and version, whether it is read-only (factory) or made with a newer version of
    /// the device (`newer`: it cannot be loaded), and the values a load would give the device.
    /// With `device`, the answer also names the preset the device shows (`current`) and
    /// whether its values differ from that preset's (`modified`; `null` when the device shows
    /// no preset or its preset is not here, such as a library preset of another computer). A
    /// query: changes nothing.
    "preset.list" (kind = "query", undo = "none", destructive = false)
    List {
        /// A device whose presets to list; leave out to give `kind` or to list all.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<DeviceId>,
        /// A device kind whose presets to list; leave out to give `device` or to list all.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
    } => {
        /// The presets, factory then project then library, each by name.
        presets: Vec<PresetEntry>,
        /// The preset `device` shows; `null` without `device` or when it shows none.
        current: Option<Box<DevicePreset>>,
        /// Whether `device`'s values differ from its preset's; `null` when unknown.
        modified: Option<bool>,
    };

    /// Put a preset of the project or of the library back as it was: `snapshot` is the whole
    /// preset (absent or `null` to take it out), under `preset`'s id. With `device`, that
    /// device shows `device.preset` again. The inverse of `preset.save`, `preset.delete`,
    /// `preset.rename` and `preset.copy`; it does not check names against the other presets
    /// (it puts back exactly what was there). Refused with `invalid` for the factory
    /// location, an id that is not a preset id or that another part of the project uses, a
    /// snapshot that breaks the preset format, and with `limit` when the location is full;
    /// `not_found` for an unknown device. Undo: `preset.restore` with what was there before.
    "preset.restore" (kind = "edit", undo = "inverse", destructive = false)
    Restore {
        preset: PresetRef,
        /// The preset to put back; absent or `null` to take it out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot: Option<Preset>,
        /// A device whose shown preset to put back as well.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<DeviceRestore>,
    } => {};

    /// Set several parameters of a built-in device at once, by key (each clamped and rounded
    /// as `device.set_param` does), and the preset the device shows (`preset`, absent or
    /// `null` for none). The inverse of `preset.load`. Refused with `not_found` for an
    /// unknown device or parameter, `invalid` for a value that is not finite, a malformed
    /// `preset` and a device without presets. Undo: `preset.restore_device` with the previous
    /// values and preset.
    "preset.restore_device" (kind = "edit", undo = "inverse", destructive = false)
    RestoreDevice {
        device: DeviceId,
        params: BTreeMap<String, f64>,
        /// The preset the device shows; absent or `null` for none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preset: Option<Box<DevicePreset>>,
    } => {};
}

type Out = Outcome<Command, Output>;

/// Applies a command of this domain.
pub fn apply(command: &Command, state: &mut State, ctx: &mut Context) -> Result<Out, CommandError> {
    apply_in(registry(), command, state, ctx)
}

fn apply_in(
    registry: &[Builtin],
    command: &Command,
    state: &mut State,
    ctx: &mut Context,
) -> Result<Out, CommandError> {
    match command {
        Command::Save {
            device,
            name,
            category,
            location,
        } => save(
            registry,
            state,
            ctx,
            device,
            name,
            category.as_deref(),
            *location,
        ),
        Command::Load { device, preset } => load(registry, state, device, preset),
        Command::Delete { preset } => delete(state, preset),
        Command::Rename { preset, name } => rename(state, preset, name),
        Command::Copy {
            preset,
            location,
            name,
        } => copy(state, ctx, preset, *location, name.as_deref()),
        Command::List { device, kind } => list(registry, state, device.as_ref(), kind.as_deref()),
        Command::Restore {
            preset,
            snapshot,
            device,
        } => restore(state, preset, snapshot.as_ref(), device.as_ref()),
        Command::RestoreDevice {
            device,
            params,
            preset,
        } => restore_device(registry, state, device, params, preset.as_deref()),
    }
}

fn not_found(entity: &str, id: impl ToString) -> CommandError {
    CommandError::NotFound {
        entity: entity.into(),
        id: id.to_string(),
    }
}

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

fn format_error(field: &str, error: &PresetError) -> CommandError {
    match error {
        PresetError::Invalid { field, message } => invalid(field, message.clone()),
        other => invalid(field, other.to_string()),
    }
}

fn unchanged(output: Output) -> Out {
    Outcome {
        output,
        undo: Undo::None,
        events: vec![],
        control: vec![],
    }
}

/// The presets of a writable location.
fn slots_mut(
    state: &mut State,
    location: PresetLocation,
) -> Result<&mut BTreeMap<PresetId, Preset>, CommandError> {
    match location {
        PresetLocation::Project => Ok(&mut state.project.presets),
        PresetLocation::Library => Ok(&mut state.library),
        PresetLocation::Factory => Err(read_only()),
    }
}

fn slots(state: &State, location: PresetLocation) -> Option<&BTreeMap<PresetId, Preset>> {
    match location {
        PresetLocation::Project => Some(&state.project.presets),
        PresetLocation::Library => Some(&state.library),
        PresetLocation::Factory => None,
    }
}

fn read_only() -> CommandError {
    invalid(
        "location",
        "factory presets are read-only: use `project` or `library`",
    )
}

fn capacity(location: PresetLocation) -> usize {
    match location {
        PresetLocation::Library => MAX_LIBRARY_PRESETS,
        _ => MAX_PROJECT_PRESETS,
    }
}

fn full(location: PresetLocation) -> CommandError {
    CommandError::Limit {
        entity: format!("{} presets", location.as_str()),
        max: capacity(location) as u64,
    }
}

/// The id of a project or library preset.
fn preset_id(preset: &PresetRef) -> Result<PresetId, CommandError> {
    if preset.location == PresetLocation::Factory {
        return Err(read_only());
    }
    PresetId::parse(&preset.id).map_err(|e| invalid("preset", format!("{}: {e}", preset.id)))
}

/// The preset a reference names, wherever it lives.
fn find<'s>(state: &'s State, preset: &PresetRef) -> Option<&'s Preset> {
    match preset.location {
        PresetLocation::Factory => factory::get(&preset.id).map(|f| &f.preset),
        location => {
            let id = PresetId::parse(&preset.id).ok()?;
            slots(state, location)?.get(&id)
        }
    }
}

/// The id of a new preset in `location`, refused with `limit` when the location is full: an id
/// that neither the location nor the project uses (the generator never repeats itself, but a
/// library staged by the host holds ids another page made).
fn fresh_id(
    state: &State,
    location: PresetLocation,
    ctx: &mut Context,
) -> Result<PresetId, CommandError> {
    let slots = slots(state, location).ok_or_else(read_only)?;
    if slots.len() >= capacity(location) {
        return Err(full(location));
    }
    loop {
        let id = PresetId::generate(&mut ctx.ids);
        if !slots.contains_key(&id) && !state.project.contains_id(&id.0) {
            return Ok(id);
        }
    }
}

fn changed(location: PresetLocation, id: &PresetId) -> ProjectEvent {
    ProjectEvent::PresetChanged {
        location,
        preset: id.to_string(),
    }
}

/// The name as the format takes it, and not used by another preset of `kind` in `location`
/// (`except` is the preset being renamed).
fn free_name<'n>(
    state: &State,
    location: PresetLocation,
    kind: &str,
    name: &'n str,
    except: Option<&PresetId>,
) -> Result<&'n str, CommandError> {
    let name = preset::check_name(name).map_err(|e| format_error("name", &e))?;
    let key = name.to_lowercase();
    let taken = slots(state, location).is_some_and(|slots| {
        slots.iter().any(|(id, p)| {
            Some(id) != except && p.device == kind && p.name.trim().to_lowercase() == key
        })
    });
    if taken {
        return Err(invalid(
            "name",
            format!(
                "a preset of {kind} named \"{name}\" is already in the {}: choose another name, or rename or delete that one first",
                location.as_str()
            ),
        ));
    }
    Ok(name)
}

/// A device that has presets: a built-in device of the registry, its descriptor.
fn with_presets<'r>(
    registry: &'r [Builtin],
    device_id: &DeviceId,
    device: &Device,
) -> Result<&'r DeviceDescriptor, CommandError> {
    descriptor_of(registry, device).ok_or_else(|| {
        let what = match &device.source {
            DeviceSource::Builtin { id, .. } => id.clone(),
            DeviceSource::Module { name, .. } => format!("the module {name}"),
        };
        invalid(
            "device",
            format!("device {device_id} ({what}) has no presets: only a built-in device with parameters has them"),
        )
    })
}

fn device<'s>(state: &'s State, id: &DeviceId) -> Result<&'s Device, CommandError> {
    state
        .project
        .device(id)
        .ok_or_else(|| not_found("device", id))
}

fn save(
    registry: &[Builtin],
    state: &mut State,
    ctx: &mut Context,
    device_id: &DeviceId,
    name: &str,
    category: Option<&str>,
    location: PresetLocation,
) -> Result<Out, CommandError> {
    let found = device(state, device_id)?;
    let descriptor = with_presets(registry, device_id, found)?;
    if location == PresetLocation::Factory {
        return Err(read_only());
    }
    let name = free_name(state, location, descriptor.kind, name, None)?.to_owned();
    let category = category
        .map(|c| preset::check_category(c).map(str::to_owned))
        .transpose()
        .map_err(|e| format_error("category", &e))?;
    let preset = Preset {
        device: descriptor.kind.to_owned(),
        version: descriptor.version,
        name: name.clone(),
        category,
        params: descriptor
            .params
            .iter()
            .map(|p| {
                let value = found.params.get(p.key).copied().unwrap_or(p.default);
                (p.key.to_owned(), value)
            })
            .collect(),
    };
    preset.check().map_err(|e| format_error("device", &e))?;
    let id = fresh_id(state, location, ctx)?;
    slots_mut(state, location)?.insert(id.clone(), preset);
    let shown = DevicePreset {
        location,
        id: id.to_string(),
        name,
    };
    let previous = state
        .project
        .device_mut(device_id)
        .map(|d| d.preset.replace(Box::new(shown)))
        .ok_or_else(|| not_found("device", device_id))?;
    let reference = PresetRef {
        location,
        id: id.to_string(),
    };
    Ok(Outcome {
        output: Output::Save {
            preset: reference.clone(),
        },
        undo: Undo::Inverse(Box::new(Command::Restore {
            preset: reference,
            snapshot: None,
            device: Some(DeviceRestore {
                device: device_id.clone(),
                preset: previous,
            }),
        })),
        events: vec![
            changed(location, &id),
            ProjectEvent::DevicePresetChanged {
                device: device_id.clone(),
            },
        ],
        control: vec![],
    })
}

fn load(
    registry: &[Builtin],
    state: &mut State,
    device_id: &DeviceId,
    reference: &PresetRef,
) -> Result<Out, CommandError> {
    let found = device(state, device_id)?;
    let descriptor = with_presets(registry, device_id, found)?;
    let preset = find(state, reference).ok_or_else(|| not_found("preset", &reference.id))?;
    let resolved =
        preset::resolve(preset, descriptor).map_err(|e| invalid("preset", e.to_string()))?;
    let shown = DevicePreset {
        location: reference.location,
        id: reference.id.clone(),
        name: preset.name.clone(),
    };
    let output = Output::Load {
        ignored: resolved.ignored.clone(),
        clamped: resolved.clamped.clone(),
        defaulted: resolved.defaulted.clone(),
        migrated_from: resolved.migrated_from,
    };
    let target = state
        .project
        .device_mut(device_id)
        .ok_or_else(|| not_found("device", device_id))?;
    let mut previous = BTreeMap::new();
    let mut events = Vec::new();
    for (key, value) in resolved.params {
        let slot = target.params.entry(key.clone()).or_insert(value);
        if *slot != value {
            previous.insert(key.clone(), *slot);
            *slot = value;
            events.push(ProjectEvent::DeviceParamChanged {
                device: device_id.clone(),
                param: key,
                value,
            });
        }
    }
    let previous_preset = target.preset.clone();
    if previous_preset.as_deref() != Some(&shown) {
        target.preset = Some(Box::new(shown));
        events.push(ProjectEvent::DevicePresetChanged {
            device: device_id.clone(),
        });
    }
    if events.is_empty() {
        return Ok(unchanged(output));
    }
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::RestoreDevice {
            device: device_id.clone(),
            params: previous,
            preset: previous_preset,
        })),
        events,
        control: vec![],
    })
}

fn delete(state: &mut State, reference: &PresetRef) -> Result<Out, CommandError> {
    let id = preset_id(reference)?;
    let removed = slots_mut(state, reference.location)?
        .remove(&id)
        .ok_or_else(|| not_found("preset", &reference.id))?;
    Ok(Outcome {
        output: Output::Delete {},
        undo: Undo::Inverse(Box::new(Command::Restore {
            preset: reference.clone(),
            snapshot: Some(removed),
            device: None,
        })),
        events: vec![changed(reference.location, &id)],
        control: vec![],
    })
}

fn rename(state: &mut State, reference: &PresetRef, name: &str) -> Result<Out, CommandError> {
    let id = preset_id(reference)?;
    let current = slots(state, reference.location)
        .and_then(|s| s.get(&id))
        .ok_or_else(|| not_found("preset", &reference.id))?;
    let kind = current.device.clone();
    let name = free_name(state, reference.location, &kind, name, Some(&id))?.to_owned();
    let slot = slots_mut(state, reference.location)?
        .get_mut(&id)
        .ok_or_else(|| not_found("preset", &reference.id))?;
    let before = slot.clone();
    let output = Output::Rename {
        previous: before.name.clone(),
    };
    if slot.name == name {
        return Ok(unchanged(output));
    }
    slot.name = name;
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::Restore {
            preset: reference.clone(),
            snapshot: Some(before),
            device: None,
        })),
        events: vec![changed(reference.location, &id)],
        control: vec![],
    })
}

fn copy(
    state: &mut State,
    ctx: &mut Context,
    reference: &PresetRef,
    location: PresetLocation,
    name: Option<&str>,
) -> Result<Out, CommandError> {
    if location == PresetLocation::Factory {
        return Err(read_only());
    }
    let mut preset = find(state, reference)
        .ok_or_else(|| not_found("preset", &reference.id))?
        .clone();
    let name = name.unwrap_or(&preset.name).to_owned();
    preset.name = free_name(state, location, &preset.device, &name, None)?.to_owned();
    let id = fresh_id(state, location, ctx)?;
    slots_mut(state, location)?.insert(id.clone(), preset);
    let copy = PresetRef {
        location,
        id: id.to_string(),
    };
    Ok(Outcome {
        output: Output::Copy {
            preset: copy.clone(),
        },
        undo: Undo::Inverse(Box::new(Command::Restore {
            preset: copy,
            snapshot: None,
            device: None,
        })),
        events: vec![changed(location, &id)],
        control: vec![],
    })
}

fn list(
    registry: &[Builtin],
    state: &State,
    device_id: Option<&DeviceId>,
    kind: Option<&str>,
) -> Result<Out, CommandError> {
    let (kind, shown) = match (device_id, kind) {
        (Some(_), Some(_)) => {
            return Err(invalid(
                "kind",
                "give `device` or `kind`, not both (or neither, for every preset)",
            ));
        }
        (Some(id), None) => {
            let found = device(state, id)?;
            let kind = match &found.source {
                DeviceSource::Builtin { id, .. } => Some(id.clone()),
                // A module has no presets: an empty list.
                DeviceSource::Module { .. } => Some(String::new()),
            };
            (kind, Some(found))
        }
        (None, kind) => (kind.map(str::to_owned), None),
    };
    let descriptor = |kind: &str| {
        registry
            .iter()
            .find(|b| b.descriptor.kind == kind)
            .map(|b| b.descriptor)
    };
    let entry = |reference: PresetRef, preset: &Preset| {
        let (params, newer) = match descriptor(&preset.device) {
            Some(d) => match preset::resolve(preset, d) {
                Ok(r) => (r.params, false),
                Err(_) => (preset.params.clone(), preset.version > d.version),
            },
            None => (preset.params.clone(), false),
        };
        PresetEntry {
            read_only: reference.location == PresetLocation::Factory,
            preset: reference,
            name: preset.name.clone(),
            category: preset.category.clone(),
            kind: preset.device.clone(),
            version: preset.version,
            newer,
            params,
        }
    };
    let wanted = |preset: &Preset| kind.as_deref().is_none_or(|k| preset.device == k);
    let mut presets: Vec<PresetEntry> = factory::all()
        .iter()
        .filter(|f| wanted(&f.preset))
        .map(|f| {
            entry(
                PresetRef {
                    location: PresetLocation::Factory,
                    id: f.id.to_owned(),
                },
                &f.preset,
            )
        })
        .collect();
    for (location, slots) in [
        (PresetLocation::Project, &state.project.presets),
        (PresetLocation::Library, &state.library),
    ] {
        presets.extend(slots.iter().filter(|(_, p)| wanted(p)).map(|(id, p)| {
            entry(
                PresetRef {
                    location,
                    id: id.to_string(),
                },
                p,
            )
        }));
    }
    presets.sort_by(|a, b| {
        (a.preset.location, a.name.to_lowercase(), &a.preset.id).cmp(&(
            b.preset.location,
            b.name.to_lowercase(),
            &b.preset.id,
        ))
    });
    let current = shown.and_then(|d| d.preset.clone());
    let modified = match (shown, &current) {
        (Some(d), Some(c)) => presets
            .iter()
            .find(|e| e.preset.location == c.location && e.preset.id == c.id && !e.newer)
            .map(|e| {
                e.params
                    .iter()
                    .any(|(k, v)| d.params.get(k).is_some_and(|now| now != v))
            }),
        _ => None,
    };
    Ok(crate::query::answer(Output::List {
        presets,
        current,
        modified,
    }))
}

fn restore(
    state: &mut State,
    reference: &PresetRef,
    snapshot: Option<&Preset>,
    device_restore: Option<&DeviceRestore>,
) -> Result<Out, CommandError> {
    let id = preset_id(reference)?;
    let location = reference.location;
    if let Some(snapshot) = snapshot {
        snapshot.check().map_err(|e| format_error("snapshot", &e))?;
    }
    if let Some(restore) = device_restore {
        device(state, &restore.device)?;
        if restore.preset.as_ref().is_some_and(|p| !p.is_valid()) {
            return Err(invalid(
                "device",
                "the preset a device shows names a location, an id of that location and a name",
            ));
        }
    }
    let present = slots(state, location).is_some_and(|s| s.contains_key(&id));
    if snapshot.is_some() && !present {
        if location == PresetLocation::Project && state.project.contains_id(&id.0) {
            return Err(invalid(
                "preset",
                format!("the identifier {id} is already in use"),
            ));
        }
        if slots(state, location).map_or(0, BTreeMap::len) >= capacity(location) {
            return Err(full(location));
        }
    }
    let slots = slots_mut(state, location)?;
    let before = match snapshot {
        Some(p) => slots.insert(id.clone(), p.clone()),
        None => slots.remove(&id),
    };
    let mut events = Vec::new();
    if before.as_ref() != snapshot {
        events.push(changed(location, &id));
    }
    let mut device_before = None;
    if let Some(restore) = device_restore
        && let Some(target) = state.project.device_mut(&restore.device)
    {
        let previous = std::mem::replace(&mut target.preset, restore.preset.clone());
        if previous != restore.preset {
            events.push(ProjectEvent::DevicePresetChanged {
                device: restore.device.clone(),
            });
        }
        device_before = Some(DeviceRestore {
            device: restore.device.clone(),
            preset: previous,
        });
    }
    if events.is_empty() {
        return Ok(unchanged(Output::Restore {}));
    }
    Ok(Outcome {
        output: Output::Restore {},
        undo: Undo::Inverse(Box::new(Command::Restore {
            preset: reference.clone(),
            snapshot: before,
            device: device_before,
        })),
        events,
        control: vec![],
    })
}

fn restore_device(
    registry: &[Builtin],
    state: &mut State,
    device_id: &DeviceId,
    params: &BTreeMap<String, f64>,
    preset: Option<&DevicePreset>,
) -> Result<Out, CommandError> {
    let found = device(state, device_id)?;
    let descriptor = with_presets(registry, device_id, found)?;
    if preset.is_some_and(|p| !p.is_valid()) {
        return Err(invalid(
            "preset",
            "the preset a device shows names a location, an id of that location and a name",
        ));
    }
    // Every value is checked before anything changes: a refusal leaves the device as it was.
    let mut values = BTreeMap::new();
    for (key, value) in params {
        let param = descriptor
            .params
            .iter()
            .find(|p| p.key == key)
            .ok_or_else(|| not_found("param", key))?;
        let clean = param
            .sanitize_f64(*value)
            .ok_or_else(|| invalid("params", format!("the value of {key} is not finite")))?;
        values.insert(key.clone(), clean);
    }
    let target = state
        .project
        .device_mut(device_id)
        .ok_or_else(|| not_found("device", device_id))?;
    let mut previous = BTreeMap::new();
    let mut events = Vec::new();
    for (key, value) in values {
        let slot = target.params.entry(key.clone()).or_insert(value);
        if *slot != value {
            previous.insert(key.clone(), *slot);
            *slot = value;
            events.push(ProjectEvent::DeviceParamChanged {
                device: device_id.clone(),
                param: key,
                value,
            });
        }
    }
    let previous_preset = std::mem::replace(&mut target.preset, preset.cloned().map(Box::new));
    if previous_preset.as_deref() != preset {
        events.push(ProjectEvent::DevicePresetChanged {
            device: device_id.clone(),
        });
    }
    if events.is_empty() {
        return Ok(unchanged(Output::RestoreDevice {}));
    }
    Ok(Outcome {
        output: Output::RestoreDevice {},
        undo: Undo::Inverse(Box::new(Command::RestoreDevice {
            device: device_id.clone(),
            params: previous,
            preset: previous_preset,
        })),
        events,
        control: vec![],
    })
}
