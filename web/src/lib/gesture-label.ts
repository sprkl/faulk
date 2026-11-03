/**
 * The text of the label that follows the pointer during a move or a resize (#612): where the clip or note
 * starts or how long it is (bar and beat), the snap in force and the reminder of the key that frees it.
 * Pure: the caller gives the translator, so the same words serve the arrangement and the piano roll.
 */

import { barTicks, beatTicks, type Signature } from '../views/transport/time'
import type { MessageKey } from './i18n/app'

export type GestureT = (key: MessageKey, vars?: Record<string, string | number>) => string

/** The labels of the snap settings a step is named by. */
export interface SnapNames {
  bar: string
  beat: string
}

/** A number of beats, written without a trailing zero (1, 1.5, 1.25). */
const plain = (n: number): number => Number(n.toFixed(2))

/** "Bar 3, beat 2.5" for the tick `tick` (bars and beats count from 1). */
export function positionText(t: GestureT, sig: Signature, tick: number): string {
  const bar = barTicks(sig)
  const at = Math.max(0, tick)
  return t('gesture.position', { bar: Math.floor(at / bar) + 1, beat: plain((at % bar) / beatTicks(sig) + 1) })
}

/** "Length 2 bars, 1.5 beats" for a span of `ticks` (a part at zero is left out, except when both are). */
export function lengthText(t: GestureT, sig: Signature, ticks: number): string {
  const bar = barTicks(sig)
  const span = Math.max(0, ticks)
  const whole = Math.floor(span / bar)
  // Three decimals: a clip of one tick is not "0 beat".
  const beats = Number(((span % bar) / beatTicks(sig)).toFixed(3))
  const parts: string[] = []
  if (whole > 0) parts.push(t(whole === 1 ? 'gesture.bar' : 'gesture.bars', { n: whole }))
  if (beats > 0 || whole === 0) parts.push(t(beats === 1 ? 'gesture.beat' : 'gesture.beats', { n: beats }))
  return t('gesture.length', { length: parts.join(', ') })
}

/**
 * The snap line: the name of the step in force ("Snap: Beat", "Snap: 1/16"), or "Snap: Free" with no step. Auto reads
 * as the step it resolves to at this zoom.
 */
export function snapText(t: GestureT, sig: Signature, step: number | null, names: SnapNames): string {
  let name: string
  if (step === null || !(step > 0)) name = t('gesture.free')
  else {
    const beat = beatTicks(sig)
    const bar = barTicks(sig)
    if (step === bar) name = names.bar
    else if (step === beat) name = names.beat
    // Finer than a beat: as a fraction of a whole note (1/16 for a quarter of a quarter-note beat).
    else if (step < beat) name = t('gesture.fraction', { n: plain((sig.denominator * beat) / step) })
    else if (step > bar) name = t(plain(step / bar) === 1 ? 'gesture.bar' : 'gesture.bars', { n: plain(step / bar) })
    else name = t('gesture.beats', { n: plain(step / beat) })
  }
  return t('gesture.snap', { name })
}
