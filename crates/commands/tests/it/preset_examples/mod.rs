//! The state the preset commands run from, and one example of each of them and of their
//! branches (#191). `contract.rs` adds [`examples`] to the catalog of examples;
//! `preset.rs` checks the behaviour in depth.

#![allow(dead_code)]

use crate::clip_examples::after;
use ::project::preset::{DevicePreset, Preset, PresetId, PresetLocation};
use ::project::{DeviceId, IdGenerator, TrackId};
use commands::preset::{DeviceRestore, PresetRef};
use commands::{Command, State, preset};
use std::collections::BTreeMap;

pub fn p(command: preset::Command) -> Command {
    Command::Preset(command)
}

/// A factory preset of the synth.
pub const ROUND_BASS: &str = "faulk.osc_synth/round-bass";

pub fn factory(id: &str) -> PresetRef {
    PresetRef {
        location: PresetLocation::Factory,
        id: id.into(),
    }
}

pub fn reference(location: PresetLocation, id: &PresetId) -> PresetRef {
    PresetRef {
        location,
        id: id.to_string(),
    }
}

/// The fixture's synth track, with a preset of the synth in the project (`warm`, which the
/// synth shows) and one in the library (`bright`).
pub struct PresetFx {
    pub state: State,
    pub track: TrackId,
    pub synth: DeviceId,
    pub warm: PresetId,
    pub bright: PresetId,
}

pub fn synth_preset(name: &str, attack: f64) -> Preset {
    Preset {
        device: "faulk.osc_synth".into(),
        version: 1,
        name: name.into(),
        category: Some("Pad".into()),
        params: BTreeMap::from([("attack".into(), attack), ("waveform".into(), 2.0)]),
    }
}

pub fn fx() -> PresetFx {
    let f = commands::testing::fixture();
    let mut state = f.state.clone();
    let mut ids = IdGenerator::new(1_640_000_000_000, 0x00DD_0000_0000_0000_0000);
    let warm = PresetId::generate(&mut ids);
    let bright = PresetId::generate(&mut ids);
    state
        .project
        .presets
        .insert(warm.clone(), synth_preset("Warm", 0.5));
    state
        .library
        .insert(bright.clone(), synth_preset("Bright", 0.01));
    state.project.device_mut(&f.device).unwrap().preset = Some(Box::new(DevicePreset {
        location: PresetLocation::Project,
        id: warm.to_string(),
        name: "Warm".into(),
    }));
    state
        .project
        .validate()
        .expect("the preset fixture is valid");
    PresetFx {
        state,
        track: f.track,
        synth: f.device,
        warm,
        bright,
    }
}

pub fn examples() -> Vec<(Command, State)> {
    let f = fx();
    let on = |command: Command| (command, f.state.clone());
    let deleted = after(
        f.state.clone(),
        &p(preset::Command::Delete {
            preset: reference(PresetLocation::Library, &f.bright),
        }),
    );
    let loaded = after(
        f.state.clone(),
        &p(preset::Command::Load {
            device: f.synth.clone(),
            preset: factory(ROUND_BASS),
        }),
    );
    vec![
        on(p(preset::Command::Save {
            device: f.synth.clone(),
            name: "Mine".into(),
            category: Some("Bass".into()),
            location: PresetLocation::Project,
        })),
        on(p(preset::Command::Save {
            device: f.synth.clone(),
            name: "  Mine  ".into(),
            category: None,
            location: PresetLocation::Library,
        })),
        on(p(preset::Command::Load {
            device: f.synth.clone(),
            preset: factory(ROUND_BASS),
        })),
        on(p(preset::Command::Load {
            device: f.synth.clone(),
            preset: reference(PresetLocation::Library, &f.bright),
        })),
        on(p(preset::Command::Delete {
            preset: reference(PresetLocation::Project, &f.warm),
        })),
        on(p(preset::Command::Delete {
            preset: reference(PresetLocation::Library, &f.bright),
        })),
        on(p(preset::Command::Rename {
            preset: reference(PresetLocation::Project, &f.warm),
            name: "Warmer".into(),
        })),
        on(p(preset::Command::Rename {
            preset: reference(PresetLocation::Library, &f.bright),
            name: "Brighter".into(),
        })),
        on(p(preset::Command::Copy {
            preset: reference(PresetLocation::Project, &f.warm),
            location: PresetLocation::Library,
            name: None,
        })),
        on(p(preset::Command::Copy {
            preset: factory(ROUND_BASS),
            location: PresetLocation::Project,
            name: Some("My Bass".into()),
        })),
        on(p(preset::Command::List {
            device: Some(f.synth.clone()),
            kind: None,
        })),
        on(p(preset::Command::List {
            device: None,
            kind: Some("faulk.filter".into()),
        })),
        on(p(preset::Command::List {
            device: None,
            kind: None,
        })),
        // The inverse of a delete (a restore with a snapshot), of a save (a restore that takes
        // the preset out and gives the device its preset back), of a load.
        (deleted.1, deleted.0),
        on(p(preset::Command::Restore {
            preset: reference(PresetLocation::Project, &f.warm),
            snapshot: None,
            device: Some(DeviceRestore {
                device: f.synth.clone(),
                preset: None,
            }),
        })),
        (loaded.1, loaded.0),
        on(p(preset::Command::RestoreDevice {
            device: f.synth.clone(),
            params: BTreeMap::from([("gain".into(), 0.25)]),
            preset: None,
        })),
    ]
}
