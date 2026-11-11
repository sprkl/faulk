/**
 * The automation lanes at the keyboard (#769), as pure functions: which key does what on a lane's head and on a point of the
 * lane (a cell of the timeline's grid), where the keyboard enters a lane, a value moved by one step, a point moved by one snap
 * step, and the one command a ramp is. Every edit stays one of the commands of `crates/commands/src/automation.rs`, kept to
 * what they accept (a tick between 0 and 2^40, one point per tick, a value in range and on the step). No DOM.
 */

import type { Point, Shape } from '../../../lib/generated/protocol'
import { MAX_TICK } from '../arrangement'
import { lowerBound, type Axis, type LanePoint } from './model'

type Keys = Pick<KeyboardEvent, 'key' | 'code' | 'altKey' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'repeat' | 'isComposing'>

/**
 * I (or Insert): a point at the playhead. By position (`code`), so the key is the same on every layout. Like Q in the piano roll
 * and R below, it plays nothing (`KEY_OFFSETS` has no I): a letter plays or it is a shortcut, never both (UI-3).
 */
export const POINT_CODE = 'KeyI'
/** R: a ramp from the playhead to a position. It plays nothing. */
export const RAMP_CODE = 'KeyR'

/** What a key does on a lane's head (any of its buttons focused). */
export type LaneAction = 'point' | 'ramp' | 'enter'

/** The keys a lane and its points share: I (or Insert) and R, with no modifier and no repeat. */
function sharedKey(event: Keys): 'point' | 'ramp' | null {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.repeat || event.isComposing) return null
  if (event.code === POINT_CODE || event.key === 'Insert') return 'point'
  if (event.code === RAMP_CODE) return 'ramp'
  return null
}

/** The key on a lane's head: I or Insert adds a point at the playhead, R opens the ramp, Right goes into the lane's points. */
export function laneKey(event: Keys): LaneAction | null {
  const shared = sharedKey(event)
  if (shared) return shared
  if (event.key === 'ArrowRight' && !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey) return 'enter'
  return null
}

/** What a key does on a focused point. */
export type PointAction =
  | { kind: 'go'; to: 'prev' | 'next' | 'first' | 'last' }
  | { kind: 'edit' }
  | { kind: 'delete' }
  | { kind: 'value'; direction: 1 | -1; big: boolean }
  | { kind: 'move'; direction: 1 | -1 }
  | { kind: 'point' }
  | { kind: 'ramp' }
  | { kind: 'leave' }

/**
 * The key on a point: Left and Right go to the previous and the next point, Home and End to the first and the last, Enter types
 * its value, Delete or Backspace removes it, Alt+Up and Down change its value by a step (a big one with Shift), Alt+Left and Right
 * move it by the snap step, I (Insert) and R as on the lane's head, Escape goes back to the head. Ctrl or Cmd belongs to the page.
 */
export function pointKey(event: Keys): PointAction | null {
  const shared = sharedKey(event)
  if (shared) return { kind: shared }
  if (event.ctrlKey || event.metaKey || event.isComposing) return null
  if (event.altKey) {
    if (event.key === 'ArrowUp' || event.key === 'ArrowDown') return { kind: 'value', direction: event.key === 'ArrowUp' ? 1 : -1, big: event.shiftKey }
    if (!event.shiftKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) return { kind: 'move', direction: event.key === 'ArrowRight' ? 1 : -1 }
    return null
  }
  if (event.shiftKey) return null
  switch (event.key) {
    case 'ArrowLeft':
      return { kind: 'go', to: 'prev' }
    case 'ArrowRight':
      return { kind: 'go', to: 'next' }
    case 'Home':
      return { kind: 'go', to: 'first' }
    case 'End':
      return { kind: 'go', to: 'last' }
    case 'Enter':
      return event.repeat ? null : { kind: 'edit' }
    case 'Delete':
    case 'Backspace':
      return { kind: 'delete' }
    case 'Escape':
      return { kind: 'leave' }
  }
  return null
}

/** The point the keyboard enters a lane on: the first at or after the playhead, else the last; -1 for a lane with no point. */
export function entryIndex(points: readonly LanePoint[], tick: number): number {
  if (points.length === 0) return -1
  return Math.min(points.length - 1, lowerBound(points, tick))
}

/** The index a `go` reaches from `index`, kept to the lane (the same index at an end). */
export function goIndex(length: number, index: number, to: 'prev' | 'next' | 'first' | 'last'): number {
  if (length === 0) return -1
  if (to === 'first') return 0
  if (to === 'last') return length - 1
  return Math.max(0, Math.min(length - 1, index + (to === 'next' ? 1 : -1)))
}

/** One step along the axis: 1 % of its travel, 10 % for a big one. */
const STEP = 0.01
const BIG_STEP = 0.1

/**
 * The value one step up or down from `value`, kept to the range and the parameter's step. A parameter whose step is coarser
 * than the move (a choice, a whole number) still moves by one of its steps; at a bound the value stays.
 */
export function stepValue(axis: Axis, value: number, direction: 1 | -1, big: boolean): number {
  const from = axis.pos(value)
  let move = big ? BIG_STEP : STEP
  for (;;) {
    const next = axis.at(Math.min(1, Math.max(0, from + direction * move)))
    if (next !== value || move >= 1) return next
    move *= 2
  }
}

/** The tick one snap step earlier or later (the next multiple of `step`), kept to the timeline; null when the point cannot go further. */
export function nudgedTick(tick: number, direction: 1 | -1, step: number): number | null {
  const unit = Math.max(1, Math.round(step))
  const next = direction > 0 ? (Math.floor(tick / unit) + 1) * unit : (Math.ceil(tick / unit) - 1) * unit
  if (next < 0 || next > MAX_TICK) return null
  return next
}

/** One end of a ramp: a tick and a value already on the axis (kept to the range and the step). */
export interface RampEnd {
  tick: number
  value: number
}

/** The one `automation.points.restore` a ramp is: the two points it writes and the ticks of the points between them it takes out. */
export interface RampPlan {
  points: Point[]
  remove: number[]
}

/**
 * The curve a new point on `tick` takes so that the lane after it goes on as it did: the curve of a point already there, else
 * that of the segment it lands in (the point before it: a point inside a held step keeps the hold), else linear.
 */
export function shapeAt(points: readonly LanePoint[], tick: number): Shape {
  const i = lowerBound(points, tick)
  if (points[i]?.tick === tick) return points[i]!.shape
  return i > 0 ? points[i - 1]!.shape : 'linear'
}

/**
 * A straight ramp between the playhead's point (`from`, the lane's value there) and the typed one (`to`), in either order. The
 * earlier point gets the linear curve (it starts the ramp); the later one keeps the curve the lane had there (`shapeAt`), so that
 * what follows the ramp sounds as before. The points strictly between are taken out, so that the segment is exactly the ramp.
 * Null when both ends are on one tick.
 */
export function rampPlan(points: readonly LanePoint[], from: RampEnd, to: RampEnd): RampPlan | null {
  if (from.tick === to.tick) return null
  const [early, late] = from.tick < to.tick ? [from, to] : [to, from]
  const lateShape = shapeAt(points, late.tick)
  const remove: number[] = []
  for (let i = lowerBound(points, early.tick + 1); i < points.length && points[i]!.tick < late.tick; i++) remove.push(points[i]!.tick)
  return {
    points: [
      { tick: early.tick, value: early.value, shape: 'linear' },
      { tick: late.tick, value: late.value, shape: lateShape },
    ],
    remove,
  }
}
