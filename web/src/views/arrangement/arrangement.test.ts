import { describe, expect, it } from 'vitest'
import type { Clip, Track } from '../../lib/generated/protocol'
import type { TrackEntry } from '../../lib/project/tracks'
import { RULER_HEIGHT } from '../../lib/components/timegrid/render'
import {
  LANE_HEIGHT,
  MAX_TICK,
  blocksOf,
  clampScrollY,
  hitBlock,
  isFree,
  laneTop,
  moveDelta,
  moved,
  playsAt,
  resizeTo,
  resized,
  rowAt,
  splittable,
  type Block,
} from './arrangement'

const midi = (length = 3840, looping: { start: number; end: number } | null = null, name = ''): Clip =>
  ({ name, content: { midi: { length, notes: [], ...(looping ? { looping } : {}) } } }) as unknown as Clip
const place = (clip: string, start: number, length: number, offset = 0) => ({ clip, start, span: { ticks: { length, offset } } })
const entry = (id: string, index: number, clips: Record<string, Clip>, placements: Record<string, ReturnType<typeof place>>): TrackEntry => ({
  id,
  index,
  track: { clips, placements, color: '#ff0000' } as unknown as Track,
})

const entries = [
  entry('t0', 0, { a: midi(), b: midi(1920, { start: 480, end: 1920 }) }, { p2: place('a', 3840, 3840), p1: place('a', 0, 3840), p3: place('b', 8000, 960, 100) }),
  entry('t1', 1, { c: midi() }, { q1: place('c', 960, 1920) }),
]
const blocks = blocksOf(entries)
const get = (id: string): Block => blocks.find((b) => b.id === id)!

describe('blocks', () => {
  it('lists the placements by lane, then by start, with what is drawn of their clip', () => {
    expect(blocks.map((b) => b.id)).toEqual(['p1', 'p2', 'p3', 'q1'])
    expect(get('p3')).toMatchObject({ row: 0, track: 't0', clip: 'b', start: 8000, length: 960, end: 8960, offset: 100, clipLength: 1920, looping: { start: 480, end: 1920 } })
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
    expect(laneTop(0, 0)).toBe(RULER_HEIGHT)
    expect(laneTop(2, 10)).toBe(RULER_HEIGHT + 2 * LANE_HEIGHT - 10)
  })

  it('finds the lane under a y, none on the ruler or past the last one', () => {
    expect(rowAt(RULER_HEIGHT - 1, 0, 2)).toBeNull()
    expect(rowAt(RULER_HEIGHT, 0, 2)).toBe(0)
    expect(rowAt(RULER_HEIGHT + LANE_HEIGHT, 0, 2)).toBe(1)
    expect(rowAt(RULER_HEIGHT + 2 * LANE_HEIGHT, 0, 2)).toBeNull()
    expect(rowAt(RULER_HEIGHT + 5, LANE_HEIGHT, 2)).toBe(1)
  })

  it('keeps the scroll between the top and the end of the lanes, and survives a NaN', () => {
    expect(clampScrollY(-5, 10, 300)).toBe(0)
    expect(clampScrollY(9999, 10, 300)).toBe(10 * LANE_HEIGHT - (300 - RULER_HEIGHT))
    expect(clampScrollY(50, 1, 300)).toBe(0)
    expect(clampScrollY(Number.NaN, 10, 300)).toBe(0)
  })
})

describe('hit test', () => {
  const view = { pxPerTick: 0.1, scrollTick: 0 }
  const y = RULER_HEIGHT + LANE_HEIGHT / 2
  it('tells the body from the two edges', () => {
    // p1 covers ticks 0..3840: x 0..384.
    expect(hitBlock(blocks, 200, y, view, 0)).toMatchObject({ zone: 'body', block: { id: 'p1' } })
    expect(hitBlock(blocks, 2, y, view, 0)?.zone).toBe('start')
    expect(hitBlock(blocks, 383, y, view, 0)?.zone).toBe('end')
  })

  it('finds a block in the second lane and nothing between blocks, on the ruler or in the inset', () => {
    expect(hitBlock(blocks, 150, y + LANE_HEIGHT, view, 0)?.block.id).toBe('q1')
    expect(hitBlock(blocks, 780, y, view, 0)).toBeNull()
    expect(hitBlock(blocks, 200, RULER_HEIGHT - 2, view, 0)).toBeNull()
    expect(hitBlock(blocks, 200, RULER_HEIGHT + 1, view, 0)).toBeNull()
    expect(hitBlock(blocks, 200, y + 3 * LANE_HEIGHT, view, 0)).toBeNull()
  })

  it('follows the scroll of the grid and of the lanes', () => {
    expect(hitBlock(blocks, 100, y - 10, { pxPerTick: 0.1, scrollTick: 960 }, 10)?.block.id).toBe('p1')
    expect(hitBlock(blocks, 50, y, { pxPerTick: 0.1, scrollTick: 3840 }, 0)?.block.id).toBe('p2')
  })

  it('keeps a body for a very short block', () => {
    const tiny: Block = { ...get('p1'), start: 0, end: 30, length: 30 }
    expect(hitBlock([tiny], 1.5, y, view, 0)?.zone).toBe('body')
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
    expect(shifted.map((b) => [b.row, b.start, b.end])).toEqual([[0, 480, 4320], [1, 1440, 3360]])
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
    // The start cannot go before tick 0 nor, without a loop, show past the last tick of the clip (3840).
    const far: Block = { ...get('p1'), start: 3000, end: 9000, length: 6000, offset: 1000 }
    expect(resizeTo(far, 'start', 0, null)).toBe(2000)
    expect(resizeTo(far, 'start', 8000, null)).toBe(3000 + 3840 - 1000 - 1)
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

  it('refuses where a clip without a loop has ended, and not where its loop goes on', () => {
    expect(playsAt(short, 3839)).toBe(true)
    expect(playsAt(short, 3840)).toBe(false)
    expect(splittable([short], 5000)).toEqual([])
    expect(splittable([looped], 5000).map((b) => b.id)).toEqual(['l'])
  })

  it('counts the offset', () => {
    expect(playsAt({ ...p1, offset: 3000 }, 1000)).toBe(false)
    expect(playsAt({ ...p1, offset: 3000 }, 500)).toBe(true)
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
