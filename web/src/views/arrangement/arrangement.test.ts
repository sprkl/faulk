import { describe, expect, it } from 'vitest'
import type { Clip, Track } from '../../lib/generated/protocol'
import type { TrackEntry } from '../../lib/project/tracks'
import { RULER_HEIGHT } from '../../lib/components/timegrid/render'
import {
  LANE_HEIGHT,
  MAX_TICK,
  blocksOf,
  clampScrollY,
  copyShift,
  hitBlock,
  isFree,
  laneTop,
  moveDelta,
  moved,
  playsAt,
  resizeTo,
  resized,
  rowAt,
  passesOf,
  passCount,
  markerJump,
  openingView,
  wholeSongView,
  clipPosition,
  splittable,
  type Block,
} from './arrangement'
import { layoutOf } from './automation/layout'

/** The layout of `n` tracks with no automation shown: one row of clips each. */
const plain = (n: number) =>
  layoutOf(
    Array.from({ length: n }, (_, i) => ({ id: `t${i}` })),
    new Set(),
    () => [],
  )

const midi = (length = 3840, looping: { start: number; end: number } | null = null, name = ''): Clip =>
  ({ name, content: { midi: { length, notes: [], ...(looping ? { looping } : {}) } } }) as unknown as Clip
const place = (clip: string, start: number, length: number, offset = 0) => ({ clip, start, span: { ticks: { length, offset } } })
const entry = (id: string, index: number, clips: Record<string, Clip>, placements: Record<string, ReturnType<typeof place>>): TrackEntry => ({
  id,
  index,
  track: { clips, placements, color: '#ff0000' } as unknown as Track,
})

const entries = [
  entry(
    't0',
    0,
    { a: midi(), b: midi(1920, { start: 480, end: 1920 }) },
    { p2: place('a', 3840, 3840), p1: place('a', 0, 3840), p3: place('b', 8000, 960, 100) },
  ),
  entry('t1', 1, { c: midi() }, { q1: place('c', 960, 1920) }),
]
const blocks = blocksOf(entries)
const get = (id: string): Block => blocks.find((b) => b.id === id)!

describe('blocks', () => {
  it('lists the placements by lane, then by start, with what is drawn of their clip', () => {
    expect(blocks.map((b) => b.id)).toEqual(['p1', 'p2', 'p3', 'q1'])
    expect(get('p3')).toMatchObject({
      row: 0,
      track: 't0',
      clip: 'b',
      start: 8000,
      length: 960,
      end: 8960,
      offset: 100,
      clipLength: 1920,
      looping: { start: 480, end: 1920 },
    })
    expect(get('q1').row).toBe(1)
    expect(get('p1').color).toBe('#ff0000')
  })

  it('leaves out an audio placement and a placement of a missing clip', () => {
    const t = entry('t', 0, { a: midi() }, { ok: place('a', 0, 10), gone: place('zz', 0, 10) })
    ;(t.track.placements as Record<string, unknown>).audio = { clip: 'a', start: 0, span: { frames: { length: 10, offset: 0 } } }
    expect(blocksOf([t]).map((b) => b.id)).toEqual(['ok'])
  })
})

describe('lanes', () => {
  it('puts the first lane under the ruler and scrolls it away', () => {
    expect(laneTop(plain(2), 0, 0)).toBe(RULER_HEIGHT)
    expect(laneTop(plain(3), 2, 10)).toBe(RULER_HEIGHT + 2 * LANE_HEIGHT - 10)
  })

  it('finds the lane under a y, none on the ruler or past the last one', () => {
    expect(rowAt(plain(2), RULER_HEIGHT - 1, 0)).toBeNull()
    expect(rowAt(plain(2), RULER_HEIGHT, 0)?.track).toBe(0)
    expect(rowAt(plain(2), RULER_HEIGHT + LANE_HEIGHT, 0)?.track).toBe(1)
    expect(rowAt(plain(2), RULER_HEIGHT + 2 * LANE_HEIGHT, 0)).toBeNull()
    expect(rowAt(plain(2), RULER_HEIGHT + 5, LANE_HEIGHT)?.track).toBe(1)
  })

  it('keeps the scroll between the top and the end of the lanes, and survives a NaN', () => {
    expect(clampScrollY(-5, 10 * LANE_HEIGHT, 300)).toBe(0)
    expect(clampScrollY(9999, 10 * LANE_HEIGHT, 300)).toBe(10 * LANE_HEIGHT - (300 - RULER_HEIGHT))
    expect(clampScrollY(50, LANE_HEIGHT, 300)).toBe(0)
    expect(clampScrollY(Number.NaN, 10 * LANE_HEIGHT, 300)).toBe(0)
  })
})

describe('hit test', () => {
  const view = { pxPerTick: 0.1, scrollTick: 0 }
  const y = RULER_HEIGHT + LANE_HEIGHT / 2
  it('tells the body from the two edges', () => {
    // p1 covers ticks 0..3840: x 0..384.
    expect(hitBlock(blocks, plain(3), 200, y, view, 0)).toMatchObject({ zone: 'body', block: { id: 'p1' } })
    expect(hitBlock(blocks, plain(3), 2, y, view, 0)?.zone).toBe('start')
    expect(hitBlock(blocks, plain(3), 383, y, view, 0)?.zone).toBe('end')
  })

  it('finds a block in the second lane and nothing between blocks, on the ruler or in the inset', () => {
    expect(hitBlock(blocks, plain(3), 150, y + LANE_HEIGHT, view, 0)?.block.id).toBe('q1')
    expect(hitBlock(blocks, plain(3), 780, y, view, 0)).toBeNull()
    expect(hitBlock(blocks, plain(3), 200, RULER_HEIGHT - 2, view, 0)).toBeNull()
    expect(hitBlock(blocks, plain(3), 200, RULER_HEIGHT + 1, view, 0)).toBeNull()
    expect(hitBlock(blocks, plain(3), 200, y + 3 * LANE_HEIGHT, view, 0)).toBeNull()
  })

  it('uses each track row its own height', () => {
    const tall = layoutOf(
      [{ id: 't0' }, { id: 't1' }],
      new Set(),
      () => [],
      (id) => (id === 't0' ? 120 : 28),
    )
    // Deep in the tall first row: still its block.
    expect(hitBlock(blocks, tall, 200, RULER_HEIGHT + 100, view, 0)?.block.id).toBe('p1')
    // The second row starts at 120 and is 28 high: its inset leaves 20 px of block.
    expect(hitBlock(blocks, tall, 150, RULER_HEIGHT + 120 + 14, view, 0)?.block.id).toBe('q1')
    expect(hitBlock(blocks, tall, 150, RULER_HEIGHT + 120 + 26, view, 0)).toBeNull()
  })

  it('follows the scroll of the grid and of the lanes', () => {
    expect(hitBlock(blocks, plain(3), 100, y - 10, { pxPerTick: 0.1, scrollTick: 960 }, 10)?.block.id).toBe('p1')
    expect(hitBlock(blocks, plain(3), 50, y, { pxPerTick: 0.1, scrollTick: 3840 }, 0)?.block.id).toBe('p2')
  })

  it('keeps a body for a very short block', () => {
    const tiny: Block = { ...get('p1'), start: 0, end: 30, length: 30 }
    expect(hitBlock([tiny], plain(3), 1.5, y, view, 0)?.zone).toBe('body')
  })
})

describe('move', () => {
  const p1 = get('p1')
  const p2 = get('p2')

  it('follows the pointer on the snap step', () => {
    expect(moveDelta([p2], p2, 3840 + 1000, 3840 + 10, 960)).toBe(960)
    expect(moveDelta([p2], p2, 3840 + 1000, 3840 + 10, null)).toBe(990)
  })

  it('never takes a block of the selection before tick 0', () => {
    expect(moveDelta([p1, p2], p2, 0, 3840, 960)).toBe(0)
    expect(moveDelta([p2, get('p3')], p2, 0, 3840, 960)).toBe(-3840)
    expect(moveDelta([p1, p2], p1, -9999, 0, null)).toBe(0)
  })

  it('never takes a block past the last tick', () => {
    expect(moveDelta([p1], p1, MAX_TICK * 2, 0, null)).toBe(MAX_TICK - p1.end)
  })

  it('moves a whole selection by the same delta and keeps the lanes', () => {
    const shifted = moved([p1, get('q1')], 480)
    expect(shifted.map((b) => [b.row, b.start, b.end])).toEqual([
      [0, 480, 4320],
      [1, 1440, 3360],
    ])
    expect(moveDelta([], p1, 5, 0, null)).toBe(0)
  })
})

describe('resize', () => {
  const p2 = get('p2') // 3840..7680, offset 0
  const p3 = get('p3') // 8000..8960, offset 100

  it('moves the end, which stays after the start', () => {
    expect(resizeTo(p2, 'end', 6000, 960)).toBe(5760)
    expect(resizeTo(p2, 'end', 0, null)).toBe(p2.start + 1)
    expect(resizeTo(p2, 'end', 7680, null)).toBeNull()
    expect(resizeTo(p2, 'end', MAX_TICK * 2, null)).toBe(MAX_TICK)
  })

  it('moves the start, which stays before the end and cannot show what is before the clip', () => {
    expect(resizeTo(p2, 'start', 4800, 960)).toBe(4800)
    expect(resizeTo(p2, 'start', 99999, null)).toBe(p2.end - 1)
    // At offset 0 the start cannot grow to the left.
    expect(resizeTo(p2, 'start', 100, null)).toBeNull()
    expect(resizeTo(p2, 'start', 3840, null)).toBeNull()
    // At offset 100 it can, by 100 ticks.
    expect(resizeTo(p3, 'start', 0, null)).toBe(p3.start - 100)
  })

  it('keeps the start edge inside the clip and the timeline', () => {
    // A placement of the 1920-long clip b, which has a loop: no upper bound but the end.
    expect(resizeTo(p3, 'start', 8900, null)).toBe(8900)
    // The start cannot go before the clip's first tick nor tick 0; a clip without a loop repeats (#473), so the start
    // may go past its last tick, up to the end.
    const far: Block = { ...get('p1'), start: 3000, end: 9000, length: 6000, offset: 1000 }
    expect(resizeTo(far, 'start', 0, null)).toBe(2000)
    expect(resizeTo(far, 'start', 8000, null)).toBe(8000)
    expect(resized(far, 'start', 8000)).toMatchObject({ start: 8000, offset: (1000 + 5000) % 3840 })
    const deep: Block = { ...far, start: 500, offset: 2000 }
    expect(resizeTo(deep, 'start', 0, null)).toBe(0)
  })

  it('previews what the project will hold', () => {
    expect(resized(p3, 'start', 7900)).toMatchObject({ start: 7900, end: 8960, length: 1060, offset: 0 })
    expect(resized(p3, 'start', 8100)).toMatchObject({ start: 8100, length: 860, offset: 200 })
    expect(resized(p3, 'end', 9000)).toMatchObject({ start: 8000, end: 9000, length: 1000, offset: 100 })
  })
})

describe('split', () => {
  const p1 = get('p1') // clip a, 3840 long, no loop
  const short: Block = { ...p1, id: 's', start: 0, end: 7680, length: 7680, offset: 0 } // longer than its clip
  const looped: Block = { ...short, id: 'l', clip: 'b', clipLength: 1920, looping: { start: 480, end: 1920 } }

  it('cuts strictly inside a block', () => {
    expect(splittable([p1], 1000).map((b) => b.id)).toEqual(['p1'])
    expect(splittable([p1], 0)).toEqual([])
    expect(splittable([p1], p1.end)).toEqual([])
    expect(splittable([p1, get('q1')], 2000).map((b) => b.id)).toEqual(['p1', 'q1'])
  })

  it('cuts anywhere inside a block: a clip repeats, its loop or the whole clip (#473)', () => {
    expect(playsAt(short, 3839)).toBe(true)
    expect(playsAt(short, 3840)).toBe(true)
    expect(splittable([short], 5000).map((b) => b.id)).toEqual(['s'])
    expect(splittable([looped], 5000).map((b) => b.id)).toEqual(['l'])
    expect(playsAt({ ...p1, offset: 3000 }, 1000)).toBe(true)
  })

  it('counts the passes and draws a boundary at each repeat', () => {
    // The whole clip (3840) twice in 7680.
    expect(passesOf(short)).toEqual([
      { at: 0, from: 0 },
      { at: 3840, from: 0 },
    ])
    // From an offset: the first pass is shorter.
    expect(passesOf({ ...short, offset: 1000 }).map((p) => p.at)).toEqual([0, 2840, 6680])
    // A loop [480, 1920): the first pass to 1920, then every 1440.
    expect(passesOf(looped).map((p) => p.at)).toEqual([0, 1920, 3360, 4800, 6240])
    expect(passesOf({ ...p1, end: p1.start + 100, length: 100 })).toHaveLength(1)
    // Counted without a walk, and listed from a tick in the middle.
    expect(passCount(short)).toBe(2)
    expect(passCount(looped)).toBe(5)
    expect(passCount({ ...short, end: 3840 * 400, length: 3840 * 400 })).toBe(400)
    expect(passesOf(looped, 256, 5000).map((p) => p.at)).toEqual([4800, 6240])
    // An offset past the loop's end plays from its place in the loop.
    const late = { ...looped, offset: 2400, end: 2880, length: 2880 }
    expect(passesOf(late)[0]).toEqual({ at: 0, from: 480 + ((2400 - 1920) % 1440) })
    expect(passCount(late)).toBe(3)
  })

  it('finds the clip position round the clip or its loop', () => {
    expect(clipPosition(short, 5000)).toBe(5000 - 3840)
    expect(clipPosition(looped, 1920)).toBe(480)
    expect(clipPosition(looped, 3400)).toBe(480 + ((3400 - 1920) % 1440))
  })
})

describe('free place', () => {
  it('is free between placements and not under one', () => {
    expect(isFree(blocks, 0, 7680, 8000)).toBe(true)
    expect(isFree(blocks, 0, 7000, 8001)).toBe(false)
    expect(isFree(blocks, 0, 3840, 7680)).toBe(false)
    expect(isFree(blocks, 1, 0, 960)).toBe(true)
    expect(isFree(blocks, 1, 2880, 3000)).toBe(true)
    expect(isFree(blocks, 1, 2879, 3000)).toBe(false)
  })

  it('is not free before the start or past the last tick', () => {
    expect(isFree([], 0, -1, 10)).toBe(false)
    expect(isFree([], 0, MAX_TICK - 5, MAX_TICK + 5)).toBe(false)
  })
})

describe('copy shift', () => {
  it('puts the copies right after the selection', () => {
    expect(copyShift([get('p1')])).toBe(3840)
    expect(copyShift([get('p1'), get('q1')])).toBe(3840)
    expect(copyShift([get('p2'), get('p3')])).toBe(8960 - 3840)
  })

  it('is none for nothing, or when the copy would leave the timeline', () => {
    expect(copyShift([])).toBeNull()
    expect(copyShift([{ ...get('p1'), start: MAX_TICK - 10, end: MAX_TICK }])).toBeNull()
  })
})

describe('sections and zoom (#473)', () => {
  const marks = [
    { id: 'a', tick: 0, name: 'Intro' },
    { id: 'b', tick: 3840 * 8, name: 'Verse' },
    { id: 'c', tick: 3840 * 8, name: 'Fill' },
    { id: 'd', tick: 3840 * 16, name: 'Chorus' },
  ]

  it('jumps to the next marker after the playhead and the previous one before it', () => {
    expect(markerJump(marks, 0, 1)?.id).toBe('b')
    expect(markerJump(marks, 3840 * 8, 1)?.id).toBe('d')
    expect(markerJump(marks, 3840 * 16, 1)).toBeNull()
    expect(markerJump(marks, 3840 * 16, -1)?.id).toBe('c')
    expect(markerJump(marks, 0, -1)).toBeNull()
    expect(markerJump([], 100, 1)).toBeNull()
  })

  it('opens on 16 to 32 bars from bar 1', () => {
    const bar = 3840
    const bars = (v: { pxPerTick: number } | null) => (v ? Math.round(1000 / (v.pxPerTick * bar)) : null)
    expect(bars(openingView(0, bar, 1000))).toBe(16)
    expect(bars(openingView(bar * 20, bar, 1000))).toBe(20)
    expect(bars(openingView(bar * 200, bar, 1000))).toBe(32)
    expect(openingView(bar * 20, bar, 1000)?.scrollTick).toBe(0)
    // No width yet: no view (it waits).
    expect(openingView(bar * 20, bar, 0)).toBeNull()
  })

  it('shows the whole song and a bar more', () => {
    const bar = 3840
    const bars = (v: { pxPerTick: number } | null) => (v ? Math.round(1000 / (v.pxPerTick * bar)) : null)
    expect(bars(wholeSongView(bar * 40, bar, 1000))).toBe(41)
    expect(bars(wholeSongView(bar * 3, bar, 1000))).toBe(17)
    expect(wholeSongView(bar * 40, bar, Number.NaN)).toBeNull()
  })
})
