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
        "faulk.filter/quarter-wobble",
        include_str!("factory/faulk.filter/quarter-wobble.json"),
    ),
    (
        "faulk.filter/slow-sweep",
        include_str!("factory/faulk.filter/slow-sweep.json"),
    ),
    (
        "faulk.filter/talking-filter",
        include_str!("factory/faulk.filter/talking-filter.json"),
    ),
    (
        "faulk.filter/random-steps",
        include_str!("factory/faulk.filter/random-steps.json"),
    ),
    (
        "faulk.ceiling/safe-master",
        include_str!("factory/faulk.ceiling/safe-master.json"),
    ),
    (
        "faulk.ceiling/loud-master",
        include_str!("factory/faulk.ceiling/loud-master.json"),
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
    (
        "faulk.trim/tight-pluck",
        include_str!("factory/faulk.trim/tight-pluck.json"),
    ),
    (
        "faulk.trim/held-bar",
        include_str!("factory/faulk.trim/held-bar.json"),
    ),
    (
        "faulk.lattice/black-keys",
        include_str!("factory/faulk.lattice/black-keys.json"),
    ),
    (
        "faulk.lattice/blues-in-c",
        include_str!("factory/faulk.lattice/blues-in-c.json"),
    ),
    (
        "faulk.lattice/root-and-fifth",
        include_str!("factory/faulk.lattice/root-and-fifth.json"),
    ),
    (
        "faulk.scatter/octave-leaps",
        include_str!("factory/faulk.scatter/octave-leaps.json"),
    ),
    (
        "faulk.scatter/scale-wander",
        include_str!("factory/faulk.scatter/scale-wander.json"),
    ),
    (
        "faulk.scatter/rising-thirds",
        include_str!("factory/faulk.scatter/rising-thirds.json"),
    ),
    (
        "faulk.hiccup/half-bar-stutter",
        include_str!("factory/faulk.hiccup/half-bar-stutter.json"),
    ),
    (
        "faulk.hiccup/drum-roll",
        include_str!("factory/faulk.hiccup/drum-roll.json"),
    ),
    (
        "faulk.hiccup/tape-scatter",
        include_str!("factory/faulk.hiccup/tape-scatter.json"),
    ),
    (
        "faulk.bloom/soft-chorus",
        include_str!("factory/faulk.bloom/soft-chorus.json"),
    ),
    (
        "faulk.bloom/wide-ensemble",
        include_str!("factory/faulk.bloom/wide-ensemble.json"),
    ),
    (
        "faulk.bloom/vibrato",
        include_str!("factory/faulk.bloom/vibrato.json"),
    ),
    (
        "faulk.contour/vocal-presence",
        include_str!("factory/faulk.contour/vocal-presence.json"),
    ),
    (
        "faulk.contour/low-cut-tidy",
        include_str!("factory/faulk.contour/low-cut-tidy.json"),
    ),
    (
        "faulk.ripple/slow-phaser",
        include_str!("factory/faulk.ripple/slow-phaser.json"),
    ),
    (
        "faulk.ripple/synced-flanger",
        include_str!("factory/faulk.ripple/synced-flanger.json"),
    ),
    (
        "faulk.ripple/envelope-phaser",
        include_str!("factory/faulk.ripple/envelope-phaser.json"),
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
