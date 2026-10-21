import { describe, expect, it } from 'vitest'
import { barTicks, beatTicks, PPQ } from '../../../views/transport/time'
import {
  DEFAULT_PX_PER_TICK,
  DEFAULT_TOTAL_BARS,
  MAX_PX_PER_TICK,
  MIN_LABEL_PX,
  MIN_MARK_PX,
  MIN_PX_PER_TICK,
  SNAP_SETTINGS,
  clampView,
  gridStep,
  minPxPerTick,
  rulerMarks,
  scrollBy,
  snapStep,
  snapTick,
  tickToX,
  totalTicksOf,
  visibleRange,
  xToTick,
  zoomAt,
  type View,
} from './grid'

const common = { numerator: 4, denominator: 4 }
const waltz = { numerator: 3, denominator: 4 }
const compound = { numerator: 6, denominator: 8 }
const septuple = { numerator: 7, denominator: 8 }
const fine = { numerator: 4, denominator: 32 }
const SIGNATURES = [common, waltz, compound, septuple, fine, { numerator: 1, denominator: 1 }, { numerator: 32, denominator: 2 }]

const WIDTH = 1000
const TOTAL = totalTicksOf(DEFAULT_TOTAL_BARS, common)
const view = (pxPerTick: number, scrollTick = 0): View => ({ pxPerTick, scrollTick })

describe('ticks and pixels', () => {
  it('converts both ways and round-trips', () => {
    const v = view(0.25, 400)
    expect(tickToX(400, v)).toBe(0)
    expect(tickToX(1400, v)).toBe(250)
    expect(xToTick(250, v)).toBe(1400)
    for (const tick of [0, 1, 959, 960, 123456, TOTAL]) expect(xToTick(tickToX(tick, v), v)).toBeCloseTo(tick, 9)
  })

  it('one bar of 4/4 is 3840 ticks, 200 bars 768000', () => {
    expect(barTicks(common)).toBe(PPQ * 4)
    expect(TOTAL).toBe(768_000)
    expect(totalTicksOf(0, common)).toBe(barTicks(common))
  })

  it('reports the visible range', () => {
    expect(visibleRange(view(0.5, 100), 200)).toEqual([100, 500])
  })
})

describe('clampView', () => {
  it('keeps zoom and scroll inside the grid', () => {
    expect(clampView(view(DEFAULT_PX_PER_TICK, -50), WIDTH, TOTAL)).toEqual(view(DEFAULT_PX_PER_TICK, 0))
    expect(clampView(view(DEFAULT_PX_PER_TICK, 1e9), WIDTH, TOTAL).scrollTick).toBe(TOTAL - WIDTH / DEFAULT_PX_PER_TICK)
    expect(clampView(view(50), WIDTH, TOTAL).pxPerTick).toBe(MAX_PX_PER_TICK)
  })

  it('refuses a zoom that shows more than the grid holds', () => {
    expect(clampView(view(1e-9), WIDTH, TOTAL).pxPerTick).toBeCloseTo(WIDTH / TOTAL, 12)
    expect(clampView(view(1e-9), WIDTH, TOTAL).scrollTick).toBe(0)
  })

  it('never goes below the absolute minimum for a very long grid', () => {
    expect(minPxPerTick(WIDTH, 1e12)).toBe(MIN_PX_PER_TICK)
  })

  it('a grid shorter than the screen sits at the start', () => {
    const v = clampView(view(1, 500), WIDTH, 300)
    expect(v.scrollTick).toBe(0)
  })

  it('survives a zero width and non-finite numbers', () => {
    expect(clampView(view(DEFAULT_PX_PER_TICK, 500), 0, TOTAL).scrollTick).toBe(500)
    for (const bad of [NaN, Infinity, -Infinity]) {
      const v = clampView({ pxPerTick: bad, scrollTick: bad }, WIDTH, TOTAL)
      expect(Number.isFinite(v.pxPerTick) && Number.isFinite(v.scrollTick)).toBe(true)
      expect(v.pxPerTick).toBeGreaterThan(0)
    }
  })
})

describe('zoomAt', () => {
  it('keeps the tick under the pointer where it is', () => {
    const v = view(0.1, 20_000)
    for (const [factor, anchor] of [[2, 300], [0.5, 700], [1.25, 0], [3, 1000]] as const) {
      const zoomed = zoomAt(v, factor, anchor, WIDTH, TOTAL)
      expect(xToTick(anchor, zoomed)).toBeCloseTo(xToTick(anchor, v), 6)
      expect(zoomed.pxPerTick).toBeCloseTo(v.pxPerTick * factor, 12)
    }
  })

  it('stops at the largest zoom and keeps the anchor there too', () => {
    const v = view(0.9, 10_000)
    const zoomed = zoomAt(v, 10, 400, WIDTH, TOTAL)
    expect(zoomed.pxPerTick).toBe(MAX_PX_PER_TICK)
    expect(xToTick(400, zoomed)).toBeCloseTo(xToTick(400, v), 6)
  })

  it('stops at the whole grid when zooming out', () => {
    const zoomed = zoomAt(view(0.1, 30_000), 1e-6, 500, WIDTH, TOTAL)
    expect(zoomed.pxPerTick).toBeCloseTo(WIDTH / TOTAL, 12)
    expect(zoomed.scrollTick).toBe(0)
  })

  it('ignores a factor that is not a number', () => {
    const v = view(0.1, 20_000)
    expect(zoomAt(v, NaN, 300, WIDTH, TOTAL)).toEqual(v)
  })
})

describe('scrollBy', () => {
  it('moves by pixels and clamps at both ends', () => {
    const v = view(0.1, 1000)
    expect(scrollBy(v, 100, WIDTH, TOTAL).scrollTick).toBe(2000)
    expect(scrollBy(v, -1e6, WIDTH, TOTAL).scrollTick).toBe(0)
    expect(scrollBy(v, 1e9, WIDTH, TOTAL).scrollTick).toBe(TOTAL - WIDTH / 0.1)
    expect(scrollBy(v, NaN, WIDTH, TOTAL)).toEqual(v)
  })
})

describe('gridStep', () => {
  it('is always at least the minimum width, and the finest such step', () => {
    for (const sig of SIGNATURES) {
      for (const px of [MIN_PX_PER_TICK, 0.002, 0.01, 0.05, 0.1, 0.3, 0.7, MAX_PX_PER_TICK]) {
        const step = gridStep(sig, px)
        expect(Number.isInteger(step), `${sig.numerator}/${sig.denominator} @${px}`).toBe(true)
        // Wide enough, unless even the largest step is too narrow.
        if (step < barTicks(sig) * 1024) expect(step * px).toBeGreaterThanOrEqual(MIN_MARK_PX)
      }
    }
  })

  it('walks from sixteenths of a beat to many bars as the zoom goes down', () => {
    expect(gridStep(common, MAX_PX_PER_TICK)).toBe(PPQ / 16)
    expect(gridStep(common, 0.1)).toBe(PPQ / 8)
    expect(gridStep(common, 0.05)).toBe(PPQ / 4)
    expect(gridStep(common, 0.03)).toBe(PPQ / 2)
    expect(gridStep(common, 0.02)).toBe(PPQ)
    expect(gridStep(common, 0.013)).toBe(PPQ)
    expect(gridStep(common, 0.0033)).toBe(barTicks(common))
    expect(gridStep(common, 0.001)).toBe(barTicks(common) * 4)
  })

  it('follows the beat of the signature', () => {
    expect(beatTicks(compound)).toBe(PPQ / 2)
    expect(gridStep(compound, 0.1)).toBe(PPQ / 2 / 4)
    // A thirty-second-note beat of 120 ticks has no whole sixteenth: the subdivisions that are not whole are skipped.
    expect(gridStep(fine, MAX_PX_PER_TICK)).toBe(15)
  })

  it('a bar is one beat long in x/1 and the step stays valid', () => {
    expect(gridStep({ numerator: 1, denominator: 1 }, 0.001)).toBeGreaterThan(0)
  })
})

describe('snapping', () => {
  it('rounds to the nearest multiple, halfway going later', () => {
    expect(snapTick(100, 240)).toBe(0)
    expect(snapTick(119, 240)).toBe(0)
    expect(snapTick(120, 240)).toBe(240)
    expect(snapTick(121, 240)).toBe(240)
    expect(snapTick(359, 240)).toBe(240)
    expect(snapTick(360, 240)).toBe(480)
  })

  it('is idempotent on its own result', () => {
    for (const tick of [0, 13, 5000, 123456.7]) {
      const once = snapTick(tick, 480)
      expect(snapTick(once, 480)).toBe(once)
    }
  })

  it('never goes before the start', () => {
    expect(snapTick(-5000, 240)).toBe(0)
    expect(snapTick(-1, null)).toBe(0)
    expect(snapTick(-0.4, 240)).toBe(0)
  })

  it('without a step only rounds to a whole tick', () => {
    expect(snapTick(1234.4, null)).toBe(1234)
    expect(snapTick(1234.6, null)).toBe(1235)
    expect(snapTick(1234.6, 0)).toBe(1235)
    expect(snapTick(1234.6, -3)).toBe(1235)
    expect(snapTick(NaN, 240)).toBe(0)
    expect(snapTick(Infinity, null)).toBe(0)
  })

  it('snaps to bars of the signature, beyond the first one', () => {
    const bar = barTicks(waltz)
    expect(bar).toBe(2880)
    expect(snapTick(bar * 7 + 1400, bar)).toBe(bar * 7 + 2880 * 0)
    expect(snapTick(bar * 7 + 1500, bar)).toBe(bar * 8)
  })

  it('has a step for every setting and signature', () => {
    for (const sig of SIGNATURES) {
      expect(snapStep('off', sig, 0.1)).toBeNull()
      for (const setting of SNAP_SETTINGS.filter((s) => s !== 'off')) {
        const step = snapStep(setting, sig, 0.1)
        expect(step, `${setting} ${sig.numerator}/${sig.denominator}`).toBeGreaterThanOrEqual(1)
      }
      expect(snapStep('bar', sig, 0.1)).toBe(barTicks(sig))
      expect(snapStep('beat', sig, 0.1)).toBe(beatTicks(sig))
      expect(snapStep('half-beat', sig, 0.1)).toBe(beatTicks(sig) / 2)
      expect(snapStep('quarter-beat', sig, 0.1)).toBe(beatTicks(sig) / 4)
    }
  })

  it('auto follows the zoom, so snapping gets coarser as the grid zooms out', () => {
    expect(snapStep('auto', common, 0.1)).toBe(gridStep(common, 0.1))
    expect(snapStep('auto', common, 0.005)).toBeGreaterThan(snapStep('auto', common, 0.1) as number)
  })

  it('a snapped position of the pointer lands on a ruler mark in auto', () => {
    const v = view(0.1, 5000)
    const marks = new Set(rulerMarks(v, WIDTH, common, TOTAL).map((m) => m.tick))
    const step = snapStep('auto', common, v.pxPerTick)
    for (let x = 0; x <= WIDTH; x += 37) {
      const snapped = snapTick(xToTick(x, v), step)
      if (snapped >= v.scrollTick && snapped <= visibleRange(v, WIDTH)[1]) expect(marks.has(snapped), `x=${x}`).toBe(true)
    }
  })
})

describe('rulerMarks', () => {
  it('covers the visible range with marks of the right kind', () => {
    const marks = rulerMarks(view(0.1, 0), WIDTH, common, TOTAL)
    expect(marks[0]).toMatchObject({ tick: 0, x: 0, kind: 'bar', label: '1' })
    const bar2 = marks.find((m) => m.tick === 3840)
    expect(bar2).toMatchObject({ kind: 'bar', label: '2', x: 384 })
    expect(marks.find((m) => m.tick === 960)?.kind).toBe('beat')
    expect(marks.find((m) => m.tick === 480)?.kind).toBe('sub')
    for (const m of marks) {
      expect(m.x).toBeGreaterThanOrEqual(0)
      expect(m.x).toBeLessThanOrEqual(WIDTH)
    }
  })

  it('includes the marks on both edges and none outside', () => {
    const v = view(0.1, 960)
    const marks = rulerMarks(v, 960 * 0.1 * 2, common, TOTAL)
    expect(marks[0].tick).toBe(960)
    expect(marks[marks.length - 1].tick).toBeLessThanOrEqual(2880)
  })

  it('stays bounded in count at every zoom, over the 200 bars', () => {
    for (const sig of SIGNATURES) {
      const total = totalTicksOf(DEFAULT_TOTAL_BARS, sig)
      for (const px of [MIN_PX_PER_TICK, 0.003, 0.02, 0.1, 0.5, MAX_PX_PER_TICK]) {
        const v = clampView(view(px, total / 3), WIDTH, total)
        expect(rulerMarks(v, WIDTH, sig, total).length).toBeLessThanOrEqual(WIDTH / MIN_MARK_PX + 2)
      }
    }
  })

  it('names bars so two names are never closer than the minimum, and every name is a real bar', () => {
    for (const px of [0.001, 0.0017, 0.003, 0.006, 0.01, 0.02, 0.1]) {
      const v = clampView(view(px, 0), WIDTH, TOTAL)
      const labelled = rulerMarks(v, WIDTH, common, TOTAL).filter((m) => m.label !== null)
      expect(labelled.length).toBeGreaterThan(0)
      for (let i = 0; i < labelled.length; i++) {
        expect(labelled[i].label).toBe(String(labelled[i].tick / barTicks(common) + 1))
        if (i > 0) expect(labelled[i].x - labelled[i - 1].x).toBeGreaterThanOrEqual(MIN_LABEL_PX - 1e-9)
      }
    }
  })

  it('stops at the end of the grid', () => {
    const v = clampView(view(0.1, 1e9), WIDTH, TOTAL)
    const marks = rulerMarks(v, WIDTH, common, TOTAL)
    expect(marks[marks.length - 1].tick).toBeLessThanOrEqual(TOTAL)
    expect(marks.some((m) => m.tick === TOTAL)).toBe(true)
  })

  it('draws bars of other signatures where they fall', () => {
    const marks = rulerMarks(view(0.1, 0), WIDTH, waltz, totalTicksOf(10, waltz))
    expect(marks.filter((m) => m.kind === 'bar').map((m) => m.tick)).toEqual([0, 2880, 5760, 8640].filter((t) => t <= 10000 && t <= totalTicksOf(10, waltz)))
    expect(marks.filter((m) => m.kind === 'beat').map((m) => m.tick)).toContain(960)
  })

  it('an empty width gives no mark past the first', () => {
    expect(rulerMarks(view(0.1, 0), 0, common, TOTAL).length).toBeLessThanOrEqual(1)
  })
})
