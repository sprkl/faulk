/** Canvas drawing of the time grid: the ruler band and the lines of the body, from `rulerMarks`. */

import type { Signature } from '../../../views/transport/time'
import { rulerMarks, tickToX, type View } from './grid'

export const RULER_HEIGHT = 28

/** Colours, read once from the CSS tokens (`readColors`), so a frame never touches the style engine. */
export interface Colors {
  bg: string
  ruler: string
  line: string
  barLine: string
  text: string
  accent: string
  play: string
}

export interface Frame {
  /** CSS pixels. */
  width: number
  height: number
  view: View
  signature: Signature
  totalTicks: number
  /** The play position, or `null`. */
  playhead: number | null
  /** The snapped tick under the pointer, or `null`. */
  hover: number | null
  colors: Colors
  font: string
}

/** The drawing surface the renderer needs: a subset of `CanvasRenderingContext2D`. */
export type Surface = Pick<CanvasRenderingContext2D, 'fillRect' | 'fillText' | 'beginPath' | 'moveTo' | 'lineTo' | 'stroke'> & { fillStyle: unknown; strokeStyle: unknown; lineWidth: number; font: string; textBaseline: unknown; globalAlpha: number }

export function readColors(element: Element): Colors {
  const style = getComputedStyle(element)
  const token = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback
  return {
    bg: token('--fk-bg', '#131211'),
    ruler: token('--fk-surface', '#1a1917'),
    line: token('--fk-line', '#34312e'),
    barLine: token('--fk-text-muted', '#8f8a85'),
    text: token('--fk-text', '#ece9e6'),
    accent: token('--fk-accent-fg', '#9c92ff'),
    play: token('--fk-play', '#3fc168'),
  }
}

/**
 * Draws one frame. The context is scaled to CSS pixels by the caller (`setTransform(dpr...)`).
 * Lines are batched: one path and one stroke per style, so a frame costs a handful of draw calls
 * whatever the zoom. Each line sits on the middle of a device pixel for a crisp 1 px stroke.
 */
export function drawGrid(ctx: Surface, frame: Frame, dpr: number): void {
  const { width, height, view, colors } = frame
  const snap = (x: number) => (Math.round(x * dpr) + 0.5) / dpr
  const marks = rulerMarks(view, width, frame.signature, frame.totalTicks)

  ctx.fillStyle = colors.bg
  ctx.fillRect(0, 0, width, height)
  ctx.fillStyle = colors.ruler
  ctx.fillRect(0, 0, width, RULER_HEIGHT)

  // Past the end of the grid the body is dimmed.
  const end = tickToX(frame.totalTicks, view)
  if (end < width) {
    ctx.fillStyle = colors.ruler
    ctx.globalAlpha = 0.6
    ctx.fillRect(Math.max(0, end), RULER_HEIGHT, width - Math.max(0, end), height - RULER_HEIGHT)
    ctx.globalAlpha = 1
  }

  ctx.lineWidth = 1
  const batch = (kind: 'bar' | 'beat' | 'sub', top: number, color: string, alpha: number) => {
    ctx.strokeStyle = color
    ctx.globalAlpha = alpha
    ctx.beginPath()
    for (const mark of marks) {
      if (mark.kind !== kind) continue
      const x = snap(mark.x)
      ctx.moveTo(x, top)
      ctx.lineTo(x, height)
    }
    ctx.stroke()
  }
  batch('sub', RULER_HEIGHT - 6, colors.line, 0.45)
  batch('beat', RULER_HEIGHT - 10, colors.line, 1)
  batch('bar', 0, colors.barLine, 0.7)
  ctx.globalAlpha = 1

  ctx.strokeStyle = colors.line
  ctx.beginPath()
  const baseline = snap(RULER_HEIGHT - 1)
  ctx.moveTo(0, baseline)
  ctx.lineTo(width, baseline)
  ctx.stroke()

  ctx.font = frame.font
  ctx.textBaseline = 'top'
  ctx.fillStyle = colors.text
  for (const mark of marks) if (mark.label !== null) ctx.fillText(mark.label, snap(mark.x) + 4, 5)

  if (frame.hover !== null) {
    const x = snap(tickToX(frame.hover, view))
    ctx.strokeStyle = colors.accent
    ctx.beginPath()
    ctx.moveTo(x, 0)
    ctx.lineTo(x, height)
    ctx.stroke()
  }
  if (frame.playhead !== null) {
    const x = snap(tickToX(frame.playhead, view))
    if (x >= -1 && x <= width + 1) {
      ctx.strokeStyle = colors.play
      ctx.lineWidth = 2
      ctx.beginPath()
      ctx.moveTo(x, 0)
      ctx.lineTo(x, height)
      ctx.stroke()
      ctx.lineWidth = 1
    }
  }
}
