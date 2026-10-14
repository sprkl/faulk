import type { SynthParam } from '../generated/protocol'
import { DEFAULT_PARAMS } from '../params'

/**
 * The project as the page sees it: a reactive mirror that only the dispatcher writes (`run`).
 * Today it holds the synth's settings, replayed into whichever engine plays; it will be fed by
 * the project's events once the project model reaches the page (§ 2.6 of the MVP breakdown).
 */
export class ProjectStore {
  params = $state<Record<SynthParam, number>>({ ...DEFAULT_PARAMS })
}
