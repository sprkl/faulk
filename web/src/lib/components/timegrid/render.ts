/** Canvas drawing of the time grid: the ruler band and the lines of the body, from `rulerMarks`. */

import type { Signature } from '../../../views/transport/time'
import { rulerMarks, tickToX, type View } from './grid'

export const RULER_HEIGHT = 28

/**
 * Colours, read from the CSS tokens (`readColors`) when the surface is sized and when the theme changes (#687), so a
 * frame never touches the style engine.
 */
export interface Colors {
  bg: string
  ruler: string
  line: string
  barLine: string
  text: string
  accent: string
  play: string
  /** How much of `text` a track hue takes where it is drawn as a line or a note (`--fk-track-shade`, 0 to 1). */
  trackShade: number
}

export interface Frame {
  /** CSS pixels. */
  width: number
  height: number
  view: View
  signature: Signature
  totalTicks: number
  /** The snapped tick under the pointer, or `null`. */
  hover: number | null
  colors: Colors
  font: string
}

/** What a host's `drawBody` is given, drawn over the grid in the same frame (the context is in CSS pixels). */
export interface BodyFrame {
  width: number
  height: number
  /** The view actually drawn (clamped). */
  view: View
  totalTicks: number
  colors: Colors
  dpr: number
}

/** The drawing surface the renderer needs: a subset of `CanvasRenderingContext2D`. */
export type Surface = Pick<CanvasRenderingContext2D, 'fillRect' | 'fillText' | 'beginPath' | 'moveTo' | 'lineTo' | 'stroke'> & {
  fillStyle: unknown
  strokeStyle: unknown
  lineWidth: number
  font: string
  textBaseline: unknown
  globalAlpha: number
}

/** The theme's colours, from the tokens of `element` (`styles/tokens.css`, the one place they are written). */
export function readColors(element: Element): Colors {
  const style = getComputedStyle(element)
  const token = (name: string) => style.getPropertyValue(name).trim()
  const shade = parseFloat(token('--fk-track-shade'))
  return {
    bg: token('--fk-bg'),
    ruler: token('--fk-surface'),
    line: token('--fk-line'),
    barLine: token('--fk-text-muted'),
    text: token('--fk-text'),
    accent: token('--fk-accent-fg'),
    play: token('--fk-play'),
    trackShade: Number.isFinite(shade) ? Math.min(1, Math.max(0, shade / 100)) : 0,
  }
}

const HEX = /^#([0-9a-f]{6})$/i
// A computed style's colour (`getComputedStyle(…).color`): three channels, maybe an alpha after them.
const COMPUTED = /^rgba?\((\d+),\s*(\d+),\s*(\d+)/

/** The red, green and blue of `#rrggbb` or of a computed colour, `null` for anything else. */
function channels(color: string): number[] | null {
  const hex = HEX.exec(color)
  if (hex) return [0, 2, 4].map((i) => parseInt(hex[1]!.slice(i, i + 2), 16))
  const computed = COMPUTED.exec(color)
  return computed ? [computed[1], computed[2], computed[3]].map(Number) : null
}
// The mixes of the current theme, by hue: emptied when the theme's text colour or shade changes, so a frame
// builds no key and a switch serves no stale mix.
const inks = new Map<string, string>()
let inksOf = { text: '', shade: 0 }

/**
 * A track hue as a line or a note is drawn on the background (#687): the hue mixed with the theme's text colour by
 * `trackShade` (none in the dark theme; enough ink in the light one to read at 3:1 on paper and on a clip's tint).
 * The hue itself stays the track's colour for fills. A colour that is neither `#rrggbb` nor a computed `rgb(…)` is
 * kept as it is.
 */
export function trackInk(color: string, colors: Pick<Colors, 'text' | 'trackShade'>): string {
  const shade = colors.trackShade
  if (!(shade > 0)) return color
  if (inksOf.text !== colors.text || inksOf.shade !== shade) {
    inks.clear()
    inksOf = { text: colors.text, shade }
  }
  const known = inks.get(color)
  if (known) return known
  const a = channels(color)
  const b = channels(colors.text)
  if (!a || !b) return color
  let out = '#'
  for (const i of [0, 1, 2])
    out += Math.round(a[i]! * (1 - shade) + b[i]! * shade)
      .toString(16)
      .padStart(2, '0')
  // A track's colour can be any `#rrggbb` (`track.set_color`): the cache never grows past a few hundred.
  if (inks.size >= 256) inks.clear()
  inks.set(color, out)
  return out
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
}

/** What `drawPlayhead` needs of a frame. */
export interface PlayheadFrame {
  width: number
  height: number
  /** The view actually drawn (clamped). */
  view: View
  /** The play position in ticks. */
  playhead: number
  colors: Colors
}

/**
 * Draws the play position: one vertical line of two CSS pixels over the whole height. The caller
 * clears its surface first; this is the cheap pass on its own canvas, so that a moving position does not
 * redraw the grid. Returns the x drawn, or `null` when the position is outside the view.
 */
export function drawPlayhead(ctx: Surface, frame: PlayheadFrame, dpr: number): number | null {
  const x = Math.round(tickToX(frame.playhead, frame.view) * dpr) / dpr
  if (!(x >= -1 && x <= frame.width + 1)) return null
  ctx.strokeStyle = frame.colors.play
  ctx.lineWidth = 2
  ctx.beginPath()
  ctx.moveTo(x, 0)
  ctx.lineTo(x, frame.height)
  ctx.stroke()
  ctx.lineWidth = 1
  return x
}
