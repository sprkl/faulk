import type { FeedbackReport } from './engine/feedback'
import type { ProjectView } from './generated/protocol'

/**
 * The tracks that no longer follow their timeline: the launch grid holds them, either with a clip
 * that plays (or is about to stop) or with none (after a stop), and their arrangement is silent
 * until `launch.back_to_arrangement`. Read from the engine's report, put back in terms of track
 * ids; a report of another epoch than the slot table's, or about a slot the page has no track
 * for, is ignored (the same rule as the launch grid's cells).
 */
export function offTimeline(report: Pick<FeedbackReport, 'epoch' | 'clips' | 'held'> | null, slots: ProjectView['slots'] | undefined): Set<string> {
  const off = new Set<string>()
  if (!report || !slots || report.epoch !== slots.epoch) return off
  const ids = new Map<number, string>()
  for (const [id, slot] of Object.entries(slots.tracks)) if (slot !== undefined) ids.set(slot, id)
  const add = (slot: number) => {
    const id = ids.get(slot)
    if (id) off.add(id)
  }
  for (const slot of report.held) add(slot)
  for (const { track, state } of report.clips) if (state === 'playing' || state === 'stopping') add(track)
  return off
}
