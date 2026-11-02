/**
 * Canvas drawing of the arrangement: the lanes, the placed clips with a small picture of their notes, the
 * selection, the preview of a gesture and the loop region. Called by `TimeGrid` in its frame, over the
 * shared ruler and lines. Only the blocks in sight are drawn.
 */

import { tickToX } from '../../lib/components/timegrid/grid'
import { RULER_HEIGHT, type BodyFrame } from '../../lib/components/timegrid/render'
import type { LoopRegion, Note } from '../../lib/generated/protocol'
import { BLOCK_INSET, laneHeight, laneTop, type Block } from './arrangement'
import type { Layout } from './automation/layout'

export interface ArrangementFrame {
  /** The rows of the tracks and of their lanes. */
  layout: Layout
  scrollY: number
  blocks: readonly Block[]
  /** The ids of the blocks a gesture is previewing elsewhere: not drawn at their place. */
  hidden: ReadonlySet<string>
  selected: ReadonlySet<string>
  /** The blocks of a gesture in progress, drawn as selected; `copy` blocks are new ones, the originals stay. */
  ghosts: readonly Block[]
  /** The loop region (the one being dragged, else the project's). */
  loop: LoopRegion | null
  loopOn: boolean
  /** The time selection (Shift+drag on the ruler), the range that Insert and Delete bars act on. */
  timeSel: LoopRegion | null
  /** The name of a clip without one. */
  fallbackName: string
  /** The tracks that make no sound (muted, or silenced by a solo): their clips are drawn at 40 %. */
  silent: ReadonlySet<string>
}

/** How much of its colour a clip of a silent track keeps. */
export const SILENT_ALPHA = 0.4

const NOTE_MIN_PX = 1.5
/** The height of a clip (inside its row) from which its notes are drawn under its name rather than across it. */
const NAME_ROOM = 30
const pitchRanges = new WeakMap<readonly Note[], [number, number]>()

function pitchRange(notes: readonly Note[]): [number, number] {
  let range = pitchRanges.get(notes)
  if (!range) {
    let low = 127
    let high = 0
    for (const note of notes) {
      low = Math.min(low, note.pitch)
      high = Math.max(high, note.pitch)
    }
    range = high < low ? [60, 60] : [low, high]
    pitchRanges.set(notes, range)
  }
  return range
}

export function drawArrangement(ctx: CanvasRenderingContext2D, frame: BodyFrame, scene: ArrangementFrame): void {
  const { width, height, view, colors, dpr } = frame
  const crisp = (v: number) => (Math.round(v * dpr) + 0.5) / dpr
  ctx.save()
  ctx.beginPath()
  ctx.rect(0, RULER_HEIGHT, width, height - RULER_HEIGHT)
  ctx.clip()

  // The lane separators.
  ctx.strokeStyle = colors.line
  ctx.lineWidth = 1
  ctx.beginPath()
  for (const row of scene.layout.rows) {
    const y = crisp(RULER_HEIGHT + row.top + row.height - scene.scrollY)
    if (y < RULER_HEIGHT || y > height) continue
    ctx.moveTo(0, y)
    ctx.lineTo(width, y)
  }
  ctx.stroke()

  // The loop region tints the body when the loop is on.
  const loop = scene.loop
  if (loop && scene.loopOn) {
    const x0 = tickToX(loop.start, view)
    const x1 = tickToX(loop.end, view)
    ctx.fillStyle = colors.accent
    ctx.globalAlpha = 0.07
    ctx.fillRect(x0, RULER_HEIGHT, x1 - x0, height - RULER_HEIGHT)
    ctx.globalAlpha = 1
  }

  const hidden = scene.hidden
  const drawBlock = (block: Block, selected: boolean, ghost: boolean) => {
    const x0 = tickToX(block.start, view)
    const x1 = tickToX(block.end, view)
    if (x1 < 0 || x0 > width) return
    const y = laneTop(scene.layout, block.row, scene.scrollY) + BLOCK_INSET
    const h = laneHeight(scene.layout, block.row) - 2 * BLOCK_INSET
    if (y + h < RULER_HEIGHT || y > height) return
    const w = Math.max(2, x1 - x0)
    const k = scene.silent.has(block.track) ? SILENT_ALPHA : 1
    ctx.globalAlpha = (ghost ? 0.7 : 1) * k
    ctx.fillStyle = block.color
    ctx.globalAlpha *= 0.32
    ctx.fillRect(x0, y, w, h)
    ctx.globalAlpha = (ghost ? 0.7 : 1) * k
    ctx.strokeStyle = selected ? colors.text : block.color
    ctx.lineWidth = selected ? 2 : 1
    ctx.strokeRect(x0 + 0.5, y + 0.5, w - 1, h - 1)

    // The notes, as lines across the part of the clip that the block shows.
    const notes = block.notes
    if (notes.length > 0 && w > 6) {
      ctx.save()
      ctx.beginPath()
      ctx.rect(x0 + 1, y + 1, w - 2, h - 2)
      ctx.clip()
      const [low, high] = pitchRange(notes)
      // Under the name when the row has room for both; a low row (the compact height) draws them across it, the name on top.
      const roomy = h >= NAME_ROOM
      const top = y + (roomy ? 16 : 2)
      const span = Math.max(1, h - (roomy ? 20 : 4))
      const rowH = Math.min(4, span / (high - low + 1))
      ctx.fillStyle = block.color
      ctx.globalAlpha = (ghost ? 0.7 : 0.95) * k
      // The clip is shown from its offset; a clip with a loop goes round it where the block outlasts it
      // (a few passes at most: a long block of a short loop is a texture, not a score).
      const cycle = block.looping ? block.looping.end : block.clipLength
      const loopStart = block.looping ? block.looping.start : 0
      let at = block.start
      let from = block.offset
      for (let pass = 0; pass < 64 && at < block.end; pass++) {
        const upto = Math.min(cycle, from + (block.end - at))
        for (const note of notes) {
          const s = Math.max(note.start, from)
          const e = Math.min(note.start + note.length, upto)
          if (e <= s) continue
          const nx = tickToX(at + (s - from), view)
          const nw = Math.max(NOTE_MIN_PX, (e - s) * view.pxPerTick)
          if (nx > width || nx + nw < 0) continue
          ctx.fillRect(nx, top + ((high - note.pitch) * (span - rowH)) / Math.max(1, high - low), nw, Math.max(1.5, rowH - 1))
        }
        if (!block.looping || upto <= from) break
        at += upto - from
        from = loopStart
      }
      ctx.restore()
    }
    // The name.
    if (w > 28) {
      ctx.globalAlpha = k
      ctx.fillStyle = colors.text
      ctx.textBaseline = 'top'
      ctx.save()
      ctx.beginPath()
      ctx.rect(x0 + 1, y + 1, w - 2, 14)
      ctx.clip()
      ctx.fillText(block.name || scene.fallbackName, x0 + 5, y + 3)
      ctx.restore()
    }
    ctx.globalAlpha = 1
  }

  ctx.font = `500 11px ${getComputedStyle(document.documentElement).getPropertyValue('--fk-font-sans').trim() || 'sans-serif'}`
  for (const block of scene.blocks) {
    if (hidden.has(block.id)) continue
    drawBlock(block, scene.selected.has(block.id), false)
  }
  for (const ghost of scene.ghosts) drawBlock(ghost, true, true)
  ctx.restore()

  // The time selection: a band over the whole height.
  if (scene.timeSel) {
    const x0 = tickToX(scene.timeSel.start, view)
    const x1 = tickToX(scene.timeSel.end, view)
    ctx.fillStyle = colors.accent
    ctx.globalAlpha = 0.14
    ctx.fillRect(x0, 0, x1 - x0, height)
    ctx.globalAlpha = 0.8
    ctx.fillRect(x0, 0, 1, height)
    ctx.fillRect(x1 - 1, 0, 1, height)
    ctx.globalAlpha = 1
  }

  // The loop region on the ruler: a band, with its two ends marked.
  if (loop) {
    const x0 = tickToX(loop.start, view)
    const x1 = tickToX(loop.end, view)
    ctx.fillStyle = colors.accent
    ctx.globalAlpha = scene.loopOn ? 0.35 : 0.18
    ctx.fillRect(x0, 0, x1 - x0, RULER_HEIGHT)
    ctx.globalAlpha = scene.loopOn ? 1 : 0.5
    ctx.fillRect(x0, 0, 2, RULER_HEIGHT)
    ctx.fillRect(x1 - 2, 0, 2, RULER_HEIGHT)
    ctx.globalAlpha = 1
  }
}
