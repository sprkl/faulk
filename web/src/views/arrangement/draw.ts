/**
 * Canvas drawing of the arrangement: the lanes, the placed clips with a small picture of their notes, the
 * selection, the preview of a gesture and the loop region. Called by `TimeGrid` in its frame, over the
 * shared ruler and lines. Only the blocks in sight are drawn.
 */

import { tickToX } from '../../lib/components/timegrid/grid'
import { RULER_HEIGHT, type BodyFrame } from '../../lib/components/timegrid/render'
import type { LoopRegion, Note } from '../../lib/generated/protocol'
import { drawHandles, type HandleSide } from '../../lib/components/timegrid/handles'
import { BLOCK_INSET, audioPasses, cycleOf, edgeZone, laneHeight, laneTop, passCount, passesOf, type AudioPart, type Block } from './arrangement'
import type { Layout } from './automation/layout'
import { drawWaveform } from '../../lib/audio/peaks/draw'
import type { Peaks } from '../../lib/audio/peaks/format'
import type { MediaRef } from '../../lib/audio/import/pipeline'

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
  /** The section markers (#473): a line down the arrangement at each; their names are buttons on the ruler. */
  markers: readonly { tick: number }[]
  /** The block whose ends are drawn as resize handles (#612): the hovered or the resized one, and the end in use (`null`: the pointer is on its body). */
  handles?: { block: Block; strong: HandleSide | null } | null
  /** The waveform peaks of a media (AU-6), `null` while they are read or made. */
  peaksOf?: (media: MediaRef) => Peaks | null
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

/** What a frame drew, for the browser checks: the audio blocks drawn with their waveform. */
export interface ArrangementDrawn {
  waveforms: number
}

export function drawArrangement(ctx: CanvasRenderingContext2D, frame: BodyFrame, scene: ArrangementFrame): ArrangementDrawn {
  let waveforms = 0
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

    // An audio placement (AU-6): the waveform of the frames it plays, through the clip's loop, from the file's peaks
    // (a faint line while they are read or made).
    if (block.audio && w > 2) {
      const roomy = h >= NAME_ROOM
      const top = y + (roomy ? 16 : 2)
      const height = Math.max(1, h - (roomy ? 18 : 4))
      if (drawAudio(ctx, frame, block, block.audio, x0, top, height, scene.peaksOf?.(block.audio.media) ?? null, (ghost ? 0.7 : 0.95) * k)) waveforms++
    }
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
      // The clip is shown from its offset and goes round its loop, or round the whole clip, where the block
      // outlasts it (#473; a few passes at most: a long block of a short loop is a texture, not a score).
      const cycle = cycleOf(block).end
      // Only the passes in sight (a long block of a short clip is drawn where it shows).
      const fromTick = Math.max(block.start, view.scrollTick)
      for (const { at, from } of passesOf(block, 64, fromTick)) {
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
      }
      ctx.restore()
    }
    // The repeat boundaries (#473): a line where each pass after the first starts, so the clip's length reads
    // as the distance between them and the block's as the block.
    const passes = block.audio ? [] : passesOf(block, 256, Math.max(block.start, view.scrollTick))
    if (!block.audio && passCount(block) > 1 && w > 6) {
      ctx.save()
      ctx.globalAlpha = (ghost ? 0.5 : 0.75) * k
      ctx.fillStyle = block.color
      for (const { at } of passes) {
        if (at === block.start) continue
        const bx = Math.round(tickToX(at, view))
        if (bx <= x0 + 1 || bx >= x0 + w - 1) continue
        ctx.fillRect(bx, y + 1, 1, h - 2)
        // A notch at the top, so the boundary reads apart from a grid line under it.
        ctx.fillRect(bx - 2, y + 1, 5, 2)
      }
      ctx.restore()
    }
    // A linked clip (used more than once) carries a link mark after its name (#473); a copy has none.
    const linked = block.uses > 1
    // The name.
    if (w > 28) {
      ctx.globalAlpha = k
      ctx.fillStyle = colors.text
      ctx.textBaseline = 'top'
      ctx.save()
      ctx.beginPath()
      ctx.rect(x0 + 1, y + 1, w - 2, 14)
      ctx.clip()
      const label = block.name || scene.fallbackName
      ctx.fillText(label, x0 + 5, y + 3)
      if (linked) drawLink(ctx, x0 + 5 + ctx.measureText(label).width + 4, y + 4)
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
  if (scene.handles) {
    const { block, strong } = scene.handles
    const x0 = tickToX(block.start, view)
    const x1 = tickToX(block.end, view)
    const y = laneTop(scene.layout, block.row, scene.scrollY) + BLOCK_INSET
    const h = laneHeight(scene.layout, block.row) - 2 * BLOCK_INSET
    if (x1 >= 0 && x0 <= width && y + h >= RULER_HEIGHT && y <= height) {
      const w = Math.max(2, x1 - x0)
      drawHandles(ctx, colors.accent, { x0, x1: x0 + w, y, height: h, zone: edgeZone(w), strong })
    }
  }
  ctx.restore()

  // The section markers: a thin line from the ruler down.
  if (scene.markers.length > 0) {
    ctx.fillStyle = colors.text
    ctx.globalAlpha = 0.35
    for (const marker of scene.markers) {
      const x = Math.round(tickToX(marker.tick, view))
      if (x < 0 || x > width) continue
      ctx.fillRect(x, RULER_HEIGHT / 2, 1, height - RULER_HEIGHT / 2)
    }
    ctx.globalAlpha = 1
  }

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
  return { waveforms }
}

/**
 * The waveform of an audio block between `top` and `top + height`: each pass of its read (`audioPasses`) drawn from the
 * file's peaks where it is in sight, clipped to the block. Only the visible columns are asked of `drawWaveform`, so a
 * long placement zoomed in costs what the screen shows.
 */
function drawAudio(
  ctx: CanvasRenderingContext2D,
  frame: BodyFrame,
  block: Block,
  audio: AudioPart,
  x0: number,
  top: number,
  height: number,
  peaks: Peaks | null,
  alpha: number,
): boolean {
  const { width, view, dpr } = frame
  const x1 = tickToX(block.end, view)
  const left = Math.max(0, x0 + 1)
  const right = Math.min(width, x1 - 1)
  if (right <= left) return false
  ctx.save()
  ctx.beginPath()
  ctx.rect(left, top, right - left, height)
  ctx.clip()
  ctx.globalAlpha = alpha
  if (!peaks) {
    ctx.globalAlpha = alpha * 0.4
    ctx.fillStyle = block.color
    ctx.fillRect(left, Math.floor(top + height / 2), right - left, 1)
    ctx.restore()
    return false
  }
  // Frames of the file a CSS pixel holds.
  const perPixel = audio.framesPerTick / view.pxPerTick
  const pxPerFrame = 1 / perPixel
  for (const pass of audioPasses(audio)) {
    const px0 = x0 + pass.at * pxPerFrame
    const px1 = px0 + pass.frames * pxPerFrame
    const a = Math.max(left, px0)
    const b = Math.min(right, px1)
    if (b <= a) continue
    drawWaveform(
      ctx,
      peaks,
      { x: a, y: top, width: b - a, height },
      { from: audio.clipStart + pass.from + (a - px0) * perPixel, framesPerPixel: perPixel },
      { color: block.color, dpr },
    )
  }
  ctx.restore()
  return true
}

/** The link mark of a linked clip: two interlocked rounded links, 11 by 7 px, drawn in the current fill colour's stroke. */
function drawLink(ctx: CanvasRenderingContext2D, x: number, y: number) {
  ctx.save()
  ctx.strokeStyle = ctx.fillStyle
  ctx.lineWidth = 1.25
  ctx.beginPath()
  ctx.roundRect(x + 0.5, y + 1.5, 6, 5, 2.5)
  ctx.roundRect(x + 4.5, y + 1.5, 6, 5, 2.5)
  ctx.stroke()
  ctx.restore()
}
