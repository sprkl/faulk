/**
 * The device panels, built from the descriptors the engine exports (`DEVICES`, CT-5): which
 * parameters a device has, their range, scale, unit, enumerated values and labels. No parameter
 * id or device name is written here.
 */

import { DEVICES, SINGLE_SYNTH } from './generated/protocol'
import { messages } from './i18n/app'
import type { Lang } from './i18n/lang'

export type DeviceDescriptor = (typeof DEVICES)[number]
export type ParamDescriptor = DeviceDescriptor['params'][number]

/** The descriptor of a device kind. */
export function descriptorOf(kind: string): DeviceDescriptor {
  const found = DEVICES.find((d) => d.kind === kind)
  if (!found) throw new Error(`no descriptor for the device kind ${kind}`)
  return found
}

/** The descriptors of the devices that can be the instrument of a track, in registry order. */
export const instrumentDescriptors = (devices: readonly DeviceDescriptor[] = DEVICES): readonly DeviceDescriptor[] =>
  devices.filter((d) => d.role === 'instrument')

/** The descriptors of the devices that can be inserted in a track's chain as effects, in registry order. */
export const effectDescriptors = (devices: readonly DeviceDescriptor[] = DEVICES): readonly DeviceDescriptor[] =>
  devices.filter((d) => (d.role as string) === 'effect')

/** The descriptors of the MIDI effects (#219), in registry order: they go before a MIDI track's instrument. */
export const midiEffectDescriptors = (devices: readonly DeviceDescriptor[] = DEVICES): readonly DeviceDescriptor[] =>
  devices.filter((d) => (d.role as string) === 'midi_effect')

/**
 * A text of the catalogue, or `undefined` when it has none: a device registered without its
 * translations still gets a panel (the parameter's key and the descriptor's description stand in);
 * the completeness test (`i18n.test.ts`) is what asks for the text.
 */
export const deviceText = (lang: Lang, key: string): string | undefined => (messages[lang] as Record<string, string | undefined>)[key]

/** The oscillator synth's descriptor, narrowed: its parameter keys are the `SynthParam`s, not those of every device. */
export type SynthDescriptor = Extract<DeviceDescriptor, { kind: 'faulk.osc_synth' }>

/** The descriptor of the one synth the page plays (`SINGLE_SYNTH.kind`). */
export const SYNTH: SynthDescriptor = ((d: DeviceDescriptor): SynthDescriptor => {
  if (d.kind !== 'faulk.osc_synth') throw new Error(`the page plays ${d.kind}, not the oscillator`)
  return d
})(descriptorOf(SINGLE_SYNTH.kind))

/** Whether a parameter is a choice among named values (shown as buttons, not a knob). */
export const isChoice = (p: ParamDescriptor): boolean => p.values.length > 0

/** The parameters shown as knobs: the continuous and stepped ones, in the descriptor's order. */
export const knobParams = <D extends DeviceDescriptor>(d: D): D['params'][number][] => (d.params as readonly D['params'][number][]).filter((p) => !isChoice(p))

/** The parameters shown as a choice of values. */
export const choiceParams = <D extends DeviceDescriptor>(d: D): D['params'][number][] => (d.params as readonly D['params'][number][]).filter(isChoice)

/** The values of a choice parameter: its number and the key of its name. */
export const choices = (p: ParamDescriptor): { value: number; key: string }[] => p.values.map((key, i) => ({ value: p.min + i * (p.step || 1), key }))

/**
 * The message key of a parameter's label, or of one of its values: `<kind without "faulk.">.<key>`
 * (labels and descriptions are translated, `i18n/`).
 */
export const labelKey = (d: DeviceDescriptor, key: string): string => `${d.kind.replace(/^faulk\./, '')}.${key}`

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))

/** Knob position (0–1) of a value, along the parameter's scale between its bounds. */
export function positionOf(p: ParamDescriptor, value: number): number {
  const v = clamp(Number.isFinite(value) ? value : p.default, p.min, p.max)
  if (p.scale === 'log') return Math.log(v / p.min) / Math.log(p.max / p.min)
  return (v - p.min) / (p.max - p.min)
}

/** Value at a knob position (0–1), rounded to the parameter's step. */
export function valueAt(p: ParamDescriptor, position: number): number {
  const t = clamp(Number.isFinite(position) ? position : 0, 0, 1)
  const v = p.scale === 'log' ? p.min * Math.pow(p.max / p.min, t) : p.min + t * (p.max - p.min)
  return snap(p, v)
}

/** A value kept to the parameter's bounds and rounded to its step (no rounding when the step is 0). */
export function snap(p: ParamDescriptor, value: number): number {
  const v = clamp(value, p.min, p.max)
  if (p.step > 0) return clamp(p.min + Math.round((v - p.min) / p.step) * p.step, p.min, p.max)
  return v
}

/** A value for display, by unit: a linear gain in decibels, a plain level (unit none, from 0 up to at most 1) as a percentage. */
export function formatValue(p: ParamDescriptor, value: number): string {
  // Every unit of the contract (`Unit`), not only those of the devices exported today.
  const unit: string = p.unit
  switch (unit) {
    case 'seconds':
      // Under 10 ms to a tenth (a look-ahead, a short attack), whole milliseconds up to 1 s.
      if (value < 0.01) return `${+(value * 1000).toFixed(1)} ms`
      return value < 1 ? `${Math.round(value * 1000)} ms` : `${value.toFixed(2)} s`
    case 'hertz':
      return value < 1000 ? `${Math.round(value)} Hz` : `${(value / 1000).toFixed(2)} kHz`
    case 'gain':
      return value > 0 ? `${(20 * Math.log10(value)).toFixed(1)} dB` : '−∞ dB'
    case 'decibels':
      return `${value.toFixed(1)} dB`
    case 'semitones':
      return `${value > 0 ? '+' : ''}${Math.round(value)} st`
    case 'cents':
      return `${value > 0 ? '+' : ''}${Math.round(value)} ct`
    case 'percent':
      return `${Math.round(value)} %`
    case 'degrees':
      return `${Math.round(value)}°`
    case 'octaves':
      return `${value > 0 && p.min < 0 ? '+' : ''}${+value.toFixed(2)} oct`
    default:
      return p.min === 0 && p.max <= 1 ? `${Math.round(value * 100)} %` : `${+value.toFixed(2)}`
  }
}

/**
 * Parameters that only matter in one mode of their device (display only: the engine keeps every parameter, and a
 * hidden one keeps its value). Per device kind, per parameter key: the choice parameter that decides, and the keys of
 * its values for which the parameter is shown. Echo: the delay time in free mode, the note value in synced mode. Stack:
 * a voice's settings while the voice is on. Lattice: the rule with a scale, the root and scale with its own, the cells with the matrix. Ceiling: the release time in manual release.
 * Compressor: the release time in manual release, the sidechain frequency with the sidechain high-pass on. Filter: the LFO rate
 * and restart in free mode, the note value in synced mode. Ripple: the stages, centre and spread in phaser mode, the
 * delay in flanger mode, and the Filter's LFO rules.
 */
export const SHOWN_IN_MODE: Readonly<Record<string, Readonly<Record<string, { by: string; values: readonly string[] }>>>> = {
  'faulk.echo': { time: { by: 'mode', values: ['free'] }, division: { by: 'mode', values: ['synced'] } },
  'faulk.stack': stackVoices(),
  'faulk.lattice': latticeSources(),
  'faulk.ceiling': { release: { by: 'release_mode', values: ['manual'] } },
  'faulk.compressor': {
    release: { by: 'release_mode', values: ['manual'] },
    sidechain_freq: { by: 'sidechain_hpf', values: ['on'] },
  },
  // Filter: the LFO's rate and restart in free mode, its note value in synced mode (#686).
  'faulk.filter': {
    lfo_rate: { by: 'lfo_sync', values: ['free'] },
    lfo_restart: { by: 'lfo_sync', values: ['free'] },
    lfo_division: { by: 'lfo_sync', values: ['synced'] },
  },
  // Ripple: the stages, centre and spread in phaser mode, the delay in flanger mode, and the
  // Filter's LFO rules (#688).
  'faulk.ripple': {
    stages: { by: 'mode', values: ['phaser'] },
    centre: { by: 'mode', values: ['phaser'] },
    spread: { by: 'mode', values: ['phaser'] },
    delay: { by: 'mode', values: ['flanger'] },
    lfo_rate: { by: 'lfo_sync', values: ['free'] },
    lfo_restart: { by: 'lfo_sync', values: ['free'] },
    lfo_division: { by: 'lfo_sync', values: ['synced'] },
  },
}

/** Stack (#690): a voice's interval, velocity and chance show while the voice is on. */
function stackVoices(): Record<string, { by: string; values: readonly string[] }> {
  const rules: Record<string, { by: string; values: readonly string[] }> = {}
  for (let n = 1; n <= 6; n++) {
    for (const part of ['interval', 'velocity', 'chance']) rules[`voice${n}_${part}`] = { by: `voice${n}`, values: ['on'] }
  }
  return rules
}

/** The parameters of a device to show now, given the value of each parameter (`valueOf` falls back to the default). */
export function shownParams(device: DeviceDescriptor, valueOf: (key: string) => number | undefined): readonly ParamDescriptor[] {
  const rules = SHOWN_IN_MODE[device.kind]
  if (!rules) return device.params
  return device.params.filter((param) => {
    const rule = rules[param.key]
    if (!rule) return true
    const by = device.params.find((p) => p.key === rule.by)
    if (!by) return true
    const value = valueOf(rule.by) ?? by.default
    const chosen = choices(by).find((choice) => choice.value === value)
    return chosen ? rule.values.includes(chosen.key) : true
  })
}

/** Lattice (#692): the rule applies to a scale, the root and scale to the device's own, the twelve cells to the matrix. */
function latticeSources(): Record<string, { by: string; values: readonly string[] }> {
  const rules: Record<string, { by: string; values: readonly string[] }> = {
    rule: { by: 'source', values: ['project', 'device'] },
    root: { by: 'source', values: ['device'] },
    scale: { by: 'source', values: ['device'] },
  }
  // The cells are the descriptor's `map_` parameters, so a cell renamed there needs no change here.
  const lattice = DEVICES.find((d) => d.kind === 'faulk.lattice')
  for (const p of lattice?.params ?? []) if (p.key.startsWith('map_')) rules[p.key] = { by: 'source', values: ['matrix'] }
  return rules
}
