/**
 * What Contour's card (#156) decides around its curve, kept apart from the component so a test checks it.
 */

import { createGesture } from '../gesture'
import type { ParamDescriptor } from '../devices'
import type { GroupRun } from '../history-group'
import { BANDS } from './response'

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

/**
 * One band's controls on the card: its number on the curve (1 to 7, the handle's), its switch (`<band>_on`, the legend's
 * partner) and its other parameters in the descriptor's order. The last group, with no band and no switch, holds the
 * parameters that belong to none (the output gain).
 */
export type ControlGroup = {
  /** The band's key (`hp`, `bell1`, ...), `output` for the group of no band. */
  key: string
  /** The handle's number on the curve, `null` for the group of no band. */
  number: number | null
  /** The band's switch, `null` for the group of no band. */
  on: ParamDescriptor | null
  params: ParamDescriptor[]
}

/** The 27 controls of Contour's card, grouped by band in the order of the signal, the rest last. A parameter is in exactly one group. */
export function controlGroups(params: readonly ParamDescriptor[]): ControlGroup[] {
  const groups: ControlGroup[] = BANDS.map((band, i) => ({ key: band.key, number: i + 1, on: null, params: [] }))
  const rest: ControlGroup = { key: 'output', number: null, on: null, params: [] }
  for (const param of params) {
    const group = groups.find((g) => param.key.startsWith(`${g.key}_`)) ?? rest
    if (group !== rest && param.key === `${group.key}_on`) group.on = param
    else group.params.push(param)
  }
  return [...groups.filter((g) => g.on || g.params.length), ...(rest.params.length ? [rest] : [])]
}
