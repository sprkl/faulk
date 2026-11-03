/**
 * Where the play position is inside the clip the piano roll edits, as pure functions of the engine's
 * report and the project. Two sources, as in the engine: a clip launched from the grid plays from the
 * launch boundary reported as its `anchor` (`clip_player.rs`), the arrangement plays a placement from
 * its `start`, `offset` into the clip (`arrangement.rs`); both read the clip with the same rule, which
 * `clipLocal` writes out. A track the grid holds does not play its arrangement.
 */

import type { ActiveClip, FeedbackReport } from '../../lib/engine/feedback'
import type { Clip, Track, TickLoop, ViewSlots } from '../../lib/generated/protocol'
import { PPQ } from '../transport/time'

/** The most the position is carried on past the last report, in milliseconds (a report comes every ~33). */
export const MAX_EXTRAPOLATION_MS = 100

/**
 * The tick of the clip at `elapsed` ticks after its position 0 (the engine's rule): played once, `elapsed`
 * through the clip's last tick and nothing after; looped `[ls, le)`, `elapsed` up to `le`, then
 * `ls + (elapsed - le) mod (le - ls)` (the same for a negative `elapsed`, so that a clip stays in phase).
 * `null` where the clip does not sound.
 */
export function clipLocal(elapsed: number, length: number, looping: TickLoop | null): number | null {
  if (!looping) return elapsed >= 0 && elapsed <= length ? elapsed : null
  const { start, end } = looping
  const span = end - start
  if (span <= 0) return null
  if (elapsed >= 0 && elapsed < end) return elapsed
  return start + ((((elapsed - end) % span) + span) % span)
}

/** The cells of one track in a report, by the slots of the project; `null` when the report names other slots (another epoch). */
function cellsOf(report: Pick<FeedbackReport, 'epoch' | 'clips'>, slots: ViewSlots, trackId: string): ActiveClip[] | null {
  const track = slots.tracks[trackId]
  if (report.epoch !== slots.epoch || track === undefined) return null
  return report.clips.filter((cell) => cell.track === track)
}

const sounding = (cell: ActiveClip) => cell.state === 'playing' || cell.state === 'stopping'

/** What plays the track, or `null` when the report cannot be read against the project's slots. */
export function sourceOf(
  report: Pick<FeedbackReport, 'epoch' | 'clips'>,
  slots: ViewSlots,
  trackId: string,
): { scene: number; anchor: number } | 'arrangement' | null {
  const cells = cellsOf(report, slots, trackId)
  if (!cells) return null
  const playing = cells.find(sounding)
  return playing ? { scene: playing.scene, anchor: playing.anchor } : 'arrangement'
}

/** The placement that plays at `tick`: the last one started, if it has not ended or been cut by the next one. */
function placementAt(track: Pick<Track, 'placements'>, tick: number) {
  const placed = Object.entries(track.placements)
    .filter(([, p]) => p !== undefined && 'ticks' in p.span)
    .sort(([, a], [, b]) => a!.start - b!.start)
  let found: (typeof placed)[number] | undefined
  let next: number | undefined
  for (const entry of placed) {
    if (entry[1]!.start <= tick) found = entry
    else {
      next = entry[1]!.start
      break
    }
  }
  const placement = found?.[1]
  if (!found || !placement || !('ticks' in placement.span)) return null
  const end = Math.min(placement.start + placement.span.ticks.length, next ?? Infinity)
  return tick < end ? { id: found[0], placement, offset: Math.max(0, placement.span.ticks.offset) } : null
}

/**
 * The tick to show when `target` comes after `shown`: the engine's report reaches the page a little late and
 * unevenly, so the carried tick lands a few milliseconds behind where the last frame put the bar. A step back of
 * less than `MAX_EXTRAPOLATION_MS` is held (the bar waits for the clock to catch up) instead of shown; a larger
 * one is a seek and is followed at once. With a transport `loop`, "behind" is read around the loop: a report taken just
 * before the loop end, received after the carried tick has already wrapped, is behind it by a few milliseconds and
 * is held too (a loop shorter than twice the tolerance cannot tell a wrap from a seek and is not held).
 */
export function settle(shown: number | null, target: number, bpm: number | null, loop: TickLoop | null = null): number {
  if (shown === null || bpm === null || !(bpm > 0)) return target
  const tolerance = (MAX_EXTRAPOLATION_MS / 1000) * (bpm / 60) * PPQ
  if (target < shown) return shown - target <= tolerance ? shown : target
  if (loop) {
    const span = loop.end - loop.start
    const inside = (tick: number) => tick >= loop.start && tick < loop.end
    if (span > 2 * tolerance && inside(shown) && inside(target) && span - (target - shown) <= tolerance) return shown
  }
  return target
}

export interface PlayheadInput {
  report: Pick<FeedbackReport, 'epoch' | 'clips'>
  slots: ViewSlots
  trackId: string
  track: Pick<Track, 'placements'>
  clipId: string
  clip: Pick<Clip, 'slot'>
  length: number
  looping: TickLoop | null
  /** The transport position, in ticks. */
  tick: number
  /** The placement the roll was opened on (#494): the arrangement's position shows only while it plays; none, any placement of the clip. */
  placement?: string | null
}

/**
 * The position inside the edited clip at the transport `tick`, or `null` when this clip does not sound
 * (another clip of the track plays, the arrangement is outside its placements, the clip has ended, the
 * report cannot be read against the project).
 */
export function playheadIn(input: PlayheadInput): number | null {
  const { report, slots, trackId, track, clipId, clip, length, looping, tick } = input
  const source = sourceOf(report, slots, trackId)
  if (source === null) return null
  if (source !== 'arrangement') {
    // The grid holds the track: only the clip of the cell that sounds has a playhead.
    const scene = clip.slot ? slots.scenes[clip.slot] : undefined
    return scene === source.scene ? clipLocal(tick - source.anchor, length, looping) : null
  }
  const at = placementAt(track, tick)
  if (!at || at.placement.clip !== clipId) return null
  if (input.placement != null && at.id !== input.placement) return null
  // The arrangement repeats a clip without a loop whole (#473): its loop is the clip.
  return clipLocal(tick - (at.placement.start - at.offset), length, looping ?? (length > 0 ? { start: 0, end: length } : null))
}

/**
 * The transport tick now, carried on from the last report at the tempo: the report comes about 30 times a
 * second, and the playhead moves at the display's rate. Never more than `MAX_EXTRAPOLATION_MS` past the
 * report (a late report must not run the bar away), and only while the transport runs. When the transport
 * loops (`loop`, the region in force) and the report is inside it, the carried tick wraps at the region's end
 * as the engine does, so that the bar does not run past the end and snap back at the next report.
 */
export function liveTick(reportTick: number, running: boolean, sinceReportMs: number, bpm: number | null, loop: TickLoop | null = null): number {
  if (!running || bpm === null || !(bpm > 0)) return reportTick
  const elapsed = Math.min(Math.max(sinceReportMs, 0), MAX_EXTRAPOLATION_MS)
  const tick = reportTick + (elapsed / 1000) * (bpm / 60) * PPQ
  if (loop && loop.end > loop.start && reportTick >= loop.start && reportTick < loop.end && tick >= loop.end)
    return loop.start + ((tick - loop.end) % (loop.end - loop.start))
  return tick
}
