/**
 * What a pointer does in a lane (AM-4), as pure functions: where a point is on screen, which point is under the pointer,
 * which points a box holds, where a click adds a point, and what a drag asks of the project. A command the project refuses
 * is a page bug (`Dispatcher.run`), so every gesture is clamped to what `crates/commands/src/automation.rs` accepts: a
 * tick between 0 and 2^40, one point per tick, a value inside the parameter's range and rounded to its step. No DOM.
 */

import type { Point } from '../../../lib/generated/protocol'
import { snapTick, tickToX, xToTick, type View } from '../../../lib/components/timegrid/grid'
import { MAX_TICK } from '../arrangement'
import { lowerBound, pointAtTick, type Axis, type LanePoint } from './model'

/** The radius of the grab zone around a point, in CSS pixels. */
export const POINT_RADIUS = 8
/** The room kept above and below the value range inside a lane's row. */
export const LANE_PAD = 10

/** Where a lane's row is in the stage. */
export interface Geo {
  top: number
  height: number
}

const clamp01 = (n: number): number => Math.min(1, Math.max(0, n))

/** The y of a position (0 bottom, 1 top) in a lane's row. */
export const yOfPos = (position: number, geo: Geo): number => geo.top + LANE_PAD + (1 - clamp01(position)) * Math.max(1, geo.height - 2 * LANE_PAD)
/** The position a y stands for, kept between 0 and 1. */
export const posOfY = (y: number, geo: Geo): number => clamp01(1 - (y - geo.top - LANE_PAD) / Math.max(1, geo.height - 2 * LANE_PAD))

/** The point under (x, y), the nearest within the radius, or null. Only the points near x are looked at. */
export function hitPoint(points: readonly LanePoint[], axis: Axis, x: number, y: number, view: View, geo: Geo, radius = POINT_RADIUS): LanePoint | null {
  const from = lowerBound(points, xToTick(x - radius, view))
  const to = lowerBound(points, xToTick(x + radius, view) + 1)
  let best: LanePoint | null = null
  let bestDistance = radius * radius
  for (let i = from; i < to; i++) {
    const p = points[i]!
    const dx = tickToX(p.tick, view) - x
    const dy = yOfPos(axis.pos(p.value), geo) - y
    const d = dx * dx + dy * dy
    if (d <= bestDistance) {
      best = p
      bestDistance = d
    }
  }
  return best
}

/** The ticks of the points inside a box (stage coordinates, any corner order). */
export function pointsInBox(points: readonly LanePoint[], axis: Axis, ax: number, ay: number, bx: number, by: number, view: View, geo: Geo): number[] {
  const x0 = Math.min(ax, bx)
  const x1 = Math.max(ax, bx)
  const y0 = Math.min(ay, by)
  const y1 = Math.max(ay, by)
  const from = lowerBound(points, xToTick(x0, view))
  const to = lowerBound(points, xToTick(x1, view) + 1)
  const ticks: number[] = []
  for (let i = from; i < to; i++) {
    const p = points[i]!
    const y = yOfPos(axis.pos(p.value), geo)
    if (y >= y0 && y <= y1) ticks.push(p.tick)
  }
  return ticks
}

/** The point a click at (x, y) adds: the tick snapped like the clips' (a whole tick without a step), the value from the height. */
export function newPoint(axis: Axis, view: View, geo: Geo, x: number, y: number, step: number | null): LanePoint {
  const tick = Math.min(MAX_TICK, snapTick(xToTick(x, view), step))
  return { tick, value: axis.at(posOfY(y, geo)), shape: 'linear' }
}

/** A point as the commands take it. */
export const asPoint = (p: LanePoint): Point => ({ tick: p.tick, value: p.value, shape: p.shape })

/** How far a drag goes, in ticks and in positions along the axis. */
export interface Delta {
  ticks: number
  position: number
}

/**
 * The move of a drag: the grabbed point's tick follows the pointer on the snap step, every selected point moves by the
 * same ticks and none leaves the timeline; the value follows the pointer's height by the same distance along the axis. A
 * vertical move shorter than the drag's slop is none, so that a horizontal drag keeps the values.
 */
export function dragDelta(
  selection: readonly LanePoint[],
  grabbed: LanePoint,
  pointerTick: number,
  grabTick: number,
  pointerPos: number,
  grabPos: number,
  step: number | null,
  verticalPx: number,
): Delta {
  if (selection.length === 0) return { ticks: 0, position: 0 }
  const wanted = snapTick(grabbed.tick + (pointerTick - grabTick), step) - grabbed.tick
  const lowest = selection[0]!.tick
  const highest = selection[selection.length - 1]!.tick
  const ticks = Math.round(Math.min(MAX_TICK - highest, Math.max(-lowest, wanted))) + 0
  return { ticks, position: verticalPx === 0 ? 0 : pointerPos - grabPos }
}

/** The selection after the move (the same points, elsewhere). A value is kept to the axis only when it moved. */
export function moveSelection(selection: readonly LanePoint[], delta: Delta, axis: Axis): LanePoint[] {
  return selection.map((p) => ({
    tick: p.tick + delta.ticks,
    value: delta.position === 0 ? p.value : axis.at(axis.pos(p.value) + delta.position),
    shape: p.shape,
  }))
}

/** What a drag asks of the project: one update, or the removal of the old ticks and the addition of the new points. */
export type MovePlan = { kind: 'update'; points: Point[] } | { kind: 'move'; remove: number[]; add: Point[] }

/**
 * The commands of a drag, or null when nothing changed. A move in time removes the old points and adds the new ones (the
 * commands move no point in time); a point that lands on a point that does not move replaces it. The plan is sent in one
 * history group.
 */
export function planMove(all: readonly LanePoint[], selection: readonly LanePoint[], moved: readonly LanePoint[]): MovePlan | null {
  if (selection.length === 0) return null
  const changed = moved.filter((m, i) => m.tick !== selection[i]!.tick || m.value !== selection[i]!.value)
  if (changed.length === 0) return null
  if (moved.every((m, i) => m.tick === selection[i]!.tick)) return { kind: 'update', points: changed.map(asPoint) }
  const own = new Set(selection.map((p) => p.tick))
  const replaced = moved.filter((m) => !own.has(m.tick) && pointAtTick(all, m.tick) !== null).map((m) => m.tick)
  return { kind: 'move', remove: [...selection.map((p) => p.tick), ...replaced].sort((a, b) => a - b), add: moved.map(asPoint) }
}

/** The lane's points as they would be after the drag: the selection leaves its ticks, takes the new ones, and replaces what it lands on. */
export function previewPoints(all: readonly LanePoint[], selectedTicks: ReadonlySet<number>, moved: readonly LanePoint[]): LanePoint[] {
  const landing = new Set(moved.map((m) => m.tick))
  const kept = all.filter((p) => !selectedTicks.has(p.tick) && !landing.has(p.tick))
  const out = kept.concat(moved)
  out.sort((a, b) => a.tick - b.tick)
  return out
}
