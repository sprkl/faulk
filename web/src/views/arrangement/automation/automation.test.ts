import { describe, expect, it } from 'vitest'
import type { AutomationLane, AutomationTarget, Shape, Track } from '../../../lib/generated/protocol'
import { RULER_HEIGHT } from '../../../lib/components/timegrid/render'
import { MAX_TICK } from '../arrangement'
import { asPoint, dragDelta, hitPoint, moveSelection, newPoint, planMove, pointsInBox, posOfY, previewPoints, yOfPos, POINT_RADIUS } from './gestures'
import { ADD_ROW, CLIP_ROW, CLIP_ROW_MAX, CLIP_ROW_MIN, LANE_ROW, clampClipRow, layoutOf, rowAtY, rowsBetween } from './layout'
import { drawLane } from './draw'
import { SHAPES, axisOf, lanesOf, optionsOf, pointAtTick, pointsOf, segmentValue, shapeWarp, targetKey, valueAt, type Labels, type LanePoint } from './model'
import { positionOf as faderPosition } from '../../../lib/project/fader'

const labels: Labels = {
  mixer: 'Mixer',
  mixerParam: (p) => (p === 'gain' ? 'Volume' : 'Pan'),
  param: (_d, p) => p.key,
  choice: (_d, k) => k,
}

const SYNTH = 'faulk.osc_synth'
const synthTrack = (automation: Track['automation'] = {}): Pick<Track, 'devices' | 'automation'> =>
  ({
    devices: { D1: { role: 'instrument', source: { builtin: { id: SYNTH, version: 1 } }, order: 'a0', enabled: true, params: {} } },
    automation,
  }) as unknown as Pick<Track, 'devices' | 'automation'>

const laneOf = (target: AutomationTarget, points: Record<number, [number, 'linear' | 'step']>): AutomationLane => ({
  target,
  points: Object.fromEntries(Object.entries(points).map(([tick, [value, shape]]) => [tick, { value, shape }])),
})
const gainTarget: AutomationTarget = { device: { device: 'D1', param: 'gain' } }
const volume: AutomationTarget = { mixer: { param: 'gain' } }
const pan: AutomationTarget = { mixer: { param: 'pan' } }

const pts = (...rows: [number, number, Shape?][]): LanePoint[] => rows.map(([tick, value, shape]) => ({ tick, value, shape: shape ?? 'linear' }))

describe('the points of a lane', () => {
  it('are sorted by tick whatever the order of the keys, and cached per lane object', () => {
    const lane = laneOf(gainTarget, { 960: [0.5, 'linear'], 0: [0.1, 'step'], 480: [0.2, 'linear'] })
    const points = pointsOf(lane)
    expect(points.map((p) => p.tick)).toEqual([0, 480, 960])
    expect(pointsOf(lane)).toBe(points)
    expect(pointAtTick(points, 480)?.value).toBe(0.2)
    expect(pointAtTick(points, 481)).toBeNull()
  })
})

describe('the parameters a track can automate', () => {
  it('lists the volume and the pan, then each parameter of the instrument', () => {
    const options = optionsOf(synthTrack(), labels)
    expect(options.slice(0, 2).map((o) => o.label)).toEqual(['Volume', 'Pan'])
    expect(options.map((o) => o.group)).toEqual(['Mixer', 'Mixer', ...Array(6).fill('Oscillator')])
    expect(options.some((o) => o.key === targetKey(gainTarget))).toBe(true)
    expect(new Set(options.map((o) => o.key)).size).toBe(options.length)
  })

  it('shows the lanes of the track in the order of the menu, and leaves out a lane the page cannot draw', () => {
    const track = synthTrack({
      L3: laneOf(gainTarget, { 0: [0.5, 'linear'] }),
      L1: laneOf(pan, { 0: [0, 'linear'] }),
      L2: laneOf({ device: { device: 'GONE', param: 'gain' } }, { 0: [0.5, 'linear'] }),
      L4: laneOf(volume, { 0: [0, 'linear'] }),
    })
    expect(lanesOf(track, labels).map((l) => l.id)).toEqual(['L4', 'L1', 'L3'])
  })
})

describe('the axes', () => {
  const track = synthTrack()
  it('map the mixer volume on the fader taper and keep to its range in 0.01 dB', () => {
    const axis = axisOf(track, volume, labels)!
    expect(axis.pos(0)).toBeCloseTo(0.78, 5)
    expect(axis.pos(-96)).toBe(0)
    expect(axis.at(0)).toBe(-96)
    expect(axis.at(1)).toBe(6)
    expect(axis.clamp(-200)).toBe(-96)
    expect(axis.clamp(0.004)).toBe(0)
    expect(axis.format(-6)).toBe('−6.0 dB')
    expect(axis.parse('-12')).toEqual({ value: -12 })
    expect(axis.parse('-inf')).toEqual({ value: -96 })
    expect(axis.parse('-200')).toEqual({ value: -96, limit: -96 })
    expect(axis.parse('20')).toEqual({ value: 6, limit: 6 })
    expect(axis.parse('nonsense')).toBeNull()
  })
  it('map the pan linearly and read it as the pan control does', () => {
    const axis = axisOf(track, pan, labels)!
    expect([axis.pos(-1), axis.pos(0), axis.pos(1)]).toEqual([0, 0.5, 1])
    expect(axis.at(0.5)).toBe(0)
    expect(Object.is(axis.at(0.5), -0)).toBe(false)
    expect(axis.format(-0.3)).toBe('L30')
    expect(axis.parse('R25')).toEqual({ value: 0.25 })
    expect(axis.parse('R150')).toEqual({ value: 1, limit: 1 })
    expect(axis.parse('-200')).toEqual({ value: -1, limit: -1 })
  })
  it('follow the descriptor of a device parameter: scale, range, unit', () => {
    const axis = axisOf(track, { device: { device: 'D1', param: 'attack' } }, labels)!
    expect(axis.min).toBe(0.002)
    expect(axis.max).toBe(30)
    expect(axis.pos(axis.at(0.5))).toBeCloseTo(0.5, 3)
    expect(axis.format(0.25)).toBe('250 ms')
    expect(axis.parse('120 ms')!.value).toBeCloseTo(0.12, 6)
    expect(axis.parse('120 ms')!.limit).toBeUndefined()
    expect(axis.parse('99 s')).toEqual({ value: 30, limit: 30 })
    expect(axis.parse('1 parsec')).toBeNull()
  })
  it('name the values of a choice parameter', () => {
    const axis = axisOf(track, { device: { device: 'D1', param: 'waveform' } }, labels)!
    expect(axis.format(2)).toBe('square')
    expect(axis.clamp(1.4)).toBe(1)
  })
  it('have none for a device without a descriptor', () => {
    expect(axisOf(synthTrack(), { device: { device: 'GONE', param: 'x' } }, labels)).toBeNull()
  })
})

describe('the value of a lane at a tick', () => {
  const lane = (_target: AutomationTarget, ...rows: [number, number, Shape?][]) => pts(...rows)
  it('holds the first value before the first point and the last after the last', () => {
    const p = lane(gainTarget, [480, 0.2], [960, 0.8])
    expect(valueAt(p, 0, gainTarget)).toBe(0.2)
    expect(valueAt(p, 5000, gainTarget)).toBe(0.8)
    expect(valueAt([], 0, gainTarget)).toBeNull()
  })
  it('goes straight between linear points and holds across a step', () => {
    const p = lane(gainTarget, [0, 0, 'linear'], [960, 1, 'step'], [1920, 0.2])
    expect(valueAt(p, 480, gainTarget)).toBeCloseTo(0.5, 9)
    expect(valueAt(p, 960, gainTarget)).toBe(1)
    expect(valueAt(p, 1500, gainTarget)).toBe(1)
    expect(valueAt(p, 1920, gainTarget)).toBe(0.2)
  })
  it("interpolates a volume lane along the fader's travel, like the engine (#389: the middle of -96 to 0 dB is -21 dB)", () => {
    const p = lane(volume, [0, -96], [1000, 0])
    // Half of the travel from the bottom to unity (0.39) is -21 dB: not -6 dB (amplitude), not -48 dB (dB).
    expect(valueAt(p, 500, volume)).toBeCloseTo(-21, 9)
    expect(valueAt(p, 0, volume)).toBe(-96)
    expect(valueAt(p, 1000, volume)).toBeCloseTo(0, 9)
    // A fade down is the same curve backwards.
    expect(valueAt(lane(volume, [0, 0], [8000, -96]), 4000, volume)).toBeCloseTo(-21, 9)
    // A pan is interpolated in its own units.
    expect(valueAt(lane(pan, [0, -1], [1000, 1]), 250, pan)).toBeCloseTo(-0.5, 9)
  })
  it('warps that same line by the curve of the point, like the engine (#881), at a quarter and at the middle', () => {
    // The progress of each shape, written out (t², 1 − (1 − t)², 3t² − 2t³): at 1/4 and 1/2.
    const progress: Record<Shape, [number, number]> = {
      linear: [0.25, 0.5],
      step: [0, 0],
      ease_in: [0.0625, 0.25],
      ease_out: [0.4375, 0.75],
      smooth: [0.15625, 0.5],
    }
    for (const shape of SHAPES) {
      const [quarter, middle] = progress[shape]
      expect(shapeWarp(shape, 0), shape).toBe(0)
      if (shape !== 'step') expect(shapeWarp(shape, 1), shape).toBe(1)
      // A device parameter and a pan, in their unit.
      const device = lane(gainTarget, [0, 0, shape], [1000, 1])
      expect(valueAt(device, 250, gainTarget), shape).toBeCloseTo(quarter, 12)
      expect(valueAt(device, 500, gainTarget), shape).toBeCloseTo(middle, 12)
      expect(valueAt(lane(pan, [0, -1, shape], [1000, 1]), 500, pan), shape).toBeCloseTo(-1 + 2 * middle, 12)
      // A volume lane, along the fader's travel: the travel of -96 dB is 0, that of 0 dB is the warped part of it.
      const fade = lane(volume, [0, -96, shape], [1000, 0])
      expect(faderPosition(valueAt(fade, 250, volume)!), shape).toBeCloseTo(faderPosition(0) * quarter, 9)
      expect(faderPosition(valueAt(fade, 500, volume)!), shape).toBeCloseTo(faderPosition(0) * middle, 9)
      expect(valueAt(fade, 1000, volume), shape).toBe(0)
    }
    // Monotonic, inside [0, 1].
    for (const shape of SHAPES.filter((s) => s !== 'step')) {
      let last = 0
      for (let k = 1; k <= 100; k++) {
        const w = shapeWarp(shape, k / 100)
        expect(w >= last && w <= 1, `${shape} at ${k}`).toBe(true)
        last = w
      }
    }
    // Out of the segment, the progress is kept to it.
    const a = { tick: 0, value: 0, shape: 'ease_in' as const }
    const b = { tick: 10, value: 1, shape: 'linear' as const }
    expect(segmentValue(a, b, -1, gainTarget)).toBe(0)
    expect(segmentValue(a, b, 2, gainTarget)).toBe(1)
  })
})

describe('the rows', () => {
  const entries = [{ id: 'a' }, { id: 'b' }, { id: 'c' }]
  const layout = layoutOf(entries, new Set(['b']), (id) => (id === 'b' ? ['l1', 'l2'] : []))
  it('puts the lanes of an unfolded track under its clips, then the row that adds one', () => {
    expect(layout.rows.map((r) => r.kind)).toEqual(['track', 'track', 'lane', 'lane', 'add', 'track'])
    expect(layout.trackTop).toEqual([0, CLIP_ROW, CLIP_ROW + CLIP_ROW + 2 * LANE_ROW + ADD_ROW])
    expect(layout.total).toBe(3 * CLIP_ROW + 2 * LANE_ROW + ADD_ROW)
  })
  it('finds the row under a y, none outside', () => {
    expect(rowAtY(layout, -1)).toBeNull()
    expect(rowAtY(layout, 0)?.trackId).toBe('a')
    expect(rowAtY(layout, 2 * CLIP_ROW + 1)).toMatchObject({ kind: 'lane', lane: 'l1' })
    expect(rowAtY(layout, 2 * CLIP_ROW + LANE_ROW)).toMatchObject({ kind: 'lane', lane: 'l2' })
    expect(rowAtY(layout, 2 * CLIP_ROW + 2 * LANE_ROW)?.kind).toBe('add')
    expect(rowAtY(layout, layout.total)).toBeNull()
    expect(rowAtY(layout, Number.NaN)).toBeNull()
  })
  it('lists the rows that touch a span', () => {
    expect(rowsBetween(layout, CLIP_ROW + 5, CLIP_ROW + 10).map((r) => r.kind)).toEqual(['track'])
    expect(rowsBetween(layout, 0, layout.total).length).toBe(layout.rows.length)
  })
  it('gives each track the height the page keeps for it, clamped, else the base height', () => {
    const heights: Record<string, number> = { a: 100, c: 5 }
    const sized = layoutOf(
      entries,
      new Set(),
      () => [],
      (id) => heights[id],
      50,
    )
    expect(sized.trackHeight).toEqual([100, 50, CLIP_ROW_MIN])
    expect(sized.trackTop).toEqual([0, 100, 150])
    expect(sized.total).toBe(150 + CLIP_ROW_MIN)
    expect(rowAtY(sized, 120)?.trackId).toBe('b')
    // No height kept and no base: the lane token's default.
    expect(layoutOf(entries, new Set(), () => []).trackHeight).toEqual([CLIP_ROW, CLIP_ROW, CLIP_ROW])
  })
  it('clamps a track height to the bounds, whole, and falls back on an unreadable one', () => {
    expect(clampClipRow(10)).toBe(CLIP_ROW_MIN)
    expect(clampClipRow(9999)).toBe(CLIP_ROW_MAX)
    expect(clampClipRow(60.4)).toBe(60)
    expect(clampClipRow(Number.NaN)).toBe(CLIP_ROW)
    expect(clampClipRow(Number.POSITIVE_INFINITY, 33)).toBe(33)
  })
  it('shows a track with no lane as its clips and the row that adds one', () => {
    const only = layoutOf([{ id: 'a' }], new Set(['a']), () => [])
    expect(only.rows.map((r) => r.kind)).toEqual(['track', 'add'])
  })
})

describe('the gestures of a lane', () => {
  const view = { pxPerTick: 0.1, scrollTick: 0 }
  const geo = { top: RULER_HEIGHT, height: 84 }
  const axis = axisOf(synthTrack(), { device: { device: 'D1', param: 'sustain' } }, labels)!
  const points = pts([0, 0.2], [960, 0.8], [1920, 0.5])

  it('converts a value and a y both ways, kept to the row', () => {
    expect(yOfPos(1, geo)).toBe(geo.top + 10)
    expect(yOfPos(0, geo)).toBe(geo.top + 74)
    expect(posOfY(geo.top - 100, geo)).toBe(1)
    expect(posOfY(geo.top + 500, geo)).toBe(0)
    expect(posOfY(yOfPos(0.25, geo), geo)).toBeCloseTo(0.25, 9)
  })

  it('hits the nearest point inside the radius and nothing beyond', () => {
    const x = 96
    const y = yOfPos(axis.pos(0.8), geo)
    expect(hitPoint(points, axis, x + 3, y - 2, view, geo)?.tick).toBe(960)
    expect(hitPoint(points, axis, x + POINT_RADIUS + 1, y, view, geo)).toBeNull()
    expect(hitPoint(points, axis, x, y + POINT_RADIUS + 1, view, geo)).toBeNull()
    expect(hitPoint([], axis, x, y, view, geo)).toBeNull()
  })

  it('selects the points of a box, whichever corner it was drawn from', () => {
    const top = yOfPos(1, geo)
    const bottom = yOfPos(0, geo)
    expect(pointsInBox(points, axis, 150, bottom, -5, top, view, geo)).toEqual([0, 960])
    expect(pointsInBox(points, axis, 90, yOfPos(axis.pos(0.5), geo), 100, bottom, view, geo)).toEqual([])
  })

  it('adds a point on the snap step with the value of the height, and a whole tick without a step', () => {
    const y = yOfPos(axis.pos(0.5), geo)
    const snapped = newPoint(axis, view, geo, 101, y, 480)
    expect(snapped.tick).toBe(960)
    expect(snapped.value).toBeCloseTo(0.5, 1)
    expect(snapped.shape).toBe('linear')
    expect(newPoint(axis, view, geo, 101.4, y, null).tick).toBe(1014)
    expect(newPoint(axis, view, geo, -50, y, null).tick).toBe(0)
  })

  it('moves the selection by the snapped distance of the grabbed point and keeps it inside the timeline', () => {
    const sel = points.slice(0, 2)
    expect(dragDelta(sel, sel[1]!, 1500, 960, 0.5, 0.5, 480, 0)).toEqual({ ticks: 480, position: 0 })
    expect(dragDelta(sel, sel[1]!, -9000, 960, 0.5, 0.5, null, 0).ticks).toBe(0)
    expect(dragDelta(sel, sel[1]!, MAX_TICK * 2, 960, 0.5, 0.5, null, 0).ticks).toBe(MAX_TICK - 960)
    // A vertical move short of the slop is none.
    expect(dragDelta(sel, sel[0]!, 0, 0, 0.9, 0.5, null, 0).position).toBe(0)
    expect(dragDelta(sel, sel[0]!, 0, 0, 0.9, 0.5, null, 1).position).toBeCloseTo(0.4, 9)
  })

  it('keeps each value to the axis when it moves, and leaves it alone when it does not', () => {
    const sel = points.slice(0, 2)
    const up = moveSelection(sel, { ticks: 0, position: 0.5 }, axis)
    expect(up.map((p) => p.value)).toEqual([axis.at(axis.pos(0.2) + 0.5), 1])
    const still = moveSelection(sel, { ticks: 480, position: 0 }, axis)
    expect(still).toEqual([
      { tick: 480, value: 0.2, shape: 'linear' },
      { tick: 1440, value: 0.8, shape: 'linear' },
    ])
  })

  describe('plans a drag as commands', () => {
    it('sends one update when only values changed, and nothing when nothing did', () => {
      const sel = [points[1]!]
      expect(planMove(points, sel, [{ ...sel[0]!, value: 0.4 }])).toEqual({ kind: 'update', points: [{ tick: 960, value: 0.4, shape: 'linear' }] })
      expect(planMove(points, sel, [sel[0]!])).toBeNull()
      expect(planMove(points, [], [])).toBeNull()
    })
    it('removes the old ticks and adds the new points when they move in time', () => {
      const sel = [points[0]!, points[1]!]
      const next = moveSelection(sel, { ticks: 480, position: 0 }, axis)
      expect(planMove(points, sel, next)).toEqual({
        kind: 'move',
        remove: [0, 960],
        add: next.map(asPoint),
      })
    })
    it('lets a point replace the one it lands on, and the plan removes that one too', () => {
      const sel = [points[0]!]
      const plan = planMove(points, sel, [{ ...sel[0]!, tick: 960 }])
      expect(plan).toEqual({ kind: 'move', remove: [0, 960], add: [{ tick: 960, value: 0.2, shape: 'linear' }] })
    })
    it('lets a selection land on the ticks it is leaving (a shift of one point)', () => {
      const sel = [points[0]!, points[1]!]
      const next = [
        { ...sel[0]!, tick: 960 },
        { ...sel[1]!, tick: 1920 },
      ]
      const plan = planMove(points, sel, next)
      // 960 is the selection's own tick, 1920 is another point: it is replaced.
      expect(plan).toMatchObject({ kind: 'move', remove: [0, 960, 1920] })
    })
    it('previews the lane as it would be, sorted, with the replaced point gone', () => {
      const sel = [points[0]!]
      const next = [{ ...sel[0]!, tick: 960 }]
      expect(previewPoints(points, new Set([0]), next).map((p) => [p.tick, p.value])).toEqual([
        [960, 0.2],
        [1920, 0.5],
      ])
    })
  })

  it('looks only at the points near the pointer among ten thousand', () => {
    const many = pts(...Array.from({ length: 10_000 }, (_, i) => [i * 10, (i % 100) / 100] as [number, number]))
    const y = yOfPos(axis.pos(0.5), geo)
    const started = performance.now()
    for (let i = 0; i < 1000; i++) hitPoint(many, axis, 5000, y, { pxPerTick: 0.1, scrollTick: 0 }, geo)
    expect(performance.now() - started).toBeLessThan(500)
    expect(hitPoint(many, axis, 5000, yOfPos(axis.pos(0), geo), { pxPerTick: 0.1, scrollTick: 0 }, geo)?.tick).toBe(50_000)
  })
})

describe('the lane drawn before its first point (#388, #394)', () => {
  /** A context that records the path of the lane's value line: the `moveTo`/`lineTo` calls made after the row's two own strokes (its bottom line and its default value). */
  function recorder() {
    let strokes = 0
    const line: [string, number, number][] = []
    const mark = (op: string) => (x: number, y: number) => {
      if (strokes === 2) line.push([op, x, y])
    }
    const noop = () => undefined
    const ctx = new Proxy(
      {
        moveTo: mark('M'),
        lineTo: mark('L'),
        stroke: () => strokes++,
        measureText: () => ({ width: 0 }),
      },
      { get: (target, key) => (key in target ? (target as Record<string, unknown>)[key as string] : noop), set: () => true },
    ) as unknown as CanvasRenderingContext2D
    return { ctx, line }
  }
  const frame = (scrollTick: number) =>
    ({
      width: 400,
      height: 200,
      view: { pxPerTick: 0.1, scrollTick },
      totalTicks: 10_000,
      colors: { bg: '', ruler: '', line: '', barLine: '', text: '' },
      dpr: 1,
    }) as unknown as Parameters<typeof drawLane>[1]
  const scene = (points: LanePoint[]) => ({
    top: 0,
    height: 100,
    axis: axisOf(synthTrack(), volume, labels)!,
    target: volume,
    points,
    selected: new Set<number>(),
    color: '#fff',
    readout: null,
  })

  it('starts the solid line at bar 1 (the start of the timeline) at the first value, when the first point is later', () => {
    const { ctx, line } = recorder()
    drawLane(ctx, frame(0), scene(pts([1000, -12], [2000, 0])))
    // Held from the left edge (bar 1) to the first point (1000 ticks at 0.1 px per tick = 100 px) at one height.
    expect(line[0]![0]).toBe('M')
    expect(line[0]![1]).toBeLessThanOrEqual(0)
    expect(line[1]).toEqual(['L', 100, line[0]![2]])
  })

  it('starts at bar 1, not at the edge, when the view shows space before it', () => {
    const { ctx, line } = recorder()
    // Scrolled 500 ticks before bar 1: bar 1 is at x = 50, the first point at x = 150.
    drawLane(ctx, frame(-500), scene(pts([1000, -12], [2000, 0])))
    expect(line[0]).toEqual(['M', 50, line[0]![2]])
    expect(line[1]).toEqual(['L', 150, line[0]![2]])
  })

  it('draws nothing before a first point that is at tick 0, with space before bar 1 in view too', () => {
    for (const scroll of [0, -500]) {
      const { ctx, line } = recorder()
      drawLane(ctx, frame(scroll), scene(pts([0, -12], [2000, 0])))
      const start = Math.abs(scroll) * 0.1
      // The line begins on the first point and never goes left of it.
      expect(line[0]![1]).toBe(start)
      expect(line.every(([, x]) => x >= start)).toBe(true)
    }
  })

  it('draws a straight volume segment as one line and a curved one through the values the engine plays (#881)', () => {
    const axis = axisOf(synthTrack(), volume, labels)!
    const yOf = (db: number) => yOfPos(axis.pos(db), { top: 0, height: 100 })
    // Points at 0 and 2000 ticks: x = 0 and 200.
    const straight = recorder()
    drawLane(straight.ctx, frame(0), scene(pts([0, -96], [2000, 0])))
    expect(straight.line.filter(([, x]) => x > 0 && x < 200)).toEqual([])
    for (const shape of ['ease_in', 'ease_out', 'smooth'] as const) {
      const { ctx, line } = recorder()
      const points = pts([0, -96, shape], [2000, 0])
      drawLane(ctx, frame(0), scene(points))
      const inside = line.filter(([, x]) => x > 0 && x < 200)
      // About one vertex every 3 px.
      expect(inside.length, shape).toBeGreaterThan(50)
      for (const [, x, y] of inside) expect(y, `${shape} at ${x}`).toBeCloseTo(yOf(segmentValue(points[0]!, points[1]!, x / 200, volume)), 6)
      // The middle of the segment where the engine has it: below the straight line for slow then fast, above it for fast then slow.
      const mid = inside.reduce((m, p) => (Math.abs(p[1] - 100) < Math.abs(m[1] - 100) ? p : m))
      const half = yOf(valueAt(pts([0, -96], [2000, 0]), 1000, volume)!)
      if (shape === 'ease_in') expect(mid[2]).toBeGreaterThan(half)
      if (shape === 'ease_out') expect(mid[2]).toBeLessThan(half)
    }
  })

  it('draws a linear segment on a logarithmic axis through its values too, since the engine goes straight in the unit', () => {
    // The synth's release: seconds on a logarithmic scale (0.002 to 30 s).
    const release: AutomationTarget = { device: { device: 'D1', param: 'release' } }
    const axis = axisOf(synthTrack(), release, labels)!
    expect(axis.straight).toBe(false)
    expect(axisOf(synthTrack(), gainTarget, labels)!.straight).toBe(true)
    const points = pts([0, 0.01], [2000, 10])
    const { ctx, line } = recorder()
    drawLane(ctx, frame(0), { ...scene(points), axis, target: release })
    const inside = line.filter(([, x]) => x > 0 && x < 200)
    expect(inside.length).toBeGreaterThan(50)
    // Each vertex is where the engine's straight line in seconds is, on the log axis: far above the straight line on the axis.
    for (const [, x, y] of inside)
      expect(y, `at ${x}`).toBeCloseTo(yOfPos(axis.pos(segmentValue(points[0]!, points[1]!, x / 200, release)), { top: 0, height: 100 }), 6)
    const mid = inside.reduce((m, p) => (Math.abs(p[1] - 100) < Math.abs(m[1] - 100) ? p : m))
    expect(mid[2]).toBeLessThan((yOfPos(axis.pos(0.01), { top: 0, height: 100 }) + yOfPos(axis.pos(10), { top: 0, height: 100 })) / 2)
  })
})
