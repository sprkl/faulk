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

/**
 * The value a text means for a parameter, in its unit, kept to its bounds and step; null when
 * unreadable. A number without a unit is in the unit the readout showed when the field opened, which
 * depends on `shown` (the value then: seconds below 1 are shown in ms, hertz below 1000 in Hz).
 */
export function parseParam(p: ParamDescriptor, text: string, shown: number): number | null {
  const unit: string = p.unit
  if (unit === 'gain' && SILENCE.test(normalise(text))) return snap(p, 0)
  const q = parseQuantity(text)
  if (!q) return null
  const { value, unit: u } = q
  let result: number
  switch (unit) {
    case 'seconds': {
      const written = u || (shown < 1 ? 'ms' : 's')
      if (written === 'ms') result = value / 1000
      else if (written === 's') result = value
      else return null
      break
    }
    case 'hertz': {
      const written = u || (shown < 1000 ? 'hz' : 'khz')
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
  return snap(p, tidy(result))
}

/** The gain a text means, in dB rounded to 0.1, `null` for silence (also what the bottom of the range stands for); `undefined` when unreadable. */
export function parseGain(text: string): number | null | undefined {
  if (SILENCE.test(normalise(text))) return null
  const q = parseQuantity(text)
  if (!q || (q.unit !== '' && q.unit !== 'db')) return undefined
  const db = Math.round(Math.min(MAX_GAIN_DB, q.value) * 10) / 10
  if (db <= MIN_GAIN_DB) return null
  return db === 0 ? 0 : db
}

/**
 * The pan a text means, from -1 (left) to 1 (right): "C", "L30", "R25" ("G" and "D" in French), or a
 * signed percentage ("-30", "25 %": negative is left). Rounded to a whole percent; `undefined` when unreadable.
 */
export function parsePan(text: string): number | undefined {
  const t = normalise(text)
  if (/^(?:c|center|centre)$/.test(t)) return 0
  const side = /^([lrgd])\s*(\d+(?:\.\d+)?)\s*%?$/.exec(t)
  let percent: number
  if (side) percent = (side[1] === 'r' || side[1] === 'd' ? 1 : -1) * Number(side[2])
  else {
    const q = parseQuantity(t)
    if (!q || (q.unit !== '' && q.unit !== '%')) return undefined
    percent = q.value
  }
  const pan = Math.round(Math.max(-100, Math.min(100, percent))) / 100
  return pan === 0 ? 0 : pan
}
