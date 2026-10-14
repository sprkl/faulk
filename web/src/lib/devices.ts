/**
 * The device panels, built from the descriptors the engine exports (`DEVICES`, CT-5): which
 * parameters a device has, their range, scale, unit, enumerated values and labels. No parameter
 * id or device name is written here.
 */

import { DEVICES, SINGLE_SYNTH } from './generated/protocol'

export type DeviceDescriptor = (typeof DEVICES)[number]
export type ParamDescriptor = DeviceDescriptor['params'][number]

/** The descriptor of a device kind. */
export function descriptorOf(kind: string): DeviceDescriptor {
  const found = DEVICES.find((d) => d.kind === kind)
  if (!found) throw new Error(`no descriptor for the device kind ${kind}`)
  return found
}

/** The descriptor of the one synth the page plays (`SINGLE_SYNTH.kind`). */
export const SYNTH: DeviceDescriptor = descriptorOf(SINGLE_SYNTH.kind)

/** Whether a parameter is a choice among named values (shown as buttons, not a knob). */
export const isChoice = (p: ParamDescriptor): boolean => p.values.length > 0

/** The parameters shown as knobs: the continuous and stepped ones, in the descriptor's order. */
export const knobParams = (d: DeviceDescriptor): ParamDescriptor[] => d.params.filter((p) => !isChoice(p))

/** The parameters shown as a choice of values. */
export const choiceParams = (d: DeviceDescriptor): ParamDescriptor[] => d.params.filter(isChoice)

/** The values of a choice parameter: its number and the key of its name. */
export const choices = (p: ParamDescriptor): { value: number; key: string }[] =>
  p.values.map((key, i) => ({ value: p.min + i * (p.step || 1), key }))

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
  if (p.step > 0) return clamp(p.min + Math.round((v - p.min) / p.step) * p.step, p.min, p.max)
  return v
}

/** A value for display, by unit. A plain level (unit none, 0 to 1) is a percentage. */
export function formatValue(p: ParamDescriptor, value: number): string {
  // Every unit of the contract (`Unit`), not only those of the devices exported today.
  const unit: string = p.unit
  switch (unit) {
    case 'seconds':
      return value < 1 ? `${Math.round(value * 1000)} ms` : `${value.toFixed(2)} s`
    case 'hertz':
      return value < 1000 ? `${Math.round(value)} Hz` : `${(value / 1000).toFixed(2)} kHz`
    case 'decibels':
      return `${value.toFixed(1)} dB`
    case 'semitones':
      return `${value > 0 ? '+' : ''}${Math.round(value)} st`
    case 'cents':
      return `${value > 0 ? '+' : ''}${Math.round(value)} ct`
    case 'percent':
      return `${Math.round(value)} %`
    default:
      return p.min === 0 && p.max === 1 ? `${Math.round(value * 100)} %` : `${+value.toFixed(2)}`
  }
}
