import { describe, expect, it } from 'vitest'
import type { AutomationTarget, Shape, Track } from '../../../lib/generated/protocol'
import { MAX_TICK } from '../arrangement'
import { CURVE_CODE, POINT_CODE, RAMP_CODE, entryIndex, goIndex, laneKey, nextShape, nudgedTick, pointKey, rampPlan, shapeAt, stepValue } from './keys'
import { PLAYING_KEYS } from '../../../lib/keyboard'
import { axisOf, type Axis, type Labels, type LanePoint } from './model'

const labels: Labels = { mixer: 'Mixer', mixerParam: (p) => p, param: (_d, p) => p.key, choice: (_d, k) => k }
const track = {
  devices: { D1: { role: 'instrument', source: { builtin: { id: 'faulk.osc_synth', version: 1 } }, order: 'a0', enabled: true, params: {} } },
  automation: {},
} as unknown as Pick<Track, 'devices'>
const axis = (target: AutomationTarget): Axis => axisOf(track, target, labels)!
const pan = axis({ mixer: { param: 'pan' } })
const volume = axis({ mixer: { param: 'gain' } })

const key = (k: string, extra: Partial<KeyboardEvent> = {}) =>
  ({ key: k, code: '', altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, repeat: false, isComposing: false, ...extra }) as KeyboardEvent
const pts = (...rows: [number, number, Shape?][]): LanePoint[] => rows.map(([tick, value, shape]) => ({ tick, value, shape: shape ?? 'linear' }))

describe('the keys of a lane (#769)', () => {
  it('I (or Insert) adds a point at the playhead and R opens the ramp, by position, on the head and on a point', () => {
    expect(laneKey(key('i', { code: 'KeyI' }))).toBe('point')
    expect(laneKey(key('Insert', { code: 'Insert' }))).toBe('point')
    // By position: the key in the place of I, whatever letter the layout gives it.
    expect(laneKey(key('î', { code: 'KeyI' }))).toBe('point')
    expect(laneKey(key('i', { code: 'KeyO' }))).toBeNull()
    expect(laneKey(key('r', { code: 'KeyR' }))).toBe('ramp')
    expect(pointKey(key('i', { code: 'KeyI' }))).toEqual({ kind: 'point' })
    expect(pointKey(key('r', { code: 'KeyR' }))).toEqual({ kind: 'ramp' })
  })

  it('takes no I or R with a modifier, a repeat or a composition', () => {
    for (const extra of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }, { shiftKey: true }, { repeat: true }, { isComposing: true }]) {
      expect(laneKey(key('i', { code: 'KeyI', ...extra }))).toBeNull()
      expect(laneKey(key('r', { code: 'KeyR', ...extra }))).toBeNull()
    }
  })

  it('goes into the points with Right on the head, and nothing else', () => {
    expect(laneKey(key('ArrowRight'))).toBe('enter')
    expect(laneKey(key('ArrowRight', { altKey: true }))).toBeNull()
    expect(laneKey(key('ArrowLeft'))).toBeNull()
    expect(laneKey(key('Enter'))).toBeNull()
  })

  it('moves, edits, deletes and nudges a point', () => {
    expect(pointKey(key('ArrowLeft'))).toEqual({ kind: 'go', to: 'prev' })
    expect(pointKey(key('ArrowRight'))).toEqual({ kind: 'go', to: 'next' })
    expect(pointKey(key('Home'))).toEqual({ kind: 'go', to: 'first' })
    expect(pointKey(key('End'))).toEqual({ kind: 'go', to: 'last' })
    expect(pointKey(key('Enter'))).toEqual({ kind: 'edit' })
    expect(pointKey(key('Enter', { repeat: true }))).toBeNull()
    expect(pointKey(key('Delete'))).toEqual({ kind: 'delete' })
    expect(pointKey(key('Backspace'))).toEqual({ kind: 'delete' })
    expect(pointKey(key('Escape'))).toEqual({ kind: 'leave' })
    expect(pointKey(key('ArrowUp', { altKey: true }))).toEqual({ kind: 'value', direction: 1, big: false })
    expect(pointKey(key('ArrowDown', { altKey: true, shiftKey: true }))).toEqual({ kind: 'value', direction: -1, big: true })
    expect(pointKey(key('ArrowRight', { altKey: true }))).toEqual({ kind: 'move', direction: 1 })
    expect(pointKey(key('ArrowLeft', { altKey: true }))).toEqual({ kind: 'move', direction: -1 })
    // Ctrl and Cmd are the page's (undo, zoom), Shift+arrows extend nothing here, plain Up and Down are not the point's.
    expect(pointKey(key('ArrowLeft', { ctrlKey: true }))).toBeNull()
    expect(pointKey(key('ArrowLeft', { shiftKey: true }))).toBeNull()
    expect(pointKey(key('ArrowRight', { altKey: true, shiftKey: true }))).toBeNull()
    expect(pointKey(key('ArrowUp'))).toBeNull()
  })

  it('C gives a point the next curve and Shift+C the previous one, by position, with no other modifier and no repeat (#881)', () => {
    expect(pointKey(key('c', { code: 'KeyC' }))).toEqual({ kind: 'curve', direction: 1 })
    expect(pointKey(key('C', { code: 'KeyC', shiftKey: true }))).toEqual({ kind: 'curve', direction: -1 })
    // By position: the key in the place of C on any layout.
    expect(pointKey(key('ç', { code: 'KeyC' }))).toEqual({ kind: 'curve', direction: 1 })
    expect(pointKey(key('c', { code: 'KeyV' }))).toBeNull()
    for (const extra of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }, { repeat: true }, { isComposing: true }]) {
      expect(pointKey(key('c', { code: 'KeyC', ...extra })), JSON.stringify(extra)).toBeNull()
    }
    // Not on the lane's head: a curve belongs to a point.
    expect(laneKey(key('c', { code: 'KeyC' }))).toBeNull()
  })

  it('cycles the curves in the order of the list, round both ends', () => {
    expect(nextShape('linear', 1)).toBe('step')
    expect(nextShape('step', 1)).toBe('ease_in')
    expect(nextShape('ease_in', 1)).toBe('ease_out')
    expect(nextShape('ease_out', 1)).toBe('smooth')
    expect(nextShape('smooth', 1)).toBe('linear')
    expect(nextShape('linear', -1)).toBe('smooth')
    expect(nextShape('ease_in', -1)).toBe('step')
  })
})

describe('where the keyboard goes in a lane', () => {
  const points = pts([0, 0], [960, 0.5], [1920, 1])
  it('enters on the first point at or after the playhead, else on the last, and on none in an empty lane', () => {
    expect(entryIndex(points, 0)).toBe(0)
    expect(entryIndex(points, 1)).toBe(1)
    expect(entryIndex(points, 960)).toBe(1)
    expect(entryIndex(points, 5000)).toBe(2)
    expect(entryIndex([], 0)).toBe(-1)
  })
  it('goes from point to point and stays at the ends', () => {
    expect(goIndex(3, 0, 'prev')).toBe(0)
    expect(goIndex(3, 0, 'next')).toBe(1)
    expect(goIndex(3, 2, 'next')).toBe(2)
    expect(goIndex(3, 1, 'first')).toBe(0)
    expect(goIndex(3, 1, 'last')).toBe(2)
    expect(goIndex(0, 0, 'next')).toBe(-1)
  })
})

describe('a value moved by a step', () => {
  it('moves by 1 % of the travel, 10 % with Shift, rounded to the step, and stays at a bound', () => {
    expect(stepValue(pan, 0, 1, false)).toBe(0.02)
    expect(stepValue(pan, 0, -1, true)).toBe(-0.2)
    expect(stepValue(pan, 1, 1, false)).toBe(1)
    expect(stepValue(pan, -1, -1, true)).toBe(-1)
  })
  it('moves a volume along the fader, kept to -96..+6 dB', () => {
    const up = stepValue(volume, 0, 1, false)
    expect(up).toBeGreaterThan(0)
    expect(up).toBeLessThanOrEqual(6)
    expect(stepValue(volume, 6, 1, true)).toBe(6)
    expect(stepValue(volume, -96, -1, false)).toBe(-96)
  })
  it('moves a parameter with a coarse step by at least one step', () => {
    const coarse: Axis = { ...pan, pos: (v) => v / 4, at: (p) => Math.round(p * 4), clamp: (v) => Math.round(v) }
    expect(stepValue(coarse, 1, 1, false)).toBe(2)
    expect(stepValue(coarse, 4, 1, false)).toBe(4)
  })
})

describe('a point moved by a snap step', () => {
  it('goes to the next multiple of the step, and not past the timeline', () => {
    expect(nudgedTick(0, 1, 960)).toBe(960)
    expect(nudgedTick(500, 1, 960)).toBe(960)
    expect(nudgedTick(500, -1, 960)).toBe(0)
    expect(nudgedTick(960, -1, 960)).toBe(0)
    expect(nudgedTick(0, -1, 960)).toBeNull()
    expect(nudgedTick(MAX_TICK, 1, 960)).toBeNull()
  })
})

describe('a ramp (one automation.points.restore)', () => {
  it('writes the playhead point (linear) and the typed one, and takes out the points between', () => {
    const points = pts([0, 0.1], [960, 0.2, 'step'], [1920, 0.3], [3840, 0.4, 'step'])
    expect(rampPlan(points, { tick: 480, value: 0.1 }, { tick: 3840, value: 0.9 })).toEqual({
      points: [
        { tick: 480, value: 0.1, shape: 'linear' },
        // The end keeps the curve of the point it lands on.
        { tick: 3840, value: 0.9, shape: 'step' },
      ],
      remove: [960, 1920],
    })
  })
  it('makes a step point at the playhead start a linear ramp, and goes backwards too', () => {
    const points = pts([960, 0.2, 'step'], [1920, 0.3])
    expect(rampPlan(points, { tick: 960, value: 0.2 }, { tick: 1920, value: 1 })).toEqual({
      points: [
        { tick: 960, value: 0.2, shape: 'linear' },
        { tick: 1920, value: 1, shape: 'linear' },
      ],
      remove: [],
    })
    // Typed before the playhead: the typed point starts the ramp, the playhead's ends it.
    expect(rampPlan(points, { tick: 1920, value: 0.3 }, { tick: 0, value: 0 })).toEqual({
      points: [
        { tick: 0, value: 0, shape: 'linear' },
        { tick: 1920, value: 0.3, shape: 'linear' },
      ],
      remove: [960],
    })
  })
  it('keeps a held step after it: a ramp back from inside a step does not fade what follows the playhead', () => {
    // 0 dB held from bar 1 until -60 dB at bar 9; the playhead at bar 5, a ramp back to bar 3.
    const held = pts([0, 0, 'step'], [8 * 3840, -60])
    expect(rampPlan(held, { tick: 4 * 3840, value: 0 }, { tick: 2 * 3840, value: -24 })?.points).toEqual([
      { tick: 2 * 3840, value: -24, shape: 'linear' },
      { tick: 4 * 3840, value: 0, shape: 'step' },
    ])
  })
  it('gives the earlier point the curve asked, whichever end the playhead is (#881)', () => {
    const points = pts([0, 0], [1920, 1, 'step'])
    expect(rampPlan(points, { tick: 480, value: 0.1 }, { tick: 3840, value: 0.9 }, 'ease_in')?.points).toEqual([
      { tick: 480, value: 0.1, shape: 'ease_in' },
      { tick: 3840, value: 0.9, shape: 'step' },
    ])
    expect(rampPlan(points, { tick: 3840, value: 0.9 }, { tick: 480, value: 0.1 }, 'smooth')?.points).toEqual([
      { tick: 480, value: 0.1, shape: 'smooth' },
      { tick: 3840, value: 0.9, shape: 'step' },
    ])
  })
  it('is no ramp on one tick, and writes two points in an empty lane', () => {
    expect(rampPlan([], { tick: 10, value: 0 }, { tick: 10, value: 1 })).toBeNull()
    expect(rampPlan([], { tick: 0, value: 0 }, { tick: 10, value: 1 })?.points).toHaveLength(2)
  })
})

describe('the curve of a new point', () => {
  it('is the curve of a point already on its tick, else of the segment it lands in, else linear', () => {
    const points = pts([960, 0, 'step'], [1920, 1])
    expect(shapeAt(points, 960)).toBe('step')
    expect(shapeAt(points, 1000)).toBe('step')
    expect(shapeAt(points, 2000)).toBe('linear')
    expect(shapeAt(points, 0)).toBe('linear')
    expect(shapeAt([], 0)).toBe('linear')
    // Inside a curve: the curve, split in two (#881).
    expect(shapeAt(pts([0, 0, 'smooth'], [960, 1]), 480)).toBe('smooth')
  })
  it('I, R and C play nothing (UI-3: a letter plays or it is a shortcut)', () => {
    expect(PLAYING_KEYS).not.toContain(POINT_CODE)
    expect(PLAYING_KEYS).not.toContain(RAMP_CODE)
    expect(PLAYING_KEYS).not.toContain(CURVE_CODE)
  })
})
