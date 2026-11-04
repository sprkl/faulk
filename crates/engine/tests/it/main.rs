//! One integration-test binary for the crate (#599): every former `tests/<name>.rs` is the module `<name>`.

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

mod all_sound_off;
mod arrangement;
mod arrangement_render;
mod audio_clip;
mod audio_source;
mod automation;
mod clip_edit;
mod clip_player;
mod clip_render;
mod compressor;
mod cycle;
mod device;
mod device_fade;
mod drums;
mod echo;
mod events;
mod filter;
mod fuzz;
mod graph;
mod grid_render;
mod growth;
mod host;
mod launcher;
mod live_stamps;
mod master_automation;
mod master_chain;
mod metronome;
mod mixer;
mod msg;
mod offline;
mod osc_synth;
mod output_limiter;
mod param;
mod project_compiler_alloc;
mod project_render;
mod recorder;
mod render;
mod reverb;
mod reverb_glide_fingerprint;
mod sample_rate;
mod stream_queue;
mod time;
mod transport;
mod warm;
