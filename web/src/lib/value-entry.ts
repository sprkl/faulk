/**
 * Reading what a person types into the field of a continuous control (`ValueField.svelte`): a number
 * with or without the unit the control shows ("250 ms", "-6 dB", "1.2k", "40 %", "L30"). Pure and free
 * of the DOM. Each reader returns the value in the unit of the command, already kept to the bounds and
 * the step of the control, or `null`/`undefined` when the text is unreadable or carries a unit the
 * control does not have. The field is pre-filled with the displayed text (`formatValue`,
 * `formatGain`, `formatPan`), so a reader must take back what the control shows.
 */

import { snap, type ParamDescriptor } from './devices'
import { MAX_GAIN_DB, MIN_GAIN_DB } from './project/tracks'

/** A number and the unit written after it ('' when none), lower case. */
export interface Quantity {
  value: number
  unit: string
}

const SILENCE = /^[-+]?\s*(?:inf|infinity|∞)(?:\s*db)?$/

const normalise = (text: string): string => text.trim().toLowerCase().replace(/[−–—]/g, '-').replace(',', '.')

/** The number and the unit of a text, or null when it holds anything else. */
export function parseQuantity(text: string): Quantity | null {
  const m = /^([+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?)\s*([a-zµ%]*)$/.exec(normalise(text))
  if (!m) return null
  const value = Number(m[1])
  return Number.isFinite(value) ? { value, unit: m[2] } : null
}

/** Twelve significant digits: "1.2k" is 1200, not 1199.9999999999998. */
const tidy = (n: number): number => Number(n.toPrecision(12))

/** What a typed text means for a parameter: the value, and the bound it was limited to when the text went past the range. */
export interface ParamEntry {
  value: number
  /** The bound the value was brought back to, in the parameter's unit; absent when the text was inside the range. */
  limit?: number
}

/**
 * The unit a bare number is read in, whatever the readout shows (#410): hertz, milliseconds, decibels, percent...
 * Empty for a plain number. A written unit is always taken as typed.
 */
export function baseUnit(p: ParamDescriptor): string {
  const unit: string = p.unit
  switch (unit) {
    case 'seconds':
      return 'ms'
    case 'hertz':
      return 'Hz'
    case 'gain':
    case 'decibels':
      return 'dB'
    case 'semitones':
      return 'st'
    case 'cents':
      return 'ct'
    case 'percent':
      return '%'
    default:
      return p.min === 0 && p.max <= 1 ? '%' : ''
  }
}

/** The value a text means for a parameter, or null when unreadable (`parseParamEntry` without the limit). */
export function parseParam(p: ParamDescriptor, text: string): number | null {
  return parseParamEntry(p, text)?.value ?? null
}

/**
 * The value a text means for a parameter, in its unit, kept to its bounds and step, and the bound it was limited to when
 * the text went past the range; null when unreadable. A number without a unit is in the base unit of the parameter
 * (`baseUnit`), never in the unit the readout shows: "450" on a cutoff shown in kHz is 450 Hz (#410).
 */
export function parseParamEntry(p: ParamDescriptor, text: string): ParamEntry | null {
  const unit: string = p.unit
  if (unit === 'gain' && SILENCE.test(normalise(text))) return { value: snap(p, 0) }
  const q = parseQuantity(text)
  if (!q) return null
  const { value, unit: u } = q
  let result: number
  switch (unit) {
    case 'seconds': {
      const written = u || 'ms'
      if (written === 'ms') result = value / 1000
      else if (written === 's') result = value
      else return null
      break
    }
    case 'hertz': {
      const written = u || 'hz'
      if (written === 'hz') result = value
      else if (written === 'khz' || written === 'k') result = value * 1000
      else return null
      break
    }
    case 'gain':
      if (u !== '' && u !== 'db') return null
      result = Math.pow(10, value / 20)
      break
    case 'decibels':
      if (u !== '' && u !== 'db') return null
      result = value
      break
    case 'semitones':
      if (u !== '' && u !== 'st') return null
      result = value
      break
    case 'cents':
      if (u !== '' && u !== 'ct') return null
      result = value
      break
    case 'percent':
      if (u !== '' && u !== '%') return null
      result = value
      break
    default:
      // A level from 0 to 1 is shown as a percentage; any other plain number is shown as it is.
      if (p.min === 0 && p.max <= 1) {
        if (u !== '' && u !== '%') return null
        result = value / 100
      } else {
        if (u !== '') return null
        result = value
      }
  }
  result = tidy(result)
  const bound = result < p.min ? p.min : result > p.max ? p.max : undefined
  return bound === undefined ? { value: snap(p, result) } : { value: snap(p, bound), limit: bound }
}

/** The gain a text means, in dB rounded to 0.1, `null` for silence (also what the bottom of the range stands for); `undefined` when unreadable. */
export function parseGain(text: string): number | null | undefined {
  const entry = parseGainEntry(text)
  return entry ? entry.gain : undefined
}

/** `parseGain` and whether the text went past the top of the fader (`limited`); null when unreadable. */
export function parseGainEntry(text: string): { gain: number | null; limited: boolean } | null {
  if (SILENCE.test(normalise(text))) return { gain: null, limited: false }
  const q = parseQuantity(text)
  if (!q || (q.unit !== '' && q.unit !== 'db')) return null
  const db = Math.round(Math.min(MAX_GAIN_DB, q.value) * 10) / 10
  const limited = q.value > MAX_GAIN_DB
  if (db <= MIN_GAIN_DB) return { gain: null, limited }
  return { gain: db === 0 ? 0 : db, limited }
}

/**
 * The pan a text means, from -1 (left) to 1 (right): "C", "L30", "R25" ("G" and "D" in French), or a
 * signed percentage ("-30", "25 %": negative is left). Rounded to a whole percent; `undefined` when unreadable.
 */
export function parsePan(text: string): number | undefined {
  return parsePanEntry(text)?.pan
}

/** `parsePan` and whether the text went past a side (`limited`); null when unreadable. */
export function parsePanEntry(text: string): { pan: number; limited: boolean } | null {
  const t = normalise(text)
  if (/^(?:c|center|centre)$/.test(t)) return { pan: 0, limited: false }
  const side = /^([lrgd])\s*(\d+(?:\.\d+)?)\s*%?$/.exec(t)
  let percent: number
  if (side) percent = (side[1] === 'r' || side[1] === 'd' ? 1 : -1) * Number(side[2])
  else {
    const q = parseQuantity(t)
    if (!q || (q.unit !== '' && q.unit !== '%')) return null
    percent = q.value
  }
  const pan = Math.round(Math.max(-100, Math.min(100, percent))) / 100
  return { pan: pan === 0 ? 0 : pan, limited: Math.abs(percent) > 100 }
}
