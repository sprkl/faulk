import { FADER_TAPER } from '../generated/protocol'
import { MAX_GAIN_DB, MIN_GAIN_DB } from './tracks'

/**
 * The fader's taper: a position from 0 (bottom) to 1 (top) and the gain in dB it stands for.
 * Linear in dB over the whole range would spend two thirds of the travel below -60 dB, where
 * nothing is mixed; the taper gives the working range (-30 to +6 dB) most of the travel.
 * Piecewise linear through these points (position, dB); the bottom is silence. The table is the
 * engine's (`engine::fader`, #389), which plays a volume lane along the same law: this is not a
 * copy, it comes from the generated schema.
 */
const POINTS: readonly (readonly [number, number])[] = FADER_TAPER

/** Where unity gain sits on the fader. */
export const UNITY_POSITION: number = POINTS.find(([, db]) => db === 0)?.[0] ?? 0.78
/** The marks drawn beside the fader, in dB. */
export const FADER_MARKS: readonly number[] = [6, 0, -6, -12, -24, -48]

/** The position of a gain; silence (`null`) is the bottom. */
export function positionOf(gainDb: number | null | undefined): number {
  if (gainDb == null || !Number.isFinite(gainDb) || gainDb <= MIN_GAIN_DB) return 0
  if (gainDb >= MAX_GAIN_DB) return 1
  for (let i = 1; i < POINTS.length; i++) {
    const [p0, d0] = POINTS[i - 1]
    const [p1, d1] = POINTS[i]
    if (gainDb <= d1) return p0 + ((gainDb - d0) / (d1 - d0)) * (p1 - p0)
  }
  return 1
}

/**
 * The gain a position means: `null` (silence) at the bottom, rounded to 0.1 dB. A position outside 0..1 is clamped.
 */
export function gainAt(position: number): number | null {
  if (!(position > 0)) return null
  const p = Math.min(1, position)
  for (let i = 1; i < POINTS.length; i++) {
    const [p0, d0] = POINTS[i - 1]
    const [p1, d1] = POINTS[i]
    if (p <= p1) {
      const db = Math.round((d0 + ((p - p0) / (p1 - p0)) * (d1 - d0)) * 10) / 10
      if (db <= MIN_GAIN_DB) return null
      // 0.78 is a step of the fader's input: 0 dB is reachable exactly, and -0 is 0.
      return db === 0 ? 0 : db
    }
  }
  return MAX_GAIN_DB
}

/** The gain in dB at a position, unrounded and clamped to the table (the bottom is `MIN_GAIN_DB`, not silence). */
export function exactDbAt(position: number): number {
  if (!(position > 0)) return MIN_GAIN_DB
  const p = Math.min(1, position)
  for (let i = 1; i < POINTS.length; i++) {
    const [p0, d0] = POINTS[i - 1]
    const [p1, d1] = POINTS[i]
    if (p <= p1) return d0 + ((p - p0) / (p1 - p0)) * (d1 - d0)
  }
  return MAX_GAIN_DB
}
