/**
 * The arrangement view's maths (AR-4), as pure functions: the placements of the tracks as blocks on lanes,
 * where a pointer hits one, and what a drag, a resize or a split may ask of the project. A command the
 * project refuses is a page bug (`Dispatcher.run`), so every gesture is clamped here to what
 * `crates/commands/src/arrangement.rs` accepts. No DOM.
 */

import type { Note, Placement, TickLoop } from '../../lib/generated/protocol'
import type { MediaRef } from '../../lib/audio/import/pipeline'
import { framesPerTick, framesToTicks } from '../../lib/audio/time'
import { snapTick, tickToX, type View } from '../../lib/components/timegrid/grid'
import { RULER_HEIGHT } from '../../lib/components/timegrid/render'
import { trackColor, type TrackEntry } from '../../lib/project/tracks'
import { CLIP_ROW, rowAtY, type Layout, type Row } from './automation/layout'

/** The default height of a track's row of clips, in CSS pixels (each track's own is in the layout). */
export const LANE_HEIGHT = CLIP_ROW
/** The inset of a block inside its lane. */
export const BLOCK_INSET = 4
/** The width of the grab zone at each end of a block, in CSS pixels (#612: wide enough to catch without aiming). */
export const EDGE_PX = 10
/** The least grab zone at each end of a block, in CSS pixels, however short the block is (#612). */
export const EDGE_MIN_PX = 6

/**
 * The grab zone at each end of a block `width` pixels wide: a third of it, between `EDGE_MIN_PX` and `EDGE_PX`, and never more than
 * what leaves a pixel for the middle (a block under about 12 px is all edges, as no zone could keep both).
 */
export const edgeZone = (width: number): number => Math.max(0, Math.min(Math.min(EDGE_PX, Math.max(EDGE_MIN_PX, width / 3)), (width - 1) / 2))
/** The last tick of the timeline (`project::limits::MAX_TICK`, 2^40). */
export const MAX_TICK = 2 ** 40

/**
 * What an audio placement plays (AU-6), in frames of its file: the clip's range in the file, the placement's part of
 * the clip, and the clip's loop, for drawing its waveform and bounding its edges.
 */
export interface AudioPart {
  media: MediaRef
  /** The clip's first frame in the file, and its length in frames. */
  clipStart: number
  clipFrames: number
  /** Where in the clip the placement starts, and how many frames it plays. */
  offset: number
  frames: number
  /** The clip's loop, in frames from the clip's start. */
  looping: { start: number; end: number } | null
  /** Clip gain in dB; `null` is silence. */
  gain: number | null
  /** Frames of the file one tick holds at the project's tempo. */
  framesPerTick: number
}

/** A placement of a clip (MIDI or audio), with what the view draws of its clip. */
export interface Block {
  id: string
  track: string
  /** The lane, 0 first. */
  row: number
  clip: string
  start: number
  length: number
  /** Where in the clip the placement starts, in ticks (an audio placement's frames at the tempo). */
  offset: number
  end: number
  name: string
  color: string
  /** The clip's length in ticks (an audio clip's frames at the tempo). */
  clipLength: number
  looping: TickLoop | null
  notes: readonly Note[]
  /**
   * Where the clip is used: its placements plus its launch-grid slot (#473). More than one is a linked clip: editing
   * it changes every use, and the block is marked so. A copy is another clip and counts on its own.
   */
  uses: number
  /** An audio placement's frames (AU-6); `null` for a MIDI one. */
  audio: AudioPart | null
}

const ticksOf = (placement: Placement): { length: number; offset: number } | null => ('ticks' in placement.span ? placement.span.ticks : null)
const framesOf = (placement: Placement): { length: number; offset: number } | null => ('frames' in placement.span ? placement.span.frames : null)

/** How many times each clip of a track is used: its placements, plus one for a launch-grid slot. */
export function usesOf(entry: TrackEntry): Map<string, number> {
  const uses = new Map<string, number>()
  for (const placement of Object.values(entry.track.placements as Record<string, Placement | undefined>)) {
    if (placement) uses.set(placement.clip, (uses.get(placement.clip) ?? 0) + 1)
  }
  for (const [id, clip] of Object.entries(entry.track.clips)) {
    if (clip?.slot) uses.set(id, (uses.get(id) ?? 0) + 1)
  }
  return uses
}

const NO_NOTES: readonly Note[] = []

function blocksOfTrack(entry: TrackEntry, bpm: number): Block[] {
  const blocks: Block[] = []
  const uses = usesOf(entry)
  for (const [id, placement] of Object.entries(entry.track.placements as Record<string, Placement | undefined>)) {
    const clip = placement && entry.track.clips[placement.clip]
    // A placement of a missing clip cannot be drawn.
    if (!placement || !clip) continue
    const common = {
      id,
      track: entry.id,
      row: entry.index,
      clip: placement.clip,
      start: placement.start,
      name: clip.name.trim(),
      color: clip.color ?? trackColor(entry.track.color, entry.index),
      uses: uses.get(placement.clip) ?? 1,
    }
    const ticks = ticksOf(placement)
    if (ticks && 'midi' in clip.content) {
      const { length, looping, notes } = clip.content.midi
      blocks.push({
        ...common,
        length: ticks.length,
        offset: ticks.offset,
        end: placement.start + ticks.length,
        clipLength: length,
        looping: looping ?? null,
        notes,
        audio: null,
      })
      continue
    }
    const frames = framesOf(placement)
    if (!frames || !('audio' in clip.content)) continue
    // An audio placement (AU-6): its ticks are its frames at the project's tempo, as the project computes them.
    const a = clip.content.audio
    const rate = a.media.sample_rate
    const length = framesToTicks(frames.length, rate, bpm)
    blocks.push({
      ...common,
      length,
      offset: frames.offset > 0 ? framesToTicks(frames.offset, rate, bpm) : 0,
      end: placement.start + length,
      clipLength: framesToTicks(a.frames, rate, bpm),
      looping: null,
      notes: NO_NOTES,
      audio: {
        media: { hash: a.media.hash, sample_rate: rate, channels: a.media.channels, frames: a.media.frames },
        clipStart: a.start,
        clipFrames: a.frames,
        offset: frames.offset,
        frames: frames.length,
        looping: a.looping ?? null,
        gain: a.gain_db ?? null,
        framesPerTick: framesPerTick(rate, bpm),
      },
    })
  }
  return blocks
}

/** Every placement of every track as a block, earliest first within a lane; `bpm` sets the ticks of audio placements. */
export function blocksOf(entries: readonly TrackEntry[], bpm = 120): Block[] {
  return entries.flatMap((entry) => blocksOfTrack(entry, bpm)).sort((a, b) => a.row - b.row || a.start - b.start || (a.id < b.id ? -1 : 1))
}

/**
 * The passes of an audio block's read through its clip (AU-6), the player's rule: from the placement's offset straight
 * to the clip's loop end, then round the loop, until the placement's frames are played. Each pass: the frame of the
 * block where it starts (from the block's first), the clip frame it reads from, and how many frames. At most `max`.
 */
export function audioPasses(audio: AudioPart, max = 256): { at: number; from: number; frames: number }[] {
  const out: { at: number; from: number; frames: number }[] = []
  const loop = audio.looping && audio.looping.start < audio.looping.end && audio.looping.end <= audio.clipFrames ? audio.looping : null
  let at = 0
  let from = audio.offset
  if (loop && from >= loop.end) from = loop.start + ((from - loop.end) % (loop.end - loop.start))
  while (at < audio.frames && out.length < max) {
    const upto = loop && from < loop.end ? loop.end : audio.clipFrames
    const frames = Math.min(audio.frames - at, upto - from)
    if (frames <= 0) break
    out.push({ at, from, frames })
    at += frames
    if (!loop) break
    from = loop.start
  }
  return out
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
    const edge = edgeZone(right - left)
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

/** The selection as it would be after a move: shifted in time and, with `rows`, that many tracks down (negative is up). */
export const moved = (selection: readonly Block[], delta: number, rows = 0): Block[] =>
  selection.map((b) => ({ ...b, start: b.start + delta, end: b.end + delta, row: b.row + rows }))

/** `rows` brought back so that no block of the selection leaves the `count` tracks. */
export function clampRows(selection: readonly Block[], rows: number, count: number): number {
  if (selection.length === 0 || count <= 0) return 0
  const top = selection.reduce((low, b) => Math.min(low, b.row), Infinity)
  const bottom = selection.reduce((high, b) => Math.max(high, b.row), -Infinity)
  // `+ 0` turns a -0 into 0.
  return Math.min(count - 1 - bottom, Math.max(-top, Math.round(rows))) + 0
}

/** What a move of `rows` tracks asks of `arrangement.move_to_track`: the pairs, or why it cannot be asked. */
export type TrackTargets = { moves: { placement: string; track: string }[] } | { refused: 'edge' } | { refused: 'incompatible'; track: string }

/**
 * Each selected block with the track `rows` tracks from its own, in track order (lanes and the master are not tracks). A target
 * past the first or last track, or a track of the other kind (a MIDI clip goes to a MIDI track, an audio clip to an audio
 * track, AU-6), refuses the whole move: the command is all or none.
 */
export function trackTargets(selection: readonly Block[], rows: number, tracks: readonly { id: string; midi: boolean }[]): TrackTargets {
  const moves: { placement: string; track: string }[] = []
  for (const b of selection) {
    const target = tracks[b.row + rows]
    if (!target) return { refused: 'edge' }
    if (target.midi === (b.audio !== null)) return { refused: 'incompatible', track: target.id }
    moves.push({ placement: b.id, track: target.id })
  }
  return { moves }
}

/**
 * The shift of tracks a keyboard move ends in (#805): `rows` when every block fits there, else the next shift in the same
 * direction where every block lands on a free place of a track of its kind, `null` when none does (the first track or the
 * last is passed). The blocks that move do not count as in the way. It is `arrangement.move_to_track`'s `if_occupied: "skip"`
 * seen from the page, which needs it to say which track was busy.
 */
export function freeRows(
  selection: readonly Block[],
  rows: number,
  delta: number,
  tracks: readonly { id: string; midi: boolean }[],
  blocks: readonly Block[],
): number | null {
  if (rows === 0 || selection.length === 0) return rows
  const moving = new Set(selection.map((b) => b.id))
  const step = Math.sign(rows)
  for (let k = rows; ; k += step) {
    let fits = true
    for (const b of selection) {
      const target = tracks[b.row + k]
      if (!target) return null
      const [start, end] = [b.start + delta, b.end + delta]
      if (target.midi === (b.audio !== null) || blocks.some((o) => o.row === b.row + k && !moving.has(o.id) && o.start < end && start < o.end)) {
        fits = false
        break
      }
    }
    if (fits) return k
  }
}

/**
 * The first block (the earliest) that a copy or a move of the selection `by` ticks would land on, on the track it stays in, or
 * `null` when it lands on nothing (#805). The blocks of the selection are not in the way: they move or are copied together.
 * It is what `if_occupied: "refuse"` sees, so that the page can name the block that is in the way.
 */
export function landingOn(selection: readonly Block[], by: number, blocks: readonly Block[]): Block | null {
  const moving = new Set(selection.map((b) => b.id))
  let found: Block | null = null
  for (const b of selection) {
    for (const o of blocks) {
      if (o.row !== b.row || moving.has(o.id) || o.start >= b.end + by || b.start + by >= o.end) continue
      if (!found || o.start < found.start) found = o
    }
  }
  return found
}

/** Whether a copy of each block, `by` ticks later on its own track, would land on a block that is not part of the copy (#805). */
export function copyBlocked(selection: readonly Block[], by: number, blocks: readonly Block[]): boolean {
  return landingOn(selection, by, blocks) !== null
}

/**
 * The first block of the track that `arrangement.repeat` of `block` up to the tick `until` would cover: the copies fill the
 * time from the end of the block to `until` (#805). Growing the end of the block to `until` (`arrangement.resize`) covers the same time (#863).
 */
export function repeatCovers(block: Block, until: number, blocks: readonly Block[]): Block | null {
  let found: Block | null = null
  for (const o of blocks) {
    if (o.row !== block.row || o.id === block.id || o.start >= until || block.end >= o.end) continue
    if (!found || o.start < found.start) found = o
  }
  return found
}

/**
 * Where an edge goes when the pointer is at `tick`, or `null` when that changes nothing. The end stays after the
 * start; the start stays before the end and, because the content must not slide, not before the first tick of the
 * clip (the offset runs out).
 */
export function resizeTo(block: Block, edge: 'start' | 'end', tick: number, step: number | null): number | null {
  const snapped = snapTick(tick, step)
  let to: number
  // An audio placement stays inside its clip (AU-6): its end stops at the clip's last frame (the command clamps there too).
  const high = block.audio ? block.start + (block.clipLength - block.offset) : MAX_TICK
  if (edge === 'end') to = Math.min(MAX_TICK, high, Math.max(block.start + 1, snapped))
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
  const audio = block.audio
  if (audio) {
    // The frames the command keeps, near enough for the preview (it computes them exactly).
    if (edge === 'end') {
      const frames = Math.max(1, Math.min(audio.clipFrames - audio.offset, Math.floor((to - block.start) * audio.framesPerTick)))
      return { ...block, end: to, length: to - block.start, audio: { ...audio, frames } }
    }
    const offset = Math.max(0, Math.min(audio.offset + audio.frames - 1, audio.offset + Math.round((to - block.start) * audio.framesPerTick)))
    const ticks = Math.max(0, block.offset + (to - block.start))
    return { ...block, start: to, length: block.end - to, offset: ticks, audio: { ...audio, offset, frames: audio.frames - (offset - audio.offset) } }
  }
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

// --- sections and zoom (#473) -------------------------------------------------------------------------

/** A section marker as the view shows it, in timeline order. */
export interface MarkerView {
  id: string
  tick: number
  name: string
}

/**
 * The marker a jump goes to from the tick `from`: the first after it, or the last before it (one at `from` itself is
 * not a jump target, so a second press goes on to the next). `null` when there is none that way.
 */
export function markerJump(markers: readonly MarkerView[], from: number, direction: 1 | -1, current: string | null = null): MarkerView | null {
  // From the marker jumped to last (when the playhead is still on it): its neighbour in (tick, id) order, so two
  // markers on one tick are both reached.
  const at = current === null ? -1 : markers.findIndex((m) => m.id === current && m.tick === from)
  if (at >= 0) return markers[at + direction] ?? null
  if (direction > 0) return markers.find((m) => m.tick > from) ?? null
  return markers.findLast((m) => m.tick < from) ?? null
}

/** The bars the arrangement opens on: the whole song when it is 16 to 32 bars long, else 16 or 32 (#473). */
export const OPEN_MIN_BARS = 16
export const OPEN_MAX_BARS = 32

/** The zoom (pixels per tick) that shows `bars` bars of `bar` ticks across `width` pixels; `null` without a width. */
export function zoomForBars(bars: number, bar: number, width: number): number | null {
  if (!(width > 0) || !(bar > 0) || !(bars > 0)) return null
  return width / (bars * bar)
}

/** The opening view of a song that ends at `end` ticks: from bar 1, 16 to 32 bars across the window. */
export function openingView(end: number, bar: number, width: number): { pxPerTick: number; scrollTick: number } | null {
  const songBars = bar > 0 ? Math.ceil(Math.max(0, end) / bar) : 0
  const bars = Math.min(OPEN_MAX_BARS, Math.max(OPEN_MIN_BARS, songBars))
  const pxPerTick = zoomForBars(bars, bar, width)
  return pxPerTick === null ? null : { pxPerTick, scrollTick: 0 }
}

/** The whole-song view: from bar 1 to the song's end (at least 16 bars) and one bar more. */
export function wholeSongView(end: number, bar: number, width: number): { pxPerTick: number; scrollTick: number } | null {
  const songBars = bar > 0 ? Math.ceil(Math.max(0, end) / bar) : 0
  const pxPerTick = zoomForBars(Math.max(OPEN_MIN_BARS, songBars) + 1, bar, width)
  return pxPerTick === null ? null : { pxPerTick, scrollTick: 0 }
}

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

// --- drawing a clip and the box (#615) ------------------------------------------------------------

/**
 * The span a drag on an empty part of a lane draws: from where it was pressed (`pressTick`, which sits in free ground) to the
 * pointer (`tick`), rightwards or leftwards, each end on the snap `step` (`null`: free). It stops at the neighbouring clips and
 * at tick 0, so the clip it asks `clip.create` for never overlaps one. `null` when nothing is left (a click, or a span under
 * a tick once snapped).
 */
export function drawnSpan(
  blocks: readonly Block[],
  row: number,
  pressTick: number,
  tick: number,
  step: number | null,
): { start: number; length: number } | null {
  const lane = blocks.filter((b) => b.row === row)
  const floor = lane.reduce((low, b) => (b.end <= pressTick ? Math.max(low, b.end) : low), 0)
  const ceiling = lane.reduce((high, b) => (b.start >= pressTick ? Math.min(high, b.start) : high), MAX_TICK)
  const a = snapTick(pressTick, step)
  const b = snapTick(tick, step)
  const start = Math.round(Math.max(floor, Math.min(a, b)))
  const end = Math.round(Math.min(ceiling, Math.max(a, b)))
  return end > start ? { start, length: end - start } : null
}

/** The blocks that a box (stage coordinates, any corner order) touches. */
export function blocksInBox(
  blocks: readonly Block[],
  layout: Layout,
  view: View,
  scrollY: number,
  box: { x0: number; y0: number; x1: number; y1: number },
): Block[] {
  const left = Math.min(box.x0, box.x1)
  const right = Math.max(box.x0, box.x1)
  const top = Math.min(box.y0, box.y1)
  const bottom = Math.max(box.y0, box.y1)
  return blocks.filter((b) => {
    const rowTop = laneTop(layout, b.row, scrollY) + BLOCK_INSET
    const rowBottom = rowTop + laneHeight(layout, b.row) - 2 * BLOCK_INSET
    return tickToX(b.end, view) >= left && tickToX(b.start, view) <= right && rowBottom >= top && rowTop <= bottom
  })
}
