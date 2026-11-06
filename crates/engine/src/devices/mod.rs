//! Faulk's built-in devices (DV, `docs/specs/02-decoupage-mvp.md` § 2.5): one file per
//! device (`devices/<kind>.rs`, the kind without `faulk.`) and the registry, one line per
//! device. The registry is what the shipping hosts resolve a `ControlMsg::DeviceInsert`
//! with ([`factory`], through `crate::engine::builtin_factory`) and what the schema bundle
//! exports ([`crate::device::builtin_descriptors`]).

use crate::device::{DeviceDescriptor, Factory};
use crate::msg::DeviceKind;

/// The audio clip player (AU-4): the instrument stage of an audio track, held by the track
/// and run by the graph, not a registry device (its module doc says why).
pub mod clip_player;
pub mod compressor;
pub mod drums;
pub mod echo;
pub mod filter;
pub mod lattice;
pub mod osc_synth;
pub mod reverb;
pub mod stack;
pub mod time_change;

/// A built-in device: its static descriptor and the factory that builds it off the audio
/// path.
#[derive(Clone, Copy)]
pub struct Builtin {
    pub descriptor: &'static DeviceDescriptor,
    pub factory: Factory,
}

impl core::fmt::Debug for Builtin {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Builtin")
            .field("kind", &self.descriptor.kind)
            .finish_non_exhaustive()
    }
}

/// The built-in devices, one line per device (the next DV tasks add theirs).
pub const REGISTRY: &[Builtin] = &[
    osc_synth::BUILTIN,
    drums::BUILTIN,
    echo::BUILTIN,
    reverb::BUILTIN,
    compressor::BUILTIN,
    filter::BUILTIN,
    stack::BUILTIN,
    lattice::BUILTIN,
];

/// The registry's line of `kind`.
pub fn builtin(kind: DeviceKind) -> Option<&'static Builtin> {
    REGISTRY.iter().find(|b| b.descriptor.kind_id == kind)
}

/// The factory of the built-in device of `kind`: the resolver of the shipping hosts
/// (`crate::engine::Resolver`).
pub fn factory(kind: DeviceKind) -> Option<Factory> {
    builtin(kind).map(|b| b.factory)
}
