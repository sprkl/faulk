/**
 * What Contour's card (#156) decides around its curve, kept apart from the component so a test checks it.
 */

import { createGesture } from '../gesture'
import type { GroupRun } from '../history-group'

/** The rate the curve's bilinear warp follows: the engine's when it runs, else the device's last spectrum's, else 48 kHz. */
export function curveRate(engineRate: number | null | undefined, spectrumRate: number | null | undefined): number {
  if (engineRate && engineRate > 0) return engineRate
  if (spectrumRate && spectrumRate > 0) return spectrumRate
  return 48_000
}

/**
 * The card's two gestures: the knobs' and the curve's. Groups do not nest, so a gesture that begins ends the other's
 * first: a knob drag started while the curve's wheel group waits for its rest is a group of its own, never cut by the
 * wheel's timer, and the other way round.
 */
export function cardGestures(run: GroupRun) {
  const knobs = createGesture(run)
  const curve = createGesture(run)
  return {
    knobs: {
      begin: () => {
        curve.end()
        knobs.begin()
      },
      end: knobs.end,
    },
    curve: {
      begin: () => {
        knobs.end()
        curve.begin()
      },
      end: curve.end,
    },
    end: () => {
      knobs.end()
      curve.end()
    },
  }
}
