import { PARAM_DEFAULTS, type SynthParam } from './generated/protocol'

/** Engine defaults, generated from `Synth::new` (crates/engine/src/synth.rs). */
export const DEFAULT_PARAMS: Record<SynthParam, number> = { ...PARAM_DEFAULTS }

/** Knob range of the time parameters, in seconds (the engine accepts up to 30 s). */
export const TIME_MIN = 0.002
export const TIME_MAX = 10

export type KnobParam = Exclude<SynthParam, 'waveform'>
export const KNOB_PARAMS: KnobParam[] = ['attack', 'decay', 'sustain', 'release', 'gain']

const isTime = (p: KnobParam) => p === 'attack' || p === 'decay' || p === 'release'

/** Knob position (0–1) of a value: logarithmic for times, linear otherwise. */
export function positionOf(param: KnobParam, value: number): number {
  if (!isTime(param)) return Math.min(1, Math.max(0, value))
  const v = Math.min(TIME_MAX, Math.max(TIME_MIN, value))
  return Math.log(v / TIME_MIN) / Math.log(TIME_MAX / TIME_MIN)
}

/** Value at a knob position (0–1). */
export function valueAt(param: KnobParam, position: number): number {
  const p = Math.min(1, Math.max(0, position))
  if (!isTime(param)) return p
  return TIME_MIN * Math.pow(TIME_MAX / TIME_MIN, p)
}

export function formatValue(param: KnobParam, value: number): string {
  if (isTime(param)) return value < 1 ? `${Math.round(value * 1000)} ms` : `${value.toFixed(2)} s`
  if (param === 'gain') return value > 0 ? `${(20 * Math.log10(value)).toFixed(1)} dB` : '−∞ dB'
  return `${Math.round(value * 100)} %`
}
