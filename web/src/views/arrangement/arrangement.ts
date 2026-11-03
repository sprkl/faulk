/**
 * The arrangement view's maths (AR-4), as pure functions: the placements of the tracks as blocks on lanes,
 * where a pointer hits one, and what a drag, a resize or a split may ask of the project. A command the
 * project refuses is a page bug (`Dispatcher.run`), so every gesture is clamped here to what
 * `crates/commands/src/arrangement.rs` accepts. No DOM.
 */

import type { Note, Placement, TickLoop } from '../../lib/generated/protocol'
import { snapTick, tickToX, type View } from '../../lib/components/timegrid/grid'
import { RULER_HEIGHT } from '../../lib/components/timegrid/render'
import { trackColor, type TrackEntry } from '../../lib/project/tracks'
import { CLIP_ROW, rowAtY, type Layout, type Row } from './automation/layout'

/** The default height of a track's row of clips, in CSS pixels (each track's own is in the layout). */
export const LANE_HEIGHT = CLIP_ROW
/** The inset of a block inside its lane. */
export const BLOCK_INSET = 4
/** The width of the grab zone at each end of a block, in CSS pixels. */
export const EDGE_PX = 7
/** The last tick of the timeline (`project::limits::MAX_TICK`, 2^40). */
export const MAX_TICK = 2 ** 40

/** A placement of a MIDI clip, with what the view draws of its clip. */
export interface Block {
  id: string
  track: string
  /** The lane, 0 first. */
  row: number
  clip: string
  start: number
  length: number
  /** Where in the clip the placement starts. */
  offset: number
  end: number
  name: string
  color: string
  clipLength: number
  looping: TickLoop | null
  notes: readonly Note[]
  /**
   * Where the clip is used: its placements plus its launch-grid slot (#473). More than one is a linked clip: editing
   * it changes every use, and the block is marked so. A copy is another clip and counts on its own.
   */
  uses: number
}

const ticksOf = (placement: Placement): { length: number; offset: number } | null => ('ticks' in placement.span ? placement.span.ticks : null)

/** How many times each clip of a track is used: its placements, plus one for a launch-grid slot. */
export function usesOf(entry: TrackEntry): Map<string, number> {
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- a plain lookup built per call
  const uses = new Map<string, number>()
  for (const placement of Object.values(entry.track.placements as Record<string, Placement | undefined>)) {
    if (placement) uses.set(placement.clip, (uses.get(placement.clip) ?? 0) + 1)
  }
  for (const [id, clip] of Object.entries(entry.track.clips)) {
    if (clip?.slot) uses.set(id, (uses.get(id) ?? 0) + 1)
  }
  return uses
}

function blocksOfTrack(entry: TrackEntry): Block[] {
  const blocks: Block[] = []
  const uses = usesOf(entry)
  for (const [id, placement] of Object.entries(entry.track.placements as Record<string, Placement | undefined>)) {
    const span = placement && ticksOf(placement)
    const clip = placement && entry.track.clips[placement.clip]
    // An audio placement (frames) comes with the audio import and is not drawn; a placement of a missing clip cannot be.
    if (!placement || !span || !clip || !('midi' in clip.content)) continue
    const { length, looping, notes } = clip.content.midi
    blocks.push({
      id,
      track: entry.id,
      row: entry.index,
      clip: placement.clip,
      start: placement.start,
      length: span.length,
      offset: span.offset,
      end: placement.start + span.length,
      name: clip.name.trim(),
      color: clip.color ?? trackColor(entry.track.color, entry.index),
      clipLength: length,
      looping: looping ?? null,
      notes,
      uses: uses.get(placement.clip) ?? 1,
    })
  }
  return blocks
}

/** Every placement of every track as a block, earliest first within a lane. */
export function blocksOf(entries: readonly TrackEntry[]): Block[] {
  return entries.flatMap(blocksOfTrack).sort((a, b) => a.row - b.row || a.start - b.start || (a.id < b.id ? -1 : 1))
}

/** The tick the last block ends at, 0 when there is none. */
export const lastEnd = (blocks: readonly Block[]): number => blocks.reduce((end, block) => Math.max(end, block.end), 0)

// --- rows -----------------------------------------------------------------------------------------

/** The top of a track's row of clips in the stage, whose first row starts under the ruler. */
export const laneTop = (layout: Layout, track: number, scrollY: number): number => RULER_HEIGHT + (layout.trackTop[track] ?? 0) - scrollY
/** The height of a track's row of clips. */
export const laneHeight = (layout: Layout, track: number): number => layout.trackHeight[track] ?? LANE_HEIGHT

/** How far the rows scroll: the height of the rows past what the stage shows of them. */
export const maxScrollY = (totalHeight: number, stageHeight: number): number => Math.max(0, totalHeight - Math.max(0, stageHeight - RULER_HEIGHT))
export const clampScrollY = (scrollY: number, totalHeight: number, stageHeight: number): number =>
  Math.min(maxScrollY(totalHeight, stageHeight), Math.max(0, Number.isFinite(scrollY) ? scrollY : 0))

/** The row under a stage y (a track's clips, a lane, or the row that adds a lane), or `null` on the ruler or under the last row. */
export function rowAt(layout: Layout, y: number, scrollY: number): Row | null {
  if (y < RULER_HEIGHT) return null
  return rowAtY(layout, y - RULER_HEIGHT + scrollY)
}

// --- hit test -------------------------------------------------------------------------------------

export type Zone = 'start' | 'body' | 'end'

export interface Hit {
  block: Block
  zone: Zone
}

/** The block under a stage point and the part of it: its ends resize, its middle moves. */
export function hitBlock(blocks: readonly Block[], layout: Layout, x: number, y: number, view: View, scrollY: number): Hit | null {
  const found = rowAt(layout, y, scrollY)
  if (!found || found.kind !== 'track') return null
  const row = found.track
  const top = laneTop(layout, row, scrollY) + BLOCK_INSET
  if (y < RULER_HEIGHT || y < top || y > top + laneHeight(layout, row) - 2 * BLOCK_INSET) return null
  // The last one drawn wins, as the eye sees it.
  for (let i = blocks.length - 1; i >= 0; i--) {
    const block = blocks[i]!
    if (block.row !== row) continue
    const left = tickToX(block.start, view)
    const right = tickToX(block.end, view)
    if (x < left || x > right) continue
    const edge = Math.min(EDGE_PX, (right - left) / 3)
    return { block, zone: x - left <= edge ? 'start' : right - x <= edge ? 'end' : 'body' }
  }
  return null
}

// --- gestures -------------------------------------------------------------------------------------

/**
 * The move of a drag, in ticks: the grabbed block's start follows the pointer on the snap step, and no
 * block of the selection starts before tick 0 or ends after the last tick.
 */
export function moveDelta(selection: readonly Block[], grabbed: Block, pointerTick: number, grabTick: number, step: number | null): number {
  if (selection.length === 0) return 0
  const wanted = snapTick(grabbed.start + (pointerTick - grabTick), step) - grabbed.start
  const lowest = selection.reduce((low, b) => Math.min(low, b.start), Infinity)
  const highest = selection.reduce((high, b) => Math.max(high, b.end), 0)
  // `+ 0` turns a -0 into 0.
  return Math.round(Math.min(MAX_TICK - highest, Math.max(-lowest, wanted))) + 0
}

/** The selection as it would be after a move: same lanes, shifted. */
export const moved = (selection: readonly Block[], delta: number): Block[] => selection.map((b) => ({ ...b, start: b.start + delta, end: b.end + delta }))

/**
 * Where an edge goes when the pointer is at `tick`, or `null` when that changes nothing. The end stays after the
 * start; the start stays before the end and, because the content must not slide, not before the first tick of the
 * clip (the offset runs out).
 */
export function resizeTo(block: Block, edge: 'start' | 'end', tick: number, step: number | null): number | null {
  const snapped = snapTick(tick, step)
  let to: number
  if (edge === 'end') to = Math.min(MAX_TICK, Math.max(block.start + 1, snapped))
  else {
    // The content must not slide: not before the clip's first tick (it repeats past its last, #473).
    const high = block.end - 1
    const low = Math.max(0, block.start - block.offset)
    if (high < low) return null
    to = Math.min(high, Math.max(low, snapped))
  }
  to = Math.round(to)
  return to === (edge === 'end' ? block.end : block.start) ? null : to
}

/** The block as it would be after the resize. */
export function resized(block: Block, edge: 'start' | 'end', to: number): Block {
  if (edge === 'end') return { ...block, end: to, length: to - block.start }
  const offset = to < block.start ? block.offset - (block.start - to) : clipPosition(block, block.offset + (to - block.start))
  return { ...block, start: to, length: block.end - to, offset }
}

/**
 * The part of the clip a block repeats (#473), the engine's rule: the clip's loop when it has one, else the whole
 * clip. A placement longer than what it plays goes round it.
 */
export const cycleOf = (block: Pick<Block, 'looping' | 'clipLength'>): TickLoop => block.looping ?? { start: 0, end: block.clipLength }

/** The clip position reached after `e` ticks played from its position 0: straight to the cycle's end, then round it. */
export function clipPosition(block: Pick<Block, 'looping' | 'clipLength'>, e: number): number {
  const { start, end } = cycleOf(block)
  if (end <= start || e < end) return e
  return start + ((e - end) % (end - start))
}

/**
 * The passes of a block: where each starts on the timeline and the clip tick it starts from. The first starts at the
 * block's start from its offset; each later one at a repeat boundary, from the cycle's start. At most `max`.
 */
export function passesOf(block: Block, max = 256, after = block.start): { at: number; from: number }[] {
  const { start, end } = cycleOf(block)
  const out: { at: number; from: number }[] = []
  let at = block.start
  // An offset at or past the loop's end plays from its place round the loop (the engine's rule).
  let from = clipPosition(block, block.offset)
  // Jump straight to the pass that holds `after` (a long block in view from its middle): no walk from the start.
  const first = end - from
  const span = end - start
  if (span > 0 && first > 0 && after >= block.start + first) {
    const k = Math.floor((after - block.start - first) / span)
    at = block.start + first + k * span
    from = start
  }
  while (at < block.end && out.length < max) {
    out.push({ at, from })
    if (end <= from) break
    at += end - from
    from = start
  }
  return out
}

/** How many passes a block plays (the first one counted even when partial), without walking them. */
export function passCount(block: Block): number {
  const { start, end } = cycleOf(block)
  const first = end - clipPosition(block, block.offset)
  const span = end - start
  if (span <= 0 || first <= 0 || block.length <= first) return 1
  return 1 + Math.ceil((block.length - first) / span)
}

/**
 * Whether the clip plays at `at` in `block`: the rule of the engine and of `arrangement.split`. A clip repeats
 * (its loop, or the whole clip, #473), so it plays all along the block.
 */
export function playsAt(block: Block, at: number): boolean {
  return block.clipLength > 0 && at >= block.start && at < block.end
}

/** The blocks a split at `at` cuts: those it falls strictly inside, and where the clip still plays. */
export const splittable = (selection: readonly Block[], at: number): Block[] => selection.filter((b) => at > b.start && at < b.end && playsAt(b, at))

/** Whether `[start, end)` is free on a lane (`clip.create` into the arrangement refuses an overlap). */
export const isFree = (blocks: readonly Block[], row: number, start: number, end: number): boolean =>
  start >= 0 && end <= MAX_TICK && !blocks.some((b) => b.row === row && b.start < end && start < b.end)

/**
 * The shift of a copy that puts the selection right after itself (the default of `arrangement.duplicate`), or `null`
 * when the copy would end past the last tick (the command refuses it).
 */
export function copyShift(selection: readonly Block[]): number | null {
  if (selection.length === 0) return null
  const first = selection.reduce((low, b) => Math.min(low, b.start), Infinity)
  const last = selection.reduce((high, b) => Math.max(high, b.end), 0)
  return last + (last - first) <= MAX_TICK ? last - first : null
}
