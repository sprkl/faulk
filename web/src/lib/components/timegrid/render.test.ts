import { describe, expect, it } from 'vitest'
import { barTicks } from '../../../views/transport/time'
import { DEFAULT_PX_PER_TICK, totalTicksOf } from './grid'
import { RULER_HEIGHT, drawGrid, drawPlayhead, type Colors, type Frame, type Surface } from './render'

const colors: Colors = { bg: '#bg', ruler: '#ruler', line: '#line', barLine: '#bar', text: '#text', accent: '#accent', play: '#play', trackShade: 0 }
const signature = { numerator: 4, denominator: 4 }

interface Recorded {
  /** [style, x] of every vertical line drawn, in order, with the style current when it was stroked. */
  lines: { style: string; x: number; from: number; to: number }[]
  strokes: number
  texts: { text: string; x: number }[]
  fills: { style: string; rect: number[] }[]
}

/** A stand-in context that records what `drawGrid` draws. */
function surface(): { ctx: Surface; out: Recorded } {
  const out: Recorded = { lines: [], strokes: 0, texts: [], fills: [] }
  let path: { x: number; from: number; to: number }[] = []
  let cursor = { x: 0, y: 0 }
  const ctx = {
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    font: '',
    textBaseline: '',
    globalAlpha: 1,
    fillRect(x: number, y: number, w: number, h: number) {
      out.fills.push({ style: String(ctx.fillStyle), rect: [x, y, w, h] })
    },
    fillText(text: string, x: number) {
      out.texts.push({ text, x })
    },
    beginPath() {
      path = []
    },
    moveTo(x: number, y: number) {
      cursor = { x, y }
    },
    lineTo(x: number, y: number) {
      if (x === cursor.x) path.push({ x, from: cursor.y, to: y })
    },
    stroke() {
      out.strokes++
      for (const line of path) out.lines.push({ style: String(ctx.strokeStyle), ...line })
    },
  }
  return { ctx: ctx as unknown as Surface, out }
}

const frame = (overrides: Partial<Frame> = {}): Frame => ({
  width: 1000,
  height: 300,
  view: { pxPerTick: DEFAULT_PX_PER_TICK, scrollTick: 0 },
  signature,
  totalTicks: totalTicksOf(200, signature),
  hover: null,
  colors,
  font: '12px mono',
  ...overrides,
})

describe('drawGrid', () => {
  it('paints the background and the ruler band first, with the token colours', () => {
    const { ctx, out } = surface()
    drawGrid(ctx, frame(), 1)
    expect(out.fills[0]).toEqual({ style: '#bg', rect: [0, 0, 1000, 300] })
    expect(out.fills[1]).toEqual({ style: '#ruler', rect: [0, 0, 1000, RULER_HEIGHT] })
  })

  it('draws bar lines at the bars, from the top, in the bar colour, on half pixels', () => {
    const { ctx, out } = surface()
    drawGrid(ctx, frame(), 1)
    const bars = out.lines.filter((l) => l.style === '#bar')
    expect(bars.map((l) => l.x)).toEqual([0.5, 384.5, 768.5])
    for (const bar of bars) expect([bar.from, bar.to]).toEqual([0, 300])
  })

  it('names the bars', () => {
    const { ctx, out } = surface()
    drawGrid(ctx, frame(), 1)
    expect(out.texts.map((t) => t.text)).toEqual(['1', '2', '3'])
  })

  it('batches the lines: a few strokes whatever the number of marks', () => {
    for (const pxPerTick of [0.01, 0.1, 1]) {
      const { ctx, out } = surface()
      drawGrid(ctx, frame({ view: { pxPerTick, scrollTick: 0 } }), 1)
      expect(out.strokes, `zoom ${pxPerTick}`).toBeLessThanOrEqual(6)
      expect(out.lines.length).toBeGreaterThan(5)
    }
  })

  it('aligns lines to device pixels at a pixel ratio of 2', () => {
    const { ctx, out } = surface()
    drawGrid(ctx, frame({ view: { pxPerTick: 0.1, scrollTick: 100 } }), 2)
    for (const line of out.lines) expect((line.x * 2) % 1, `x ${line.x}`).toBeCloseTo(0.5, 9)
  })

  it('draws the hover guide in the accent colour, and no play position (that is the overlay pass)', () => {
    const { ctx, out } = surface()
    drawGrid(ctx, frame({ hover: 960 }), 1)
    expect(out.lines.filter((l) => l.style === '#accent').map((l) => l.x)).toEqual([96.5])
    expect(out.lines.some((l) => l.style === '#play')).toBe(false)
  })

  it('dims the body past the end of the grid', () => {
    const { ctx, out } = surface()
    const total = totalTicksOf(2, signature)
    drawGrid(ctx, frame({ totalTicks: total }), 1)
    const dimmed = out.fills.filter((f) => f.rect[1] === RULER_HEIGHT)
    expect(dimmed).toHaveLength(1)
    expect(dimmed[0].rect[0]).toBe(768)
  })
})

describe('drawPlayhead', () => {
  const playFrame = (playhead: number, over: Partial<Parameters<typeof drawPlayhead>[1]> = {}) => ({
    width: 1000,
    height: 300,
    view: { pxPerTick: DEFAULT_PX_PER_TICK, scrollTick: 0 },
    playhead,
    colors,
    ...over,
  })

  it('draws one line of two CSS pixels in the play colour, over the whole height', () => {
    const { ctx, out } = surface()
    const x = drawPlayhead(ctx, playFrame(barTicks(signature)), 1)
    expect(x).toBe(384)
    expect(out.strokes).toBe(1)
    expect(out.lines).toEqual([{ style: '#play', x: 384, from: 0, to: 300 }])
    expect(ctx.lineWidth).toBe(1)
  })

  it('follows the scroll and the pixel ratio', () => {
    const { ctx, out } = surface()
    expect(drawPlayhead(ctx, playFrame(1000, { view: { pxPerTick: 0.1, scrollTick: 500 } }), 2)).toBe(50)
    expect(out.lines.map((l) => l.x)).toEqual([50])
  })

  it('draws nothing for a position outside the view', () => {
    const { ctx, out } = surface()
    expect(drawPlayhead(ctx, playFrame(barTicks(signature) * 50), 1)).toBeNull()
    expect(drawPlayhead(ctx, playFrame(-100000), 1)).toBeNull()
    expect(drawPlayhead(ctx, playFrame(Number.NaN), 1)).toBeNull()
    expect(out.strokes).toBe(0)
  })
})
