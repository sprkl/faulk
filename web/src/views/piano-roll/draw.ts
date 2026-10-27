/**
 * Canvas drawing of the piano roll: the pitch rows and the notes over the shared grid (`drawRoll`,
 * called by `TimeGrid` in its frame), the keyboard column (`drawKeys`) and the velocity lane
 * (`drawLane`). Notes are batched by style and culled to the visible window, so a frame costs the
 * notes in sight, not the notes of the clip.
 */

import { RULER_HEIGHT, type BodyFrame, type Colors } from '../../lib/components/timegrid/render'
import { tickToX, type View } from '../../lib/components/timegrid/grid'
import type { Note, TickLoop } from '../../lib/generated/protocol'
import { MAX_PITCH, MAX_VELOCITY, MIN_NOTE_PX, MIN_PITCH, isBlackKey, noteKey, pitchName, pitchTop, type Layout, type Rect } from './roll'

/** What the roll draws besides the grid. */
export interface RollFrame {
  layout: Layout
  /** The notes of the clip, as the project holds them. */
  notes: readonly Note[]
  /** Keys of the notes drawn elsewhere while a gesture moves them (their preview is in `ghosts`). */
  hidden: ReadonlySet<string>
  selected: ReadonlySet<string>
  /** The notes of a gesture in progress, drawn as selected. */
  ghosts: readonly Note[]
  /** The rectangle of a selection drag. */
  band: Rect | null
  clipLength: number
  loop: TickLoop | null
  /** The names of the keys the instrument answers to (a drum kit), or null for a pitched instrument: the other rows are dimmed. */
  names: ReadonlyMap<number, string> | null
}

const VELOCITY_FLOOR = 0.3
const alphaOf = (velocity: number): number => VELOCITY_FLOOR + (1 - VELOCITY_FLOOR) * (velocity / MAX_VELOCITY)

/** The pitches whose rows meet the body `[RULER_HEIGHT, height]`, highest first. */
function visiblePitches(layout: Layout, height: number): [number, number] {
  const high = Math.min(MAX_PITCH, MAX_PITCH - Math.floor((RULER_HEIGHT - layout.top + layout.scrollY) / layout.rowHeight))
  const low = Math.max(MIN_PITCH, MAX_PITCH - Math.floor((height - layout.top + layout.scrollY) / layout.rowHeight))
  return [high, low]
}

export function drawRoll(ctx: CanvasRenderingContext2D, frame: BodyFrame, roll: RollFrame): void {
  const { width, height, view, colors, dpr } = frame
  const { layout } = roll
  const crisp = (v: number) => (Math.round(v * dpr) + 0.5) / dpr
  ctx.save()
  // Nothing of the roll is drawn over the ruler.
  ctx.beginPath()
  ctx.rect(0, RULER_HEIGHT, width, height - RULER_HEIGHT)
  ctx.clip()

  const [high, low] = visiblePitches(layout, height)
  ctx.fillStyle = colors.ruler
  ctx.globalAlpha = 0.55
  for (let pitch = high; pitch >= low; pitch--) {
    if (isBlackKey(pitch)) ctx.fillRect(0, pitchTop(pitch, layout), width, layout.rowHeight)
  }
  ctx.globalAlpha = 0.6
  ctx.lineWidth = 1
  ctx.strokeStyle = colors.line
  ctx.beginPath()
  for (let pitch = high; pitch >= low; pitch--) {
    if (pitch % 12 === 0) continue
    const y = crisp(pitchTop(pitch, layout) + layout.rowHeight)
    ctx.moveTo(0, y)
    ctx.lineTo(width, y)
  }
  ctx.stroke()
  // The octave lines (under each C) stand out.
  ctx.strokeStyle = colors.barLine
  ctx.globalAlpha = 0.5
  ctx.beginPath()
  for (let pitch = high; pitch >= low; pitch--) {
    if (pitch % 12 !== 0) continue
    const y = crisp(pitchTop(pitch, layout) + layout.rowHeight)
    ctx.moveTo(0, y)
    ctx.lineTo(width, y)
  }
  ctx.stroke()
  ctx.globalAlpha = 1

  // Rows with no sound on a drum kit stay drawn, dimmed.
  if (roll.names) {
    ctx.fillStyle = colors.bg
    ctx.globalAlpha = 0.5
    for (let pitch = high; pitch >= low; pitch--) {
      if (!roll.names.has(pitch)) ctx.fillRect(0, pitchTop(pitch, layout), width, layout.rowHeight)
    }
    ctx.globalAlpha = 1
  }

  // Past the end of the clip nothing can be drawn: dim it.
  const end = tickToX(roll.clipLength, view)
  if (end < width) {
    ctx.fillStyle = colors.bg
    ctx.globalAlpha = 0.6
    ctx.fillRect(Math.max(0, end), RULER_HEIGHT, width - Math.max(0, end), height - RULER_HEIGHT)
    ctx.globalAlpha = 1
    ctx.strokeStyle = colors.text
    ctx.beginPath()
    ctx.moveTo(crisp(end), RULER_HEIGHT)
    ctx.lineTo(crisp(end), height)
    ctx.stroke()
  }
  ctx.restore()

  if (roll.loop) drawLoop(ctx, roll.loop, view, width, colors)

  ctx.save()
  ctx.beginPath()
  ctx.rect(0, RULER_HEIGHT, width, height - RULER_HEIGHT)
  ctx.clip()
  drawNotes(ctx, roll, frame)
  if (roll.band) {
    ctx.fillStyle = colors.accent
    ctx.globalAlpha = 0.15
    ctx.fillRect(roll.band.x, roll.band.y, roll.band.w, roll.band.h)
    ctx.globalAlpha = 0.9
    ctx.strokeStyle = colors.accent
    ctx.strokeRect(crisp(roll.band.x), crisp(roll.band.y), Math.round(roll.band.w), Math.round(roll.band.h))
    ctx.globalAlpha = 1
  }
  ctx.restore()
}

/** The loop is shown on the ruler: a band between its ends, and a line at each end down the body. */
function drawLoop(ctx: CanvasRenderingContext2D, loop: TickLoop, view: View, width: number, colors: Colors) {
  const left = tickToX(loop.start, view)
  const right = tickToX(loop.end, view)
  if (right < 0 || left > width) return
  ctx.fillStyle = colors.accent
  ctx.globalAlpha = 0.28
  ctx.fillRect(Math.max(0, left), 0, Math.min(width, right) - Math.max(0, left), RULER_HEIGHT - 1)
  ctx.globalAlpha = 0.9
  ctx.fillRect(Math.max(0, left), RULER_HEIGHT - 4, Math.min(width, right) - Math.max(0, left), 3)
  ctx.globalAlpha = 1
}

function drawNotes(ctx: CanvasRenderingContext2D, roll: RollFrame, frame: BodyFrame) {
  const { view, width, height, colors } = frame
  const { layout } = roll
  const hasHidden = roll.hidden.size > 0
  const rows = layout.rowHeight
  const visible: Note[] = []
  const chosen: Note[] = []
  for (const n of roll.notes) {
    const x = tickToX(n.start, view)
    if (x > width) break // sorted by start: nothing later is in sight
    if (x + Math.max(MIN_NOTE_PX, n.length * view.pxPerTick) < 0) continue
    const y = pitchTop(n.pitch, layout)
    if (y + rows < RULER_HEIGHT || y > height) continue
    const key = hasHidden || roll.selected.size > 0 ? noteKey(n) : ''
    if (hasHidden && roll.hidden.has(key)) continue
    if (roll.selected.size > 0 && roll.selected.has(key)) chosen.push(n)
    else visible.push(n)
  }
  const paint = (list: readonly Note[], selected: boolean) => {
    ctx.fillStyle = selected ? colors.text : colors.accent
    for (const n of list) {
      const x = tickToX(n.start, view)
      const w = Math.max(MIN_NOTE_PX, n.length * view.pxPerTick - 1)
      ctx.globalAlpha = selected ? 0.55 + 0.45 * (n.velocity / MAX_VELOCITY) : alphaOf(n.velocity)
      ctx.fillRect(x, pitchTop(n.pitch, layout) + 1, w, rows - 1)
    }
  }
  paint(visible, false)
  paint(chosen, true)
  paint(roll.ghosts, true)
  // A bright edge on the selected notes and the notes of a gesture, so that they read on any fill.
  ctx.globalAlpha = 1
  ctx.strokeStyle = colors.accent
  ctx.lineWidth = 1
  ctx.beginPath()
  for (const list of [chosen, roll.ghosts]) {
    for (const n of list) {
      const w = Math.max(MIN_NOTE_PX, n.length * view.pxPerTick - 1)
      ctx.rect(tickToX(n.start, view) + 0.5, pitchTop(n.pitch, layout) + 1.5, Math.max(1, w - 1), rows - 2)
    }
  }
  ctx.stroke()
  ctx.globalAlpha = 1
}

/** The keyboard column: one key per row, black keys narrower, the Cs named, the pitch played lit. */
export function drawKeys(ctx: CanvasRenderingContext2D, width: number, height: number, layout: Layout, colors: Colors, font: string, active: ReadonlySet<number>, names: ReadonlyMap<number, string> | null = null): void {
  ctx.fillStyle = colors.ruler
  ctx.fillRect(0, 0, width, height)
  ctx.save()
  ctx.beginPath()
  ctx.rect(0, layout.top, width, height - layout.top)
  ctx.clip()
  const [high, low] = visiblePitches(layout, height)
  ctx.font = font
  ctx.textBaseline = 'middle'
  for (let pitch = high; pitch >= low; pitch--) {
    const y = pitchTop(pitch, layout)
    const black = isBlackKey(pitch)
    ctx.fillStyle = active.has(pitch) ? colors.accent : black ? colors.line : colors.bg
    ctx.fillRect(0, y, black ? width * 0.62 : width, layout.rowHeight - 1)
    const name = names?.get(pitch)
    if (name !== undefined) {
      ctx.fillStyle = active.has(pitch) ? colors.bg : colors.text
      ctx.textAlign = 'left'
      ctx.fillText(name, 6, y + layout.rowHeight / 2)
      ctx.textAlign = 'start'
    } else if (pitch % 12 === 0) {
      ctx.fillStyle = active.has(pitch) ? colors.bg : colors.text
      ctx.fillText(pitchName(pitch), width - 28, y + layout.rowHeight / 2)
    }
    if (names && name === undefined) {
      // No sound on this key: the row is dimmed.
      ctx.fillStyle = colors.bg
      ctx.globalAlpha = 0.5
      ctx.fillRect(0, y, width, layout.rowHeight)
      ctx.globalAlpha = 1
    }
  }
  ctx.restore()
}

/** The velocity lane: one bar per note at its start, as high as its velocity, the selected ones bright. */
export function drawLane(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  view: View,
  notes: readonly Note[],
  selected: ReadonlySet<string>,
  ghosts: readonly Note[],
  hidden: ReadonlySet<string>,
  clipLength: number,
  colors: Colors,
  hover: string | null = null,
): void {
  ctx.fillStyle = colors.bg
  ctx.fillRect(0, 0, width, height)
  const end = tickToX(clipLength, view)
  if (end < width) {
    ctx.fillStyle = colors.ruler
    ctx.globalAlpha = 0.6
    ctx.fillRect(Math.max(0, end), 0, width - Math.max(0, end), height)
    ctx.globalAlpha = 1
  }
  ctx.strokeStyle = colors.line
  ctx.lineWidth = 1
  ctx.beginPath()
  for (const v of [0, 0.25, 0.5, 0.75]) {
    const y = Math.round(height * v) + 0.5
    ctx.moveTo(0, y)
    ctx.lineTo(width, y)
  }
  ctx.stroke()
  const bar = (n: Note, color: string) => {
    const x = tickToX(n.start, view)
    if (x < -4 || x > width) return
    const h = Math.max(2, (n.velocity / MAX_VELOCITY) * (height - 4))
    ctx.fillStyle = color
    ctx.fillRect(x, height - h, 3, h)
  }
  const hasHidden = hidden.size > 0
  ctx.globalAlpha = 0.85
  for (const n of notes) {
    const x = tickToX(n.start, view)
    if (x > width) break
    const key = hasHidden || selected.size > 0 ? noteKey(n) : ''
    if (hasHidden && hidden.has(key)) continue
    bar(n, selected.size > 0 && selected.has(key) ? colors.text : colors.accent)
  }
  ctx.globalAlpha = 1
  for (const n of ghosts) bar(n, colors.text)
  // The bar the next drag would change: wider and fully opaque.
  if (hover !== null) {
    const n = notes.find((c) => noteKey(c) === hover)
    if (n && !hidden.has(hover)) {
      const x = tickToX(n.start, view)
      const h = Math.max(2, (n.velocity / MAX_VELOCITY) * (height - 4))
      ctx.fillStyle = colors.text
      ctx.fillRect(x - 1, height - h, 5, h)
    }
  }
}
