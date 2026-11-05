//! One integration-test binary for the crate (#599): every former `tests/<name>.rs` is the module `<name>`.

mod common;

mod folder;
mod folder_docs;
mod folder_errors;
mod folder_load;
mod folder_reference;
mod load_bench;
mod markers;
mod master_automation;
mod master_chain;
mod migrate_v2;
mod migrate_v3;
mod model;
mod presets;
mod properties;
mod scale;

// The load bench's modules (compiled by `load_bench`) name `crate::synth`, as in the bench.
use load_bench::synth;
