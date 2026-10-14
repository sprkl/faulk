import { PARAM_DEFAULTS, type SynthParam } from './generated/protocol'

/** Engine defaults of the one synth, generated from the engine (`crates/engine/src/devices/osc_synth.rs`). */
export const DEFAULT_PARAMS: Record<SynthParam, number> = { ...PARAM_DEFAULTS }
