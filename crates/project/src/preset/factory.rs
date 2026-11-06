//! The factory presets: a few sounds built into Faulk for each built-in device, read-only,
//! which also show what the device can do. Each one is a preset file of the common format
//! under `factory/<kind>/<slug>.json`, embedded in the build; its id is `<kind>/<slug>`
//! (`faulk.osc_synth/round-bass`). Their names are English and never translated, like the
//! devices' names.

use super::Preset;
use std::sync::OnceLock;

/// A factory preset: its id and the preset.
#[derive(Clone, Debug, PartialEq)]
pub struct Factory {
    /// `<kind>/<slug>`.
    pub id: &'static str,
    pub preset: Preset,
}

/// The files, in the order the lists show them within a device (by name there).
const FILES: &[(&str, &str)] = &[
    (
        "faulk.osc_synth/round-bass",
        include_str!("factory/faulk.osc_synth/round-bass.json"),
    ),
    (
        "faulk.osc_synth/soft-pad",
        include_str!("factory/faulk.osc_synth/soft-pad.json"),
    ),
    (
        "faulk.osc_synth/square-lead",
        include_str!("factory/faulk.osc_synth/square-lead.json"),
    ),
    (
        "faulk.osc_synth/glass-pluck",
        include_str!("factory/faulk.osc_synth/glass-pluck.json"),
    ),
    (
        "faulk.drums/tight-kit",
        include_str!("factory/faulk.drums/tight-kit.json"),
    ),
    (
        "faulk.drums/deep-boom",
        include_str!("factory/faulk.drums/deep-boom.json"),
    ),
    (
        "faulk.echo/quarter-repeat",
        include_str!("factory/faulk.echo/quarter-repeat.json"),
    ),
    (
        "faulk.echo/slapback",
        include_str!("factory/faulk.echo/slapback.json"),
    ),
    (
        "faulk.reverb/small-room",
        include_str!("factory/faulk.reverb/small-room.json"),
    ),
    (
        "faulk.reverb/large-hall",
        include_str!("factory/faulk.reverb/large-hall.json"),
    ),
    (
        "faulk.compressor/gentle-glue",
        include_str!("factory/faulk.compressor/gentle-glue.json"),
    ),
    (
        "faulk.compressor/drum-smash",
        include_str!("factory/faulk.compressor/drum-smash.json"),
    ),
    (
        "faulk.filter/warm-lowpass",
        include_str!("factory/faulk.filter/warm-lowpass.json"),
    ),
    (
        "faulk.filter/thin-highpass",
        include_str!("factory/faulk.filter/thin-highpass.json"),
    ),
    (
        "faulk.stack/major-triad",
        include_str!("factory/faulk.stack/major-triad.json"),
    ),
    (
        "faulk.stack/minor-triad",
        include_str!("factory/faulk.stack/minor-triad.json"),
    ),
    (
        "faulk.stack/fifth-and-octave",
        include_str!("factory/faulk.stack/fifth-and-octave.json"),
    ),
    (
        "faulk.stack/scale-seventh",
        include_str!("factory/faulk.stack/scale-seventh.json"),
    ),
];

/// The raw files, for the tests that hold them to the canonical format.
pub fn files() -> &'static [(&'static str, &'static str)] {
    FILES
}

/// Every factory preset. A file that does not read is left out (the tests prove none is).
pub fn all() -> &'static [Factory] {
    static ALL: OnceLock<Vec<Factory>> = OnceLock::new();
    ALL.get_or_init(|| {
        FILES
            .iter()
            .filter_map(|(id, text)| {
                Preset::parse(text.as_bytes())
                    .ok()
                    .map(|preset| Factory { id, preset })
            })
            .collect()
    })
}

/// The factory preset of this id.
pub fn get(id: &str) -> Option<&'static Factory> {
    all().iter().find(|f| f.id == id)
}
