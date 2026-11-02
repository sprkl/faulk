import { describe, expect, it } from 'vitest'
import type { ActiveClip } from '../../lib/engine/feedback'
import type { Placement, ViewSlots } from '../../lib/generated/protocol'
import { MAX_EXTRAPOLATION_MS, clipLocal, liveTick, playheadIn, settle, type PlayheadInput } from './playhead'

const slots: ViewSlots = { epoch: 4, tracks: { T1: 0, T2: 1 }, scenes: { S1: 0, S2: 1 } }
const place = (clip: string, start: number, length: number, offset = 0): Placement => ({ clip, start, span: { ticks: { length, offset } } })

function input(over: Partial<PlayheadInput> & { cells?: ActiveClip[]; epoch?: number }): PlayheadInput {
  const { cells = [], epoch = 4, ...rest } = over
  return {
    report: { epoch, clips: cells },
    slots,
    trackId: 'T1',
    track: { placements: {} },
    clipId: 'C1',
    clip: { slot: null },
    length: 3840,
    looping: null,
    tick: 0,
    ...rest,
  }
}

describe('clipLocal', () => {
  it('plays once: the position through the last tick, nothing before or after', () => {
    expect(clipLocal(0, 3840, null)).toBe(0)
    expect(clipLocal(3840, 3840, null)).toBe(3840)
    expect(clipLocal(3841, 3840, null)).toBeNull()
    expect(clipLocal(-1, 3840, null)).toBeNull()
  })

  it('wraps on the loop after its end, keeping the intro before the loop', () => {
    const loop = { start: 960, end: 2880 }
    expect(clipLocal(100, 3840, loop)).toBe(100)
    expect(clipLocal(2879, 3840, loop)).toBe(2879)
    expect(clipLocal(2880, 3840, loop)).toBe(960)
    expect(clipLocal(2880 + 1919, 3840, loop)).toBe(2879)
    expect(clipLocal(2880 + 1920, 3840, loop)).toBe(960)
    expect(clipLocal(2880 + 5 * 1920 + 7, 3840, loop)).toBe(967)
  })

  it('stays in phase before the clip start', () => {
    const loop = { start: 0, end: 1920 }
    expect(clipLocal(-1, 3840, loop)).toBe(1919)
    expect(clipLocal(-1920, 3840, loop)).toBe(0)
  })

  it('refuses an empty loop instead of dividing by zero', () => {
    expect(clipLocal(5000, 3840, { start: 100, end: 100 })).toBeNull()
  })
})

describe('playheadIn, a clip launched from the grid', () => {
  const clip = { slot: 'S1' }
  const playing: ActiveClip = { track: 0, scene: 0, state: 'playing', anchor: 7680 }

  it('counts from the launch boundary', () => {
    expect(playheadIn(input({ clip, cells: [playing], tick: 7680 + 500 }))).toBe(500)
  })

  it('wraps on the clip loop', () => {
    expect(playheadIn(input({ clip, cells: [playing], looping: { start: 0, end: 1920 }, tick: 7680 + 1920 + 10 }))).toBe(10)
  })

  it('keeps the position of a clip that is stopping', () => {
    expect(playheadIn(input({ clip, cells: [{ ...playing, state: 'stopping' }], tick: 7680 + 40 }))).toBe(40)
  })

  it('has none for a clip that only waits for its boundary, or for another clip of the track', () => {
    expect(playheadIn(input({ clip, cells: [{ ...playing, state: 'queued' }], tick: 7680 + 40 }))).toBeNull()
    expect(playheadIn(input({ clip: { slot: 'S2' }, cells: [playing], tick: 7680 + 40 }))).toBeNull()
    expect(playheadIn(input({ clip: { slot: null }, cells: [playing], tick: 7680 + 40 }))).toBeNull()
  })

  it('holds the arrangement back: a placed clip has no playhead while the grid plays the track', () => {
    const track = { placements: { P1: place('C1', 0, 3840) } }
    expect(playheadIn(input({ track, clip: { slot: null }, cells: [playing], tick: 100 }))).toBeNull()
  })

  it('ignores the cells of another track', () => {
    const other: ActiveClip = { track: 1, scene: 0, state: 'playing', anchor: 0 }
    const track = { placements: { P1: place('C1', 0, 3840) } }
    expect(playheadIn(input({ track, clip, cells: [other], tick: 100 }))).toBe(100)
  })

  it('does not read a report of another epoch against the slots', () => {
    expect(playheadIn(input({ clip, cells: [playing], epoch: 5, tick: 7680 + 500 }))).toBeNull()
    expect(playheadIn(input({ clip, cells: [playing], trackId: 'unknown', tick: 7680 + 500 }))).toBeNull()
  })
})

describe('playheadIn, the arrangement', () => {
  it('counts from the placement start, offset into the clip', () => {
    const track = { placements: { P1: place('C1', 3840, 1920, 960) } }
    expect(playheadIn(input({ track, tick: 3840 }))).toBe(960)
    expect(playheadIn(input({ track, tick: 3840 + 500 }))).toBe(1460)
  })

  it('is silent before and after the placement', () => {
    const track = { placements: { P1: place('C1', 3840, 1920) } }
    expect(playheadIn(input({ track, tick: 3839 }))).toBeNull()
    expect(playheadIn(input({ track, tick: 3840 + 1920 }))).toBeNull()
    expect(playheadIn(input({ track, tick: -960 }))).toBeNull()
  })

  it('repeats the loop of the clip inside a longer placement, and ends silent for a clip without loop', () => {
    const track = { placements: { P1: place('C1', 0, 9600) } }
    expect(playheadIn(input({ track, looping: { start: 0, end: 1920 }, tick: 1920 * 3 + 50 }))).toBe(50)
    expect(playheadIn(input({ track, tick: 5000 }))).toBeNull()
  })

  it('follows the placement that plays, among linked placements of the same clip', () => {
    const track = { placements: { P1: place('C1', 0, 3840), P2: place('C1', 7680, 3840) } }
    expect(playheadIn(input({ track, tick: 7680 + 10 }))).toBe(10)
    expect(playheadIn(input({ track, tick: 5000 }))).toBeNull()
  })

  it('shows only the placement the roll was opened on, when one was (#494)', () => {
    const track = { placements: { P1: place('C1', 0, 3840), P2: place('C1', 7680, 3840) } }
    expect(playheadIn(input({ track, placement: 'P2', tick: 7680 + 10 }))).toBe(10)
    expect(playheadIn(input({ track, placement: 'P2', tick: 10 }))).toBeNull()
    expect(playheadIn(input({ track, placement: null, tick: 10 }))).toBe(10)
  })

  it('has none for a placement of another clip', () => {
    const track = { placements: { P1: place('C2', 0, 3840) } }
    expect(playheadIn(input({ track, tick: 100 }))).toBeNull()
  })

  it('stops a placement at the start of the next one (the last started wins)', () => {
    const track = { placements: { P1: place('C1', 0, 7680), P2: place('C2', 1920, 1920) } }
    expect(playheadIn(input({ track, looping: { start: 0, end: 1920 }, tick: 1919 }))).toBe(1919)
    expect(playheadIn(input({ track, tick: 1920 }))).toBeNull()
  })

  it('does not read a placement in frames', () => {
    const track = { placements: { P1: { clip: 'C1', start: 0, span: { frames: { length: 48000, offset: 0 } } } as Placement } }
    expect(playheadIn(input({ track, tick: 100 }))).toBeNull()
  })

  it('skips an empty slot of the table', () => {
    const track = { placements: { P0: undefined, P1: place('C1', 0, 3840) } }
    expect(playheadIn(input({ track, tick: 100 }))).toBe(100)
  })
})

describe('settle', () => {
  it('holds a small step back and follows a large one (a seek, a loop wrap)', () => {
    // 120 bpm: 100 ms is 192 ticks.
    expect(settle(1000, 900, 120)).toBe(1000)
    expect(settle(1000, 808, 120)).toBe(1000)
    expect(settle(1000, 807, 120)).toBe(807)
    expect(settle(3800, 40, 120)).toBe(40)
  })

  it('holds a report taken just before the loop end when the carried tick has already wrapped', () => {
    const loop = { start: 0, end: 3840 }
    // Shown 20 ticks into the next pass, the report says 3800: 60 ticks behind around the loop.
    expect(settle(20, 3800, 120, loop)).toBe(20)
    // Shown well into the pass, a report near the end is ahead, not behind.
    expect(settle(1000, 3800, 120, loop)).toBe(3800)
    // Behind by more than the tolerance (192 ticks at 120 bpm): a seek, followed.
    expect(settle(300, 3600, 120, loop)).toBe(3600)
    // A report outside the loop is not read around it.
    expect(settle(20, 4500, 120, loop)).toBe(4500)
  })

  it('does not hold around a loop shorter than twice the tolerance (it cannot tell a wrap from a seek)', () => {
    expect(settle(10, 350, 120, { start: 0, end: 380 })).toBe(350)
  })

  it('follows a step forward, and starts from nothing', () => {
    expect(settle(1000, 1010, 120)).toBe(1010)
    expect(settle(null, 5, 120)).toBe(5)
    expect(settle(1000, 900, null)).toBe(900)
  })
})

describe('liveTick', () => {
  it('carries the tick on at the tempo, 960 ticks per quarter', () => {
    // 120 bpm: 2 quarters a second, 1920 ticks a second, 19.2 ticks in 10 ms.
    expect(liveTick(1000, true, 10, 120)).toBeCloseTo(1019.2, 6)
  })

  it('never runs more than the cap past the report', () => {
    expect(liveTick(0, true, 5000, 120)).toBeCloseTo((MAX_EXTRAPOLATION_MS / 1000) * 1920, 6)
  })

  it('wraps at the end of the transport loop instead of running past it', () => {
    const loop = { start: 0, end: 3840 }
    // 100 ms at 120 bpm is 192 ticks: 3800 + 192 = 3992, 152 ticks into the next pass.
    expect(liveTick(3800, true, 100, 120, loop)).toBeCloseTo(152, 6)
    expect(liveTick(1000, true, 10, 120, loop)).toBeCloseTo(1019.2, 6)
    // A report outside the region (before it, or after it) is not wrapped.
    expect(liveTick(4000, true, 10, 120, loop)).toBeCloseTo(4019.2, 6)
  })

  it('stands still when stopped, without a tempo, or with a clock that went back', () => {
    expect(liveTick(500, false, 30, 120)).toBe(500)
    expect(liveTick(500, true, 30, null)).toBe(500)
    expect(liveTick(500, true, 30, 0)).toBe(500)
    expect(liveTick(500, true, -30, 120)).toBe(500)
  })
})
