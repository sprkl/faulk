import type { SynthParam } from '../generated/protocol'
import { setParam } from '../synth-commands'
import type { Engine } from './engine'

/**
 * The helper keeps its own copy of the project, one synth at its defaults, until BO-1 gives it
 * the page's snapshot (spec § 2.1). Until then, entering Boost brings that copy to the synth's
 * settings the page holds. BO-1 deletes this file.
 */
export function bringHelperCopyUpToDate(native: Engine, params: Record<SynthParam, number>) {
  for (const [param, value] of Object.entries(params) as [SynthParam, number][]) {
    native.send(setParam(param, value), new Uint8Array())
  }
}
