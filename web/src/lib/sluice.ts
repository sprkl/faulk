/**
 * Sluice's own state for its card (#158). The engine's strip reduction is the largest of the strip's devices, so the
 * gate reports its own in its device readout (`Device::readout`) under an id that is not a parameter: `r` while the
 * gate lets the sound through (open or opening), `-1 - r` while it shuts it (closed or closing), `r` the reduction in
 * decibels now. The trigger decides, so the state is right through the hold, the release and the inverted mode.
 */

/** The id of the gate's readout (`sluice::STATE_READOUT`): not one of its parameters, so no control is drawn for it. */
export const SLUICE_STATE_READOUT = 100

export interface GateState {
  /** The gate lets the sound through (open or opening). */
  open: boolean
  /** The gain it takes off now, in decibels (0 or more). */
  reductionDb: number
}

/** The gate's state from its readout, or `null` when the engine reports none (bypassed, not playing, no report). */
export function gateState(readout: number | undefined): GateState | null {
  if (readout === undefined || !Number.isFinite(readout)) return null
  return readout >= 0 ? { open: true, reductionDb: readout } : { open: false, reductionDb: Math.max(0, -1 - readout) }
}
