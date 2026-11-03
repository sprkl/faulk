import { MAX_TEMPO, MIN_TEMPO } from './time'

/** A pause this long (milliseconds) between two taps starts a new series. */
export const TAP_RESET_MS = 2000
/** The most intervals the average looks at: the last 8, so the tempo follows a hand that speeds up or slows down. */
export const TAP_MAX_INTERVALS = 8

export interface TapState {
  /** The times of the taps of the current series (milliseconds, oldest first): at most `TAP_MAX_INTERVALS + 1`. */
  taps: readonly number[]
  /** The tempo the series gives so far, or `null` until the second tap. */
  bpm: number | null
}

export const NO_TAPS: TapState = { taps: [], bpm: null }

/** The tempo of a series of taps: 60 s over the mean interval, clamped to the project's range and rounded to 0.1 BPM. */
function tempoOf(taps: readonly number[]): number | null {
  if (taps.length < 2) return null
  const mean = (taps[taps.length - 1]! - taps[0]!) / (taps.length - 1)
  if (!(mean > 0)) return null
  const bpm = Math.min(MAX_TEMPO, Math.max(MIN_TEMPO, 60_000 / mean))
  return Math.round(bpm * 10) / 10
}

/**
 * One tap at time `now` (milliseconds, any clock that only goes forward). The series starts over after `TAP_RESET_MS` without a
 * tap (or if the clock went back); the first tap of a series gives no tempo, the next ones give the mean of the last
 * `TAP_MAX_INTERVALS` intervals. A second tap in the same millisecond is ignored (it would be an interval of zero).
 */
export function tap(state: TapState, now: number): TapState {
  const last = state.taps[state.taps.length - 1]
  if (last === undefined || now < last || now - last > TAP_RESET_MS) return { taps: [now], bpm: null }
  if (now === last) return state
  const taps = [...state.taps, now].slice(-(TAP_MAX_INTERVALS + 1))
  return { taps, bpm: tempoOf(taps) }
}
