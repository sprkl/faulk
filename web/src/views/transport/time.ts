/** Musical time for the transport bar: pure functions, one tempo and one time signature (MVP). */

/** Ticks per quarter note (`project::PPQ`). */
export const PPQ = 960
export const MIN_TEMPO = 20
export const MAX_TEMPO = 999
export const MIN_NUMERATOR = 1
export const MAX_NUMERATOR = 32
export const DENOMINATORS = [1, 2, 4, 8, 16, 32] as const
/** The loop a first press of the loop button sets when the project has none, in bars. */
export const DEFAULT_LOOP_BARS = 4

export interface Signature {
  numerator: number
  denominator: number
}

/** Ticks in one beat: the denominator's note value (3/8 has an eighth-note beat of 480). */
export const beatTicks = (sig: Signature): number => (PPQ * 4) / sig.denominator
/** Ticks in one bar. */
export const barTicks = (sig: Signature): number => beatTicks(sig) * sig.numerator

const pad = (n: number, width: number): string => String(n).padStart(width, '0')

/**
 * Bars.beats.ticks as text, bars and beats from 1, ticks from 0 and padded to three digits.
 * A position before the start (a count-in) falls in bar 0 and below, as sequencers read it.
 */
export function formatPosition(tick: number, sig: Signature): string {
  const whole = Math.round(tick)
  const bar = Math.floor(whole / barTicks(sig))
  const inBar = whole - bar * barTicks(sig)
  const beat = Math.floor(inBar / beatTicks(sig))
  const ticks = inBar - beat * beatTicks(sig)
  return `${bar + 1}.${beat + 1}.${pad(ticks, 3)}`
}

/** The time of a position in seconds at a constant tempo. */
export const secondsOf = (tick: number, bpm: number): number => (tick / PPQ) * (60 / bpm)

/** Minutes:seconds.milliseconds, with a true minus sign before the start. */
export function formatTime(seconds: number): string {
  const ms = Math.round(Math.abs(seconds) * 1000)
  const sign = seconds < 0 && ms > 0 ? '−' : ''
  const minutes = Math.floor(ms / 60000)
  return `${sign}${pad(minutes, 2)}:${pad(Math.floor(ms / 1000) % 60, 2)}.${pad(ms % 1000, 3)}`
}

/** A tempo as text: at most two decimals, none when whole. */
export const formatTempo = (bpm: number): string => String(Math.round(bpm * 100) / 100)

/**
 * The tempo a field's text means, or `null` when the command would refuse it: not a number, or
 * outside 20 to 999 BPM. A comma is accepted as the decimal mark.
 */
export function parseTempo(text: string): number | null {
  const clean = text.trim().replace(',', '.')
  if (!/^(\d+\.?\d*|\.\d+)$/.test(clean)) return null
  const bpm = Number(clean)
  return Number.isFinite(bpm) && bpm >= MIN_TEMPO && bpm <= MAX_TEMPO ? bpm : null
}

/** The tempo after an arrow key: the next whole BPM up or down, kept inside 20 to 999. */
export function stepTempo(bpm: number, direction: 1 | -1): number {
  const next = direction === 1 ? Math.floor(bpm) + 1 : Math.ceil(bpm) - 1
  return Math.min(MAX_TEMPO, Math.max(MIN_TEMPO, next))
}

/** The numerator a field's text means, or `null` when out of 1 to 32 or not a whole number. */
export function parseNumerator(text: string): number | null {
  const clean = text.trim()
  if (!/^\d+$/.test(clean)) return null
  const n = Number(clean)
  return n >= MIN_NUMERATOR && n <= MAX_NUMERATOR ? n : null
}
