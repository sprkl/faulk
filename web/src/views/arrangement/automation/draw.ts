/**
 * Canvas drawing of the automation lanes (AM-4), over the shared time grid and in the same frame as the clips: for each
 * lane in sight its row, the range at both ends, the value line, the points, the selection, the box being drawn and the
 * readout of the point being moved. Only the points in sight are looked at (a binary search), and a lane denser than
 * one point per pixel is drawn as one vertical stroke per pixel column, so that a frame costs the width of the screen,
 * not the number of points.
 */

import { tickToX, xToTick } from '../../../lib/components/timegrid/grid'
import { RULER_HEIGHT, trackInk, type BodyFrame } from '../../../lib/components/timegrid/render'
import { yOfPos } from './gestures'
import { lowerBound, segmentValue, type Axis, type LanePoint } from './model'
import type { AutomationTarget, Shape } from '../../../lib/generated/protocol'

/** A lane to draw. */
export interface LaneScene {
  /** The top of the lane's row in the stage, and its height. */
  top: number
  height: number
  axis: Axis
  /** What the lane drives: its segments are drawn as the engine plays them (`segmentValue`). */
  target: AutomationTarget
  /** The points to draw (the preview while a drag is in progress). */
  points: readonly LanePoint[]
  selected: ReadonlySet<number>
  color: string
  /** The point whose value is read out, if any. */
  readout: LanePoint | null
}

export interface AutomationFrame {
  lanes: readonly LaneScene[]
  /** The box being drawn, in stage coordinates. */
  box: { x0: number; y0: number; x1: number; y1: number } | null
}

/** Points are drawn one by one only when they are at least this far apart (CSS pixels). */
const GLYPH_MIN_GAP = 6
const GLYPH_MAX = 3000
const GLYPH = 3.5

/** A curved segment is drawn with a vertex about every this many CSS pixels (#881), and at most this many per segment. */
const SAMPLE_PX = 3
const MAX_SAMPLES = 64

/**
 * Whether the segment that leaves a point of this shape is a straight line on the lane's axis: a linear segment on an axis
 * along which the engine draws its line (`Axis.straight`: the fader's travel of a volume lane, #389, a pan, a parameter on a
 * linear scale). Any other is drawn through the values the engine plays (`segmentValue`): a curve (#881), or a straight line
 * in Hz on a logarithmic axis.
 */
function straight(shape: Shape, axis: Axis): boolean {
  return shape === 'linear' && axis.straight === true
}

/** How a frame went: the points looked at, for the tests and the measure. */
export interface LaneStats {
  visible: number
  dense: boolean
}

export function drawLane(ctx: CanvasRenderingContext2D, frame: BodyFrame, lane: LaneScene): LaneStats {
  const { width, view, colors, dpr } = frame
  const { top, height, axis, points } = lane
  const geo = { top, height }
  const crisp = (v: number) => (Math.round(v * dpr) + 0.5) / dpr

  ctx.save()
  ctx.beginPath()
  ctx.rect(0, Math.max(top, RULER_HEIGHT), width, height - Math.max(0, RULER_HEIGHT - top))
  ctx.clip()

  // The row: a tint, the line under it, the default value, the range at both ends.
  ctx.fillStyle = colors.ruler
  ctx.globalAlpha = 0.45
  ctx.fillRect(0, top, width, height)
  ctx.globalAlpha = 1
  ctx.strokeStyle = colors.line
  ctx.lineWidth = 1
  ctx.beginPath()
  ctx.moveTo(0, crisp(top + height))
  ctx.lineTo(width, crisp(top + height))
  ctx.stroke()
  ctx.save()
  ctx.setLineDash([3, 4])
  ctx.globalAlpha = 0.6
  ctx.beginPath()
  const home = crisp(yOfPos(axis.home, geo))
  ctx.moveTo(0, home)
  ctx.lineTo(width, home)
  ctx.stroke()
  ctx.restore()
  ctx.fillStyle = colors.barLine
  ctx.globalAlpha = 0.9
  ctx.textBaseline = 'middle'
  ctx.fillText(axis.format(axis.max), 4, yOfPos(1, geo) + 1)
  ctx.fillText(axis.format(axis.min), 4, yOfPos(0, geo) - 1)
  ctx.globalAlpha = 1

  const n = points.length
  const stats: LaneStats = { visible: 0, dense: false }
  if (n > 0) {
    const first = Math.max(0, lowerBound(points, xToTick(-6, view)) - 1)
    const last = Math.min(n - 1, lowerBound(points, xToTick(width + 6, view)))
    const count = last - first + 1
    stats.visible = count
    const xOf = (i: number) => tickToX(points[i]!.tick, view)
    const yOf = (i: number) => yOfPos(axis.pos(points[i]!.value), geo)
    ctx.strokeStyle = trackInk(lane.color, colors)
    ctx.fillStyle = trackInk(lane.color, colors)
    ctx.lineWidth = 1.5
    ctx.lineJoin = 'round'
    if (count > width) {
      // Denser than one point a pixel: the lowest and the highest value of each pixel column.
      stats.dense = true
      ctx.beginPath()
      let column = Number.NaN
      let low = 0
      let high = 0
      const flush = () => {
        if (!Number.isNaN(column)) {
          ctx.moveTo(column + 0.5, low)
          ctx.lineTo(column + 0.5, Math.max(high, low + 1))
        }
      }
      for (let i = first; i <= last; i++) {
        const x = Math.floor(xOf(i))
        const y = yOf(i)
        if (x !== column) {
          flush()
          column = x
          low = y
          high = y
        } else {
          low = Math.min(low, y)
          high = Math.max(high, y)
        }
      }
      flush()
      ctx.lineWidth = 1
      ctx.stroke()
    } else {
      ctx.beginPath()
      const x0 = xOf(first)
      const y0 = yOf(first)
      // Before the first point the lane holds the first value (AM-2): a solid line from bar 1 (the start of the timeline).
      const start = Math.max(-6, tickToX(0, view))
      const held = first === 0 && x0 > start
      if (held) {
        ctx.moveTo(start, y0)
        ctx.lineTo(x0, y0)
      } else ctx.moveTo(x0, y0)
      for (let i = first; i < last; i++) {
        const x = xOf(i + 1)
        const y = yOf(i + 1)
        const a = points[i]!
        // `step` holds the value to the next point; every other shape goes to it along the curve the engine plays.
        if (a.shape === 'step') ctx.lineTo(x, yOf(i))
        else if (!straight(a.shape, axis)) {
          const xa = xOf(i)
          const b = points[i + 1]!
          // About one vertex every SAMPLE_PX of the part in sight, at most MAX_SAMPLES per segment.
          const from = Math.max(xa, -6)
          const to = Math.min(x, width + 6)
          const steps = Math.min(MAX_SAMPLES, Math.max(1, Math.ceil((to - from) / SAMPLE_PX)))
          for (let k = 0; k < steps && to > from; k++) {
            const px = from + ((to - from) * k) / steps
            const value = segmentValue(a, b, (px - xa) / (x - xa), lane.target)
            ctx.lineTo(px, yOfPos(axis.pos(value), geo))
          }
        }
        ctx.lineTo(x, y)
      }
      // After the last point it holds the last value.
      if (last === n - 1) ctx.lineTo(width + 6, yOf(last))
      ctx.stroke()
      // The area under the line, lightly.
      ctx.lineTo(width + 6, yOfPos(0, geo))
      ctx.lineTo(held ? start : x0, yOfPos(0, geo))
      ctx.closePath()
      ctx.globalAlpha = 0.12
      ctx.fill()
      ctx.globalAlpha = 1

      // The points, one by one when there is room for them.
      if (count <= GLYPH_MAX && width / Math.max(1, count) >= GLYPH_MIN_GAP) {
        ctx.beginPath()
        for (let i = first; i <= last; i++) {
          if (lane.selected.has(points[i]!.tick)) continue
          const x = xOf(i)
          if (x < -GLYPH || x > width + GLYPH) continue
          ctx.rect(x - GLYPH, yOf(i) - GLYPH, 2 * GLYPH, 2 * GLYPH)
        }
        ctx.fill()
        ctx.beginPath()
        for (let i = first; i <= last; i++) {
          if (!lane.selected.has(points[i]!.tick)) continue
          const x = xOf(i)
          if (x < -GLYPH || x > width + GLYPH) continue
          ctx.rect(x - GLYPH - 1.5, yOf(i) - GLYPH - 1.5, 2 * GLYPH + 3, 2 * GLYPH + 3)
        }
        ctx.fillStyle = colors.text
        ctx.fill()
        ctx.lineWidth = 1.5
        ctx.strokeStyle = trackInk(lane.color, colors)
        ctx.stroke()
      }
    }
  }

  // The readout of the point being moved or pointed at.
  if (lane.readout) {
    const p = lane.readout
    const x = tickToX(p.tick, view)
    const y = yOfPos(axis.pos(p.value), geo)
    const text = axis.format(p.value)
    const w = ctx.measureText(text).width + 10
    const left = Math.min(Math.max(2, x + 10), width - w - 2)
    const above = y - 22 >= top + 2
    const by = above ? y - 22 : y + 8
    ctx.fillStyle = colors.bg
    ctx.globalAlpha = 0.92
    ctx.fillRect(left, by, w, 16)
    ctx.globalAlpha = 1
    ctx.strokeStyle = trackInk(lane.color, colors)
    ctx.lineWidth = 1
    ctx.strokeRect(left + 0.5, by + 0.5, w - 1, 15)
    ctx.fillStyle = colors.text
    ctx.textBaseline = 'middle'
    ctx.fillText(text, left + 5, by + 8.5)
  }
  ctx.restore()
  return stats
}

/** Draws every lane in sight and the selection box. */
export function drawAutomation(ctx: CanvasRenderingContext2D, frame: BodyFrame, scene: AutomationFrame): LaneStats {
  const total: LaneStats = { visible: 0, dense: false }
  for (const lane of scene.lanes) {
    if (lane.top + lane.height < RULER_HEIGHT || lane.top > frame.height) continue
    const s = drawLane(ctx, frame, lane)
    total.visible += s.visible
    total.dense = total.dense || s.dense
  }
  const box = scene.box
  if (box) {
    ctx.save()
    ctx.fillStyle = frame.colors.accent
    ctx.strokeStyle = frame.colors.accent
    ctx.globalAlpha = 0.12
    ctx.fillRect(box.x0, box.y0, box.x1 - box.x0, box.y1 - box.y0)
    ctx.globalAlpha = 0.9
    ctx.lineWidth = 1
    ctx.strokeRect(box.x0 + 0.5, box.y0 + 0.5, box.x1 - box.x0, box.y1 - box.y0)
    ctx.restore()
  }
  return total
}
